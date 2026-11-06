//! Handoff and message queues, their addresses, and who may see an item.

use serde::Serialize;

use crate::domain::name::FeatureName;

pub const MAX_TEXT_BYTES: usize = 16 * 1024;
pub const MAX_LIST_ITEMS: usize = 50;
pub const DEFAULT_TTL_DAYS: u32 = 7;
pub const TERMINAL_RETENTION_DAYS: i64 = 30;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Queue {
    Feature(FeatureName),
    Hall,
}

impl Queue {
    #[must_use]
    pub fn column(&self) -> String {
        match self {
            Self::Feature(name) => name.to_string(),
            Self::Hall => String::new(),
        }
    }

    #[must_use]
    pub fn from_column(value: &str) -> Option<Self> {
        if value.is_empty() {
            return Some(Self::Hall);
        }
        FeatureName::new(value).ok().map(Self::Feature)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Address {
    Provider(String),
    Session(String),
}

impl Address {
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Provider(_) => "provider",
            Self::Session(_) => "session",
        }
    }

    #[must_use]
    pub fn value(&self) -> &str {
        match self {
            Self::Provider(value) | Self::Session(value) => value,
        }
    }

    #[must_use]
    pub fn from_columns(kind: &str, value: &str) -> Option<Self> {
        match kind {
            "provider" => Some(Self::Provider(value.to_owned())),
            "session" => Some(Self::Session(value.to_owned())),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Owner {
    pub key: String,
    pub provider: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Viewer {
    pub session_key: Option<String>,
    pub provider: String,
    pub feature: Option<FeatureName>,
}

impl Viewer {
    #[must_use]
    pub fn sees(&self, queue: &Queue, address: Option<&Address>) -> bool {
        let in_queue = match queue {
            Queue::Hall => true,
            Queue::Feature(name) => self.feature.as_ref() == Some(name),
        };
        let addressed_to_me = match address {
            None => true,
            Some(Address::Provider(provider)) => *provider == self.provider,
            Some(Address::Session(session)) => {
                self.session_key.as_deref() == Some(session.as_str())
            }
        };
        in_queue && addressed_to_me
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffState {
    Open,
    Accepted,
    Cancelled,
    Expired,
}

impl HandoffState {
    pub const ALL: [Self; 4] = [Self::Open, Self::Accepted, Self::Cancelled, Self::Expired];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Accepted => "accepted",
            Self::Cancelled => "cancelled",
            Self::Expired => "expired",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.as_str() == value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageState {
    Pending,
    Popped,
    Cancelled,
    Expired,
}

impl MessageState {
    pub const ALL: [Self; 4] = [Self::Pending, Self::Popped, Self::Cancelled, Self::Expired];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Popped => "popped",
            Self::Cancelled => "cancelled",
            Self::Expired => "expired",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.as_str() == value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewHandoff {
    pub id: String,
    pub queue: Queue,
    pub address: Option<Address>,
    pub owner: Owner,
    pub summary: String,
    pub pending_tasks: Vec<String>,
    pub open_decisions: Vec<String>,
    pub modified_files: Vec<String>,
    pub created_at: i64,
    pub expires_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handoff {
    pub id: String,
    pub queue: Queue,
    pub address: Option<Address>,
    pub owner: Owner,
    pub summary: String,
    pub pending_tasks: Vec<String>,
    pub open_decisions: Vec<String>,
    pub modified_files: Vec<String>,
    pub created_at: i64,
    pub expires_at: i64,
    pub state: HandoffState,
    pub accepted_by: Option<String>,
    pub accepted_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMessage {
    pub id: String,
    pub queue: Queue,
    pub address: Option<Address>,
    pub sender: Owner,
    pub subject: Option<String>,
    pub body: String,
    pub created_at: i64,
    pub expires_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub id: String,
    pub queue: Queue,
    pub address: Option<Address>,
    pub sender: Owner,
    pub subject: Option<String>,
    pub body: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub state: MessageState,
    pub popped_by: Option<String>,
    pub popped_at: Option<i64>,
}

#[cfg(test)]
#[path = "../../tests/unit/domain/coordination.rs"]
mod tests;
