use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::{Failure, FixAction};

/// A run's identity — a UUID, generated once at start and never rewritten.
///
/// Validated rather than a bare `String` because it is also a *filename*:
/// archived receipts live at `archive/runs/<run-id>.json`, and a value that
/// could hold `..` or `/` would turn `status --run <id>` into a path
/// traversal. The rule is the same one [`SessionId`] uses, for the same
/// reason.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct RunId(String);

impl RunId {
    /// Validates `value` as a UUID. The only constructor — there is no
    /// unchecked path in or out, so an id read off disk is as safe to join
    /// onto a path as one this process minted.
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidRunId> {
        let value = value.into();
        uuid::Uuid::parse_str(&value).map_err(|_| InvalidRunId(value.clone()))?;
        Ok(Self(value))
    }

    /// The validated value, borrowed.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for RunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("RunId").field(&self.0).finish()
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for RunId {
    /// Routes through [`RunId::new`]. A derived impl would let a hand-edited
    /// `run.json` smuggle a traversal past the type and straight into an
    /// archive path.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::new(raw).map_err(serde::de::Error::custom)
    }
}

/// A run id that is not a UUID.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` is not a run id — expected a UUID")]
pub struct InvalidRunId(pub String);

impl From<InvalidRunId> for Failure {
    fn from(error: InvalidRunId) -> Self {
        Failure::blocked("execute.invalid_run_id", error.to_string()).fix(FixAction::safe(
            "execute.list_runs",
            "Run `ivar feature execute status <feature> --history` for the run ids that exist.",
        ))
    }
}
