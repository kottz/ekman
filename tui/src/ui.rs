//! UI rendering.

use crate::state::{App, AuthField, ExerciseState, Focus, ManageMode, View};
use chrono::Utc;
use ekman_core::{ActivityDay, Graph};
use qrcode::QrCode;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    symbols,
    text::{Line, Span},
    widgets::{
        Axis, Block, Cell, Chart, Dataset, GraphType, List, ListItem, Paragraph, Row, Table,
    },
};
use std::fmt::Write;
use tui_qrcode::{Colors, QrCodeWidget};

const WORKOUT_HINTS: &str = "←/→: set • Tab: navigate • ↑/↓: weight/reps • W/F: ±2.5kg • N/E: exercise • A/S: day • R: today • D: delete • F2: manage • q: quit";
const MANAGE_HINTS: &str = "N/E: day • ↑/↓: exercise • A: add • D: remove • F1: workout • q: quit";
const MANAGE_ADD_HINTS: &str = "Type to search • ↑/↓: select • Enter: confirm • Esc: cancel";

const WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

pub fn render(app: &App, frame: &mut Frame) {
    match app.view {
        View::Auth => render_auth(app, frame),
        View::Workout => render_workout(app, frame),
        View::Manage => render_manage(app, frame),
    }
}

// ============================================================================
// Workout View
// ============================================================================

fn render_workout(app: &App, frame: &mut Frame) {
    let [top, main, status] = Layout::vertical([
        Constraint::Length(5),
        Constraint::Min(0),
        Constraint::Length(4),
    ])
    .areas(frame.area());

    let [day_area, activity_area] =
        Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)]).areas(top);

    let [graph_area, exercise_area] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(main);

    render_day(frame, day_area, app);
    render_activity(frame, activity_area, &app.activity, &app.day.to_string());
    render_graphs(frame, graph_area, &app.graphs);
    render_exercises(frame, exercise_area, &app.exercises, app.selected);
    render_status(frame, status, &app.status, WORKOUT_HINTS);
}

fn render_day(frame: &mut Frame, area: Rect, app: &App) {
    let today = Utc::now().date_naive();
    let offset = app.day.signed_duration_since(today).num_days();
    let relative = match offset {
        0 => "Today".into(),
        1 => "Tomorrow".into(),
        -1 => "Yesterday".into(),
        d if d > 0 => format!("In {d} days"),
        d => format!("{} days ago", d.abs()),
    };

    let plan = app.current_plan_name().unwrap_or("No plan");

    let lines = vec![
        Line::from(vec![
            Span::styled(app.day.format("%A").to_string(), Style::default().bold()),
            Span::raw(format!(" • {}", app.day.format("%Y-%m-%d"))),
        ]),
        Line::from(format!("Plan: {plan}")),
        Line::from(format!("{relative} • a/s: prev/next  r: today")),
    ];

    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().title("Day")),
        area,
    );
}

fn render_activity(frame: &mut Frame, area: Rect, days: &[ActivityDay], selected: &str) {
    if days.is_empty() {
        frame.render_widget(
            Paragraph::new("No activity data").block(Block::bordered().title("Activity")),
            area,
        );
        return;
    }

    let today = Utc::now().date_naive().format("%Y-%m-%d").to_string();
    let mut spans = Vec::with_capacity(days.len() * 2);

    for (i, day) in days.iter().enumerate() {
        let color = match day.sets_completed {
            c if c >= 9 => Color::Green,
            c if c >= 1 => Color::Yellow,
            _ => Color::Red,
        };

        let mut style = Style::default().fg(color);
        if day.date == today {
            style = style.bold();
        }
        if day.date == selected {
            style = style.bold().add_modifier(Modifier::UNDERLINED);
        }

        spans.push(Span::styled("●", style));
        if i + 1 < days.len() {
            spans.push(Span::raw(" "));
        }
    }

    frame.render_widget(
        Paragraph::new(Line::from(spans))
            .block(Block::bordered().title("Activity"))
            .alignment(Alignment::Center),
        area,
    );
}

fn render_graphs(frame: &mut Frame, area: Rect, graphs: &[Graph]) {
    if graphs.is_empty() {
        frame.render_widget(
            Paragraph::new("No graph data").block(Block::bordered().title("Progress")),
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

fn render_graph(frame: &mut Frame, area: Rect, graph: &Graph) {
    let data: Vec<(f64, f64)> = graph
        .points
        .iter()
        .enumerate()
        .map(|(i, p)| (i as f64, p.value))
        .collect();

    if data.is_empty() {
        frame.render_widget(
            Paragraph::new("No data").block(Block::bordered().title(graph.exercise_name.as_str())),
            area,
        );
        return;
    }

    let (min_y, max_y) = data
        .iter()
        .map(|(_, v)| *v)
        .fold((f64::MAX, f64::MIN), |(min, max), v| {
            (v.min(min), v.max(max))
        });

    let (min_y, max_y) = if min_y == max_y {
        (min_y - 1.0, max_y + 1.0)
    } else {
        let pad = (max_y - min_y) * 0.1;
        (min_y - pad, max_y + pad)
    };

    let x_max = (data.len().saturating_sub(1) as f64).max(1.0);

    let x_labels: Vec<String> = match graph.points.len() {
        0 | 1 => vec!["".into(); 3],
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
        .name(graph.exercise_name.as_str())
        .marker(symbols::Marker::Braille)
        .graph_type(GraphType::Line)
        .style(Style::default().cyan())
        .data(&data);

    let chart = Chart::new(vec![dataset])
        .block(Block::bordered().title(format!("Progress • {}", graph.exercise_name)))
        .x_axis(
            Axis::default()
                .title("Sessions")
                .bounds([0.0, x_max])
                .labels(x_labels),
        )
        .y_axis(
            Axis::default()
                .title("Weight")
                .bounds([min_y, max_y])
                .labels([format!("{min_y:.0}"), format!("{max_y:.0}")]),
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
    let visible = ex.visible_len(selected);

    let weight_cells: Vec<Cell> = ex
        .sets
        .iter()
        .take(visible)
        .enumerate()
        .map(|(i, set)| {
            let style = if selected && ex.focus == Focus::Weight && ex.cursor == i {
                Style::default().yellow().bold()
            } else {
                Style::default()
            };
            Cell::from(format!("{} kg", set.weight_display())).style(style)
        })
        .collect();

    let reps_cells: Vec<Cell> = ex
        .sets
        .iter()
        .take(visible)
        .enumerate()
        .map(|(i, set)| {
            let mut text = set.reps_display();
            if let Some(t) = set.completed_local() {
                let _ = write!(text, "\n{}", t.format("%H:%M:%S"));
            }
            let style = if selected && ex.focus == Focus::Reps && ex.cursor == i {
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

fn render_status(frame: &mut Frame, area: Rect, status: &str, hints: &str) {
    let lines = vec![Line::from(status.to_string()), Line::from(hints)];

    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().title("Status")),
        area,
    );
}

// ============================================================================
// Manage View
// ============================================================================

fn render_manage(app: &App, frame: &mut Frame) {
    let [main, status] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(4)]).areas(frame.area());

    let hints = match app.manage.mode {
        ManageMode::Browse => MANAGE_HINTS,
        ManageMode::AddExercise => MANAGE_ADD_HINTS,
    };

    render_manage_main(frame, main, app);
    render_status(frame, status, &app.status, hints);
}

fn render_manage_main(frame: &mut Frame, area: Rect, app: &App) {
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(area);

    render_weekly_plans(frame, left, app);

    match app.manage.mode {
        ManageMode::Browse => render_plan_details(frame, right, app),
        ManageMode::AddExercise => render_exercise_search(frame, right, app),
    }
}

fn render_weekly_plans(frame: &mut Frame, area: Rect, app: &App) {
    let items: Vec<ListItem> = WEEKDAYS
        .iter()
        .enumerate()
        .map(|(i, day_name)| {
            let plan = app.plan_for_weekday(i);
            let exercise_count = plan.map(|p| p.exercises.len()).unwrap_or(0);
            let plan_name = plan.map(|p| p.name.as_str()).unwrap_or("No plan");

            let style = if i == app.manage.selected_day {
                Style::default().yellow().bold()
            } else {
                Style::default()
            };

            let content = format!("{day_name}: {plan_name} ({exercise_count} exercises)");
            ListItem::new(content).style(style)
        })
        .collect();

    let list = List::new(items)
        .block(Block::bordered().title("Weekly Plans"))
        .highlight_style(Style::default().yellow().bold());

    frame.render_widget(list, area);
}

fn render_plan_details(frame: &mut Frame, area: Rect, app: &App) {
    let day_name = WEEKDAYS[app.manage.selected_day];
    let plan = app.plan_for_weekday(app.manage.selected_day);

    let title = match plan {
        Some(p) => format!("{} - {}", day_name, p.name),
        None => format!("{} - No plan", day_name),
    };

    let items: Vec<ListItem> = match plan {
        Some(p) => p
            .exercises
            .iter()
            .enumerate()
            .map(|(i, ex)| {
                let style = if i == app.manage.selected_exercise {
                    Style::default().cyan().bold()
                } else {
                    Style::default()
                };
                let sets_info = ex
                    .target_sets
                    .map(|s| format!(" ({s} sets)"))
                    .unwrap_or_default();
                ListItem::new(format!("• {}{}", ex.name, sets_info)).style(style)
            })
            .collect(),
        None => vec![ListItem::new("No exercises configured").style(Style::default().dim())],
    };

    let list = List::new(items).block(Block::bordered().title(title));
    frame.render_widget(list, area);
}

fn render_exercise_search(frame: &mut Frame, area: Rect, app: &App) {
    let [search_area, results_area] =
        Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).areas(area);

    // Search input
    let search_text = if app.manage.search_query.is_empty() {
        "Type to search exercises...".to_string()
    } else {
        app.manage.search_query.clone()
    };

    let search_style = if app.manage.search_query.is_empty() {
        Style::default().dim()
    } else {
        Style::default().yellow()
    };

    let search = Paragraph::new(search_text)
        .style(search_style)
        .block(Block::bordered().title("Search Exercise"));
    frame.render_widget(search, search_area);

    // Results
    let items: Vec<ListItem> = app
        .manage
        .search_results
        .iter()
        .enumerate()
        .map(|(i, ex)| {
            let style = if i == app.manage.search_cursor {
                Style::default().green().bold()
            } else {
                Style::default()
            };
            ListItem::new(ex.name.clone()).style(style)
        })
        .collect();

    let results_title = format!("Results ({})", app.manage.search_results.len());
    let results = List::new(items).block(Block::bordered().title(results_title));
    frame.render_widget(results, results_area);
}

// ============================================================================
// Auth
// ============================================================================

fn render_auth(app: &App, frame: &mut Frame) {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(12),
        Constraint::Length(3),
    ])
    .areas(frame.area());

    // Header with tabs
    let title = if app.auth.register_mode {
        "Register"
    } else {
        "Login"
    };
    let tabs = Line::from(vec![
        Span::styled(
            " Login ",
            if !app.auth.register_mode {
                Style::default().bold().green()
            } else {
                Style::default().dim()
            },
        ),
        Span::raw(" • "),
        Span::styled(
            " Register ",
            if app.auth.register_mode {
                Style::default().bold().cyan()
            } else {
                Style::default().dim()
            },
        ),
    ]);

    let header_block = Block::bordered()
        .title(format!("Ekman • {title}"))
        .title_alignment(Alignment::Center);
    let header_inner = header_block.inner(header);
    frame.render_widget(header_block, header);
    frame.render_widget(
        Paragraph::new(tabs).alignment(Alignment::Center),
        header_inner,
    );

    // Body
    let body_cols: [Rect; 2] = Layout::horizontal([
        Constraint::Percentage(if app.auth.register_mode { 55 } else { 70 }),
        Constraint::Percentage(if app.auth.register_mode { 45 } else { 30 }),
    ])
    .areas(body);

    render_auth_form(frame, body_cols[0], app);

    if app.auth.register_mode {
        render_auth_qr(frame, body_cols[1], app);
    } else {
        render_auth_help(frame, body_cols[1]);
    }

    // Footer
    let footer_text = if !app.auth.status.is_empty() {
        app.auth.status.clone()
    } else {
        "Ctrl+L login • Ctrl+R register • Ctrl+G new secret • Esc quits".into()
    };

    frame.render_widget(
        Paragraph::new(footer_text)
            .alignment(Alignment::Center)
            .block(Block::bordered()),
        footer,
    );
}

fn render_auth_form(frame: &mut Frame, area: Rect, app: &App) {
    let auth = &app.auth;

    let lines = vec![
        field_line(
            "Username",
            &auth.username,
            auth.field == AuthField::Username,
            false,
        ),
        field_line(
            "Password",
            &auth.password,
            auth.field == AuthField::Password,
            true,
        ),
        field_line(
            "TOTP code",
            &auth.totp_code,
            auth.field == AuthField::Totp,
            false,
        ),
    ];

    let title = if auth.register_mode {
        "Create account (2FA required)"
    } else {
        "Enter credentials"
    };

    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(title)),
        area,
    );
}

fn render_auth_qr(frame: &mut Frame, area: Rect, app: &App) {
    let [qr_area, info_area] =
        Layout::vertical([Constraint::Min(10), Constraint::Length(5)]).areas(area);

    let qr_block = Block::bordered().title("Scan QR in authenticator");
    let qr_inner = qr_block.inner(qr_area);
    frame.render_widget(qr_block, qr_area);

    match QrCode::new(app.auth.otpauth_url().as_bytes()) {
        Ok(code) => {
            frame.render_widget(QrCodeWidget::new(code).colors(Colors::Inverted), qr_inner);
        }
        Err(_) => {
            frame.render_widget(
                Paragraph::new("QR error").alignment(Alignment::Center),
                qr_inner,
            );
        }
    }

    let info = Paragraph::new(vec![
        Line::from("1) Scan the QR code"),
        Line::from("2) Enter the 6-digit code"),
        Line::from("Ctrl+G to regenerate secret"),
    ])
    .block(Block::bordered().title("Instructions"));

    frame.render_widget(info, info_area);
}

fn render_auth_help(frame: &mut Frame, area: Rect) {
    let help = Paragraph::new(vec![
        Line::from("Enter username, password, and TOTP code."),
        Line::from("Ctrl+R to register instead."),
        Line::from("Tab to move fields, Enter to submit."),
    ])
    .block(Block::bordered().title("Help"));

    frame.render_widget(help, area);
}

fn field_line(label: &str, value: &str, focused: bool, mask: bool) -> Line<'static> {
    let display = if mask {
        "•".repeat(value.chars().count())
    } else {
        value.to_string()
    };

    let display = if display.is_empty() {
        "_".repeat(6)
    } else {
        display
    };

    let style = if focused {
        Style::default().fg(Color::Yellow).bold()
    } else {
        Style::default()
    };

    Line::from(vec![
        Span::styled(format!("{label}: "), Style::default().bold()),
        Span::styled(display, style),
    ])
}
