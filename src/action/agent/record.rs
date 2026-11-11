//! After a successful run, commit what the agent wrote into the hall.
//!
//! An agent's `write:` roots are where it keeps its output; the ones inside
//! the hall (`reports/memory-health`, `brand/metricas`) are hall content that
//! nothing else commits. [`record_run`] hands them to [`sync_hall`] as the
//! operation's paths, so `git status` for those roots decides what the commit
//! holds and a root outside the hall (`/tmp`, `~/...`, another hall's
//! `.ivar`) is dropped there.

use camino::Utf8PathBuf;

use crate::action::hall::{OperationCommit, SyncRequest, Trigger, sync_hall};
use crate::domain::agent::receipt::{AgentRunOutcome, AgentRunReceipt};
use crate::domain::agent::spec::{AgentSpec, WritePath};
use crate::git;
use crate::store::layout::Layout;

/// Whether a run's output is committed to the hall: only one that succeeded.
pub(crate) fn records(receipt: &AgentRunReceipt) -> bool {
    receipt.outcome == AgentRunOutcome::Succeeded
}

/// The commit message: `docs(<agent>): Record the <AAAA-MM-DD> run`.
fn message(spec: &AgentSpec, receipt: &AgentRunReceipt) -> String {
    let day = receipt.started_at.get(..10).unwrap_or(&receipt.started_at);
    format!("docs({}): Record the {day} run", spec.name)
}

/// The agent's `write:` roots, absolute. Hall-relative ones resolve against
/// the hall, `~/` ones against `$HOME` (dropped when it is unset).
fn write_roots(layout: &Layout, spec: &AgentSpec) -> Vec<Utf8PathBuf> {
    let home = std::env::var("HOME").ok().map(Utf8PathBuf::from);
    spec.write
        .iter()
        .filter_map(|write| match write {
            WritePath::Absolute(path) => Some(path.clone()),
            WritePath::Hall(rel) => Some(layout.root().join(rel)),
            WritePath::Home(rel) => home.as_ref().map(|home| home.join(rel)),
        })
        .collect()
}

/// Commit the run's changes under the agent's `write:` roots inside the hall,
/// then everything else pending, and push. The warnings come back as
/// `code: what` lines for the receipt; the outcome of the run is never
/// touched.
pub(crate) fn record_run(
    layout: &Layout,
    spec: &AgentSpec,
    receipt: &AgentRunReceipt,
) -> Vec<String> {
    let roots = write_roots(layout, spec);
    let message = message(spec, receipt);
    sync_hall(
        layout,
        &git::System,
        &SyncRequest {
            trigger: Trigger::Automatic,
            operation: Some(OperationCommit {
                paths: &roots,
                message: &message,
            }),
        },
    )
    .warnings
    .into_iter()
    .map(|warning| format!("{}: {}", warning.code, warning.what))
    .collect()
}

#[cfg(test)]
#[path = "../../../tests/unit/action/agent/record.rs"]
mod tests;
