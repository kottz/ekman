use chrono::{DateTime, Local};
use color_eyre::eyre::WrapErr;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ekman_core::models::PopulatedTemplate;
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Style, Stylize},
    text::Line,
    widgets::{Block, Cell, Paragraph, Row, Table},
};
use std::fmt::Write;

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
        let exercises = DUMMY_EXERCISES
            .iter()
            .map(ExerciseState::from_template)
            .collect();

        let mut app = Self {
            running: false,
            exercises,
            selected_exercise: 0,
            status_line: String::new(),
            backend_status: String::from("Backend: not loaded"),
            hints_line: String::from(
                "Left/Right: adjust weight or move set cursor • Up/Down: move between weight/sets \
                 • Tab: toggle focus • N: next exercise • E: previous • digits: edit fields",
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
        self.render_exercises(frame, layout[0]);
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

        let inner_layout =
            Layout::vertical([Constraint::Length(3), Constraint::Length(5)]).split(inner);

        let weight_style = if selected && matches!(exercise.focus, InputFocus::Weight) {
            Style::default().cyan().bold()
        } else {
            Style::default()
        };
        let weight_display = exercise.weight.display_value();
        let weight = Paragraph::new(format!("Weight: {weight_display} kg"))
            .style(weight_style)
            .block(Block::bordered().title("Load"));
        frame.render_widget(weight, inner_layout[0]);

        let set_cells = exercise
            .sets
            .iter()
            .enumerate()
            .map(|(set_idx, set)| {
                let mut cell_text = String::new();
                if let Some(value) = set.value {
                    let _ = write!(cell_text, "{value}");
                } else {
                    cell_text.push_str("--");
                }
                if let Some(started_at) = set.started_at {
                    let _ = write!(cell_text, "\n{}", started_at.format("%H:%M:%S"));
                }

                let mut style = Style::default();
                if selected
                    && matches!(exercise.focus, InputFocus::Sets)
                    && exercise.set_cursor == set_idx
                {
                    style = Style::default().yellow().bold();
                }
                Cell::from(cell_text).style(style)
            })
            .collect::<Vec<_>>();

        let widths = vec![Constraint::Percentage(25); 4];
        let sets = Table::new(vec![Row::new(set_cells)], widths)
            .column_spacing(1)
            .block(Block::bordered().title("Sets (4)"));
        frame.render_widget(sets, inner_layout[1]);
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
        match exercise.focus {
            InputFocus::Weight => exercise.bump_weight(-2.5),
            InputFocus::Sets => exercise.move_set_cursor(-1),
        }
    }

    fn on_move_right(&mut self) {
        let exercise = self.current_exercise_mut();
        match exercise.focus {
            InputFocus::Weight => exercise.bump_weight(2.5),
            InputFocus::Sets => exercise.move_set_cursor(1),
        }
    }

    fn apply_digit(&mut self, ch: char) {
        let exercise = self.current_exercise_mut();
        match exercise.focus {
            InputFocus::Weight => exercise.push_weight_char(ch),
            InputFocus::Sets => exercise.push_set_char(ch),
        }
    }

    fn apply_backspace(&mut self) {
        let exercise = self.current_exercise_mut();
        match exercise.focus {
            InputFocus::Weight => exercise.backspace_weight(),
            InputFocus::Sets => exercise.backspace_set(),
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
    }

    /// Set running to false to quit the application.
    fn quit(&mut self) {
        self.running = false;
    }

    fn load_daily_plans(&mut self) {
        match fetch_daily_plans() {
            Ok(plans) => {
                self.backend_status = format!("Backend: loaded {} plans", plans.len());
                self.daily_plans = plans;
            }
            Err(error) => {
                self.backend_status = format!("Backend unavailable: {error}");
            }
        }
    }

    fn refresh_status(&mut self) {
        if let Some(exercise) = self.exercises.get(self.selected_exercise) {
            self.status_line = format!(
                "Selected: {} • Focus: {} • Set {}/4",
                exercise.name,
                exercise.focus.label(),
                exercise.set_cursor + 1
            );
        } else {
            self.status_line.clear();
        }
    }

    fn current_exercise_mut(&mut self) -> &mut ExerciseState {
        self.exercises
            .get_mut(self.selected_exercise)
            .expect("selected exercise should exist")
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
    weight: WeightEntry,
    focus: InputFocus,
    sets: [SetEntry; 4],
    set_inputs: [String; 4],
    set_cursor: usize,
}

impl ExerciseState {
    fn from_template(template: &ExerciseTemplate) -> Self {
        Self {
            name: template.name.to_string(),
            weight: WeightEntry::new(template.starting_weight),
            focus: InputFocus::Weight,
            sets: std::array::from_fn(|_| SetEntry::default()),
            set_inputs: std::array::from_fn(|_| String::new()),
            set_cursor: 0,
        }
    }

    fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            InputFocus::Weight => InputFocus::Sets,
            InputFocus::Sets => InputFocus::Weight,
        };
    }

    fn bump_weight(&mut self, delta: f32) {
        self.weight.bump(delta);
    }

    fn focus_down(&mut self) {
        self.focus = InputFocus::Sets;
    }

    fn focus_up(&mut self) {
        self.focus = InputFocus::Weight;
    }

    fn push_weight_char(&mut self, ch: char) {
        self.weight.push_char(ch);
    }

    fn backspace_weight(&mut self) {
        self.weight.backspace();
    }

    fn move_set_cursor(&mut self, delta: i32) {
        let len = self.sets.len() as i32;
        let next = (self.set_cursor as i32 + delta).clamp(0, len - 1);
        self.set_cursor = next as usize;
    }

    fn push_set_char(&mut self, ch: char) {
        if !ch.is_ascii_digit() {
            return;
        }
        let idx = self.set_cursor;
        let buffer = &mut self.set_inputs[idx];
        buffer.push(ch);
        let value = buffer.parse::<u32>().ok();
        self.apply_set_value(idx, value);
    }

    fn backspace_set(&mut self) {
        let idx = self.set_cursor;
        let buffer = &mut self.set_inputs[idx];
        if buffer.pop().is_none() {
            self.apply_set_value(idx, None);
            return;
        }
        let value = if buffer.is_empty() {
            None
        } else {
            buffer.parse::<u32>().ok()
        };
        self.apply_set_value(idx, value);
    }

    fn apply_set_value(&mut self, idx: usize, value: Option<u32>) {
        let entry = &mut self.sets[idx];
        match value {
            Some(v) => {
                if entry.value.is_none() {
                    entry.started_at = Some(Local::now());
                }
                entry.value = Some(v);
                self.set_inputs[idx] = v.to_string();
            }
            None => {
                entry.value = None;
                entry.started_at = None;
                self.set_inputs[idx].clear();
            }
        }
    }
}

#[derive(Debug, Clone)]
struct WeightEntry {
    value: f32,
    buffer: String,
}

impl WeightEntry {
    fn new(value: f32) -> Self {
        Self {
            value,
            buffer: format!("{value:.1}"),
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
        self.buffer.push(ch);
        if let Ok(parsed) = self.buffer.parse::<f32>() {
            self.value = parsed;
        }
    }

    fn backspace(&mut self) {
        self.buffer.pop();
        self.value = self.buffer.parse::<f32>().unwrap_or(0.0);
    }

    fn bump(&mut self, delta: f32) {
        let next = (self.value + delta).max(0.0);
        self.value = (next * 10.0).round() / 10.0;
        self.buffer = format!("{:.1}", self.value);
    }
}

#[derive(Debug, Default, Clone)]
struct SetEntry {
    value: Option<u32>,
    started_at: Option<DateTime<Local>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputFocus {
    Weight,
    Sets,
}

impl InputFocus {
    fn label(self) -> &'static str {
        match self {
            InputFocus::Weight => "Weight",
            InputFocus::Sets => "Sets",
        }
    }
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
