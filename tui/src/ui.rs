//! UI rendering.

use crate::state::{App, AuthField, AuthMode, ExerciseState, Focus};
use chrono::Utc;
use ekman_core::models::{ActivityDay, GraphResponse};
use qrcode::QrCode;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    symbols,
    text::{Line, Span},
    widgets::{Axis, Block, Cell, Chart, Dataset, GraphType, Paragraph, Row, Table},
};
use std::fmt::Write;
use tui_qrcode::{Colors, QrCodeWidget};

const HINTS: &str = "←/→: set cursor • Tab/Shift+Tab: navigate • ↑/↓: weight/reps • W/F: ±2.5kg • N/E: exercise • digits: edit • q: quit";

pub fn render(app: &App, frame: &mut Frame) {
    if !app.is_authenticated() {
        render_auth(app, frame);
        return;
    }

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

fn render_auth(app: &App, frame: &mut Frame) {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(12),
        Constraint::Length(3),
    ])
    .areas(frame.area());

    render_auth_header(frame, header, app);
    render_auth_body(frame, body, app);
    render_auth_footer(frame, footer, app);
}

fn render_auth_header(frame: &mut Frame, area: Rect, app: &App) {
    let title = match app.auth.mode {
        AuthMode::Login => "Login",
        AuthMode::Register => "Register",
    };

    let tabs = Line::from(vec![
        Span::styled(
            " Login ",
            if app.auth.mode == AuthMode::Login {
                Style::default().bold().green()
            } else {
                Style::default().dim()
            },
        ),
        Span::raw(" • "),
        Span::styled(
            " Register ",
            if app.auth.mode == AuthMode::Register {
                Style::default().bold().cyan()
            } else {
                Style::default().dim()
            },
        ),
    ]);

    let block = Block::bordered()
        .title(format!("Ekman • {title}"))
        .title_alignment(Alignment::Center);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(tabs).alignment(Alignment::Center), inner);
}

fn render_auth_body(frame: &mut Frame, area: Rect, app: &App) {
    let columns: [Rect; 2] = Layout::horizontal([
        Constraint::Percentage(if app.auth.mode == AuthMode::Register {
            55
        } else {
            70
        }),
        Constraint::Percentage(if app.auth.mode == AuthMode::Register {
            45
        } else {
            30
        }),
    ])
    .areas(area);

    render_auth_form(frame, columns[0], app);

    if app.auth.mode == AuthMode::Register {
        render_auth_qr(frame, columns[1], app);
    } else {
        render_auth_help(frame, columns[1]);
    }
}

fn render_auth_form(frame: &mut Frame, area: Rect, app: &App) {
    let auth = &app.auth;
    let mut lines = Vec::new();

    lines.push(field_line(
        "Username",
        &auth.username,
        auth.focus == AuthField::Username,
        false,
    ));
    lines.push(field_line(
        "Password",
        &auth.password,
        auth.focus == AuthField::Password,
        true,
    ));
    lines.push(field_line(
        "TOTP code",
        &auth.totp_code,
        auth.focus == AuthField::Totp,
        false,
    ));

    if auth.mode == AuthMode::Register {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("Secret: ", Style::default().bold()),
            Span::raw(auth.totp_secret.clone()),
        ]));
        lines.push(Line::from(vec![
            Span::styled("otpauth:// ", Style::default().bold()),
            Span::raw(auth.otpauth_url.clone()),
        ]));
    }

    let block = Block::bordered().title(match auth.mode {
        AuthMode::Login => "Enter credentials",
        AuthMode::Register => "Create account (2FA required)",
    });

    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .alignment(Alignment::Left),
        area,
    );
}

fn render_auth_qr(frame: &mut Frame, area: Rect, app: &App) {
    let chunks: [Rect; 2] =
        Layout::vertical([Constraint::Min(10), Constraint::Length(5)]).areas(area);
    let qr_block = Block::bordered().title("Scan QR in your authenticator");
    let qr_inner = qr_block.inner(chunks[0]);
    frame.render_widget(qr_block, chunks[0]);

    match QrCode::new(app.auth.otpauth_url.as_bytes()) {
        Ok(code) => {
            let widget = QrCodeWidget::new(code).colors(Colors::Inverted);
            frame.render_widget(widget, qr_inner);
        }
        Err(_) => frame.render_widget(
            Paragraph::new("Unable to render QR").alignment(Alignment::Center),
            qr_inner,
        ),
    }

    let info = Paragraph::new(vec![
        Line::from("1) Scan the QR code."),
        Line::from("2) Enter the current 6-digit code from your app."),
        Line::from("Ctrl+G to generate a new secret."),
    ])
    .block(Block::bordered().title("Instructions"));

    frame.render_widget(info, chunks[1]);
}

fn render_auth_help(frame: &mut Frame, area: Rect) {
    let help = Paragraph::new(vec![
        Line::from("Enter username, password, and 6-digit TOTP code."),
        Line::from("Ctrl+R to jump to register."),
        Line::from("Tab/Shift+Tab to move fields, Enter to submit."),
    ])
    .block(Block::bordered().title("Help"));

    frame.render_widget(help, area);
}

fn render_auth_footer(frame: &mut Frame, area: Rect, app: &App) {
    let status = if !app.auth.status.is_empty() {
        app.auth.status.clone()
    } else {
        "Ctrl+L login • Ctrl+R register • Ctrl+G new secret • Esc quits".into()
    };
    frame.render_widget(
        Paragraph::new(status)
            .alignment(Alignment::Center)
            .block(Block::bordered()),
        area,
    );
}

fn field_line(label: &str, value: &str, focused: bool, mask: bool) -> Line<'static> {
    let mut display = if mask {
        "•".repeat(value.chars().count())
    } else {
        value.to_string()
    };
    if display.is_empty() {
        display = "_".repeat(6);
    }

    let mut spans = vec![
        Span::styled(format!("{label}: "), Style::default().bold()),
        Span::raw(display),
    ];

    if focused {
        for span in spans.iter_mut() {
            span.style = span.style.fg(Color::Yellow).add_modifier(Modifier::BOLD);
        }
    }

    Line::from(spans)
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
            if let Some(t) = set.completed_at_local() {
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
