//! `ivar session relay` — relay to a new session on the same feature under a
//! different provider.
//!
//! A thin alias over `session start --relay`: delegates to the same code path,
//! then reports the new session and, when present, the current run receipt.
//! A receipt is audit evidence, not a scheduler: relay never infers progress
//! or workstream counts.

use std::io;

use serde::Serialize;

use crate::action::Ctx;
use crate::action::execute::import_legacy;
use crate::domain::feature::{RunReceipt, RunStatus};
use crate::domain::name::FeatureName;
use crate::error::{Outcome, Report, WriteHuman};

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

/// Output of `ivar session relay`.
#[derive(Debug, Clone, Serialize)]
pub struct RelayOutcome {
    /// The new session's id.
    pub session_id: String,
    /// The feature this session is bound to.
    pub feature: FeatureName,
    /// The provider that ran the relayed session.
    pub provider: crate::domain::provider::Provider,
    /// The current run id, when this feature has an in-flight receipt.
    pub run_id: Option<String>,
    /// The current run status, when this feature has an in-flight receipt.
    pub run_status: Option<RunStatus>,
}

impl WriteHuman for RelayOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Session `{}` for feature `{}` relayed.",
            self.session_id, self.feature
        )?;
        writeln!(w, "Provider: {}", self.provider)?;
        match (&self.run_id, self.run_status) {
            (Some(id), Some(status)) => writeln!(w, "plan preserved · run {id} is {status}")?,
            _ => writeln!(w, "plan preserved")?,
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
    import_legacy(
        &layout,
        &feature_name,
        layout.plan_dir(&feature_name).join("plan.md"),
    )?;

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

    let receipt = RunReceipt::read(&layout, &feature_name)?;

    Ok(Report::with_warnings(
        RelayOutcome {
            session_id: start_outcome.session_id.clone(),
            feature: feature_name,
            provider: start_outcome.provider,
            run_id: receipt.as_ref().map(|receipt| receipt.id.to_string()),
            run_status: receipt.map(|receipt| receipt.status),
        },
        report.warnings.clone(),
    ))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/session/relay.rs"]
mod tests;
