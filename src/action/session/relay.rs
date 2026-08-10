//! `ivar session relay` — relay to a new session on the same feature under a
//! different provider.
//!
//! A thin alias over `session start --relay`: delegates to the same code path,
//! then formats the outcome as four lines of human-readable output:
//!
//! ```text
//! Session `<id>` for feature `<name>` relayed.
//! Provider: <provider>
//! plan preserved · N of M steps done
//!
//! ```
//!
//! The third line reads the execution board's workstream status: `N` is the
//! count of completed workstreams, `M` is the total. When no board exists,
//! it shows `0 of 0`.

use std::io;

use serde::Serialize;

use crate::action::Ctx;
use crate::domain::feature::ExecutionBoard;
use crate::domain::name::FeatureName;
use crate::error::{Outcome, Report, WriteHuman};
use crate::store::layout::Layout;

use super::super::discover_hall;
use super::start;

/// What `ivar session relay` needs.
#[derive(Debug, Clone)]
pub struct RelayInput {
    /// The feature to relay on. Required — relay creates a new session on the
    /// same feature under a different provider.
    pub feature: String,
    /// The provider to relay to. Required — relay must switch providers.
    pub provider: String,
}

/// Output of `ivar session relay`: four lines of human-readable text.
#[derive(Debug, Clone, Serialize)]
pub struct RelayOutcome {
    /// The new session's id.
    pub session_id: String,
    /// The feature this session is bound to.
    pub feature: FeatureName,
    /// The provider that ran the relayed session.
    pub provider: crate::domain::provider::Provider,
    /// Steps done / total from the execution board (only when a board exists).
    pub steps_done: Option<u64>,
    pub steps_total: Option<u64>,
}

impl WriteHuman for RelayOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Session `{}` for feature `{}` relayed.",
            self.session_id, self.feature
        )?;
        writeln!(w, "Provider: {}", self.provider)?;
        match (self.steps_done, self.steps_total) {
            (Some(done), Some(total)) => {
                writeln!(w, "plan preserved · {done} of {total} steps done")?;
            }
            _ => {
                writeln!(w, "plan preserved · 0 of 0 steps done")?;
            }
        }
        // Fourth line: blank separator.
        writeln!(w)
    }
}

/// Relay: create a fresh session on the same feature under a different
/// provider. This is a thin wrapper around `start` with `relay=true` — if the
/// two paths diverge, that is the bug this operation exists to prevent.
pub fn relay(ctx: &Ctx, input: RelayInput) -> Outcome<RelayOutcome> {
    let layout = discover_hall(ctx)?;

    let feature_name = FeatureName::new(input.feature)?;

    // Delegate to start with relay flag — same gates, same logic.
    let report = start::start(
        ctx,
        start::StartInput {
            feature: Some(feature_name.to_string()),
            resume: false,
            provider: Some(input.provider.clone()),
            detached: true,
            relay: true,
        },
    )?;

    let start_outcome = &report.value;

    // Read the execution board for the step count.
    let (steps_done, steps_total) = read_board_steps(&layout, &feature_name);

    Ok(Report::with_warnings(
        RelayOutcome {
            session_id: start_outcome.session_id.clone(),
            feature: feature_name,
            provider: start_outcome.provider,
            steps_done,
            steps_total,
        },
        report.warnings.clone(),
    ))
}

/// Read the execution board and return (done, total) workstream counts.
fn read_board_steps(layout: &Layout, feature_name: &FeatureName) -> (Option<u64>, Option<u64>) {
    match ExecutionBoard::read(layout, feature_name) {
        Ok(Some(board)) => {
            let total = board.graph.workstreams.len() as u64;
            let done = board
                .graph
                .workstreams
                .iter()
                .filter(|ws| ws.status == crate::domain::feature::WorkstreamStatus::Done)
                .count() as u64;
            (Some(done), Some(total))
        }
        Ok(None) | Err(_) => (None, None),
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/action/session/relay.rs"]
mod tests;
