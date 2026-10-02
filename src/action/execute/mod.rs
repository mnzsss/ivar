//! Provider-neutral Run Receipt lifecycle actions.

use camino::{Utf8Path, Utf8PathBuf};

use crate::action::Ctx;

use crate::domain::feature::RunId;
use crate::domain::name::FeatureName;
use crate::domain::session::rfc3339_now;
use crate::error::{Failure, FixAction};
use crate::store::feature::run;
use crate::store::layout::Layout;

pub mod accept_revision;
pub mod checkpoint;
pub mod finish;
pub mod interrupt;
pub mod plan_fingerprint;
mod snapshot;
pub mod start;
pub mod status;

use crate::action::session::lookup;
use crate::domain::feature::RunReceipt;
use crate::domain::name::SessionId;
use crate::domain::provider::Provider;

pub(crate) fn resolve_coordinator(
    layout: &Layout,
    feature: &FeatureName,
    receipt: &RunReceipt,
) -> Result<(SessionId, Provider), Failure> {
    match lookup::resolve(layout, None, Some(feature.as_str())) {
        Ok(session) => {
            if let Some(state) = session.state {
                Ok((session.id, state.provider))
            } else if let Some(coordinator) = receipt.current_coordinator() {
                Ok((coordinator.session.clone(), coordinator.provider))
            } else {
                Err(Failure::blocked(
                    "execute.session_state_missing",
                    "feature session has no state record",
                ))
            }
        }
        Err(failure) if failure.code == "session.not_found" => {
            if let Some(coordinator) = receipt.current_coordinator() {
                Ok((coordinator.session.clone(), coordinator.provider))
            } else {
                Err(failure)
            }
        }
        Err(failure) => Err(failure),
    }
}
pub(crate) fn plan_path(
    ctx: &Ctx,
    layout: &Layout,
    feature: &FeatureName,
    plan: Option<&str>,
) -> Utf8PathBuf {
    plan.map_or_else(
        || layout.plan_dir(feature).join("plan.md"),
        |plan| ctx.resolve(Utf8Path::new(plan)),
    )
}

/// The fix every "run has no receipt yet" refusal offers.
pub(crate) fn start_run_fix(feature: &FeatureName) -> FixAction {
    let command = format!("ivar feature execute start {feature}");
    FixAction::safe(
        "execute.start_run",
        format!("Start a run with `{command}`."),
    )
    .command(command)
}

pub(crate) fn run_missing(feature: &FeatureName) -> Failure {
    Failure::blocked("execute.run_missing", "no current run receipt exists")
        .fix(start_run_fix(feature))
}

pub(crate) fn plan_not_approved(feature: &FeatureName, what: &str) -> Failure {
    let command = format!("ivar plan approve {feature} plan");
    Failure::blocked("execute.plan_not_approved", what.to_owned()).fix(
        FixAction::safe(
            "execute.approve_plan",
            format!("Approve the plan with `{command}`, then run this again."),
        )
        .command(command),
    )
}

/// Abandoning a run discards the coordinator's place in it, so a human decides.
pub(crate) fn finish_or_interrupt_fix(feature: &FeatureName) -> FixAction {
    let command = format!("ivar feature execute interrupt {feature}");
    FixAction::unsafe_(
        "execute.finish_or_interrupt",
        format!(
            "Finish the run with `ivar feature execute finish {feature}`, accept a revision with \
             `ivar feature execute accept-revision {feature}`, or abandon it with `{command}`."
        ),
    )
    .command(command)
}

/// Preserve legacy execution evidence before an action reads or changes receipts.
pub(crate) fn import_legacy(
    layout: &Layout,
    feature: &FeatureName,
    plan_path: Utf8PathBuf,
) -> Result<(), Failure> {
    let _ = run::import(
        layout,
        feature,
        plan_path,
        RunId::new(uuid::Uuid::new_v4().to_string())?,
        &rfc3339_now(),
    )?;
    Ok(())
}
