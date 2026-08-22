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
mod snapshot;
pub mod start;
pub mod status;

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
