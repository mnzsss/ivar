//! The unit-test adapter: linked from `src/lib.rs` as `crate::test_support`,
//! so library unit tests keep `use crate::test_support::…` unchanged.
//!
//! `#[cfg(test)]` items are not part of the compiled library, which is why
//! integration tests under `tests/` cannot see this module — they link
//! [`integration`](integration) instead. Both adapters pull their
//! implementation from [`shared`](shared), so the two boundaries share one
//! helper set rather than two drifting copies.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use camino::Utf8PathBuf;
use tempfile::TempDir;

use crate::action::Ctx;
use crate::action::hall::{self, InitInput};

#[path = "shared.rs"]
mod shared;

pub(crate) use shared::{
    canonical_temp_dir, empty_repo, git, hall_root, seeded_repo, utf8_temp_dir,
};

/// A canonicalised hall root with a freshly initialised hall named `acme`.
///
/// Shared by every action-layer unit-test module that needs a real hall to
/// operate against rather than a bare directory: initialising once here means
/// the 14 call sites that used to carry their own byte-identical copy stay in
/// sync automatically when `hall::init`'s input shape changes.
pub(crate) fn seeded_hall() -> (TempDir, Utf8PathBuf) {
    let (guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());
    hall::init(
        &ctx,
        InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
        },
    )
    .unwrap();
    (guard, root)
}
