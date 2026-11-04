//! The Observation recorded for each session event, and the fixed rules that
//! stamp its identity and importance.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceEvent {
    SessionStarted,
    SessionPrompted,
    SessionTool,
    SessionEnded,
}

impl SourceEvent {
    const ALL: [Self; 4] = [
        Self::SessionStarted,
        Self::SessionPrompted,
        Self::SessionTool,
        Self::SessionEnded,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SessionStarted => "session.started",
            Self::SessionPrompted => "session.prompted",
            Self::SessionTool => "session.tool",
            Self::SessionEnded => "session.ended",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|event| event.as_str() == value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    User,
    Harness,
}

impl Origin {
    const ALL: [Self; 2] = [Self::User, Self::Harness];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Harness => "harness",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|origin| origin.as_str() == value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolEffect {
    Failed,
    Writes,
    Reads,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Importance(u8);

impl Importance {
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }

    #[must_use]
    pub const fn session_started() -> Self {
        Self(1)
    }

    #[must_use]
    pub const fn session_ended() -> Self {
        Self(1)
    }

    #[must_use]
    pub const fn session_prompted(origin: Origin) -> Self {
        match origin {
            Origin::User => Self(5),
            Origin::Harness => Self(1),
        }
    }

    #[must_use]
    pub const fn session_tool(effect: ToolEffect) -> Self {
        match effect {
            ToolEffect::Failed => Self(6),
            ToolEffect::Writes => Self(4),
            ToolEffect::Reads => Self(2),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observation {
    pub event_id: String,
    pub source_event: String,
    pub occurred_at: String,
    pub session_id: String,
    pub native_session_id: String,
    pub provider: String,
    pub ivar_version: String,
    pub feature: String,
    pub parent_feature: String,
    pub repo: String,
    pub origin: String,
    pub importance: Importance,
    pub attempt_id: String,
    pub body: String,
}

/// Never hashes the body: it may hold a secret before sanitisation. The process
/// id separates two hook processes that stamp the same nanosecond.
#[must_use]
pub fn event_id(
    native_session_id: &str,
    source_event: SourceEvent,
    occurred_at: &str,
    pid: u32,
) -> String {
    let key = format!(
        "{native_session_id}\u{1f}{}\u{1f}{occurred_at}\u{1f}{pid}",
        source_event.as_str()
    );
    Sha256::digest(key.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
#[path = "../../tests/unit/domain/observation.rs"]
mod tests;
