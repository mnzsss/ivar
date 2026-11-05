//! Which conversation a delivery event belongs to, and the session page file
//! a conversation is written to.

pub const IDLE_SECS: i64 = 30 * 60;
pub const SESSION_PAGES_DIR: &str = ".ivar/memory/session/";

#[must_use]
pub fn attribute(windows: &[(i64, i64)], event_at: i64) -> Option<usize> {
    let holding = windows
        .iter()
        .enumerate()
        .filter(|(_, (first, last))| *first <= event_at && event_at <= last + IDLE_SECS)
        .max_by_key(|(_, (first, _))| *first);
    holding
        .or_else(|| {
            windows
                .iter()
                .enumerate()
                .filter(|(_, (_, last))| *last <= event_at)
                .max_by_key(|(_, (_, last))| *last)
        })
        .map(|(index, _)| index)
}

#[must_use]
pub fn page_rel(native_session_id: &str) -> String {
    format!("{SESSION_PAGES_DIR}{}.md", page_stem(native_session_id))
}

#[must_use]
pub fn page_stem(native_session_id: &str) -> String {
    let safe = !native_session_id.is_empty()
        && native_session_id.len() <= 128
        && !native_session_id.starts_with('.')
        && native_session_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if safe {
        native_session_id.to_owned()
    } else {
        crate::infra::hash::text(native_session_id)
            .chars()
            .take(16)
            .collect()
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/memory/consolidate_attribute.rs"]
mod tests;
