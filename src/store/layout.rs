//! Every path under a hall is computed here. Nowhere else.
//!
//! The predecessor scattered path construction across ten modules, and
//! consolidating seven dotdirs into one meant touching all of them. This module
//! is the answer to that: a `Layout` holds the hall root and hands out typed
//! paths, so the next reorganisation is one file.
//!
//! # The layout, as decided
//!
//! ```text
//! <hall>/
//!   ivar.json                          the manifest — COMMITTED, and outside .ivar/
//!                                      on purpose: it is the identity file and has
//!                                      to be visible in a pull request review.
//!   .ivar/
//!     state.json                       local hall state (gitignored)
//!     graph.db                         the code graph, derived and disposable
//!     graph.lock                       graph index writer lock
//!     memory.db                        the hall memory; only `ivar memory clean` deletes it
//!     memory.lock                      hall memory writer lock (the graph lock before the split)
//!     hall-sync.lock                   held while the hall's commits are made and pushed
//!     repos/<repo>/.bare/              the bare clone
//!     repos/<repo>/<branch>/           a worktree off that bare
//!     features/<feature>/              promotion records and local working documents:
//!                                      requirements.md · analysis.md · plan.md ·
//!                                      tasks/ · discovery.md (gitignored)
//!     features/<feature>/planning/     approval-gate state (approvals.json)
//!     features/<feature>/execution/    the current Run Receipt (run.json)
//!     features/<feature>/execution/archive/runs/<run-id>.json
//!                                      terminal receipts, immutable
//!     features/<feature>/execution/archive/boards/<hash>.json
//!                                      raw normalized legacy boards, content-addressed
//!     features/<feature>/integration/  throwaway local-integration staging
//!                                      worktrees (candidate/source per repo)
//!     features/<feature>/sessions/<id>/  feature-session view dirs
//!     sessions/<id>/                   discovery-session view dirs
//!     secrets/                         secret material (gitignored — see below):
//!                                      hand-maintained per-repo secrets, plus
//!                                      mcp.env for Ivar-managed MCP credentials
//!     setups/<repo>.sh                 per-repo setup scripts — COMMITTED
//!     setups/<repo>.session.sh         per-repo session hooks — COMMITTED
//!     skills/                          hall-scoped skills — COMMITTED
//!     skills-local/                    personal skills (gitignored)
//!     scripts/                         hall scripts the team shares — COMMITTED
//!     scripts-local/                   personal hall scripts (gitignored)
//!   docs/                              flat topic documentation (product/, updates/,
//!                                      repo-relations/) — COMMITTED
//!   .claude/  .opencode/               harness-dictated. These are the TARGET of the
//!                                      view dir's symlinks, not the source, which is
//!                                      why they stay at the root.
//! ```
//!
//! Note that children inside `.ivar/` carry no leading dot. One dotdir at the top,
//! plain names underneath.
//!
//! # Contract
//!
//! - `Layout::discover(from: &Utf8Path)` — walk **up** from a directory looking for
//!   `ivar.json`, the way `git` finds `.git`. Returns `Ok(None)` when there is no
//!   hall above it; that is the normal "you are not in a hall" case, not an error.
//!   The one way this fails is `from` itself not being resolvable (it does not
//!   exist, a component is unreadable, …) — resolved once, up front, which is also
//!   what keeps the walk from chasing a moving target forever.
//! - `Layout::at(root)` — for `init`, where the root is chosen rather than found.
//! - Accessors returning `Utf8PathBuf`, taking validated newtypes from
//!   [`crate::domain::name`] rather than `&str`: `manifest()`, `state()`,
//!   `repo_bare(&RepoName)`, `repo_worktree(&RepoName, &BranchName)`,
//!   `feature_dir(&FeatureName)`, `planning_dir(&FeatureName)`,
//!   `execution_dir(&FeatureName)`, `run_receipt(&FeatureName)`,
//!   `archived_run(&FeatureName, &RunId)`, `archived_board(&FeatureName, &str)`,
//!   `feature_session(&FeatureName, &SessionId)`, `discovery_session(&SessionId)`,
//!   `hall_setups()`, `setup_script(&RepoName)`, `session_hook(&RepoName)`, `secrets_dir()`,
//!   `hall_skills()`, `hall_scripts()`, `hall_scripts_local()`, `plan_dir(&FeatureName)`, `discovery_doc(&FeatureName)`,
//!   `docs_memory_dir()`, `page_dir(&PageKind)`, `update_pages_dir()`, `graph_db()`, `graph_lock()`, `memory_db()`, `memory_lock()`,
//!   `harness_dir(&Provider)`, `commands_dir(&Provider)`.
//! - `gitignore_lines()` — the exact patterns the hall's `.gitignore` needs.
//!
//! # Why `secrets/` needs no gitignore line of its own
//!
//! The hall's `.gitignore` excludes `.ivar/*` and negates exactly the three
//! committed children. Anything else under `.ivar/` is therefore ignored by
//! construction, and `secrets/` is deliberately placed there rather than at the
//! hall root for precisely that reason: a secrets directory that depends on
//! someone remembering to add a line is a secrets directory that eventually
//! leaks. Adding a negation for a personal folder would be the trap described below;
//! not adding one is the whole design.
//!
//! # The gitignore trap
//!
//! It must be `.ivar/*` plus `!.ivar/skills/`, `!.ivar/setups/` and
//! `!.ivar/scripts/` — **never**
//! `.ivar/`. Git does not re-include a child of an excluded directory, so `.ivar/`
//! followed by negations silently drops the hall's committed children from
//! version control. Silently. Put a test on this; the failure mode is invisible
//! until a teammate clones and the skills (or the setup scripts, or the hall scripts) are not there.
//!
//! Two further lines — `.claude/commands/ivar-*.md` and
//! `.opencode/commands/ivar-*.md` — keep the shipped workflow commands `ivar`
//! materialises out of the hall's git history. They match only the reserved
//! `ivar-*` prefix, so a user's own commands remain committable.
//!
//! `docs/reference/on-disk-format.md` marks the committed children of `.ivar/` —
//! `skills/`, `setups/` and `scripts/` — and `docs/architecture/environment-contract.md`
//! independently calls `.ivar/setups/<repo>.sh` "committed" in its own words. The
//! "two traps" callout earlier in that same document names only `skills/` in its
//! `.gitignore` example, which undercounts by one line against its own later
//! text; `gitignore_lines()` here follows the on-disk-format contract (both
//! negated) rather than that abbreviated example.
//!
//! # No legacy names
//!
//! The manifest has already been renamed once (`workdir.json` → `hall.json` →
//! `ivar.json`). Do **not** port a fallback for either old name. A fresh
//! implementation should not be born carrying compatibility debt it never had.

use camino::{Utf8Path, Utf8PathBuf};

use crate::domain::feature::RunId;
use crate::domain::name::{BranchName, FeatureName, RepoName, SessionId};
use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction};

/// The manifest's filename. Never one of its two predecessors — see the module
/// doc comment.
const MANIFEST_FILE_NAME: &str = "ivar.json";

/// The one dotdir everything `ivar` manages lives under.
const IVAR_DIR: &str = ".ivar";

/// The current Run Receipt's filename, under a feature's execution directory.
const RUN_FILE: &str = "run.json";

/// Every path under a hall, computed from its root.
///
/// See the module doc comment for the full layout this hands out paths for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    root: Utf8PathBuf,
}

impl Layout {
    /// Build a `Layout` at a root that has already been chosen.
    ///
    /// This is `ivar init`'s case: there is nothing to find yet, because the
    /// hall does not exist on disk until this call's caller creates it.
    #[must_use]
    pub fn at(root: impl Into<Utf8PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Walk up from `from` looking for `ivar.json`, the way `git` finds `.git`.
    ///
    /// `Ok(None)` means there is no hall above `from` — the normal "you are not
    /// in a hall" case, not an error at this layer.
    ///
    /// `from` is resolved to an absolute, symlink-free path exactly once, up
    /// front; every step after that is pure string arithmetic on
    /// [`Utf8Path::parent`], which strictly shortens the path and therefore
    /// cannot loop forever. The only way this returns `Err` is that first
    /// resolution failing — `from` does not exist, or a component of it is not
    /// reachable.
    ///
    /// # Errors
    ///
    /// Returns [`DiscoverError::Unresolvable`] if `from` cannot be
    /// canonicalized (it does not exist, or a component of it is not
    /// reachable).
    pub fn discover(from: &Utf8Path) -> Result<Option<Self>, DiscoverError> {
        let mut current =
            from.canonicalize_utf8()
                .map_err(|source| DiscoverError::Unresolvable {
                    path: from.to_path_buf(),
                    source,
                })?;

        loop {
            if current.join(MANIFEST_FILE_NAME).is_file() {
                return Ok(Some(Self::at(current)));
            }
            match current.parent() {
                Some(parent) => current = parent.to_path_buf(),
                None => return Ok(None),
            }
        }
    }

    /// The hall root — the directory `ivar.json` lives in.
    #[must_use]
    pub fn root(&self) -> &Utf8Path {
        &self.root
    }
}

mod features;
mod hall;

/// Why [`Layout::discover`] could not even start walking. This is the only
/// fallible operation in this module — everything else is pure path
/// arithmetic.
#[derive(Debug, thiserror::Error)]
pub enum DiscoverError {
    /// The starting path could not be resolved to an absolute, symlink-free
    /// form: it does not exist, or a component of it is not reachable.
    #[error("could not resolve `{path}`: {source}")]
    Unresolvable {
        path: Utf8PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl From<DiscoverError> for Failure {
    fn from(error: DiscoverError) -> Self {
        match error {
            DiscoverError::Unresolvable { path, source } => Failure::blocked(
                "layout.discover_unresolvable",
                format!("could not resolve `{path}` to look for a hall"),
            )
            .expected("a starting directory that exists and is reachable")
            .actual(source.to_string())
            .fix(FixAction::safe(
                "layout.check_starting_path",
                format!("Check that `{path}` exists and is reachable, then try again."),
            )),
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/store/layout.rs"]
mod tests;
