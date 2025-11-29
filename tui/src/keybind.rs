//! Keybinding configuration and parsing.

use crate::command::Command;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Maps key events to commands.
pub struct KeyBindings {
    bindings: HashMap<KeySpec, Command>,
}

/// A normalized key specification for lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct KeySpec {
    code: KeyCode,
    ctrl: bool,
    alt: bool,
    shift: bool,
}

impl From<KeyEvent> for KeySpec {
    fn from(event: KeyEvent) -> Self {
        Self {
            code: event.code,
            ctrl: event.modifiers.contains(KeyModifiers::CONTROL),
            alt: event.modifiers.contains(KeyModifiers::ALT),
            shift: event.modifiers.contains(KeyModifiers::SHIFT),
        }
    }
}

impl KeyBindings {
    /// Create default keybindings.
    pub fn defaults() -> Self {
        let mut bindings = HashMap::new();

        // Quit
        bindings.insert(key(KeyCode::Esc), Command::Quit);
        bindings.insert(key(KeyCode::Char('q')), Command::Quit);
        bindings.insert(ctrl(KeyCode::Char('c')), Command::Quit);

        // Navigation between exercises
        bindings.insert(key(KeyCode::Char('n')), Command::NextExercise);
        bindings.insert(key(KeyCode::Char('e')), Command::PrevExercise);

        // Field focus (weight/reps toggle)
        bindings.insert(key(KeyCode::Down), Command::NextField);
        bindings.insert(key(KeyCode::Up), Command::PrevField);

        // Set cursor movement
        bindings.insert(key(KeyCode::Left), Command::MoveLeft);
        bindings.insert(key(KeyCode::Right), Command::MoveRight);

        // Tab navigation
        bindings.insert(key(KeyCode::Tab), Command::NextSet);
        bindings.insert(shift(KeyCode::BackTab), Command::PrevSet);

        // Weight adjustment
        bindings.insert(key(KeyCode::Char('w')), Command::BumpWeightUp);
        bindings.insert(key(KeyCode::Char('f')), Command::BumpWeightDown);

        // Editing
        bindings.insert(key(KeyCode::Backspace), Command::Backspace);

        Self { bindings }
    }

    /// Load keybindings from a config file, falling back to defaults.
    pub fn load(path: &Path) -> Self {
        let mut bindings = Self::defaults();

        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return bindings,
        };

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with('$') {
                continue;
            }
            // Skip section headers like [compose]
            if line.starts_with('[') {
                continue;
            }

            if let Some((key_str, cmd_str)) = line.split_once('=') {
                let key_str = key_str.trim();
                let cmd_str = cmd_str.trim().trim_matches(':').trim_end_matches("<Enter>");

                if let (Some(spec), Ok(cmd)) = (parse_key_spec(key_str), cmd_str.parse()) {
                    bindings.bindings.insert(spec, cmd);
                }
            }
        }

        bindings
    }

    /// Look up a command for a key event. Digits are handled specially.
    pub fn get(&self, event: KeyEvent) -> Option<Command> {
        // Handle digits directly (they carry data)
        if let KeyCode::Char(ch) = event.code
            && (ch.is_ascii_digit() || ch == '.')
        {
            return Some(Command::Digit(ch));
        }

        self.bindings.get(&KeySpec::from(event)).copied()
    }
}

fn key(code: KeyCode) -> KeySpec {
    KeySpec {
        code,
        ctrl: false,
        alt: false,
        shift: false,
    }
}

fn ctrl(code: KeyCode) -> KeySpec {
    KeySpec {
        code,
        ctrl: true,
        alt: false,
        shift: false,
    }
}

fn shift(code: KeyCode) -> KeySpec {
    KeySpec {
        code,
        ctrl: false,
        alt: false,
        shift: true,
    }
}

/// Parse a key specification string like "<C-x>", "<Esc>", "<tab>", "n".
fn parse_key_spec(s: &str) -> Option<KeySpec> {
    let s = s.trim();

    if s.starts_with('<') && s.ends_with('>') {
        let inner = &s[1..s.len() - 1];
        parse_bracketed_key(inner)
    } else if s.len() == 1 {
        let ch = s.chars().next()?;
        Some(key(KeyCode::Char(ch)))
    } else {
        None
    }
}

fn parse_bracketed_key(s: &str) -> Option<KeySpec> {
    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;

    let parts: Vec<&str> = s.split('-').collect();
    let key_part = parts.last()?;

    for &part in &parts[..parts.len().saturating_sub(1)] {
        match part {
            "C" => ctrl = true,
            "A" => alt = true,
            "S" => shift = true,
            _ => {}
        }
    }

    let code = match key_part.to_lowercase().as_str() {
        "esc" | "escape" => KeyCode::Esc,
        "tab" => KeyCode::Tab,
        "backtab" => {
            shift = true;
            KeyCode::BackTab
        }
        "enter" | "return" => KeyCode::Enter,
        "backspace" => KeyCode::Backspace,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "space" => KeyCode::Char(' '),
        s if s.len() == 1 => KeyCode::Char(s.chars().next()?),
        _ => return None,
    };

    Some(KeySpec {
        code,
        ctrl,
        alt,
        shift,
    })
}
