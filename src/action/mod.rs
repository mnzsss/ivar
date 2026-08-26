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

pub mod confirm;
pub mod execute;
pub mod feature;
pub mod hall;
pub mod mcp;
pub mod plan;
pub mod provider;
pub mod repo;
pub mod session;
pub mod skill;
pub mod sync;

use std::io;
use std::sync::Arc;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::error::{Failure, FixAction, WriteHuman};
use crate::infra::proc;
use crate::infra::progress::{self, Progress};

use self::confirm::Confirm;

/// The interpreter a setup script or session hook runs under.
///
/// Named explicitly rather than executing the script directly, so a script does
/// not need its executable bit set — a `.sh` file arriving through a `git
/// clone` on a filesystem that drops modes would otherwise fail with "permission
/// denied", which names the wrong problem. The script's own shebang is
/// advisory; this is what actually runs it.
pub(crate) const SETUP_INTERPRETER: &str = "bash";

/// The outcome of a verb that has nothing to report — it simply completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Done;

impl WriteHuman for Done {
    fn write_human(&self, _w: &mut impl io::Write) -> io::Result<()> {
        Ok(())
    }
}
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

/// Ambient context every action reads from.
///
/// Built once: from the real process in `bin/ivar.rs`
/// (`std::env::current_dir()`), and from a tempdir in every test. No action
/// calls `std::env::current_dir()` itself — routing "where am I running
/// from" through `Ctx` is what lets a test point it at a tempdir without
/// touching the test process's real working directory.
///
/// The progress sink rides here for the same reason: an action must not
/// decide whether anyone is watching, and threading a reporter through ~60
/// dispatch arms would put that decision in every one of them. It defaults to
/// [`progress::Silent`], so a verb that never asks for one — and every test —
/// behaves exactly as it did before there was a sink at all.
#[derive(Debug, Clone)]
pub struct Ctx {
    /// The directory `ivar` is running from.
    pub cwd: Utf8PathBuf,
    /// Where a long-running verb says what it is doing right now. Private:
    /// it is reached through [`Ctx::progress`], which hands out a `&dyn` so
    /// no action can hold on to the `Arc` past the call.
    progress: Arc<dyn Progress>,
    /// The confirmation seam for verbs that must ask before they act.
    /// Private: it is reached through [`Ctx::confirm`]. Defaults to
    /// [`confirm::reporter`]`(false)` — never consenting — which is also
    /// every test's case.
    confirm: Arc<dyn Confirm>,
}

impl Ctx {
    /// Build a `Ctx` rooted at `cwd`, reporting no progress and never
    /// consenting to a prompt.
    #[must_use]
    pub fn new(cwd: impl Into<Utf8PathBuf>) -> Self {
        Self {
            cwd: cwd.into(),
            progress: Arc::new(progress::Silent),
            confirm: confirm::reporter(false),
        }
    }

    /// The same context, reporting progress to `progress`.
    ///
    /// `bin/ivar.rs` is the only caller in the binary — it is the layer that
    /// knows whether `--json` was passed and whether stderr is a terminal, see
    /// [`progress::reporter`]. A test uses it to pass a recording sink.
    #[must_use]
    pub fn with_progress(mut self, progress: Arc<dyn Progress>) -> Self {
        self.progress = progress;
        self
    }

    /// The same context, confirming through `confirm`.
    ///
    /// `bin/ivar.rs` decides once whether this run may prompt (`--json`,
    /// `$CI`, and non-tty runs may not); a test installs a fixed answer.
    #[must_use]
    pub fn with_confirm(mut self, confirm: Arc<dyn Confirm>) -> Self {
        self.confirm = confirm;
        self
    }

    /// Where to report what is happening right now.
    ///
    /// Nothing written here reaches the outcome — see [`progress`] for why a
    /// transient line does not violate "an action returns data, it never
    /// prints".
    pub(crate) fn progress(&self) -> &dyn Progress {
        self.progress.as_ref()
    }

    /// Ask the confirmation seam. `bin/ivar.rs` decided whether the seam may
    /// actually prompt; an action only ever asks.
    pub(crate) fn confirm(&self, question: &str, caveat: Option<&str>) -> Result<bool, Failure> {
        self.confirm.confirm(question, caveat)
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

/// The `IVAR_*` environment core every worktree-scoped command shares: the
/// five variables ARCHITECTURE.md's "Environment contract" table checks in
/// both the setup-script and session-hook columns.
///
/// `sync`, `feature promote`, and the session hook each add their own
/// remaining variables (`IVAR_WORKTREE_KIND`, and where it applies
/// `IVAR_FEATURE`, `IVAR_SESSION_ID`, `IVAR_SESSION_PATH`) on top of what this
/// returns — those differ by site for reasons each site's own comment
/// explains, so they are not folded in here.
pub(crate) fn worktree_env(
    cmd: proc::Command,
    layout: &Layout,
    repo: &str,
    branch: &str,
    worktree: &Utf8Path,
) -> proc::Command {
    cmd.env("IVAR_HALL", layout.root().as_str())
        .env("IVAR_REPO", repo)
        .env("IVAR_BRANCH", branch)
        .env("IVAR_WORKTREE", worktree.as_str())
        .env("IVAR_SECRETS_DIR", layout.secrets_dir().as_str())
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
#[path = "../../tests/unit/action/mod.rs"]
mod tests;
