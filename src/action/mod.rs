//! One function per leaf command — the unit of behaviour.
//!
//! See ARCHITECTURE.md, "1. `action` is the unit, and it has one output
//! shape": every verb is shaped
//!
//! ```text
//! fn verb(ctx: &Ctx, input: Input) -> Outcome<Outcome_>
//! ```
//!
//! `Outcome_` is `Serialize`. `--json` prints it; the human surface formats
//! the *same value*. There is exactly one code path that computes what to
//! show, so the two surfaces cannot drift. `action` returns data — it never
//! prints; rendering is `bin/ivar.rs`'s job.
//!
//! `action` may import anything below it (`domain`, `store`, `git`,
//! `harness`, `tui`, `infra`) but never `cli` — see `tests/architecture.rs`,
//! which enforces this lexically over every file in this directory.

pub mod feature;
pub mod hall;
pub mod plan;
pub mod provider;
pub mod repo;
pub mod session;
pub mod skill;
pub mod sync;

use camino::{Utf8Path, Utf8PathBuf};

use crate::error::{Failure, FixAction};
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

/// Ambient context every action reads from.
///
/// Built once: from the real process in `bin/ivar.rs`
/// (`std::env::current_dir()`), and from a tempdir in every test. No action
/// calls `std::env::current_dir()` itself — routing "where am I running
/// from" through `Ctx` is what lets a test point it at a tempdir without
/// touching the test process's real working directory.
#[derive(Debug, Clone)]
pub struct Ctx {
    /// The directory `ivar` is running from.
    pub cwd: Utf8PathBuf,
}

impl Ctx {
    /// Build a `Ctx` rooted at `cwd`.
    #[must_use]
    pub fn new(cwd: impl Into<Utf8PathBuf>) -> Self {
        Self { cwd: cwd.into() }
    }

    /// Resolve `path` against this context: an absolute path passes through
    /// unchanged; a relative one joins onto [`Self::cwd`].
    #[must_use]
    pub fn resolve(&self, path: &Utf8Path) -> Utf8PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.cwd.join(path)
        }
    }
}

/// Discover the hall containing [`Ctx::cwd`], or a [`Failure`] saying there
/// is none. Every verb except `init` starts here.
pub(crate) fn discover_hall(ctx: &Ctx) -> Result<Layout, Failure> {
    Layout::discover(&ctx.cwd)?.ok_or_else(|| {
        Failure::blocked(
            "hall.not_found",
            format!("no hall at `{}` or above it", ctx.cwd),
        )
        .expected("an ivar.json in this directory or an ancestor")
        .actual("no ivar.json found walking up to the filesystem root")
        .fix(FixAction::safe("hall.init", "Create a hall here first.").command("ivar init"))
    })
}

/// Read the manifest [`discover_hall`] just proved exists.
///
/// The `None` arm is a genuine race — `ivar.json` deleted between the walk-up
/// and this read — not an impossible state, so it gets a real message rather
/// than an `unwrap`.
pub(crate) fn read_manifest(layout: &Layout) -> Result<Manifest, Failure> {
    Manifest::read(layout)?.ok_or_else(|| {
        Failure::blocked(
            "hall.manifest_vanished",
            format!("`{}` disappeared while reading it", layout.manifest()),
        )
        .expected("the ivar.json that was there a moment ago")
        .actual("it is gone")
        .fix(FixAction::safe("hall.retry", "Run the command again.").command("ivar sync"))
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn resolve_passes_an_absolute_path_through_unchanged() {
        let ctx = Ctx::new("/somewhere");
        assert_eq!(
            ctx.resolve(Utf8Path::new("/elsewhere")),
            Utf8PathBuf::from("/elsewhere")
        );
    }

    #[test]
    fn resolve_joins_a_relative_path_onto_cwd() {
        let ctx = Ctx::new("/somewhere");
        assert_eq!(
            ctx.resolve(Utf8Path::new("child")),
            Utf8PathBuf::from("/somewhere/child")
        );
    }

    #[test]
    fn resolve_treats_dot_as_cwd_itself() {
        let ctx = Ctx::new("/somewhere");
        assert_eq!(
            ctx.resolve(Utf8Path::new(".")),
            Utf8PathBuf::from("/somewhere/.")
        );
    }
}
