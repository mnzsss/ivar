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
    DeliveryRefused,
    DeliveryPushed,
    DeliveryPr,
    DeliveryLanded,
    DeliveryChecks,
    DeliveryRebased,
    DeliveryIntegrated,
    DeliveryMerged,
    DeliveryCleaned,
    DeliveryClosed,
    DeliveryDeleted,
}

impl SourceEvent {
    const ALL: [Self; 15] = [
        Self::SessionStarted,
        Self::SessionPrompted,
        Self::SessionTool,
        Self::SessionEnded,
        Self::DeliveryRefused,
        Self::DeliveryPushed,
        Self::DeliveryPr,
        Self::DeliveryLanded,
        Self::DeliveryChecks,
        Self::DeliveryRebased,
        Self::DeliveryIntegrated,
        Self::DeliveryMerged,
        Self::DeliveryCleaned,
        Self::DeliveryClosed,
        Self::DeliveryDeleted,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SessionStarted => "session.started",
            Self::SessionPrompted => "session.prompted",
            Self::SessionTool => "session.tool",
            Self::SessionEnded => "session.ended",
            Self::DeliveryRefused => "delivery.refused",
            Self::DeliveryPushed => "delivery.pushed",
            Self::DeliveryPr => "delivery.pr",
            Self::DeliveryLanded => "delivery.landed",
            Self::DeliveryChecks => "delivery.checks",
            Self::DeliveryRebased => "delivery.rebased",
            Self::DeliveryIntegrated => "delivery.integrated",
            Self::DeliveryMerged => "delivery.merged",
            Self::DeliveryCleaned => "delivery.cleaned",
            Self::DeliveryClosed => "delivery.closed",
            Self::DeliveryDeleted => "delivery.deleted",
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

    #[must_use]
    pub const fn delivery_failure() -> Self {
        Self(7)
    }

    #[must_use]
    pub const fn delivery_merged() -> Self {
        Self(6)
    }

    #[must_use]
    pub const fn delivery_routine() -> Self {
        Self(4)
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
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

impl std::fmt::Debug for Observation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Observation")
            .field("event_id", &self.event_id)
            .field("source_event", &self.source_event)
            .field("occurred_at", &self.occurred_at)
            .field("session_id", &self.session_id)
            .field("native_session_id", &self.native_session_id)
            .field("provider", &self.provider)
            .field("ivar_version", &self.ivar_version)
            .field("feature", &self.feature)
            .field("parent_feature", &self.parent_feature)
            .field("repo", &self.repo)
            .field("origin", &self.origin)
            .field("importance", &self.importance)
            .field("attempt_id", &self.attempt_id)
            .field("body", &format_args!("[{} bytes]", self.body.len()))
            .finish()
    }
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
    digest(&key)
}

fn digest(key: &str) -> String {
    Sha256::digest(key.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[must_use]
pub fn delivery_event_id(
    attempt_id: &str,
    source_event: SourceEvent,
    repo: &str,
    sequence: u32,
) -> String {
    digest(&format!(
        "{attempt_id}\u{1f}{}\u{1f}{repo}\u{1f}{sequence}",
        source_event.as_str()
    ))
}

#[must_use]
pub fn merged_event_id(repo: &str, url: &str, merge_commit: &str) -> String {
    digest(&format!(
        "delivery.merged\u{1f}{repo}\u{1f}{url}\u{1f}{merge_commit}"
    ))
}

#[cfg(test)]
#[path = "../../tests/unit/domain/observation.rs"]
mod tests;
