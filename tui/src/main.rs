use chrono::{DateTime, Datelike, Local};
use color_eyre::eyre::WrapErr;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ekman_core::models::{
    GraphPoint, GraphResponse, PopulatedExercise, PopulatedTemplate, SetCompact,
};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Style, Stylize},
    symbols,
    text::Line,
    widgets::{Axis, Block, Cell, Chart, Dataset, GraphType, Paragraph, Row, Table},
};
use std::{
    fmt::Write,
    time::{Duration, Instant},
};

const INPUT_RESET_TIMEOUT: Duration = Duration::from_secs(1);

const BACKEND_BASE_URL: &str = "http://localhost:3000";
const DAILY_PLANS_PATH: &str = "/api/plans/daily";

const DUMMY_EXERCISES: &[ExerciseTemplate] = &[
    ExerciseTemplate {
        name: "Back Squat",
        starting_weight: 60.0,
    },
    ExerciseTemplate {
        name: "Bench Press",
        starting_weight: 45.0,
    },
    ExerciseTemplate {
        name: "Bent Row",
        starting_weight: 40.0,
    },
];

#[derive(Debug, Clone, Copy)]
struct ExerciseTemplate {
    name: &'static str,
    starting_weight: f32,
}

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    let terminal = ratatui::init();
    let result = App::new().run(terminal);
    ratatui::restore();
    result
}

/// The main application which holds the state and logic of the application.
#[derive(Debug)]
pub struct App {
    running: bool,
    exercises: Vec<ExerciseState>,
    graphs: Vec<GraphResponse>,
    selected_exercise: usize,
    status_line: String,
    backend_status: String,
    hints_line: String,
    daily_plans: Vec<PopulatedTemplate>,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    /// Construct a new instance of [`App`].
    pub fn new() -> Self {
        let exercises = ExerciseState::defaults();
        let graphs = demo_graphs();

        let mut app = Self {
            running: false,
            exercises,
            graphs,
            selected_exercise: 0,
            status_line: String::new(),
            backend_status: String::from("Backend: not loaded"),
            hints_line: String::from(
                "Left/Right: move set cursor • Up/Down/Tab: toggle weight/reps • N: next \
                 exercise • E: previous • digits: edit weight/reps",
            ),
            daily_plans: Vec::new(),
        };
        app.refresh_status();
        app
    }

    /// Run the application's main loop.
    pub fn run(mut self, mut terminal: DefaultTerminal) -> color_eyre::Result<()> {
        self.running = true;
        self.load_daily_plans();
        while self.running {
            terminal.draw(|frame| self.render(frame))?;
            self.handle_crossterm_events()?;
        }
        Ok(())
    }

    /// Renders the user interface.
    fn render(&mut self, frame: &mut Frame) {
        let layout =
            Layout::vertical([Constraint::Min(0), Constraint::Length(4)]).split(frame.area());
        let [graph_area, exercise_area] =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                .areas(layout[0]);
        self.render_graphs(frame, graph_area);
        self.render_exercises(frame, exercise_area);
        self.render_status(frame, layout[1]);
    }

    fn render_exercises(&self, frame: &mut Frame, area: Rect) {
        if self.exercises.is_empty() {
            frame.render_widget(
                Paragraph::new("No exercises configured")
                    .block(Block::bordered().title("Exercises")),
                area,
            );
            return;
        }

        let constraints =
            vec![Constraint::Ratio(1, self.exercises.len() as u32); self.exercises.len()];
        let rows = Layout::vertical(constraints).split(area);
        for (idx, (exercise, chunk)) in self.exercises.iter().zip(rows.iter()).enumerate() {
            self.render_exercise(frame, *chunk, idx, exercise);
        }
    }

    fn render_exercise(&self, frame: &mut Frame, area: Rect, idx: usize, exercise: &ExerciseState) {
        let selected = idx == self.selected_exercise;
        let title_style = if selected {
            Style::default().bold().cyan()
        } else {
            Style::default().bold()
        };
        let block = Block::bordered()
            .title(Line::from(format!("{}. {}", idx + 1, exercise.name)).style(title_style));
        frame.render_widget(block.clone(), area);
        let inner = block.inner(area);

        let weight_cells = exercise
            .sets
            .iter()
            .enumerate()
            .map(|(set_idx, set)| {
                let mut style = Style::default();
                if selected
                    && matches!(exercise.focus, InputFocus::SetWeight)
                    && exercise.set_cursor == set_idx
                {
                    style = Style::default().yellow().bold();
                }
                let cell_text = format!("{} kg", set.weight.display_or_placeholder());
                Cell::from(cell_text).style(style)
            })
            .collect::<Vec<_>>();

        let reps_cells = exercise
            .sets
            .iter()
            .enumerate()
            .map(|(set_idx, set)| {
                let mut cell_text = set.reps_display();
                if let Some(started_at) = set.started_at {
                    let _ = write!(cell_text, "\n{}", started_at.format("%H:%M:%S"));
                }

                let mut style = Style::default();
                if selected
                    && matches!(exercise.focus, InputFocus::SetReps)
                    && exercise.set_cursor == set_idx
                {
                    style = Style::default().yellow().bold();
                }
                Cell::from(cell_text).style(style)
            })
            .collect::<Vec<_>>();

        let col_count = weight_cells.len().max(1);
        let col_width = (100 / col_count) as u16;
        let widths = vec![Constraint::Percentage(col_width); col_count];
        let sets = Table::new(vec![Row::new(weight_cells), Row::new(reps_cells)], widths)
            .column_spacing(1)
            .block(Block::bordered().title(format!("Sets ({})", exercise.sets.len().max(1))));
        frame.render_widget(sets, inner);
    }

    fn render_graphs(&self, frame: &mut Frame, area: Rect) {
        if self.graphs.is_empty() {
            frame.render_widget(
                Paragraph::new("No graph data loaded").block(Block::bordered().title("Progress")),
                area,
            );
            return;
        }

        let constraints = vec![Constraint::Ratio(1, self.graphs.len() as u32); self.graphs.len()];
        let rows = Layout::vertical(constraints).split(area);
        for (graph, chunk) in self.graphs.iter().zip(rows.iter()) {
            self.render_graph(frame, *chunk, graph);
        }
    }

    fn render_graph(&self, frame: &mut Frame, area: Rect, graph: &GraphResponse) {
        let data: Vec<(f64, f64)> = graph
            .points
            .iter()
            .enumerate()
            .map(|(idx, point)| (idx as f64, point.value))
            .collect();

        let (min_y, max_y) = data
            .iter()
            .map(|(_, value)| *value)
            .fold((f64::MAX, f64::MIN), |(min, max), val| {
                (val.min(min), val.max(max))
            });
        let (min_y, max_y) = if min_y == f64::MAX {
            (0.0, 1.0)
        } else {
            (min_y, max_y)
        };
        let y_padding = ((max_y - min_y) * 0.1).max(1.0);
        let y_bounds = [min_y - y_padding, max_y + y_padding];

        let x_bounds = if data.is_empty() {
            [0.0, 1.0]
        } else {
            [0.0, (data.len().saturating_sub(1) as f64).max(1.0)]
        };

        let labels = match graph.points.len() {
            0 => vec!["".to_string(), "".to_string(), "".to_string()],
            1 => {
                let label = graph.points[0].date.clone();
                vec![label.clone(), label.clone(), label]
            }
            len => {
                let mid = len / 2;
                vec![
                    graph
                        .points
                        .first()
                        .map(|p| p.date.clone())
                        .unwrap_or_default(),
                    graph
                        .points
                        .get(mid)
                        .map(|p| p.date.clone())
                        .unwrap_or_default(),
                    graph
                        .points
                        .last()
                        .map(|p| p.date.clone())
                        .unwrap_or_default(),
                ]
            }
        };

        let dataset = Dataset::default()
            .name(graph.exercise_name.clone())
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().cyan())
            .data(&data);

        let chart = Chart::new(vec![dataset])
            .block(
                Block::bordered().title(Line::from(format!("Progress • {}", graph.exercise_name))),
            )
            .x_axis(
                Axis::default()
                    .title("Sessions")
                    .bounds(x_bounds)
                    .labels(labels),
            )
            .y_axis(
                Axis::default()
                    .title("Weight / volume")
                    .bounds(y_bounds)
                    .labels([format!("{:.0}", y_bounds[0]), format!("{:.0}", y_bounds[1])]),
            );

        frame.render_widget(chart, area);
    }

    fn render_status(&self, frame: &mut Frame, area: Rect) {
        let lines = vec![
            Line::from(self.status_line.clone()),
            Line::from(self.backend_status.clone()),
            Line::from(self.hints_line.clone()),
        ];
        let status = Paragraph::new(lines)
            .block(Block::bordered().title("Status"))
            .alignment(Alignment::Left);
        frame.render_widget(status, area);
    }

    /// Reads the crossterm events and updates the state of [`App`].
    fn handle_crossterm_events(&mut self) -> color_eyre::Result<()> {
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => self.on_key_event(key),
            Event::Mouse(_) => {}
            Event::Resize(_, _) => {}
            _ => {}
        }
        Ok(())
    }

    /// Handles the key events and updates the state of [`App`].
    fn on_key_event(&mut self, key: KeyEvent) {
        if let Some(command) = Command::from_key(key) {
            self.apply_command(command);
        }
    }

    fn apply_command(&mut self, command: Command) {
        match command {
            Command::Quit => self.quit(),
            Command::NextListRow => self.move_exercise(1),
            Command::PrevListRow => self.move_exercise(-1),
            Command::ToggleFocus => self.current_exercise_mut().toggle_focus(),
            Command::NextField => self.current_exercise_mut().focus_down(),
            Command::PrevField => self.current_exercise_mut().focus_up(),
            Command::MoveLeft => self.on_move_left(),
            Command::MoveRight => self.on_move_right(),
            Command::Digit(char) => self.apply_digit(char),
            Command::Backspace => self.apply_backspace(),
        }
        self.refresh_status();
    }

    fn on_move_left(&mut self) {
        let exercise = self.current_exercise_mut();
        exercise.move_set_cursor(-1);
    }

    fn on_move_right(&mut self) {
        let exercise = self.current_exercise_mut();
        exercise.move_set_cursor(1);
    }

    fn apply_digit(&mut self, ch: char) {
        if self.exercises.is_empty() {
            return;
        }

        let focus = self
            .exercises
            .get(self.selected_exercise)
            .map(|ex| ex.focus)
            .unwrap_or(InputFocus::SetWeight);

        match focus {
            InputFocus::SetWeight => {
                self.current_exercise_mut().push_set_weight_char(ch);
            }
            InputFocus::SetReps => {
                if self
                    .exercises
                    .get(self.selected_exercise)
                    .is_some_and(ExerciseState::should_auto_advance_set)
                {
                    self.advance_set_cursor();
                }
                let should_advance = self.current_exercise_mut().push_set_reps_char(ch);
                if should_advance {
                    self.advance_set_cursor();
                }
            }
        }
        self.refresh_status();
    }

    fn apply_backspace(&mut self) {
        let exercise = self.current_exercise_mut();
        match exercise.focus {
            InputFocus::SetWeight => exercise.backspace_set_weight(),
            InputFocus::SetReps => exercise.backspace_set_reps(),
        }
    }

    fn move_exercise(&mut self, delta: i32) {
        if self.exercises.is_empty() {
            return;
        }
        let len = self.exercises.len() as i32;
        let mut next = self.selected_exercise as i32 + delta;
        if next < 0 {
            next = 0;
        } else if next >= len {
            next = len - 1;
        }
        self.selected_exercise = next as usize;
        if let Some(ex) = self.exercises.get_mut(self.selected_exercise) {
            ex.reset_set_timer();
        }
    }

    /// Set running to false to quit the application.
    fn quit(&mut self) {
        self.running = false;
    }

    fn load_daily_plans(&mut self) {
        match fetch_daily_plans() {
            Ok(plans) => {
                if let Some(plan) = select_plan_for_today(&plans) {
                    let exercises: Vec<_> = plan
                        .exercises
                        .iter()
                        .map(ExerciseState::from_populated_exercise)
                        .collect();
                    if exercises.is_empty() {
                        self.backend_status = "Backend: no exercises found in plans".to_string();
                        self.exercises = ExerciseState::defaults();
                        self.daily_plans.clear();
                    } else {
                        self.backend_status = format!(
                            "Backend: loaded {} plans (showing {})",
                            plans.len(),
                            plan.name
                        );
                        self.daily_plans = plans;
                        self.exercises = exercises;
                        self.selected_exercise = 0;
                    }
                } else {
                    self.backend_status = "Backend: no plans available".to_string();
                    self.exercises = ExerciseState::defaults();
                    self.daily_plans.clear();
                }
            }
            Err(error) => {
                self.backend_status = format!("Backend unavailable: {error}");
                self.exercises = ExerciseState::defaults();
                self.selected_exercise = 0;
                self.daily_plans.clear();
            }
        }
        self.refresh_status();
    }

    fn refresh_status(&mut self) {
        if let Some(exercise) = self.exercises.get(self.selected_exercise) {
            let total_sets = exercise.sets.len().max(1);
            let current_set = (exercise.set_cursor + 1).min(total_sets);
            self.status_line = format!(
                "Selected: {} • Focus: {} • Set {}/{}",
                exercise.name,
                exercise.focus.label(),
                current_set,
                total_sets
            );
        } else {
            self.status_line.clear();
        }
    }

    fn current_exercise_mut(&mut self) -> &mut ExerciseState {
        if self.exercises.is_empty() {
            panic!("no exercises available");
        }
        let idx = self
            .selected_exercise
            .min(self.exercises.len().saturating_sub(1));
        self.selected_exercise = idx;
        self.exercises
            .get_mut(idx)
            .expect("selected exercise should exist")
    }

    fn advance_set_cursor(&mut self) {
        if self.exercises.is_empty() {
            return;
        }
        let current_idx = self.selected_exercise;
        let mut moved = false;
        if let Some(exercise) = self.exercises.get_mut(current_idx) {
            exercise.reset_set_timer();
            if exercise.set_cursor + 1 < exercise.sets.len() {
                exercise.set_cursor += 1;
                moved = true;
            }
        }
        if moved {
            return;
        }
        if current_idx + 1 < self.exercises.len() {
            self.selected_exercise += 1;
            if let Some(next) = self.exercises.get_mut(self.selected_exercise) {
                next.focus = InputFocus::SetWeight;
                next.set_cursor = 0;
                next.reset_set_timer();
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Command {
    Quit,
    NextListRow,
    PrevListRow,
    ToggleFocus,
    NextField,
    PrevField,
    MoveLeft,
    MoveRight,
    Digit(char),
    Backspace,
}

impl Command {
    fn from_key(key: KeyEvent) -> Option<Self> {
        match (key.modifiers, key.code) {
            (_, KeyCode::Esc | KeyCode::Char('q'))
            | (KeyModifiers::CONTROL, KeyCode::Char('c') | KeyCode::Char('C')) => Some(Self::Quit),
            (_, KeyCode::Char('n') | KeyCode::Char('N')) => Some(Self::NextListRow),
            (_, KeyCode::Char('e') | KeyCode::Char('E')) => Some(Self::PrevListRow),
            (_, KeyCode::Down) => Some(Self::NextField),
            (_, KeyCode::Up) => Some(Self::PrevField),
            (_, KeyCode::Left) => Some(Self::MoveLeft),
            (_, KeyCode::Right) => Some(Self::MoveRight),
            (_, KeyCode::Tab | KeyCode::BackTab) => Some(Self::ToggleFocus),
            (_, KeyCode::Backspace) => Some(Self::Backspace),
            (_, KeyCode::Char(ch)) if ch.is_ascii_digit() => Some(Self::Digit(ch)),
            (_, KeyCode::Char('.')) => Some(Self::Digit('.')),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
struct ExerciseState {
    name: String,
    focus: InputFocus,
    sets: Vec<SetEntry>,
    set_cursor: usize,
    last_set_input: Option<Instant>,
}

impl ExerciseState {
    fn defaults() -> Vec<Self> {
        DUMMY_EXERCISES
            .iter()
            .map(ExerciseState::from_template)
            .collect()
    }

    fn from_template(template: &ExerciseTemplate) -> Self {
        Self::with_set_slots(template.name.to_string(), template.starting_weight, 4, &[])
    }

    fn from_populated_exercise(exercise: &PopulatedExercise) -> Self {
        let set_count = exercise.target_sets.unwrap_or(4).max(1) as usize;
        let starting_weight = exercise
            .last_session_sets
            .first()
            .map(|set| set.weight as f32)
            .unwrap_or(0.0);
        Self::with_set_slots(
            exercise.name.clone(),
            starting_weight,
            set_count,
            &exercise.last_session_sets,
        )
    }

    fn with_set_slots(
        name: String,
        starting_weight: f32,
        set_count: usize,
        previous_sets: &[SetCompact],
    ) -> Self {
        let slots = set_count.max(1);
        let mut sets = Vec::with_capacity(slots);

        for idx in 0..slots {
            let reps = previous_sets.get(idx).map(|set| set.reps.max(0) as u32);
            let weight_value = previous_sets
                .get(idx)
                .map(|set| set.weight as f32)
                .unwrap_or(starting_weight);
            let prefill_weight = previous_sets.get(idx).is_some();
            sets.push(SetEntry::new(reps, weight_value, prefill_weight));
        }

        Self {
            name,
            focus: InputFocus::SetWeight,
            sets,
            set_cursor: 0,
            last_set_input: None,
        }
    }

    fn toggle_focus(&mut self) {
        self.focus = self.focus.next();
    }

    fn focus_down(&mut self) {
        self.focus = self.focus.next();
    }

    fn focus_up(&mut self) {
        self.focus = self.focus.prev();
    }

    fn move_set_cursor(&mut self, delta: i32) {
        if self.sets.is_empty() {
            return;
        }
        let len = self.sets.len() as i32;
        let next = (self.set_cursor as i32 + delta).clamp(0, len - 1);
        self.set_cursor = next as usize;
        self.reset_set_timer();
    }

    fn push_set_weight_char(&mut self, ch: char) {
        if !(ch.is_ascii_digit() || ch == '.') {
            return;
        }
        if self.set_cursor >= self.sets.len() {
            return;
        }
        let idx = self.set_cursor;
        self.update_set_weight(idx, |weight| weight.push_char(ch));
    }

    fn backspace_set_weight(&mut self) {
        if self.set_cursor >= self.sets.len() {
            return;
        }
        let idx = self.set_cursor;
        self.update_set_weight(idx, WeightEntry::backspace);
    }

    fn update_set_weight<F>(&mut self, idx: usize, mut update: F)
    where
        F: FnMut(&mut WeightEntry),
    {
        if let Some(entry) = self.sets.get_mut(idx) {
            update(&mut entry.weight);
            let new_weight = entry.weight.value;
            self.propagate_weight_to_open_sets(new_weight, idx);
        }
    }

    fn propagate_weight_to_open_sets(&mut self, weight: f32, origin_idx: usize) {
        for (idx, set) in self.sets.iter_mut().enumerate() {
            if idx == origin_idx {
                continue;
            }
            if set.reps.is_none() {
                set.weight.set_value(weight);
            }
        }
    }

    fn push_set_reps_char(&mut self, ch: char) -> bool {
        if !ch.is_ascii_digit() {
            return false;
        }
        if self.set_cursor >= self.sets.len() {
            return false;
        }
        let idx = self.set_cursor;
        let Some(set) = self.sets.get_mut(idx) else {
            return false;
        };
        let buffer = &mut set.reps_input;
        let now = Instant::now();
        let should_reset = self
            .last_set_input
            .is_none_or(|last| now.duration_since(last) > INPUT_RESET_TIMEOUT);
        if should_reset {
            buffer.clear();
        }
        if buffer.is_empty() && ch > '2' {
            buffer.clear();
            buffer.push(ch);
            let value = buffer.parse::<u32>().ok();
            self.apply_reps_value(idx, value);
            self.last_set_input = Some(now);
            return true;
        }
        buffer.push(ch);
        let value = buffer.parse::<u32>().ok();
        self.apply_reps_value(idx, value);
        self.last_set_input = Some(now);
        false
    }

    fn backspace_set_reps(&mut self) {
        if self.set_cursor >= self.sets.len() {
            return;
        }
        let idx = self.set_cursor;
        let Some(set) = self.sets.get_mut(idx) else {
            return;
        };
        let buffer = &mut set.reps_input;
        if buffer.pop().is_none() {
            self.apply_reps_value(idx, None);
            return;
        }
        let value = if buffer.is_empty() {
            None
        } else {
            buffer.parse::<u32>().ok()
        };
        self.apply_reps_value(idx, value);
    }

    fn apply_reps_value(&mut self, idx: usize, value: Option<u32>) {
        if let Some(entry) = self.sets.get_mut(idx) {
            match value {
                Some(v) => {
                    if entry.reps.is_none() {
                        entry.started_at = Some(Local::now());
                    }
                    entry.reps = Some(v);
                    entry.reps_input = v.to_string();
                }
                None => {
                    entry.reps = None;
                    entry.started_at = None;
                    entry.reps_input.clear();
                }
            }
        }
    }

    fn should_auto_advance_set(&self) -> bool {
        self.last_set_input
            .is_some_and(|last| last.elapsed() > INPUT_RESET_TIMEOUT)
    }

    fn reset_set_timer(&mut self) {
        self.last_set_input = None;
    }
}

#[derive(Debug, Clone)]
struct WeightEntry {
    value: f32,
    buffer: String,
    last_input: Option<Instant>,
}

impl WeightEntry {
    fn new(value: f32) -> Self {
        Self {
            value,
            buffer: format!("{value:.1}"),
            last_input: None,
        }
    }

    fn new_unset(value: f32) -> Self {
        Self {
            value,
            buffer: String::new(),
            last_input: None,
        }
    }

    fn set_value(&mut self, value: f32) {
        self.value = (value * 10.0).round() / 10.0;
        self.buffer = format!("{:.1}", self.value);
        self.last_input = None;
    }

    fn display_or_placeholder(&self) -> String {
        if self.buffer.is_empty() {
            "__".to_string()
        } else {
            self.display_value()
        }
    }

    fn display_value(&self) -> String {
        if self.buffer.is_empty() {
            format!("{:.1}", self.value)
        } else {
            self.buffer.clone()
        }
    }

    fn push_char(&mut self, ch: char) {
        if !(ch.is_ascii_digit() || ch == '.') {
            return;
        }
        let now = Instant::now();
        let should_reset = self
            .last_input
            .is_none_or(|last| now.duration_since(last) > INPUT_RESET_TIMEOUT);
        if should_reset {
            self.buffer.clear();
        }
        self.buffer.push(ch);
        if let Ok(parsed) = self.buffer.parse::<f32>() {
            self.value = parsed;
        }
        self.last_input = Some(now);
    }

    fn backspace(&mut self) {
        self.buffer.pop();
        self.value = self.buffer.parse::<f32>().unwrap_or(0.0);
    }
}

#[derive(Debug, Clone)]
struct SetEntry {
    reps: Option<u32>,
    reps_input: String,
    weight: WeightEntry,
    started_at: Option<DateTime<Local>>,
}

impl SetEntry {
    fn new(reps: Option<u32>, weight: f32, prefill_weight: bool) -> Self {
        Self {
            reps,
            reps_input: reps.map(|val| val.to_string()).unwrap_or_default(),
            weight: if prefill_weight {
                WeightEntry::new(weight)
            } else {
                WeightEntry::new_unset(weight)
            },
            started_at: None,
        }
    }

    fn reps_display(&self) -> String {
        self.reps
            .map(|val| val.to_string())
            .unwrap_or_else(|| "__".to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputFocus {
    SetWeight,
    SetReps,
}

impl InputFocus {
    fn label(self) -> &'static str {
        match self {
            InputFocus::SetWeight => "Set Weight",
            InputFocus::SetReps => "Set Reps",
        }
    }

    fn next(self) -> Self {
        match self {
            InputFocus::SetWeight => InputFocus::SetReps,
            InputFocus::SetReps => InputFocus::SetWeight,
        }
    }

    fn prev(self) -> Self {
        match self {
            InputFocus::SetWeight => InputFocus::SetReps,
            InputFocus::SetReps => InputFocus::SetWeight,
        }
    }
}

fn select_plan_for_today(plans: &[PopulatedTemplate]) -> Option<&PopulatedTemplate> {
    if plans.is_empty() {
        return None;
    }
    let today = Local::now().weekday().num_days_from_monday() as i32;
    plans
        .iter()
        .find(|plan| plan.day_of_week == Some(today))
        .or_else(|| plans.first())
}

/// Fetch daily workout plans from the backend API.
fn fetch_daily_plans() -> color_eyre::Result<Vec<PopulatedTemplate>> {
    let runtime = tokio::runtime::Runtime::new().wrap_err("failed to start async runtime")?;
    runtime.block_on(async {
        let client = reqwest::Client::new();
        client
            .get(format!("{BACKEND_BASE_URL}{DAILY_PLANS_PATH}"))
            .send()
            .await
            .wrap_err("request to backend failed")?
            .error_for_status()
            .wrap_err("backend returned an error status")?
            .json()
            .await
            .wrap_err("failed to parse backend response")
    })
}

fn demo_graphs() -> Vec<GraphResponse> {
    vec![
        GraphResponse {
            exercise_id: 1,
            exercise_name: "Back Squat".to_string(),
            points: vec![
                GraphPoint {
                    date: "2024-09-01".to_string(),
                    value: 60.0,
                },
                GraphPoint {
                    date: "2024-09-08".to_string(),
                    value: 65.0,
                },
                GraphPoint {
                    date: "2024-09-15".to_string(),
                    value: 67.5,
                },
                GraphPoint {
                    date: "2024-09-22".to_string(),
                    value: 70.0,
                },
                GraphPoint {
                    date: "2024-09-29".to_string(),
                    value: 72.5,
                },
            ],
        },
        GraphResponse {
            exercise_id: 2,
            exercise_name: "Bench Press".to_string(),
            points: vec![
                GraphPoint {
                    date: "2024-09-01".to_string(),
                    value: 45.0,
                },
                GraphPoint {
                    date: "2024-09-08".to_string(),
                    value: 47.5,
                },
                GraphPoint {
                    date: "2024-09-15".to_string(),
                    value: 50.0,
                },
                GraphPoint {
                    date: "2024-09-22".to_string(),
                    value: 52.5,
                },
                GraphPoint {
                    date: "2024-09-29".to_string(),
                    value: 55.0,
                },
            ],
        },
    ]
}
