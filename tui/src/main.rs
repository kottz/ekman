//! Workout tracker TUI.

mod api;
mod state;
mod ui;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use state::{App, View};
use std::time::{Duration, Instant};

const TICK_RATE: Duration = Duration::from_millis(16);

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;

    let mut app = App::new()?;
    app.try_resume_session();

    let mut terminal = ratatui::init();
    let result = run(&mut app, &mut terminal);
    ratatui::restore();

    result
}

fn run(app: &mut App, terminal: &mut ratatui::DefaultTerminal) -> color_eyre::Result<()> {
    let mut last_tick = Instant::now();

    while app.running {
        app.poll_io();
        app.tick();

        terminal.draw(|f| ui::render(app, f))?;

        let timeout = TICK_RATE.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)?
            && let Event::Key(key) = event::read()?
        {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            match app.view {
                View::Auth => handle_auth_key(app, key.code, key.modifiers),
                View::Dashboard => handle_dashboard_key(app, key.code, key.modifiers),
            }
        }

        if last_tick.elapsed() >= TICK_RATE {
            last_tick = Instant::now();
        }
    }

    Ok(())
}

fn handle_auth_key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    use KeyCode::*;

    match code {
        Esc => app.running = false,
        Tab => app.auth.next_field(),
        BackTab => app.auth.prev_field(),
        Enter => app.submit_auth(),
        Backspace => app.auth.backspace(),
        Char('l') if mods.contains(KeyModifiers::CONTROL) => app.set_auth_mode(false),
        Char('r') if mods.contains(KeyModifiers::CONTROL) => app.set_auth_mode(true),
        Char('g') if mods.contains(KeyModifiers::CONTROL) => app.auth.regenerate_secret(),
        Char(ch) => app.auth.push_char(ch),
        _ => {}
    }
}

fn handle_dashboard_key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    use KeyCode::*;

    // Quit
    if code == Esc
        || code == Char('q')
        || (code == Char('c') && mods.contains(KeyModifiers::CONTROL))
    {
        app.running = false;
        return;
    }

    match code {
        // Day navigation
        Char('a') => app.move_day(-1),
        Char('s') => app.move_day(1),
        Char('r') => app.jump_to_today(),

        // Exercise navigation
        Char('n') => app.select_exercise(1),
        Char('e') => app.select_exercise(-1),

        // Field focus
        Up | Down => app.toggle_focus(),

        // Set cursor
        Left => app.move_set_cursor(-1),
        Right => app.move_set_cursor(1),

        // Tab navigation
        Tab => app.tab_next(),
        BackTab => app.tab_prev(),

        // Weight bumps
        Char('w') => app.bump_weight(2.5),
        Char('f') => app.bump_weight(-2.5),

        // Delete set
        Char('d') => app.delete_current_set(),

        // Digit input
        Char(ch) if ch.is_ascii_digit() || ch == '.' => app.input_char(ch),

        // Backspace
        Backspace => app.backspace(),

        _ => {}
    }
}
