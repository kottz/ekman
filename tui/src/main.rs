//! Workout tracker TUI application.

mod command;
mod io;
mod keybind;
mod state;
mod ui;

use crate::command::Command;
use crate::keybind::KeyBindings;
use crate::state::App;
use crossterm::event::{self, Event, KeyEventKind};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const TICK_RATE: Duration = Duration::from_millis(16);

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;

    let bindings = KeyBindings::load(&config_path());
    let (io_tx, io_rx) = io::spawn();
    let mut app = App::new(io_tx, io_rx);

    app.request_daily_plans();
    app.request_activity_history();
    app.refresh_status();

    let mut terminal = ratatui::init();
    let result = run(&mut app, &mut terminal, &bindings);
    ratatui::restore();

    result
}

fn run(
    app: &mut App,
    terminal: &mut ratatui::DefaultTerminal,
    bindings: &KeyBindings,
) -> color_eyre::Result<()> {
    let mut last_tick = Instant::now();

    while app.running {
        app.poll_io();

        terminal.draw(|frame| ui::render(app, frame))?;

        let timeout = TICK_RATE.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
            && let Some(cmd) = bindings.get(key)
        {
            execute(app, cmd);
        }

        if last_tick.elapsed() >= TICK_RATE {
            last_tick = Instant::now();
        }
    }

    Ok(())
}

fn execute(app: &mut App, cmd: Command) {
    match cmd {
        Command::Quit => app.running = false,
        Command::NextExercise => app.select_exercise(1),
        Command::PrevExercise => app.select_exercise(-1),
        Command::NextField => {
            if let Some(ex) = app.current_exercise_mut() {
                ex.toggle_focus();
            }
            app.refresh_status();
        }
        Command::PrevField => {
            if let Some(ex) = app.current_exercise_mut() {
                ex.toggle_focus();
            }
            app.refresh_status();
        }
        Command::MoveLeft => {
            if let Some(ex) = app.current_exercise_mut() {
                ex.move_set_cursor(-1);
            }
            app.refresh_status();
        }
        Command::MoveRight => {
            if let Some(ex) = app.current_exercise_mut() {
                ex.move_set_cursor(1);
            }
            app.refresh_status();
        }
        Command::NextSet => app.tab_next(),
        Command::PrevSet => app.tab_prev(),
        Command::BumpWeightUp => {
            if let Some(set_idx) = app.current_exercise_mut().map(|ex| {
                let set_idx = ex.set_cursor;
                ex.bump_weight(2.5);
                set_idx
            }) {
                app.sync_set(app.selected, set_idx);
            }
            app.refresh_status();
        }
        Command::BumpWeightDown => {
            if let Some(set_idx) = app.current_exercise_mut().map(|ex| {
                let set_idx = ex.set_cursor;
                ex.bump_weight(-2.5);
                set_idx
            }) {
                app.sync_set(app.selected, set_idx);
            }
            app.refresh_status();
        }
        Command::Digit(ch) => app.input_digit(ch),
        Command::Backspace => app.backspace(),
    }
}

fn config_path() -> PathBuf {
    config_dir()
        .map(|d| d.join("binds.conf"))
        .unwrap_or_else(|| "binds.conf".into())
}

fn config_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|p| p.join("ekman"))
}
