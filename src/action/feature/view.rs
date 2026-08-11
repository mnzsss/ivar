//! `ivar feature view <feature>` — an interactive, multi-shell view over a
//! feature's promoted repos.
//!
//! One shell per promoted repo, each running in its repo's feature worktree
//! (`.ivar/repos/<repo>/<branch>/`). The TUI is a sidebar of repos with a
//! focused shell on the right: the prefix key opens navigation (`j`/`k`
//! move, `Enter` focuses a repo's shell, `q` quits), and `<prefix> [` opens
//! scrollback. The prefix is `Ctrl+O` by default and configurable with
//! `IVAR_TUI_PREFIX` (`ctrl+<letter>` or `f<number>`) — `Ctrl+B` is taken by
//! `tmux` and by Orca, so a view running inside either would never see it.
//! The panel's title bar always names the current prefix.
//! Shells spawn lazily — a repo's shell starts the first time it is focused,
//! and keeps running (and accumulating scrollback) while another repo is on
//! screen.
//!
//! The TUI runs only when stdout is a terminal. On a pipe it does the same
//! validation and reports what a view would open, so the command stays
//! scriptable the way `session start` is.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::feature::{Feature, WorktreeState};
use crate::domain::name::{FeatureName, RepoName};
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::proc;
use crate::infra::term;
use crate::tui;
use crate::tui::driver::ShellSpec;
use crate::tui::widget::Row;

use super::super::discover_hall;
use crate::action::Ctx;

/// What `ivar feature view` needs.
#[derive(Debug, Clone)]
pub struct ViewInput {
    /// The feature to view.
    pub feature: String,
}

/// What `ivar feature view` covered — a summary, since the interactive part
/// ends when the user quits the TUI.
#[derive(Debug, Clone, Serialize)]
pub struct ViewOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The feature.
    pub feature: FeatureName,
    /// The branch every promoted repo's worktree is on.
    pub branch: String,
    /// The promoted repos, in name order — one shell each.
    pub repos: Vec<RepoName>,
}

impl WriteHuman for ViewOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Feature `{}` (branch: {}) in {}:",
            self.feature, self.branch, self.root
        )?;
        for repo in &self.repos {
            writeln!(w, "  {repo}")?;
        }
        writeln!(
            w,
            "{} shell{} opened",
            self.repos.len(),
            if self.repos.len() == 1 { "" } else { "s" }
        )
    }
}

/// View `input.feature`: collect its promoted repos and their worktrees, and
/// open one shell per repo in the Feature View TUI.
///
/// Refused (`Blocked`) when the feature does not exist or has nothing
/// promoted — a view with an empty sidebar would only confuse.
pub fn view(ctx: &Ctx, input: ViewInput) -> Outcome<ViewOutcome> {
    let layout = discover_hall(ctx)?;
    let name = FeatureName::new(input.feature)?;

    let feature = Feature::read(&layout, &name)?.ok_or_else(|| {
        Failure::blocked(
            "feature.not_found",
            format!("feature `{name}` does not exist"),
        )
        .expected("an existing feature")
        .actual(format!("`{name}` has no feature.json"))
        .fix(FixAction::safe(
            "feature.create_first",
            format!("Create it first with `ivar feature create {name}`."),
        ))
    })?;

    if feature.promotions.is_empty() {
        return Err(Failure::blocked(
            "feature.view_no_repos",
            format!("feature `{name}` has no repos promoted"),
        )
        .expected("at least one promoted repo to open a shell in")
        .actual("no promotions recorded")
        .fix(FixAction::safe(
            "feature.promote_first",
            format!("Promote a repo first with `ivar feature promote {name} <repo>`."),
        )));
    }

    // Promotions are a BTreeMap, so `keys()` is already repo-name order — the
    // sidebar order and the shell list agree.
    let repos: Vec<RepoName> = feature.promotions.keys().cloned().collect();
    let shell_program = user_shell();
    let shells = repos
        .iter()
        .map(|repo| {
            let worktree = layout.repo_worktree(repo, &feature.branch);
            ShellSpec {
                label: repo.to_string(),
                cwd: worktree.clone(),
                command: proc::Command::new(shell_program.clone()).cwd(&worktree),
            }
        })
        .collect();
    let rows = repos
        .iter()
        .map(|repo| Row {
            label: repo.to_string(),
            status: state_word(
                feature
                    .worktree_state(repo)
                    .unwrap_or(WorktreeState::Pending),
            )
            .to_owned(),
        })
        .collect();

    // The interactive TUI needs a real terminal; on a pipe, report what a
    // view would have opened instead.
    if term::is_tty(term::Stream::Stdout) {
        tui::master_detail::run(tui::master_detail::FeatureView {
            title: name.to_string(),
            rows,
            shells,
        })?;
    }

    Ok(Report::new(ViewOutcome {
        root: layout.root().to_path_buf(),
        feature: name,
        branch: feature.branch.to_string(),
        repos,
    }))
}

/// The shell each repo's view spawns: the user's `SHELL`, or `bash`.
fn user_shell() -> String {
    resolve_shell(std::env::var("SHELL").ok().as_deref())
}

/// Pure half of [`user_shell`], so the fallback is testable without touching
/// the process environment.
#[must_use]
fn resolve_shell(shell: Option<&str>) -> String {
    shell
        .map(str::to_owned)
        .unwrap_or_else(|| "bash".to_owned())
}

/// The one-word status the sidebar shows for a promoted repo — the same
/// words `feature status` uses.
fn state_word(state: WorktreeState) -> &'static str {
    match state {
        WorktreeState::Pending => "pending",
        WorktreeState::Ready => "ready",
        WorktreeState::Failed => "failed",
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/action/feature/view.rs"]
mod tests;
