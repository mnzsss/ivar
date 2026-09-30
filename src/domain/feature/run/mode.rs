use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::{Failure, FixAction};

/// Execution mode for a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunMode {
    /// Human gates are interactive.
    #[default]
    Default,
    /// Automated goal mode: runs until delivery gate without stopping.
    Goal,
}

impl RunMode {
    /// Parse a mode string from the CLI or JSON.
    ///
    /// # Errors
    ///
    /// Returns [`UnknownRunMode`] if the string is not `"default"` or `"goal"`.
    pub fn parse(value: &str) -> Result<Self, UnknownRunMode> {
        match value {
            "default" => Ok(Self::Default),
            "goal" => Ok(Self::Goal),
            other => Err(UnknownRunMode(other.to_owned())),
        }
    }
}

impl fmt::Display for RunMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Default => "default",
            Self::Goal => "goal",
        };
        f.pad(label)
    }
}

/// The supplied execution mode is not recognised.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown mode `{0}` — expected one of: default, goal")]
pub struct UnknownRunMode(pub String);

impl From<UnknownRunMode> for Failure {
    fn from(err: UnknownRunMode) -> Self {
        let msg = err.to_string();
        Failure::blocked("execute.unknown_mode", msg).fix(FixAction::safe(
            "execute.valid_mode",
            "Use one of: default, goal.",
        ))
    }
}
