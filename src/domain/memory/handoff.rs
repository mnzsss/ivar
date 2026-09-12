//! Handoff data models for shared memory.

use serde::{Deserialize, Serialize};

use crate::domain::memory::sanitizer::sanitize_text;
use crate::domain::name::SessionId;

/// Staged handoff information produced by a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffPayload {
    /// Unique identifier for the handoff (e.g. ULID or UUID or timestamp-based ID).
    pub id: String,
    /// Session that produced this handoff.
    pub source_session: SessionId,
    /// High-level summary of work performed and current state.
    pub summary: String,
    /// List of pending or unfinished tasks.
    pub open_tasks: Vec<String>,
    /// Key architectural or implementation decisions made.
    pub decisions: Vec<String>,
    /// Relative or repo-relative paths modified during the session.
    pub modified_paths: Vec<String>,
}

impl HandoffPayload {
    /// Create a new sanitized handoff payload.
    pub fn new(
        id: impl Into<String>,
        source_session: SessionId,
        summary: impl AsRef<str>,
        open_tasks: Vec<String>,
        decisions: Vec<String>,
        modified_paths: Vec<String>,
    ) -> Self {
        let sanitized_summary = sanitize_text(summary.as_ref()).into_inner();
        let sanitized_tasks = open_tasks
            .into_iter()
            .map(|t| sanitize_text(&t).into_inner())
            .collect();
        let sanitized_decisions = decisions
            .into_iter()
            .map(|d| sanitize_text(&d).into_inner())
            .collect();
        let sanitized_paths = modified_paths
            .into_iter()
            .map(|p| sanitize_text(&p).into_inner())
            .collect();

        Self {
            id: id.into(),
            source_session,
            summary: sanitized_summary,
            open_tasks: sanitized_tasks,
            decisions: sanitized_decisions,
            modified_paths: sanitized_paths,
        }
    }

    /// Sanitize in-place.
    pub fn sanitize(&mut self) {
        self.summary = sanitize_text(&self.summary).into_inner();
        self.open_tasks = self
            .open_tasks
            .iter()
            .map(|t| sanitize_text(t).into_inner())
            .collect();
        self.decisions = self
            .decisions
            .iter()
            .map(|d| sanitize_text(d).into_inner())
            .collect();
        self.modified_paths = self
            .modified_paths
            .iter()
            .map(|p| sanitize_text(p).into_inner())
            .collect();
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/domain/memory/handoff.rs"]
mod tests;
