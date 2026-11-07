//! The automation decisions in `memory.db`. Read-only, and only ever a
//! warning: an accepted decision whose artifact is gone is ignored by `ivar
//! automation scan`, so its group is a candidate again. `ivar` does not delete
//! the decision.

use crate::action::automation::{artifact_exists, open_read_only};
use crate::domain::automation::DecisionKind;
use crate::store::layout::Layout;
use crate::store::memory::automation::list;

use super::Diagnosis;

pub(super) fn automation_diagnoses(layout: &Layout) -> Vec<Diagnosis> {
    // A database that cannot be read is the database checks' to report.
    let Ok(Some(conn)) = open_read_only(layout) else {
        return Vec::new();
    };
    let Ok(decisions) = list(&conn) else {
        return Vec::new();
    };
    decisions
        .iter()
        .filter(|decision| decision.kind == DecisionKind::Accepted)
        .filter(|decision| !artifact_exists(layout, decision))
        .map(|decision| {
            let artifact = decision.artifact.as_deref().unwrap_or_default();
            Diagnosis {
                code: "automation.artifact_missing",
                what: format!(
                    "accepted decision {} (`{}`) points at `{artifact}`, which does not exist, \
                     so it is ignored and its group is a candidate again",
                    decision.id, decision.intent
                ),
                fix: format!(
                    "Restore `{artifact}`, or run `ivar automation undo {}` to forget the decision.",
                    decision.id
                ),
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/hall/doctor/automation.rs"]
mod tests;
