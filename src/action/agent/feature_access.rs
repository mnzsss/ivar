//! `access: feature`: an agent writes into its own feature worktree; the host
//! commits the result and previews delivery, never pushes.

use camino::{Utf8Path, Utf8PathBuf};

use super::run::Workspace;
use crate::action::Ctx;
use crate::action::feature::create::{self, CreateInput};
use crate::action::feature::deliver::{self, input::DeliverInput};
use crate::action::feature::promote::{self, PromoteInput};
use crate::domain::agent::container::Mount;
use crate::domain::agent::spec::AgentSpec;
use crate::domain::name::{BranchName, FeatureName, RepoName};
use crate::error::Failure;
use crate::infra::fs;
use crate::infra::proc::{self, Command};
use crate::store::layout::Layout;

pub(crate) struct Delivered {
    pub diff: Option<String>,
    pub fingerprint: Option<String>,
    pub error: Option<String>,
}

impl Delivered {
    fn failed(error: impl Into<String>) -> Self {
        Self {
            diff: None,
            fingerprint: None,
            error: Some(error.into()),
        }
    }
}

/// Ensures feature `agent-<name>` exists with the workdir repo promoted, and
/// returns its worktree as the only, writable, mount.
pub(crate) fn prepare(
    ctx: &Ctx,
    layout: &Layout,
    spec: &AgentSpec,
) -> Result<(Workspace, Utf8PathBuf), Failure> {
    let feature = spec.feature_name();
    let repo = spec
        .workdir_repo()
        .ok_or_else(|| Failure::failed("agent.invalid", "`access: feature` needs `workdir`"))?;
    if !fs::exists(&layout.feature_dir(&FeatureName::new(feature.clone())?))? {
        create::create(
            ctx,
            CreateInput {
                name: feature.clone(),
                branch: None,
                base: None,
                parent: None,
                via: None,
                strategy: None,
            },
        )?;
    }
    let worktree = layout.repo_worktree(&RepoName::new(repo)?, &BranchName::new(feature.clone())?);
    if !fs::exists(&worktree)? {
        promote::promote(
            ctx,
            PromoteInput {
                feature,
                repo: repo.to_owned(),
                base: None,
            },
        )?;
    }
    let target = format!("/workspace/{repo}");
    let sub = spec
        .workdir
        .as_deref()
        .and_then(|w| w.strip_prefix(repo))
        .unwrap_or("");
    let workspace = Workspace {
        mounts: vec![Mount {
            source: worktree.to_string(),
            target: target.clone(),
            read_only: false,
        }],
        workdir: format!("{target}{sub}"),
    };
    Ok((workspace, worktree))
}

// The agent could write anything in the worktree, so nothing in it is trusted:
// the git dir comes from the bare clone (never mounted) and its own `gitdir`
// record, hooks and fsmonitor are disabled, and the `.git` pointer is rewritten
// before host git reads it.
fn trusted_git_dir(bare: &Utf8Path, worktree: &Utf8Path) -> Option<Utf8PathBuf> {
    let expected = worktree.join(".git");
    fs::read_dir(&bare.join("worktrees"))
        .ok()?
        .into_iter()
        .find(|dir| {
            fs::read_text(&dir.join("gitdir"))
                .ok()
                .flatten()
                .is_some_and(|text| Utf8Path::new(text.trim()) == expected)
        })
}

fn host_git(git_dir: &Utf8Path, worktree: &Utf8Path, args: &[&str]) -> Result<String, String> {
    let command = Command::new("git")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
        ])
        .arg(format!("--git-dir={git_dir}"))
        .arg(format!("--work-tree={worktree}"))
        .args(args.iter().copied());
    match proc::capture(&command) {
        Ok(output) if output.success() => Ok(output.stdout),
        Ok(output) => Err(output.diagnostic()),
        Err(error) => Err(error.to_string()),
    }
}

/// Commits the run's changes on the host and records a delivery preview.
pub(crate) fn commit_and_preview(
    ctx: &Ctx,
    layout: &Layout,
    spec: &AgentSpec,
    worktree: &Utf8Path,
    run_id: &str,
) -> Delivered {
    let git_dir = RepoName::new(spec.workdir_repo().unwrap_or_default())
        .ok()
        .and_then(|repo| trusted_git_dir(&layout.repo_bare(&repo), worktree));
    let Some(git_dir) = git_dir else {
        return Delivered::failed("worktree is not registered in its bare clone");
    };
    let pointer = worktree.join(".git");
    let restored = fs::remove_path(&pointer)
        .and_then(|()| fs::write_atomic(&pointer, format!("gitdir: {git_dir}\n").as_bytes()));
    if restored.is_err() {
        return Delivered::failed("could not restore the worktree's .git pointer");
    }
    let git = |args: &[&str]| host_git(&git_dir, worktree, args);
    match git(&["status", "--porcelain"]) {
        Ok(status) if status.trim().is_empty() => {
            return Delivered {
                diff: None,
                fingerprint: None,
                error: None,
            };
        }
        Ok(_) => {}
        Err(error) => return Delivered::failed(error),
    }
    let message = format!("agent {}: run {run_id}", spec.name);
    if let Err(error) = git(&["add", "-A"]).and_then(|_| git(&["commit", "-m", &message])) {
        return Delivered::failed(error);
    }
    let diff = git(&["show", "--format=", "HEAD"]).ok();
    let preview = DeliverInput {
        feature: spec.feature_name(),
        preview: true,
        ..DeliverInput::default()
    };
    match deliver::deliver(ctx, preview) {
        Ok(report) => Delivered {
            diff,
            fingerprint: Some(report.value.preview.fingerprint),
            error: None,
        },
        Err(failure) => Delivered {
            diff,
            fingerprint: None,
            error: Some(failure.what),
        },
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/action/agent/feature_access.rs"]
mod tests;
