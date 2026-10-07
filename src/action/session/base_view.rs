//! The **effective base view** of an unpromoted repo in a feature session:
//! which worktree the session's view dir links for a repo the feature has
//! not promoted.
//!
//! - The nearest ancestor (walking `parent`) that promotes the repo and has
//!   its feature worktree on disk → that worktree ([`BaseView::Parent`]),
//!   never write-guarded by the child (its owner is working in it).
//! - Otherwise the root feature's explicit `base`, when it is not the repo's
//!   `default_branch` → the read-only base worktree
//!   `.ivar/repos/<repo>/<base>` ([`BaseView::BaseBranch`]).
//! - Otherwise the repo's `default_branch` worktree ([`BaseView::Default`]).
//!
//! A subfeature's own `base` is its parent's branch, so the chain walk —
//! not `domain::feature::effective_base`, which answers "what does promote
//! branch from" — is what answers "what does this session read".
//!
//! [`prepare`] is the git half, run by `session start`/`connect`/`convert`
//! before the view is materialised: it creates the base worktree of every
//! [`BaseView::BaseBranch`] repo and fast-forwards an existing one. It never
//! fails a session; every problem is a `session.base_*` warning, and the
//! view then links whatever [`resolve_on_disk`] finds.

use std::collections::BTreeSet;

use camino::Utf8PathBuf;

use crate::action::repo::pull::{self, PullStatus};
use crate::domain::feature::Feature;
use crate::domain::name::{BranchName, FeatureName};
use crate::error::{Failure, Warning};
use crate::git::Git;
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, Repo};

/// Where an unpromoted repo of a feature session is viewed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BaseView {
    /// The repo's `default_branch` worktree (today's behaviour).
    Default,
    /// The root feature's explicit base branch, viewed through the
    /// read-only base worktree `.ivar/repos/<repo>/<branch>`.
    BaseBranch(BranchName),
    /// The nearest ancestor that promotes the repo: its feature worktree.
    Parent {
        feature: FeatureName,
        branch: BranchName,
    },
}

impl BaseView {
    /// The worktree this view links: `.ivar/repos/<repo>/<branch>` for the
    /// default, base or parent branch.
    pub(crate) fn worktree(&self, layout: &Layout, repo: &Repo) -> Utf8PathBuf {
        let branch = match self {
            Self::Default => repo.default_branch(),
            Self::BaseBranch(branch) | Self::Parent { branch, .. } => branch,
        };
        layout.repo_worktree(repo.name(), branch)
    }

    /// `false` only for [`Self::Parent`]: a parent worktree's write bits are
    /// never touched — the parent session works in it.
    pub(crate) fn guards_read_only(&self) -> bool {
        !matches!(self, Self::Parent { .. })
    }
}

/// The intended view of `repo` for `feature`, without checking that a base
/// worktree exists on disk.
///
/// Walks `feature.parent` upward; the first ancestor that promotes `repo`
/// and has its worktree on disk is [`BaseView::Parent`]. A missing parent
/// record or a revisited name (a cycle) is [`BaseView::Default`]. At the
/// root, an explicit `base` other than the repo's `default_branch` is
/// [`BaseView::BaseBranch`]; anything else is [`BaseView::Default`].
///
/// # Errors
///
/// Returns [`Failure`] if an ancestor's `feature.json` cannot be read or a
/// worktree path cannot be inspected.
pub(crate) fn resolve(
    layout: &Layout,
    repo: &Repo,
    feature: &Feature,
) -> Result<BaseView, Failure> {
    let mut seen = BTreeSet::from([feature.name.clone()]);
    let mut next = feature.parent.clone();
    let mut root_base = feature.base.clone();
    while let Some(name) = next {
        if !seen.insert(name.clone()) {
            return Ok(BaseView::Default);
        }
        let Some(ancestor) = Feature::read(layout, &name)? else {
            return Ok(BaseView::Default);
        };
        if ancestor.is_promoted(repo.name())
            && fs::is_dir(&layout.repo_worktree(repo.name(), &ancestor.branch))?
        {
            return Ok(BaseView::Parent {
                feature: ancestor.name,
                branch: ancestor.branch,
            });
        }
        next = ancestor.parent;
        root_base = ancestor.base;
    }
    Ok(match root_base {
        Some(base) if base != *repo.default_branch() => BaseView::BaseBranch(base),
        _ => BaseView::Default,
    })
}

/// [`resolve`], then a [`BaseView::BaseBranch`] whose worktree is not on
/// disk falls back to [`BaseView::Default`] — what the view, the graph and
/// plan approve link to. Returns the view with the worktree it links.
///
/// # Errors
///
/// As [`resolve`].
pub(crate) fn resolve_on_disk(
    layout: &Layout,
    repo: &Repo,
    feature: &Feature,
) -> Result<(BaseView, Utf8PathBuf), Failure> {
    let view = match resolve(layout, repo, feature)? {
        BaseView::BaseBranch(base) if !fs::is_dir(&layout.repo_worktree(repo.name(), &base))? => {
            BaseView::Default
        }
        view => view,
    };
    let worktree = view.worktree(layout, repo);
    Ok((view, worktree))
}

/// Bring the base worktree of every repo `feature` does not promote and
/// views at an explicit base branch onto disk, at the remote's tip.
///
/// Never fails: a resolve, fetch, fast-forward or worktree-creation problem
/// becomes a warning whose subject is the repo name.
pub(crate) fn prepare(
    git: &impl Git,
    layout: &Layout,
    manifest: &Manifest,
    feature: &Feature,
) -> Vec<Warning> {
    let mut warnings = Vec::new();
    for repo in manifest.repos() {
        if feature.is_promoted(repo.name()) {
            continue;
        }
        match resolve(layout, repo, feature) {
            Ok(BaseView::BaseBranch(branch)) => {
                prepare_base(git, layout, repo, &branch, &mut warnings)
            }
            Ok(BaseView::Default | BaseView::Parent { .. }) => {}
            Err(failure) => warnings.push(Warning::new(
                "session.base_resolve_failed",
                repo.name().as_str(),
                format!("cannot resolve the base view: {}", failure.what),
            )),
        }
    }
    warnings
}

/// Create (absent) or fast-forward (present) `repo`'s base worktree on `branch`.
fn prepare_base(
    git: &impl Git,
    layout: &Layout,
    repo: &Repo,
    branch: &BranchName,
    warnings: &mut Vec<Warning>,
) {
    let subject = repo.name().as_str();
    let dest = layout.repo_worktree(repo.name(), branch);
    match fs::is_dir(&dest) {
        Ok(true) => {
            warnings.extend(refresh_warning(
                subject,
                branch,
                pull::refresh_worktree(git, &dest, branch),
            ));
            return;
        }
        Ok(false) => {}
        Err(error) => {
            warnings.push(Warning::new(
                "session.base_worktree_failed",
                subject,
                format!("cannot inspect the base worktree `{dest}`: {error}"),
            ));
            return;
        }
    }

    // Fetch first: a branch the origin gained after the clone exists only
    // as `refs/remotes/origin/<branch>`, which `add_worktree` checks out as
    // a new tracking branch.
    let bare = layout.repo_bare(repo.name());
    let fetched = match git.fetch(&bare) {
        Ok(()) => true,
        Err(error) => {
            warnings.push(Warning::new(
                "session.base_refresh_failed",
                subject,
                format!("cannot fetch base `{branch}`: {error}"),
            ));
            false
        }
    };
    let created = dest
        .parent()
        .map_or(Ok(()), fs::ensure_dir)
        .map_err(|error| error.to_string())
        .and_then(|()| {
            git.add_worktree(&bare, &dest, branch.as_str())
                .map_err(|error| error.to_string())
        });
    if let Err(error) = created {
        // "Absent" only when the fetch worked: offline, a branch that exists
        // only remotely is unknown, not missing.
        let absent = fetched
            && git
                .list_branches(&bare)
                .is_ok_and(|branches| !branches.iter().any(|name| name == branch.as_str()));
        warnings.push(if absent {
            Warning::new(
                "session.base_absent",
                subject,
                format!(
                    "base `{branch}` not found in `{subject}`; using `{}`",
                    repo.default_branch()
                ),
            )
        } else {
            Warning::new(
                "session.base_worktree_failed",
                subject,
                format!("cannot create the base worktree `{branch}`: {error}"),
            )
        });
        return;
    }
    // A local `branch` copied by the original bare clone may be behind the
    // remote: start the new worktree from the remote tip.
    if fetched {
        warnings.extend(refresh_warning(
            subject,
            branch,
            pull::refresh_worktree(git, &dest, branch),
        ));
    }
}

/// The warning a base-worktree refresh owes, if any.
fn refresh_warning(subject: &str, branch: &BranchName, status: PullStatus) -> Option<Warning> {
    match status {
        PullStatus::Refreshed | PullStatus::Resolved => None,
        PullStatus::Failed { reason } => Some(Warning::new(
            "session.base_refresh_failed",
            subject,
            format!("cannot refresh base `{branch}`: {reason}"),
        )),
        PullStatus::Skipped { reason, .. } => Some(Warning::new(
            "session.base_refresh_skipped",
            subject,
            reason,
        )),
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/action/session/base_view.rs"]
mod tests;
