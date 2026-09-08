//! Episode payload data models for session memory persistence.

use serde::{Deserialize, Serialize};

use crate::domain::memory::sanitizer::sanitize_text;
use crate::domain::name::{FeatureName, SessionId};

/// Summary episode produced upon session completion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpisodePayload {
    /// Session identifier.
    pub session_id: SessionId,
    /// Associated feature, if this was a feature session.
    pub feature: Option<FeatureName>,
    /// RFC 3339 start timestamp.
    pub started: String,
    /// RFC 3339 stop timestamp.
    pub stopped: String,
    /// Summary of the work done in the session.
    pub summary: String,
    /// Files touched/written during the session.
    pub files_touched: Vec<String>,
}

impl EpisodePayload {
    /// Create a new sanitized episode payload.
    pub fn new(
        session_id: SessionId,
        feature: Option<FeatureName>,
        started: impl Into<String>,
        stopped: impl Into<String>,
        summary: impl AsRef<str>,
        files_touched: Vec<String>,
    ) -> Self {
        let sanitized_summary = sanitize_text(summary.as_ref()).into_inner();
        let sanitized_files = files_touched
            .into_iter()
            .map(|f| sanitize_text(&f).into_inner())
            .collect();

        Self {
            session_id,
            feature,
            started: started.into(),
            stopped: stopped.into(),
            summary: sanitized_summary,
            files_touched: sanitized_files,
        }
    }

    /// Render markdown representation for storage in `<hall>/memory/sessions/<session_id>.md`.
    pub fn render_markdown(&self) -> String {
        let feature_line = match &self.feature {
            Some(f) => format!("- **Feature**: `{f}`"),
            None => "- **Feature**: _(discovery / none)_".to_string(),
        };

        let mut lines = Vec::new();
        lines.push(format!("# Session Episode: {}", self.session_id));
        lines.push(String::new());
        lines.push(feature_line);
        lines.push(format!("- **Started**: {}", self.started));
        lines.push(format!("- **Stopped**: {}", self.stopped));
        lines.push(String::new());
        lines.push("## Summary".to_string());
        lines.push(String::new());
        lines.push(self.summary.trim().to_string());
        lines.push(String::new());
        lines.push("## Files Touched".to_string());
        lines.push(String::new());
        if self.files_touched.is_empty() {
            lines.push("_(none)_".to_string());
        } else {
            for file in &self.files_touched {
                lines.push(format!("- `{file}`"));
            }
        }
        lines.push(String::new());

        lines.join("\n")
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/domain/memory/episode.rs"]
mod tests;
