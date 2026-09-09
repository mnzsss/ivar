//! Provider-neutral Run Receipt lifecycle actions.

use camino::Utf8PathBuf;

use crate::domain::feature::RunId;
use crate::domain::name::FeatureName;
use crate::domain::session::rfc3339_now;
use crate::error::Failure;
use crate::store::feature::run;
use crate::store::layout::Layout;

pub mod accept_revision;
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
