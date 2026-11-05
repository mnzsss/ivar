//! Which conversation a delivery event belongs to.

pub const IDLE_SECS: i64 = 30 * 60;

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

#[cfg(test)]
#[path = "../../../../tests/unit/action/memory/consolidate_attribute.rs"]
mod tests;
