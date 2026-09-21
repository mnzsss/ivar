use clap::{Args, Subcommand};

use crate::action::repo::{
    add, create as repo_create, pull, remove, setup as repo_setup, upstream as repo_upstream,
};

/// The `ivar repo` surface: what a repo is, who owns it, and how the hall's
/// copy of it stays current. Each subcommand is one action file under
/// `action/repo/` — see ARCHITECTURE.md's module map.
#[derive(Debug, Subcommand)]
pub enum RepoCommand {
    /// List the repos in ivar.json and their state.
    List,
    /// Declare a repo in ivar.json, clone it bare, and materialise its
    /// default-branch worktree.
    Add(RepoAddArgs),
    /// Create a brand-new repo — stored in the hall's origin (`--local`) or on
    /// GitHub (`--remote`) — and declare it in ivar.json.
    Create(RepoCreateArgs),
    /// Remove a repo from ivar.json and tear down its files. Refuses while
    /// the repo is promoted in a feature or referenced by a live session;
    /// `--force` lifts both gates and cascades.
    Remove(RepoRemoveArgs),
    /// Refresh one or all repos' default branches from their remotes.
    Pull(RepoPullArgs),
    /// Run the setup script for one repo.
    Setup(RepoSetupArgs),
    /// Manage remote upstream for a repo.
    Upstream(RepoUpstreamArgs),
}

/// Arguments for `ivar repo add`.
#[derive(Debug, Args)]
pub struct RepoAddArgs {
    /// The repo's name — one path segment, unique within the hall.
    pub name: String,
    /// The git remote URL to clone from.
    pub url: String,
    /// The branch a fresh worktree defaults to. Defaults to `main`.
    #[arg(long)]
    pub default_branch: Option<String>,
    /// Reuse a bare clone already present at the expected path.
    #[arg(long, conflicts_with = "fresh")]
    pub reuse: bool,
    /// Delete an existing bare clone (and its worktree) and clone anew.
    #[arg(long, conflicts_with = "reuse")]
    pub fresh: bool,
}

/// Arguments for `ivar repo create`.
#[derive(Debug, Args)]
#[command(group(clap::ArgGroup::new("mode").required(true).args(["local", "remote"])))]
pub struct RepoCreateArgs {
    /// The repo's name — one path segment, unique within the hall.
    pub name: String,
    /// Store the repo under `refs/heads/repos/<name>/` in the hall's origin.
    #[arg(long)]
    pub local: bool,
    /// Create the repo on GitHub with `gh`.
    #[arg(long)]
    pub remote: bool,
    /// Make the GitHub repo public. Only with `--remote`.
    #[arg(long, requires = "remote", conflicts_with = "local")]
    pub public: bool,
    /// The branch the initial commit lands on. Defaults to `main`.
    #[arg(long)]
    pub default_branch: Option<String>,
}

/// Arguments for `ivar repo remove`.
#[derive(Debug, Args)]
pub struct RepoRemoveArgs {
    /// The repo's name, as declared in ivar.json.
    pub name: String,
    /// Tear down even while the repo is promoted in a feature or referenced
    /// by a live session. Cascades: removes its worktrees, scrubs its
    /// promotion records, repairs view-dir symlinks, and regenerates the
    /// providers' config.
    #[arg(long)]
    pub force: bool,
}

/// Arguments for `ivar repo pull`.
#[derive(Debug, Args)]
pub struct RepoPullArgs {
    /// The repo to fetch. Fetches every repo when omitted.
    pub repo: Option<String>,
    /// When a repo cannot fast-forward, report the divergence in detail —
    /// the local and remote commits each side has. Read-only.
    #[arg(long)]
    pub diagnose: bool,
    /// Automatically reconcile a diverged default branch when it is safe:
    /// reset it to the remote tip when every local commit is a duplicate of
    /// work already upstream (same patch-id). Never touches a branch with
    /// genuine local work, and implies `--diagnose` for the repos it cannot
    /// resolve.
    #[arg(long)]
    pub resolve: bool,
}

/// Arguments for `ivar repo setup`.
#[derive(Debug, Args)]
pub struct RepoSetupArgs {
    /// The repo whose setup script to run. Runs every repo's setup when omitted.
    pub repo: Option<String>,
    /// Ignore the receipt and run the setup script even if unchanged.
    #[arg(long)]
    pub force_setup: bool,
}

/// Arguments for `ivar repo upstream`.
#[derive(Debug, Args)]
pub struct RepoUpstreamArgs {
    /// The repo to manage.
    pub repo: String,
    /// The upstream remote URL to set (or remove with `--remove`).
    #[arg(long)]
    pub url: Option<String>,
    /// Remove the upstream remote entirely.
    #[arg(long, conflicts_with = "url")]
    pub remove: bool,
}

impl From<RepoAddArgs> for add::AddInput {
    /// `--reuse` / `--fresh` are a tri-state on the wire and an
    /// `Option<bool>` in the action: reuse an existing bare clone, replace it,
    /// or refuse to guess. clap already rejects passing both.
    fn from(args: RepoAddArgs) -> Self {
        let RepoAddArgs {
            name,
            url,
            default_branch,
            reuse,
            fresh,
        } = args;
        let reuse_existing = match (reuse, fresh) {
            (true, false) => Some(true),
            (false, true) => Some(false),
            _ => None,
        };
        Self {
            name,
            url,
            default_branch,
            reuse_existing,
            ref_prefix: None,
        }
    }
}

impl From<RepoCreateArgs> for repo_create::CreateInput {
    fn from(args: RepoCreateArgs) -> Self {
        let mode = if args.remote {
            repo_create::CreateMode::Remote {
                public: args.public,
            }
        } else {
            repo_create::CreateMode::Local
        };
        Self {
            name: args.name,
            mode,
            default_branch: args.default_branch,
        }
    }
}

impl From<RepoRemoveArgs> for remove::RemoveInput {
    fn from(args: RepoRemoveArgs) -> Self {
        let RepoRemoveArgs { name, force } = args;
        Self { name, force }
    }
}

impl From<RepoPullArgs> for pull::PullInput {
    fn from(args: RepoPullArgs) -> Self {
        let RepoPullArgs {
            repo,
            diagnose,
            resolve,
        } = args;
        Self {
            repo,
            diagnose,
            resolve,
        }
    }
}

impl From<RepoSetupArgs> for repo_setup::SetupInput {
    /// An omitted repo means every repo, which the action spells as an empty
    /// name.
    fn from(args: RepoSetupArgs) -> Self {
        let RepoSetupArgs { repo, force_setup } = args;
        Self {
            repo: repo.unwrap_or_default(),
            force: force_setup,
        }
    }
}

impl From<RepoUpstreamArgs> for repo_upstream::UpstreamInput {
    fn from(args: RepoUpstreamArgs) -> Self {
        let RepoUpstreamArgs { repo, url, remove } = args;
        Self {
            repo,
            url: url.unwrap_or_default(),
            remove,
        }
    }
}
