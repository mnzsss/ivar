//! The root command surface.
//!
//! The settled v1 surface (ARCHITECTURE.md's module map):
//! `ivar init · sync · status · doctor · cleanup · migrate · repo · feature ·
//! session · provider · plan · skill · mcp`. Every verb dispatches to an action file that
//! returns `Failure::blocked("…not implemented yet")` — never a silent success
//! and never `todo!()`. See ARCHITECTURE.md's build order: those verbs land in
//! later slices, not stubbed 40-deep now.

use camino::Utf8PathBuf;
use clap::{Args, Parser, Subcommand, ValueEnum};

#[cfg(test)]
use crate::action::feature::deliver;
use crate::action::hall::InitInput;
#[cfg(test)]
use crate::action::repo::create as repo_create;
#[cfg(test)]
use crate::action::repo::view as repo_view;
use crate::action::session::guard_cmd;
use crate::action::sync::SyncInput;
use crate::error::Failure;

/// Mount the repos a feature spans into one directory, on one branch, for
/// one agent session.
#[derive(Debug, Parser)]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// Emit machine-readable output.
    ///
    /// Prints exactly the value the command computed. The human-readable text
    /// is a rendering of that same value, so the two can never tell you
    /// different things — script against this.
    #[arg(long, global = true)]
    pub json: bool,

    /// Output token-optimized compact pipe-delimited records with schema header.
    #[arg(long, global = true)]
    pub compact: bool,

    /// When to colour output.
    ///
    /// `auto` follows `NO_COLOR`, then `FORCE_COLOR`, then whether the stream
    /// is a terminal — a pipe or a redirect gets none. `always` and `never`
    /// override all of that. Only labels are ever coloured; values never are,
    /// so `--json` is unaffected either way.
    #[arg(long = "color", global = true, value_enum, default_value_t = ColorMode::Auto)]
    pub color: ColorMode,

    #[command(subcommand)]
    pub command: Command,
}

/// The root verbs. See the module doc comment for which ones do anything in
/// this slice.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create a hall: `ivar.json`, `.ivar/`, and the hall's `.gitignore`
    /// lines.
    Init(InitArgs),
    /// Bring the local hall in line with `ivar.json`: clone missing repos,
    /// materialise harness config, run setup scripts.
    Sync(SyncArgs),
    /// Report hall health.
    Status,
    /// Diagnose problems and suggest fixes.
    Doctor,
    /// Reconcile stale state (interactive; asks before deleting).
    Cleanup,
    /// Advance `ivar.json`'s schema version (interactive; shows the change,
    /// then asks).
    ///
    /// Only ever needed after upgrading `ivar` to a build whose format is
    /// newer than the one your hall was written with. Local state migrates
    /// itself; `ivar.json` is committed, so advancing it is a decision you
    /// make and then commit.
    Migrate,
    /// Manage repos.
    #[command(subcommand)]
    Repo(RepoCommand),
    /// Manage features.
    #[command(subcommand)]
    Feature(FeatureCommand),
    /// Manage sessions.
    #[command(subcommand)]
    Session(SessionCommand),
    /// Manage providers.
    #[command(subcommand)]
    Provider(ProviderCommand),
    /// Manage discovery docs: a unit of work's working brief.
    #[command(subcommand)]
    Discovery(DiscoveryCommand),
    /// Manage SPDD plans.
    #[command(subcommand)]
    Plan(PlanCommand),
    /// Review a feature's local changes.
    #[command(subcommand)]
    Review(ReviewCommand),
    /// Manage skills.
    #[command(subcommand)]
    Skill(SkillCommand),
    /// Authenticate the hall's declared MCP servers.
    #[command(subcommand)]
    Mcp(McpCommand),
    /// Query and index the codebase dependency graph.
    #[command(subcommand)]
    Graph(super::graph::GraphCommand),
    /// Guard: evaluate a tool request against the session's writable set.
    Guard(GuardArgs),
    /// Answer git's credential helper protocol on stdin. Registered as
    /// `credential.https://github.com.helper = !ivar git-credential` so a
    /// token never lands in `.git/config`.
    #[command(hide = true)]
    GitCredential(GitCredentialArgs),
}

/// The operation git appends when it invokes a credential helper.
#[derive(Debug, Args)]
pub struct GitCredentialArgs {
    /// What git is asking for: `get`, `store`, or `erase`.
    ///
    /// A free string rather than a value enum on purpose. gitcredentials(7)
    /// requires a helper to *ignore* an operation it does not implement, and a
    /// helper cannot ignore what clap rejected before it ran — a git release
    /// that names a new operation would otherwise print a usage error in the
    /// middle of every push. Optional for the same reason a bare invocation is
    /// read as `get`: only a human runs this without an operation.
    #[arg(value_name = "OPERATION")]
    pub operation: Option<String>,
}

/// Arguments for `ivar guard`.
#[derive(Debug, Args)]
pub struct GuardArgs {
    /// The provider whose hook protocol to use for output shaping.
    #[arg(long)]
    pub provider: String,
}

impl TryFrom<GuardArgs> for guard_cmd::GuardInput {
    type Error = crate::error::Failure;

    fn try_from(args: GuardArgs) -> Result<Self, Self::Error> {
        let provider = args.provider.parse().map_err(|e| {
            Failure::blocked(
                "guard.invalid_provider",
                format!("unknown provider `{}`: {e}", args.provider),
            )
        })?;
        Ok(Self { provider })
    }
}

#[path = "discovery.rs"]
mod discovery;
#[path = "feature.rs"]
mod feature;
#[path = "mcp.rs"]
mod mcp;
#[path = "plan.rs"]
mod plan;
#[path = "provider.rs"]
mod provider;
#[path = "repo.rs"]
mod repo;
#[path = "review.rs"]
mod review;
#[path = "session.rs"]
mod session;
#[path = "skill.rs"]
mod skill;

pub use discovery::{
    DiscoveryAmendArgs, DiscoveryArgs, DiscoveryCloseArgs, DiscoveryCommand, DiscoveryCreateArgs,
    DiscoveryListArgs, DiscoveryShowArgs,
};
pub use feature::{
    ExecuteAcceptRevisionArgs, ExecuteCheckpointArgs, ExecuteCommand, ExecuteFinishArgs,
    ExecuteInterruptArgs, ExecuteStartArgs, ExecuteStatusArgs, FeatureCleanupArgs,
    FeatureCloseArgs, FeatureCommand, FeatureCreateArgs, FeatureDeleteArgs, FeatureDeliverArgs,
    FeatureDemoteArgs, FeatureIntegrateArgs, FeaturePromoteArgs, FeatureRebaseArgs,
    FeatureRenameArgs, FeatureReparentArgs, FeatureStatusArgs, FeatureViewArgs,
    FeatureWorkspaceArgs,
};
pub use mcp::{McpAuthArgs, McpCommand};
pub use plan::{
    PlanApproveArgs, PlanCommand, PlanCreateArgs, PlanInvalidateArgs, PlanShowArgs, PlanStatusArgs,
};
pub use provider::{ProviderAddArgs, ProviderCommand};
pub use repo::{
    RepoAddArgs, RepoCommand, RepoCreateArgs, RepoPullArgs, RepoRemoveArgs, RepoSetupArgs,
    RepoUpstreamArgs, RepoViewArgs,
};
pub use review::{
    CommentAddArgs, CommentCommand, CommentListArgs, CommentResolveArgs, CommentStatusArg,
    ReviewCommand,
};
pub use session::{
    SessionCommand, SessionConnectArgs, SessionConvertArgs, SessionEnvArgs, SessionRelayArgs,
    SessionSandboxArgs, SessionStartArgs, SessionStopArgs,
};
pub use skill::{
    SkillAddArgs, SkillCommand, SkillCreateArgs, SkillDetachArgs, SkillRemoveArgs, SkillUpdateArgs,
};

/// Arguments for `ivar init`.
///
/// `name` and `provider` stay plain strings here — validating them into
/// `HallName` / `Provider` needs `domain`, which `cli` may not import (see
/// the layering table in ARCHITECTURE.md). That validation is
/// `action::hall::init`'s job; this type only carries what clap parsed.
#[derive(Debug, Args)]
pub struct InitArgs {
    /// Directory to create the hall in. Defaults to the current directory.
    #[arg(default_value = ".")]
    pub path: Utf8PathBuf,

    /// The hall's name. Defaults to the target directory's name.
    #[arg(long)]
    pub name: Option<String>,

    /// The provider to record as the hall's sole available (and default)
    /// provider. Defaults to `claude-code`.
    #[arg(long)]
    pub provider: Option<String>,
}

/// Arguments for `ivar sync`.
///
/// No path argument: `sync` acts on the hall the current directory is inside,
/// found by walking up the way `git` finds `.git`. A `--path` would be a second
/// answer to "which hall?" and the first one is already the one people expect.
#[derive(Debug, Args)]
pub struct SyncArgs {
    /// Run every repo's setup script even if it has already run for this
    /// version of the script. For when a script's effect was undone outside
    /// `ivar` — a deleted `node_modules`, a dropped database.
    #[arg(long)]
    pub force_setup: bool,
}

// -- args → input ------------------------------------------------------------
//
// Every conversion below **destructures its args struct exhaustively**. That is
// the whole point of writing them out rather than reaching for `args.field`:
// adding a flag to a `*Args` struct and forgetting to forward it stops being a
// flag the parser advertises and the action never sees, and becomes a compile
// error naming the field.
//
// The direction Rust already covers is the other one — an `*Input` field the
// CLI does not supply cannot be constructed at all. The direction it does not
// cover is a declared arg nobody reads, and that is the one that ships help
// text promising a flag that does nothing. Hence the `let Xxx { .. } = args;`
// line in each impl. Do not replace it with field access.
//
// No validation happens here, and none may: turning a `String` into a
// `FeatureName` needs `domain`, which `cli` must not import (see the layering
// table in ARCHITECTURE.md). These are shape conversions only.

impl From<InitArgs> for InitInput {
    fn from(args: InitArgs) -> Self {
        let InitArgs {
            path,
            name,
            provider,
        } = args;
        Self {
            path,
            name,
            provider,
        }
    }
}

impl From<SyncArgs> for SyncInput {
    fn from(args: SyncArgs) -> Self {
        let SyncArgs { force_setup } = args;
        Self { force_setup }
    }
}

/// Colour control for the root command. See [`Cli::color`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ColorMode {
    /// Follow `NO_COLOR` / `FORCE_COLOR` / tty detection.
    Auto,
    /// Force colour on.
    Always,
    /// Force colour off.
    Never,
}

impl ColorMode {
    /// The `Option<bool>` override `infra::term::colour` expects. `cli`
    /// cannot import `infra` itself (see the layering table) — this is a
    /// plain value conversion, applied by `bin/ivar.rs`, which can reach
    /// both `cli` and `infra`.
    #[must_use]
    pub const fn as_override(self) -> Option<bool> {
        match self {
            Self::Auto => None,
            Self::Always => Some(true),
            Self::Never => Some(false),
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/cli/root.rs"]
mod tests;
