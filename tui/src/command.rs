//! Application commands and their string representations.

use std::str::FromStr;

/// All commands the application can execute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Command {
    Quit,
    NextExercise,
    PrevExercise,
    NextField,
    PrevField,
    MoveLeft,
    MoveRight,
    NextSet,
    PrevSet,
    BumpWeightUp,
    BumpWeightDown,
    Digit(char),
    Backspace,
}

impl FromStr for Command {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "quit" => Ok(Self::Quit),
            "next-exercise" => Ok(Self::NextExercise),
            "prev-exercise" => Ok(Self::PrevExercise),
            "next-field" => Ok(Self::NextField),
            "prev-field" => Ok(Self::PrevField),
            "move-left" => Ok(Self::MoveLeft),
            "move-right" => Ok(Self::MoveRight),
            "next-set" => Ok(Self::NextSet),
            "prev-set" => Ok(Self::PrevSet),
            "bump-weight-up" => Ok(Self::BumpWeightUp),
            "bump-weight-down" => Ok(Self::BumpWeightDown),
            "backspace" => Ok(Self::Backspace),
            _ => Err(()),
        }
    }
}

impl Command {
    /// Returns the canonical name for this command.
    pub fn name(self) -> &'static str {
        match self {
            Self::Quit => "quit",
            Self::NextExercise => "next-exercise",
            Self::PrevExercise => "prev-exercise",
            Self::NextField => "next-field",
            Self::PrevField => "prev-field",
            Self::MoveLeft => "move-left",
            Self::MoveRight => "move-right",
            Self::NextSet => "next-set",
            Self::PrevSet => "prev-set",
            Self::BumpWeightUp => "bump-weight-up",
            Self::BumpWeightDown => "bump-weight-down",
            Self::Digit(_) => "digit",
            Self::Backspace => "backspace",
        }
    }
}
