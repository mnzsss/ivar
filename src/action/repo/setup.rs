//! `ivar repo setup <repo>` — run one repo's setup script in isolation.
//!
//! `ivar sync` runs every repo's setup script when it needs running; this
//! verb does the same for exactly one repo, through the same function
//! ([`crate::action::sync::run_setup_script`]), so the two paths share the
//! receipt logic and cannot drift.
//!
//! The receipt is respected: a script whose content has not changed since its
//! last run is reported as already-run and not executed again. `--force-setup`
//! ignores the receipt and runs it anyway — the same flag `ivar sync` honours.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::name::RepoName;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::git::{self, Git, TargetState};
use crate::infra::fs;

use super::super::{discover_hall, read_manifest};
use crate::action::Ctx;
use crate::action::sync::{self, Change};

/// What `ivar repo setup` needs.
#[derive(Debug, Clone)]
pub struct SetupInput {
    /// The repo whose setup script to run, as declared in `ivar.json`.
    pub repo: String,
    /// Ignore the receipt and run the setup script even if its content has
    /// not changed since the last run.
    pub force: bool,
}

/// What `ivar repo setup` did.
#[derive(Debug, Clone, Serialize)]
pub struct SetupOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The repo whose setup script was (or was not) run.
    pub repo: RepoName,
    /// The script's expected location, `.ivar/setups/<repo>.sh`.
    pub script: Utf8PathBuf,
    /// What happened to the setup state. `None` when the repo has no script —
    /// the explained no-op.
    pub change: Option<Change>,
    /// Anything worth saying beyond the change — why the script was skipped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl WriteHuman for SetupOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        match self.change {
            None => writeln!(
                w,
                "No setup script for `{}` at `{}` — nothing to run.",
                self.repo, self.script
            ),
            Some(Change::Created) => writeln!(w, "Ran setup script for `{}`.", self.repo),
            Some(Change::Updated) => writeln!(w, "Re-ran setup script for `{}`.", self.repo),
            Some(Change::Unchanged) => match &self.detail {
                Some(detail) => {
                    writeln!(w, "Setup script for `{}` not run — {detail}.", self.repo)
                }
                None => writeln!(w, "Setup script for `{}` not run.", self.repo),
            },
            Some(other) => writeln!(w, "Setup script for `{}`: {:?}", self.repo, other),
        }
    }
}

/// Run `input.repo`'s setup script in its default worktree, if it has one.
///
/// Blocked when the repo is not registered in `ivar.json`, or when its
/// default-branch worktree does not exist (nothing for the script to run in —
/// `ivar sync` materialises worktrees). A repo without a script is an
/// explained no-op, not an error.
pub fn setup(ctx: &Ctx, input: SetupInput) -> Outcome<SetupOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let git = git::System;

    let name = RepoName::new(input.repo)?;
    let repo = manifest
        .repos()
        .iter()
        .find(|repo| repo.name() == &name)
        .ok_or_else(|| {
            Failure::blocked(
                "repo.setup_repo_not_found",
                format!("repo `{name}` is not in ivar.json"),
            )
            .expected("a repo declared in `ivar.json`")
            .actual(format!("`{name}` is not among the declared repos"))
            .fix(FixAction::safe(
                "repo.add_first",
                format!("Add it first with `ivar repo add {name}`, then run setup again."),
            ))
        })?;

    let script = layout.setup_script(&name);

    // A repo without a script is an explained no-op — nothing to run, and no
    // worktree requirement either.
    if !fs::is_file(&script)? {
        return Ok(Report::new(SetupOutcome {
            root: layout.root().to_path_buf(),
            repo: name,
            script,
            change: None,
            detail: None,
        }));
    }

    let worktree = layout.repo_worktree(&name, repo.default_branch());
    match git.target_state(&worktree)? {
        TargetState::Repository => {}
        _ => {
            return Err(Failure::blocked(
                "repo.setup_worktree_missing",
                format!("`{worktree}` is not a materialised worktree for `{name}`"),
            )
            .expected("the repo's default-branch worktree to exist")
            .actual("it is missing, or is not a git worktree")
            .fix(
                FixAction::safe(
                    "repo.sync_first",
                    "Run `ivar sync` to materialise the worktree, then run setup again.",
                )
                .command("ivar sync"),
            ));
        }
    }

    let surface = format!("repo {name}");
    let (change, detail) =
        match sync::run_setup_script(&git, &layout, repo, &worktree, &surface, input.force)? {
            None => (None, None),
            Some(entry) => (Some(entry.change), entry.detail),
        };
    Ok(Report::new(SetupOutcome {
        root: layout.root().to_path_buf(),
        repo: name,
        script,
        change,
        detail,
    }))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/repo/setup.rs"]
mod tests;
