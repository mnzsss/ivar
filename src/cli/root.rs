//! The root command surface.
//!
//! The settled surface (ARCHITECTURE.md's module map):
//! `ivar init · sync · status · doctor · cleanup · repo · feature · session ·
//! provider · plan · skill`. Only `init` carries real arguments this slice —
//! every other verb is a bare placeholder `bin/ivar.rs` turns into a
//! `Failure` naming it as not implemented yet, never a silent success and
//! never `todo!()`. See ARCHITECTURE.md's build order: those verbs land in
//! later slices, not stubbed 40-deep now.

use camino::Utf8PathBuf;
use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::action::hall::InitInput;
use crate::action::sync::SyncInput;

/// Mount the repos a feature spans into one directory, on one branch, for
/// one agent session.
#[derive(Debug, Parser)]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// Emit machine-readable output. `--json` and the human surface render
    /// the exact same value the action returned — see ARCHITECTURE.md,
    /// "1. `action` is the unit, and it has one output shape".
    #[arg(long, global = true)]
    pub json: bool,

    /// Colour control. `auto` (the default) follows `NO_COLOR` /
    /// `FORCE_COLOR` / tty detection; `always` and `never` are an explicit
    /// override fed to `infra::term::colour`.
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
    /// Manage SPDD plans.
    #[command(subcommand)]
    Plan(PlanCommand),
    /// Manage skills.
    #[command(subcommand)]
    Skill(SkillCommand),
}

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
    /// Remove a repo from ivar.json and tear down its files. Refuses while
    /// the repo is promoted in a feature or referenced by a live session;
    /// `--force` lifts both gates and cascades.
    Remove(RepoRemoveArgs),
    /// Refresh one or all repos' default branches from their remotes.
    Pull(RepoPullArgs),
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
}

/// The `ivar feature` surface: one branch across the repos it has promoted.
#[derive(Debug, Subcommand)]
pub enum FeatureCommand {
    /// Create a feature: name, branch, no repos promoted yet.
    Create(FeatureCreateArgs),
    /// List features and how far each got.
    List,
    /// Promote a repo onto a feature's branch: create the branch off the
    /// repo's default branch and materialise its worktree.
    Promote(FeaturePromoteArgs),
    /// Remove a repo from a feature. Its worktree stays on disk.
    Demote(FeatureDemoteArgs),
    /// Show one feature in detail: every promoted repo and its state.
    Status(FeatureStatusArgs),
    /// Prepare a feature's execution board from its plan and execution graph.
    Execute(FeatureExecuteArgs),
    /// Preview, then push, a feature's promoted repos. `--preview` prints the
    /// side-effect-free summary (with its fingerprint) and pushes nothing;
    /// applying with `--fingerprint` is refused if the state has drifted.
    Deliver(FeatureDeliverArgs),
}

/// Arguments for `ivar feature create`.
#[derive(Debug, Args)]
pub struct FeatureCreateArgs {
    /// The feature's name — one path segment, unique within the hall.
    pub name: String,
}

/// Arguments for `ivar feature promote`.
#[derive(Debug, Args)]
pub struct FeaturePromoteArgs {
    /// The feature to promote into.
    pub feature: String,
    /// The repo to promote onto the feature's branch.
    pub repo: String,
}

/// Arguments for `ivar feature demote`.
#[derive(Debug, Args)]
pub struct FeatureDemoteArgs {
    /// The feature to demote from.
    pub feature: String,
    /// The repo to demote.
    pub repo: String,
}

/// Arguments for `ivar feature status`.
#[derive(Debug, Args)]
pub struct FeatureStatusArgs {
    /// The feature to inspect.
    pub feature: String,
}

/// Arguments for `ivar feature execute`.
#[derive(Debug, Args)]
pub struct FeatureExecuteArgs {
    /// The feature to prepare an execution board for.
    pub feature: String,
    /// Path to the execution graph JSON — workstreams with
    /// `id`/`title`/`operations`/`depends_on`/`write_contract`.
    #[arg(long)]
    pub graph_json: String,
}

/// Arguments for `ivar feature deliver`.
#[derive(Debug, Args)]
pub struct FeatureDeliverArgs {
    /// The feature to deliver.
    pub name: String,
    /// Print the delivery preview and push nothing.
    #[arg(long)]
    pub preview: bool,
    /// The fingerprint from the preview the human approved; required to apply.
    /// Apply recomputes the preview and refuses when the fingerprint differs —
    /// the state has drifted since the preview.
    #[arg(long)]
    pub fingerprint: Option<String>,
}

/// The `ivar session` surface.
#[derive(Debug, Subcommand)]
pub enum SessionCommand {
    /// Open a session: view dir over a feature's promoted repos, agent
    /// running in it, TUI on top.
    Start(SessionStartArgs),
    /// Re-bind to an existing live session: locate it, re-materialise its
    /// view dir, and emit the binding as `IVAR_*` env vars.
    Connect(SessionConnectArgs),
    /// Bind a discovery session to a feature (one-way), moving its view dir
    /// into the feature's session tree.
    Convert(SessionConvertArgs),
}

/// Arguments for `ivar session start`.
#[derive(Debug, Args)]
pub struct SessionStartArgs {
    /// The feature to open a session for.
    pub feature: String,
    /// Resume an existing session, where the harness supports it.
    #[arg(long)]
    pub resume: bool,
    /// The provider to run. Defaults to the hall's default provider.
    #[arg(long)]
    pub provider: Option<String>,
    /// Create the session without launching a provider. The view dir persists
    /// after this command returns, until an explicit stop.
    #[arg(long)]
    pub detached: bool,
    /// Relay: a fresh session on the same feature under a different provider
    /// than the feature's most recent session. Requires `--provider`.
    #[arg(long)]
    pub relay: bool,
}

/// Arguments for `ivar session connect`.
#[derive(Debug, Args)]
pub struct SessionConnectArgs {
    /// The session id, or a unique prefix of one.
    pub session_id: Option<String>,
    /// Narrow the search to sessions bound to this feature.
    #[arg(long)]
    pub feature: Option<String>,
}

/// Arguments for `ivar session convert`.
#[derive(Debug, Args)]
pub struct SessionConvertArgs {
    /// The discovery session's id, or a unique prefix of one.
    pub session_id: String,
    /// The feature to bind the session to. Must already exist.
    pub feature: String,
}

/// The `ivar plan` surface: the SPDD artifacts, committed per feature, and
/// the approval gates that transition a feature through the SPDD lifecycle.
#[derive(Debug, Subcommand)]
pub enum PlanCommand {
    /// Scaffold a feature's SPDD artifacts (requirements, analysis, plan).
    Create(PlanCreateArgs),
    /// List which features have plans, and how complete.
    List,
    /// Print one feature's SPDD artifact.
    Show(PlanShowArgs),
    /// Approve one of a feature's SPDD gates: requirements, analysis, plan,
    /// or execution-graph. Requires the gate upstream of it to be approved
    /// first, and records a fingerprint of the artifact's content.
    Approve(PlanApproveArgs),
    /// Declare a revision of an approved gate, marking it — and every gate
    /// downstream — as needing revision.
    Invalidate(PlanInvalidateArgs),
}

/// Arguments for `ivar plan create`.
#[derive(Debug, Args)]
pub struct PlanCreateArgs {
    /// The feature to scaffold plans for.
    pub feature: String,
}

/// Arguments for `ivar plan show`.
#[derive(Debug, Args)]
pub struct PlanShowArgs {
    /// The feature whose artifact to show.
    pub feature: String,
    /// Which artifact: `requirements`, `analysis`, or `plan`.
    pub artifact: crate::action::plan::show::Artifact,
}

/// Arguments for `ivar plan approve`.
#[derive(Debug, Args)]
pub struct PlanApproveArgs {
    /// The feature whose gate to approve.
    pub feature: String,
    /// The gate: `requirements`, `analysis`, `plan`, or `execution-graph`.
    pub gate: String,
}

/// Arguments for `ivar plan invalidate`.
#[derive(Debug, Args)]
pub struct PlanInvalidateArgs {
    /// The feature whose gate to invalidate.
    pub feature: String,
    /// The gate: `requirements`, `analysis`, `plan`, or `execution-graph`.
    pub gate: String,
}

/// The `ivar skill` surface: the hall's shared skills directory.
#[derive(Debug, Subcommand)]
pub enum SkillCommand {
    /// List the skills in the hall's shared skills directory.
    List,
    /// Scaffold a new skill: a folder with a SKILL.md.
    Create(SkillCreateArgs),
}

/// The `ivar provider` surface: which harnesses a hall knows about.
#[derive(Debug, Subcommand)]
pub enum ProviderCommand {
    /// List the hall's providers and the default one.
    List,
}

/// Arguments for `ivar skill create`.
#[derive(Debug, Args)]
pub struct SkillCreateArgs {
    /// The skill's id — one path segment, unique within the skills dir.
    pub id: String,
    /// The skill's description, for the SKILL.md frontmatter.
    #[arg(long)]
    pub description: String,
}

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

impl From<InitArgs> for InitInput {
    /// A straight field copy — no validation. See the type doc comment.
    fn from(args: InitArgs) -> Self {
        Self {
            path: args.path,
            name: args.name,
            provider: args.provider,
        }
    }
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

impl From<SyncArgs> for SyncInput {
    fn from(args: SyncArgs) -> Self {
        Self {
            force_setup: args.force_setup,
        }
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
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use clap::CommandFactory as _;

    use super::*;

    #[test]
    fn cli_definition_is_valid() {
        // `debug_assert` panics on a malformed clap definition (duplicate
        // ids, conflicting args, ...) — the cheapest test that the derive
        // actually produced a usable `Command`.
        Cli::command().debug_assert();
    }

    #[test]
    fn init_args_convert_into_init_input_without_change() {
        let args = InitArgs {
            path: Utf8PathBuf::from("some/dir"),
            name: Some("acme".to_owned()),
            provider: Some("opencode".to_owned()),
        };

        let input: InitInput = args.into();

        assert_eq!(input.path, Utf8PathBuf::from("some/dir"));
        assert_eq!(input.name, Some("acme".to_owned()));
        assert_eq!(input.provider, Some("opencode".to_owned()));
    }

    #[test]
    fn color_mode_maps_to_the_override_colour_expects() {
        assert_eq!(ColorMode::Auto.as_override(), None);
        assert_eq!(ColorMode::Always.as_override(), Some(true));
        assert_eq!(ColorMode::Never.as_override(), Some(false));
    }
}
