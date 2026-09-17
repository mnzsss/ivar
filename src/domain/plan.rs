//! `plan.md` frontmatter: the repos a plan edits, and how the feature closed.
//!
//! Parsing lives in `action` (`domain` may not import `infra::frontmatter`).
//! No `deny_unknown_fields`: frontmatter is hand-edited across ivar versions,
//! and `write_close` round-trips it, so an unknown key must survive.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PlanFrontmatter {
    /// Kept as strings so a malformed name never breaks reading a close record;
    /// names are validated where the plan gate is approved.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub repos: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
