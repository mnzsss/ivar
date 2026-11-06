//! `memory.*` Observations the binary records through the delivery spool.

use std::io::Write;

use serde_json::Value;

use super::capture::sanitize_context;
use crate::domain::observation::{Importance, Observation, SourceEvent, event_id};
use crate::domain::sanitize::Sanitized;
use crate::domain::session::rfc3339_now;
use crate::store::layout::Layout;
use crate::store::memory::spool::{self, Appended, SpoolKey};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryEventScope {
    pub session_id: String,
    pub feature: String,
    pub parent_feature: String,
}

const fn importance(event: SourceEvent) -> Option<Importance> {
    match event {
        SourceEvent::MemoryPageWritten => Some(Importance::memory_page_written()),
        SourceEvent::MemoryReviewed => Some(Importance::memory_reviewed()),
        SourceEvent::MemoryReviewSkipped => Some(Importance::memory_review_skipped()),
        _ => None,
    }
}

/// # Errors
///
/// When `event` is not a `memory.*` event, the body cannot be sanitized,
/// or the spool refuses the line.
#[allow(clippy::needless_pass_by_value)]
pub fn record_memory_event(
    layout: &Layout,
    scope: &MemoryEventScope,
    event: SourceEvent,
    body: Value,
) -> Result<(), String> {
    let importance =
        importance(event).ok_or_else(|| format!("`{}` is not a memory event", event.as_str()))?;
    let occurred_at = rfc3339_now();
    let observation = Observation {
        event_id: event_id(&scope.session_id, event, &occurred_at, std::process::id()),
        source_event: event.as_str().to_owned(),
        occurred_at,
        session_id: scope.session_id.clone(),
        native_session_id: String::new(),
        provider: String::new(),
        ivar_version: env!("CARGO_PKG_VERSION").to_owned(),
        feature: scope.feature.clone(),
        parent_feature: scope.parent_feature.clone(),
        repo: String::new(),
        origin: String::new(),
        importance,
        attempt_id: String::new(),
        body: body.to_string(),
    };
    let sanitized = Sanitized::sanitize(observation, &sanitize_context(layout))
        .map_err(|_| "the event body could not be sanitized".to_owned())?;
    match spool::append(
        &layout.memory_spool_dir(),
        &SpoolKey::delivery(),
        &sanitized,
    ) {
        Ok(Appended::Written) => Ok(()),
        Ok(Appended::Dropped) => Err("the delivery spool reached its size cap".to_owned()),
        Err(error) => Err(error.to_string()),
    }
}

pub fn report_memory_event(
    layout: &Layout,
    scope: &MemoryEventScope,
    event: SourceEvent,
    body: Value,
) {
    if let Err(reason) = record_memory_event(layout, scope, event, body) {
        let _ = writeln!(
            std::io::stderr(),
            "ivar: memory.event_not_recorded: {reason}"
        );
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/action/memory/events.rs"]
mod tests;
