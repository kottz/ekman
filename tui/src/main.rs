use color_eyre::eyre::WrapErr;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    DefaultTerminal, Frame,
    style::Stylize,
    text::Line,
    widgets::{Block, Paragraph},
};
use serde::Deserialize;
use std::fmt::Write;

const BACKEND_BASE_URL: &str = "http://localhost:3000";
const DAILY_PLANS_PATH: &str = "/api/plans/daily";

#[derive(Debug, Deserialize)]
struct PopulatedTemplate {
    name: String,
    day_of_week: Option<i32>,
    exercises: Vec<PopulatedExercise>,
}

#[derive(Debug, Deserialize)]
struct PopulatedExercise {
    name: String,
    target_sets: Option<i32>,
}

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    let terminal = ratatui::init();
    let result = App::new().run(terminal);
    ratatui::restore();
    result
}

/// The main application which holds the state and logic of the application.
#[derive(Debug, Default)]
pub struct App {
    /// Is the application running?
    running: bool,
    daily_plans: Vec<PopulatedTemplate>,
    status_line: String,
}

impl App {
    /// Construct a new instance of [`App`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Run the application's main loop.
    pub fn run(mut self, mut terminal: DefaultTerminal) -> color_eyre::Result<()> {
        self.running = true;
        match fetch_daily_plans() {
            Ok(plans) => {
                self.status_line = format!("Loaded {} plans from backend", plans.len());
                self.daily_plans = plans;
            }
            Err(error) => {
                self.status_line = format!("Failed to reach backend: {error}");
            }
        };
        while self.running {
            terminal.draw(|frame| self.render(frame))?;
            self.handle_crossterm_events()?;
        }
        Ok(())
    }

    /// Renders the user interface.
    ///
    /// This is where you add new widgets. See the following resources for more information:
    ///
    /// - <https://docs.rs/ratatui/latest/ratatui/widgets/index.html>
    /// - <https://github.com/ratatui/ratatui/tree/main/ratatui-widgets/examples>
    fn render(&mut self, frame: &mut Frame) {
        let title = Line::from("Ratatui Simple Template")
            .bold()
            .blue()
            .centered();
        let plans = self.format_plans();
        let text = format!(
            concat!(
                "Hello, Ratatui!\n\n",
                "Daily plans from backend:\n{plans}\n",
                "{status}\n\n",
                "Press `Esc`, `Ctrl-C` or `q` to stop running."
            ),
            plans = plans,
            status = self.status_line
        );
        frame.render_widget(
            Paragraph::new(text)
                .block(Block::bordered().title(title))
                .centered(),
            frame.area(),
        )
    }

    /// Reads the crossterm events and updates the state of [`App`].
    ///
    /// If your application needs to perform work in between handling events, you can use the
    /// [`event::poll`] function to check if there are any events available with a timeout.
    fn handle_crossterm_events(&mut self) -> color_eyre::Result<()> {
        match event::read()? {
            // it's important to check KeyEventKind::Press to avoid handling key release events
            Event::Key(key) if key.kind == KeyEventKind::Press => self.on_key_event(key),
            Event::Mouse(_) => {}
            Event::Resize(_, _) => {}
            _ => {}
        }
        Ok(())
    }

    /// Handles the key events and updates the state of [`App`].
    fn on_key_event(&mut self, key: KeyEvent) {
        match (key.modifiers, key.code) {
            (_, KeyCode::Esc | KeyCode::Char('q'))
            | (KeyModifiers::CONTROL, KeyCode::Char('c') | KeyCode::Char('C')) => self.quit(),
            // Add other key handlers here.
            _ => {}
        }
    }

    /// Set running to false to quit the application.
    fn quit(&mut self) {
        self.running = false;
    }

    fn format_plans(&self) -> String {
        let mut out = String::new();
        if self.daily_plans.is_empty() {
            out.push_str("No daily plans found.");
            return out;
        }

        for template in &self.daily_plans {
            let _ = writeln!(
                out,
                "- {} ({})",
                template.name,
                template
                    .day_of_week
                    .map(|day| format!("day {}", day))
                    .unwrap_or_else(|| "no day set".to_string())
            );
            if template.exercises.is_empty() {
                let _ = writeln!(out, "  (no exercises)");
                continue;
            }
            for exercise in &template.exercises {
                let _ = writeln!(
                    out,
                    "  - {} (target sets: {})",
                    exercise.name,
                    exercise
                        .target_sets
                        .map(|sets| sets.to_string())
                        .unwrap_or_else(|| "-".to_string())
                );
            }
        }

        out
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
