//! `hall`: the verbs that act on the hall itself — `init · status · doctor ·
//! cleanup · migrate`. See ARCHITECTURE.md's module map.
//!
//! Each verb lives in its own file (`init.rs`, `status.rs`, `doctor.rs`,
//! `migrate.rs`, `cleanup.rs`); this facade owns what they share — the
//! discovery/read helpers — and reexports the public command surface so
//! `action::hall::{init, status, …}` keeps its established names. The
//! interactive prompt these verbs used to share now lives in the
//! [`Confirm`](crate::action::confirm) seam on [`Ctx`], decided once by
//! `bin/ivar.rs`.

use crate::error::Failure;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::Ctx;

mod cleanup;
mod doctor;
mod init;
mod migrate;
mod status;

pub use cleanup::CleanupOutcome;
pub use cleanup::cleanup;
pub use doctor::Diagnosis;
pub use doctor::DoctorOutcome;
pub use doctor::doctor;
pub use init::InitInput;
pub use init::InitOutcome;
pub use init::init;
pub use migrate::MigrateOutcome;
pub use migrate::migrate;
pub use status::RepoStatusEntry;
pub use status::StatusOutcome;
pub use status::status;

/// The hall [`Ctx::cwd`] is inside, or a [`Failure`] saying there is none.
/// Shared with the other verbs that operate on the current hall.
fn discover_hall(ctx: &Ctx) -> Result<Layout, Failure> {
    super::discover_hall(ctx)
}

/// The manifest [`Layout::discover`] just proved exists.
fn read_manifest(layout: &Layout) -> Result<Manifest, Failure> {
    super::read_manifest(layout)
}

#[cfg(test)]
#[path = "../../../tests/unit/action/hall.rs"]
mod tests;
