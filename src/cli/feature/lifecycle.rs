use camino::Utf8PathBuf;
use clap::Args;

use crate::action::feature::{
    close, create, delete, demote, integrate, promote, rebase, rename, reparent, status, view,
    workspace,
};

/// Arguments for `ivar feature create`.
#[derive(Debug, Args)]
pub struct FeatureCreateArgs {
    /// The feature's name — one path segment, unique within the hall.
    pub name: String,
    /// The branch to work on. Defaults to the feature's name. Use it to
    /// adopt a branch a feature name cannot spell, such as `feat/login`.
    #[arg(long)]
    pub branch: Option<String>,
    /// The branch new promotions should start from, per repo. Defaults to
    /// each repo's own default branch. Conflicts with `--parent`: a child's
    /// base is always derived from its immediate parent's branch.
    #[arg(long, conflicts_with = "parent")]
    pub base: Option<String>,
    /// The parent feature this subfeature integrates into. Conflicts with
    /// `--base`: the child's base is derived from the parent's branch.
    #[arg(long, conflicts_with = "base")]
    pub parent: Option<String>,
    /// This feature's integration via override: `pr` or `local`. Omitted,
    /// the hall default (or the embedded `local`) applies. Persisted at
    /// creation; there is no policy-configure command.
    #[arg(long)]
    pub via: Option<String>,
    /// This feature's integration strategy override: `squash`, `merge`, or
    /// `rebase`. Omitted, the hall default (or the embedded `squash`)
    /// applies. Persisted at creation.
    #[arg(long)]
    pub strategy: Option<String>,
}

/// Arguments for `ivar feature promote`.
#[derive(Debug, Args)]
#[command(allow_missing_positional = true)]
pub struct FeaturePromoteArgs {
    /// The feature to promote into.
    pub feature: Option<String>,
    /// The repo to promote onto the feature's branch.
    pub repo: String,
    /// Override the branch a new worktree starts from, for this repo only.
    /// Defaults to the feature's declared base, or the repo's default branch.
    #[arg(long)]
    pub base: Option<String>,
}

/// Arguments for `ivar feature demote`.
#[derive(Debug, Args)]
#[command(allow_missing_positional = true)]
pub struct FeatureDemoteArgs {
    /// The feature to demote from.
    pub feature: Option<String>,
    /// The repo to demote.
    pub repo: String,
}

/// Arguments for `ivar feature status`.
#[derive(Debug, Args)]
pub struct FeatureStatusArgs {
    /// The feature to inspect.
    pub feature: Option<String>,
    /// Render the feature's whole subtree — itself and every descendant, in
    /// deterministic pre-order — with each feature's derived state, repos,
    /// and blockers.
    #[arg(long)]
    pub recursive: bool,
}

/// Arguments for `ivar feature integrate`.
#[derive(Debug, Args)]
pub struct FeatureIntegrateArgs {
    /// The child feature to integrate.
    pub feature: Option<String>,
    /// The via override for this run: `pr` or `local`. Ignored once the
    /// first receipt froze the policy.
    #[arg(long)]
    pub via: Option<String>,
    /// The strategy override for this run: `squash`, `merge`, or `rebase`.
    /// Ignored once the first receipt froze the policy.
    #[arg(long)]
    pub strategy: Option<String>,
    /// The integration title, e.g. `feat: add checkout tax`: the squash or
    /// merge commit message on the parent, and with `--via pr` the PR title
    /// and merge subject. Defaults to `feat: integrate <child>`.
    #[arg(long)]
    pub name: Option<String>,
}

/// Arguments for `ivar feature reparent`.
#[derive(Debug, Args)]
pub struct FeatureReparentArgs {
    /// The child feature to move.
    pub child: Option<String>,
    /// The new parent feature. The child's `base` is rewritten to the new
    /// parent's branch in the same record write.
    #[arg(long)]
    pub parent: String,
}

/// Arguments for `ivar feature rename`.
#[derive(Debug, Args)]
pub struct FeatureRenameArgs {
    /// The feature to rename.
    pub feature: Option<String>,
    /// The feature's new name. Requires at least one of `--name`/`--branch`
    /// to differ from the current value.
    #[arg(long)]
    pub name: Option<String>,
    /// The feature's new branch. Requires at least one of `--name`/`--branch`
    /// to differ from the current value.
    #[arg(long)]
    pub branch: Option<String>,
}

/// Arguments for `ivar feature close`.
#[derive(Debug, Args)]
pub struct FeatureCloseArgs {
    /// The feature to close.
    pub name: Option<String>,
    /// How the feature ended: `delivered` or `abandoned`.
    #[arg(long)]
    pub outcome: String,
}

/// Arguments for `ivar feature delete`.
#[derive(Debug, Args)]
pub struct FeatureDeleteArgs {
    /// The feature to delete.
    pub name: Option<String>,
    /// Delete even with a live session or uncommitted or untracked changes
    /// in a promoted worktree, discarding them.
    #[arg(long)]
    pub force: bool,
}

/// Arguments for `ivar feature rebase`.
#[derive(Debug, Args)]
pub struct FeatureRebaseArgs {
    /// The feature to rebase.
    pub name: Option<String>,
    /// Collapse the base: rebase every selected repo onto this branch, and
    /// record it as the declared base for each repo that lands there. The
    /// verb for once a feature's own base has landed.
    #[arg(long)]
    pub onto: Option<String>,
    /// Rebase only this promoted repo; repeat to select several. Without
    /// it every promoted repo is rebased.
    #[arg(long = "repo", value_name = "REPO")]
    pub repos: Vec<String>,
    /// Make no network call: rebase onto the local base ref instead of the
    /// remote tip `deliver` checks.
    #[arg(long)]
    pub offline: bool,
}

/// Arguments for `ivar feature view`.
#[derive(Debug, Args)]
pub struct FeatureViewArgs {
    /// The feature to view.
    pub name: Option<String>,
}

/// Arguments for `ivar feature cleanup`.
#[derive(Debug, Args)]
pub struct FeatureCleanupArgs {
    /// The feature to clean up.
    pub name: Option<String>,
    /// Preview only: compute and print the summary, teardown nothing.
    #[arg(long, conflicts_with = "record")]
    pub preview: bool,
    /// Path to the approved cleanup record; required to perform teardown.
    #[arg(long, conflicts_with = "preview")]
    pub record: Option<Utf8PathBuf>,
}

/// Arguments for `ivar feature workspace`.
#[derive(Debug, Args)]
pub struct FeatureWorkspaceArgs {
    /// The feature to generate a workspace for.
    pub feature: Option<String>,
    /// Which declared repos to include; includes all when omitted.
    pub repos: Vec<String>,
}

impl From<FeatureCreateArgs> for create::CreateInput {
    fn from(args: FeatureCreateArgs) -> Self {
        let FeatureCreateArgs {
            name,
            branch,
            base,
            parent,
            via,
            strategy,
        } = args;
        Self {
            name,
            branch,
            base,
            parent,
            via,
            strategy,
        }
    }
}

impl From<FeaturePromoteArgs> for promote::PromoteInput {
    fn from(args: FeaturePromoteArgs) -> Self {
        let FeaturePromoteArgs {
            feature,
            repo,
            base,
        } = args;
        Self {
            feature: feature.unwrap_or_default(),
            repo,
            base,
        }
    }
}

impl From<FeatureDemoteArgs> for demote::DemoteInput {
    fn from(args: FeatureDemoteArgs) -> Self {
        let FeatureDemoteArgs { feature, repo } = args;
        Self {
            feature: feature.unwrap_or_default(),
            repo,
        }
    }
}

impl From<FeatureStatusArgs> for status::StatusInput {
    fn from(args: FeatureStatusArgs) -> Self {
        let FeatureStatusArgs { feature, recursive } = args;
        Self {
            feature: feature.unwrap_or_default(),
            recursive,
        }
    }
}

impl From<FeatureIntegrateArgs> for integrate::IntegrateInput {
    fn from(args: FeatureIntegrateArgs) -> Self {
        let FeatureIntegrateArgs {
            feature,
            via,
            strategy,
            name,
        } = args;
        Self {
            feature: feature.unwrap_or_default(),
            via,
            strategy,
            name,
        }
    }
}

impl From<FeatureReparentArgs> for reparent::ReparentInput {
    fn from(args: FeatureReparentArgs) -> Self {
        let FeatureReparentArgs { child, parent } = args;
        Self {
            child: child.unwrap_or_default(),
            parent,
        }
    }
}

impl From<FeatureRenameArgs> for rename::RenameInput {
    fn from(args: FeatureRenameArgs) -> Self {
        let FeatureRenameArgs {
            feature,
            name,
            branch,
        } = args;
        Self {
            feature: feature.unwrap_or_default(),
            name,
            branch,
        }
    }
}

impl From<FeatureCloseArgs> for close::CloseInput {
    fn from(args: FeatureCloseArgs) -> Self {
        let FeatureCloseArgs { name, outcome } = args;
        Self {
            name: name.unwrap_or_default(),
            outcome,
        }
    }
}

impl From<FeatureDeleteArgs> for delete::DeleteInput {
    fn from(args: FeatureDeleteArgs) -> Self {
        let FeatureDeleteArgs { name, force } = args;
        Self {
            name: name.unwrap_or_default(),
            force,
        }
    }
}

impl From<FeatureRebaseArgs> for rebase::RebaseInput {
    fn from(args: FeatureRebaseArgs) -> Self {
        let FeatureRebaseArgs {
            name,
            onto,
            repos,
            offline,
        } = args;
        Self {
            name: name.unwrap_or_default(),
            onto,
            repos,
            offline,
        }
    }
}

impl From<FeatureViewArgs> for view::ViewInput {
    fn from(args: FeatureViewArgs) -> Self {
        let FeatureViewArgs { name } = args;
        Self {
            feature: name.unwrap_or_default(),
        }
    }
}

impl From<FeatureWorkspaceArgs> for workspace::WorkspaceInput {
    fn from(args: FeatureWorkspaceArgs) -> Self {
        let FeatureWorkspaceArgs { feature, repos } = args;
        Self {
            feature: feature.unwrap_or_default(),
            repos,
            open: false,
        }
    }
}
