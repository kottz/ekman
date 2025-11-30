//! UI rendering.

use crate::state::{App, ExerciseState, Focus};
use chrono::Utc;
use ekman_core::models::{ActivityDay, GraphResponse};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Style, Stylize},
    symbols,
    text::{Line, Span},
    widgets::{Axis, Block, Cell, Chart, Dataset, GraphType, Paragraph, Row, Table},
};
use std::fmt::Write;

const HINTS: &str = "←/→: set cursor • Tab/Shift+Tab: navigate • ↑/↓: weight/reps • W/F: ±2.5kg • N/E: exercise • digits: edit • q: quit";

pub fn render(app: &App, frame: &mut Frame) {
    let [activity, main, status] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(4),
    ])
    .areas(frame.area());

    let [graph_area, exercise_area] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(main);

    render_activity_bar(frame, activity, &app.activity);
    render_graphs(frame, graph_area, &app.graphs);
    render_exercises(frame, exercise_area, &app.exercises, app.selected);
    render_status(frame, status, &app.status.exercise, &app.status.backend);
}

fn render_activity_bar(frame: &mut Frame, area: Rect, days: &[ActivityDay]) {
    if days.is_empty() {
        frame.render_widget(
            Paragraph::new("No activity data").block(Block::bordered().title("Activity")),
            area,
        );
        return;
    }

    let today = Utc::now().date_naive().format("%Y-%m-%d").to_string();
    let mut spans: Vec<Span> = Vec::with_capacity(days.len() * 2);

    for (idx, day) in days.iter().enumerate() {
        let color = match day.sets_completed {
            c if c >= 9 => Color::Green,
            c if c >= 1 => Color::Yellow,
            _ => Color::Red,
        };
        let mut style = Style::default().fg(color);
        if day.date == today {
            style = style.bold();
        }
        spans.push(Span::styled("●", style));
        if idx + 1 < days.len() {
            spans.push(Span::raw(" "));
        }
    }

    let content = Paragraph::new(Line::from(spans))
        .block(Block::bordered().title("Activity"))
        .alignment(Alignment::Center);

    frame.render_widget(content, area);
}

fn render_graphs(frame: &mut Frame, area: Rect, graphs: &[GraphResponse]) {
    if graphs.is_empty() {
        frame.render_widget(
            Paragraph::new("No graph data loaded").block(Block::bordered().title("Progress")),
            area,
        );
        return;
    }

    let constraints = vec![Constraint::Ratio(1, graphs.len() as u32); graphs.len()];
    let rows = Layout::vertical(constraints).split(area);

    for (graph, chunk) in graphs.iter().zip(rows.iter()) {
        render_graph(frame, *chunk, graph);
    }
}

fn render_graph(frame: &mut Frame, area: Rect, graph: &GraphResponse) {
    let data: Vec<(f64, f64)> = graph
        .points
        .iter()
        .enumerate()
        .map(|(i, p)| (i as f64, p.value))
        .collect();

    let (min_y, max_y) = data
        .iter()
        .map(|(_, v)| *v)
        .fold((f64::MAX, f64::MIN), |(min, max), v| {
            (v.min(min), v.max(max))
        });

    let (min_y, max_y) = if min_y == f64::MAX {
        (0.0, 1.0)
    } else {
        (min_y, max_y)
    };

    let padding = ((max_y - min_y) * 0.1).max(1.0);
    let y_bounds = [min_y - padding, max_y + padding];
    let x_bounds = [0.0, (data.len().saturating_sub(1) as f64).max(1.0)];

    let x_labels = match graph.points.len() {
        0 => vec!["".into(), "".into(), "".into()],
        1 => {
            let d = graph.points[0].date.clone();
            vec![d.clone(), d.clone(), d]
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
        .block(Block::bordered().title(format!("Progress • {}", graph.exercise_name)))
        .x_axis(
            Axis::default()
                .title("Sessions")
                .bounds(x_bounds)
                .labels(x_labels),
        )
        .y_axis(
            Axis::default()
                .title("Weight")
                .bounds(y_bounds)
                .labels([format!("{:.0}", y_bounds[0]), format!("{:.0}", y_bounds[1])]),
        );

    frame.render_widget(chart, area);
}

fn render_exercises(frame: &mut Frame, area: Rect, exercises: &[ExerciseState], selected: usize) {
    if exercises.is_empty() {
        frame.render_widget(
            Paragraph::new("No exercises").block(Block::bordered().title("Exercises")),
            area,
        );
        return;
    }

    let constraints = vec![Constraint::Ratio(1, exercises.len() as u32); exercises.len()];
    let rows = Layout::vertical(constraints).split(area);

    for (i, (ex, chunk)) in exercises.iter().zip(rows.iter()).enumerate() {
        render_exercise(frame, *chunk, ex, i, i == selected);
    }
}

fn render_exercise(frame: &mut Frame, area: Rect, ex: &ExerciseState, idx: usize, selected: bool) {
    let title_style = if selected {
        Style::default().bold().cyan()
    } else {
        Style::default().bold()
    };

    let block =
        Block::bordered().title(Line::from(format!("{}. {}", idx + 1, ex.name)).style(title_style));
    frame.render_widget(block.clone(), area);

    let inner = block.inner(area);

    let weight_cells: Vec<_> = ex
        .sets
        .iter()
        .enumerate()
        .map(|(i, set)| {
            let style = if selected && ex.focus == Focus::Weight && ex.set_cursor == i {
                Style::default().yellow().bold()
            } else {
                Style::default()
            };
            Cell::from(format!("{} kg", set.weight.display())).style(style)
        })
        .collect();

    let reps_cells: Vec<_> = ex
        .sets
        .iter()
        .enumerate()
        .map(|(i, set)| {
            let mut text = set.reps_display();
            if let Some(t) = set.started_at {
                let _ = write!(text, "\n{}", t.format("%H:%M:%S"));
            }
            let style = if selected && ex.focus == Focus::Reps && ex.set_cursor == i {
                Style::default().yellow().bold()
            } else {
                Style::default()
            };
            Cell::from(text).style(style)
        })
        .collect();

    let col_count = weight_cells.len().max(1);
    let widths = vec![Constraint::Ratio(1, col_count as u32); col_count];

    let table = Table::new(vec![Row::new(weight_cells), Row::new(reps_cells)], widths)
        .column_spacing(1)
        .block(Block::bordered().title(format!("Sets ({})", ex.sets.len())));

    frame.render_widget(table, inner);
}

fn render_status(frame: &mut Frame, area: Rect, exercise: &str, backend: &str) {
    let lines = vec![
        Line::from(exercise.to_string()),
        Line::from(backend.to_string()),
        Line::from(HINTS),
    ];

    let status = Paragraph::new(lines)
        .block(Block::bordered().title("Status"))
        .alignment(Alignment::Left);

    frame.render_widget(status, area);
}
