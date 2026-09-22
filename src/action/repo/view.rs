//! `ivar repo view` — an interactive, multi-shell view over the hall's
//! repos on their default branches.
//!
//! One shell per declared repo, each running in that repo's default-branch
//! worktree (`.ivar/repos/<repo>/<default-branch>/`). The host loop is the
//! one `ivar feature view` drives; only the source differs — the manifest
//! rather than a feature's promotions.
//!
//! **This is an inspection view.** Those worktrees are held read-only
//! whenever a session guards them (`action/session/view.rs` clears the
//! write bits of every repo a session does not promote), and nothing here
//! lifts that guard. `ivar repo pull` remains the verb that refreshes a
//! default branch.
//!
//! A repo whose worktree is not materialised is listed with the reason and
//! never spawned into — the action knows the path is absent, so the driver
//! is told rather than left to fail.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::name::RepoName;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::git;
use crate::infra::proc;
use crate::infra::term;
use crate::store::manifest::Repo;
use crate::tui;
use crate::tui::driver::ShellSpec;
use crate::tui::widget::Row;

use super::super::{discover_hall, read_manifest};
use super::list::{RepoStatus, status_of};
use crate::action::Ctx;

/// What `ivar repo view` needs.
#[derive(Debug, Clone)]
pub struct ViewInput {
    /// Which declared repos to open; every declared repo when empty.
    pub repos: Vec<String>,
}

/// One repo's place in the view.
#[derive(Debug, Clone, Serialize)]
pub struct RepoView {
    /// The repo's name, as declared in `ivar.json`.
    pub name: RepoName,
    /// The branch its worktree is on.
    pub default_branch: String,
    /// The worktree the shell runs in.
    pub worktree: Utf8PathBuf,
    /// Whether a shell can actually start there.
    pub openable: bool,
    /// Why it cannot, when it cannot.
    pub reason: Option<String>,
}

/// What `ivar repo view` covered — a summary, since the interactive part
/// ends when the user quits the TUI.
#[derive(Debug, Clone, Serialize)]
pub struct ViewOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The repos in the view, in manifest order.
    pub repos: Vec<RepoView>,
}

impl WriteHuman for ViewOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(w, "Repos in {}:", self.root)?;
        for repo in &self.repos {
            match &repo.reason {
                None => writeln!(
                    w,
                    "  {}  {}  {}",
                    repo.name, repo.default_branch, repo.worktree
                )?,
                Some(reason) => writeln!(w, "  {}  {reason}", repo.name)?,
            }
        }
        let open = self.repos.iter().filter(|repo| repo.openable).count();
        writeln!(
            w,
            "{open} shell{} opened",
            if open == 1 { "" } else { "s" }
        )
    }
}

/// Open one shell per declared repo, in that repo's default-branch
/// worktree.
///
/// Refused (`Blocked`) when a named repo is not declared, or when no repo
/// in the view has a worktree to open — a sidebar where every row is an
#[allow(clippy::needless_pass_by_value)]
pub fn view(ctx: &Ctx, input: ViewInput) -> Outcome<ViewOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let git = git::System;

    let selected = select(manifest.repos(), &input.repos)?;
    let repos: Vec<RepoView> = selected
        .iter()
        .map(|repo| {
            let status = status_of(&git, &layout, repo);
            let reason = unavailable_reason(&status);
            RepoView {
                worktree: layout.repo_worktree(repo.name(), repo.default_branch()),
                name: status.name,
                default_branch: status.default_branch,
                openable: reason.is_none(),
                reason,
            }
        })
        .collect();

    if !repos.iter().any(|repo| repo.openable) {
        return Err(Failure::blocked(
            "repo.view_nothing_openable",
            "no repo in this view has a default-branch worktree",
        )
        .expected("at least one repo with a materialised worktree")
        .actual(format!("{} repo(s), none materialised", repos.len()))
        .fix(FixAction::safe(
            "repo.sync_first",
            "Materialise the hall's worktrees with `ivar sync`.",
        )));
    }

    let shell_program = proc::user_shell();
    let shells = repos
        .iter()
        .map(|repo| ShellSpec {
            cwd: repo.worktree.clone(),
            command: proc::Command::new(shell_program.clone()).cwd(&repo.worktree),
            unavailable: repo.reason.clone(),
        })
        .collect();
    // The sidebar's status column, both halves derived from `status_of`'s
    // probe. `feature view` shows worktree state there because its branch
    // is the same for every row; here the branch is the thing that varies
    // per repo, so it is what the column is worth spending on.
    let rows = repos
        .iter()
        .map(|repo| Row {
            label: repo.name.to_string(),
            status: if repo.openable {
                repo.default_branch.clone()
            } else {
                "unavailable".to_owned()
            },
        })
        .collect();

    // The interactive TUI needs a real terminal; on a pipe, report what a
    // view would have opened instead.
    if term::is_tty(term::Stream::Stdout) {
        tui::master_detail::run(tui::master_detail::ShellView {
            title: manifest.name().to_string(),
            rows,
            shells,
        })?;
    }

    Ok(Report::new(ViewOutcome {
        root: layout.root().to_path_buf(),
        repos,
    }))
}

/// The repos the input names, or every declared repo when it names none.
///
/// A name absent from `ivar.json` is refused rather than skipped: a typo
/// that silently opened the wrong set is worse than one that stops.
fn select<'a>(declared: &'a [Repo], requested: &[String]) -> Result<Vec<&'a Repo>, Failure> {
    if requested.is_empty() {
        return Ok(declared.iter().collect());
    }
    requested
        .iter()
        .map(|name| {
            declared
                .iter()
                .find(|repo| repo.name().as_str() == name)
                .ok_or_else(|| {
                    let names: Vec<&str> =
                        declared.iter().map(|repo| repo.name().as_str()).collect();
                    Failure::blocked(
                        "repo.not_declared",
                        format!("`{name}` is not in ivar.json"),
                    )
                    .expected("a repo declared in ivar.json")
                    .actual(format!("`{name}` is not declared"))
                    .fix(FixAction::safe(
                        "repo.declare_first",
                        format!("Declared repos: {}.", names.join(", ")),
                    ))
                })
        })
        .collect()
}

/// Why a repo cannot be opened, or `None` when it can.
///
/// Both answers point at `ivar sync`, which is what materialises a bare
/// clone and a default-branch worktree (`action/sync/mod.rs`).
fn unavailable_reason(status: &RepoStatus) -> Option<String> {
    if !status.bare_cloned {
        return Some("no bare clone; run `ivar sync`".to_owned());
    }
    if !status.default_worktree {
        return Some(format!(
            "no {} worktree; run `ivar sync`",
            status.default_branch
        ));
    }
    None
}

#[cfg(test)]
#[path = "../../../tests/unit/action/repo/view.rs"]
mod tests;
