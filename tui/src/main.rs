//! Workout tracker TUI.

mod api;
mod state;
mod ui;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use state::{App, ExerciseEditMode, ManageMode, View};
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
                View::Workout => handle_workout_key(app, key.code, key.modifiers),
                View::Manage => handle_manage_key(app, key.code, key.modifiers),
                View::Exercises => handle_exercises_key(app, key.code, key.modifiers),
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

fn handle_workout_key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    use KeyCode::*;

    // Quit
    if code == Esc
        || code == Char('q')
        || (code == Char('c') && mods.contains(KeyModifiers::CONTROL))
    {
        app.running = false;
        return;
    }

    // View switching
    if code == F(2) {
        app.switch_to_manage();
        return;
    }
    if code == F(3) {
        app.switch_to_exercises();
        return;
    }

    // Weight row is selected
    if app.weight_selected {
        match code {
            // Day navigation (always available)
            Char('a') => app.move_day(-1),
            Char('s') => app.move_day(1),
            Char('r') => app.jump_to_today(),

            // Navigate down to exercises
            Char('n') => app.select_from_weight_to_exercise(),

            // Weight adjustments
            Char('w') => app.bump_body_weight(0.1),
            Char('f') => app.bump_body_weight(-0.1),
            Char('d') => app.delete_body_weight(),
            Enter => app.confirm_weight(),
            Char(ch) if ch.is_ascii_digit() || ch == '.' => app.weight_input_char(ch),
            Backspace => app.weight_backspace(),
            _ => {}
        }
        return;
    }

    match code {
        // Day navigation
        Char('a') => app.move_day(-1),
        Char('s') => app.move_day(1),
        Char('r') => app.jump_to_today(),

        // Exercise navigation (e goes up, can reach weight row)
        Char('n') => app.select_exercise(1),
        Char('e') => app.select_exercise_or_weight(-1),

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

fn handle_manage_key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    use KeyCode::*;

    // Global keys
    if code == Esc {
        if app.manage.mode == ManageMode::AddExercise {
            app.manage_cancel_add();
        } else {
            app.running = false;
        }
        return;
    }

    if code == F(1) {
        app.switch_to_workout();
        return;
    }

    if code == F(3) {
        app.switch_to_exercises();
        return;
    }

    if code == Char('q') && app.manage.mode == ManageMode::Browse {
        app.running = false;
        return;
    }

    if code == Char('c') && mods.contains(KeyModifiers::CONTROL) {
        app.running = false;
        return;
    }

    match app.manage.mode {
        ManageMode::Browse => match code {
            // Day navigation (up/down through weekdays)
            Char('n') => app.manage_select_day(1),
            Char('e') => app.manage_select_day(-1),

            // Exercise navigation within day
            Down => app.manage_select_exercise(1),
            Up => app.manage_select_exercise(-1),

            // Add exercise
            Char('a') => app.manage_start_add(),

            // Delete exercise from plan
            Char('d') => app.manage_delete_exercise(),

            _ => {}
        },

        ManageMode::AddExercise => match code {
            // Navigate search results
            Down => app.manage_search_move(1),
            Up => app.manage_search_move(-1),

            // Confirm selection
            Enter => app.manage_confirm_add(),

            // Cancel
            Backspace if app.manage.search_query.is_empty() => app.manage_cancel_add(),
            Backspace => app.manage_search_backspace(),

            // Type search query
            Char(ch) if !ch.is_control() => app.manage_search_input(ch),

            _ => {}
        },
    }
}

fn handle_exercises_key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    use KeyCode::*;

    // Global keys
    if code == Esc {
        if app.exercise_edit.mode != ExerciseEditMode::Browse {
            app.exercise_cancel();
        } else {
            app.running = false;
        }
        return;
    }

    if code == F(1) {
        app.switch_to_workout();
        return;
    }

    if code == F(2) {
        app.switch_to_manage();
        return;
    }

    if code == Char('q') && app.exercise_edit.mode == ExerciseEditMode::Browse {
        app.running = false;
        return;
    }

    if code == Char('c') && mods.contains(KeyModifiers::CONTROL) {
        app.running = false;
        return;
    }

    match app.exercise_edit.mode {
        ExerciseEditMode::Browse => match code {
            // Navigate exercises
            Down | Char('n') => app.exercise_select(1),
            Up | Char('e') => app.exercise_select(-1),

            // Add new exercise
            Char('a') => app.exercise_start_add(),

            // Rename selected exercise
            Char('r') => app.exercise_start_rename(),

            // Archive/unarchive selected exercise
            Char('x') => app.exercise_archive(),

            // Toggle showing archived
            Char('h') => app.exercise_toggle_archived(),

            _ => {}
        },

        ExerciseEditMode::Add | ExerciseEditMode::Rename => match code {
            // Confirm
            Enter => app.exercise_confirm(),

            // Cancel
            Backspace if app.exercise_edit.input.is_empty() => app.exercise_cancel(),
            Backspace => app.exercise_backspace(),

            // Type name
            Char(ch) if !ch.is_control() => app.exercise_input(ch),

            _ => {}
        },
    }
}
