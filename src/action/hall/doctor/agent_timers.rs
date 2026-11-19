//! A `catchup` agent timer that is installed but has no systemd stamp. Read-only.
//!
//! Without the stamp, `Persistent=true` has no last run to count from, so a run
//! missed while the machine was off is never caught up and nothing says so.
//! `ivar agent status` shows the same case as `stale catchup`.

use camino::Utf8Path;

use crate::action::agent::{hall_hash, status};
use crate::domain::agent::spec::AgentSpec;
use crate::store::layout::Layout;

use super::Diagnosis;

pub(super) fn catchup_stamp_diagnoses(
    layout: &Layout,
    specs: &[AgentSpec],
    unit_dir: &Utf8Path,
    data_dir: &Utf8Path,
    ivar_bin: &Utf8Path,
) -> Vec<Diagnosis> {
    let hash = hall_hash(layout);
    specs
        .iter()
        .filter(|spec| spec.catchup)
        // Only a unit that is installed and current: an agent not yet synced, or
        // one whose unit is outdated, is `ivar agent status`'s to report.
        .filter(|spec| {
            status::installed_sync(layout, &hash, spec, unit_dir, ivar_bin)
                .is_ok_and(|sync| sync == status::UnitSync::InSync)
        })
        .filter(|spec| status::catchup_stamp_missing(spec, &hash, data_dir))
        .map(|spec| Diagnosis {
            code: "agent.catchup_stamp_missing",
            what: format!(
                "agent `{}` has `catchup: true` but its timer has no systemd stamp, \
                 so a run missed while the machine was off is never caught up",
                spec.name
            ),
            fix: "Run `ivar agent sync`; it writes the stamp at the schedule's last occurrence."
                .to_owned(),
        })
        .collect()
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/hall/doctor/agent_timers.rs"]
mod tests;
