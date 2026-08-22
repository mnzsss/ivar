//! The root command surface.
//!
//! The settled v1 surface (ARCHITECTURE.md's module map):
//! `ivar init · sync · status · doctor · cleanup · migrate · repo · feature ·
//! session · provider · plan · skill`. Every verb dispatches to an action file that
//! returns `Failure::blocked("…not implemented yet")` — never a silent success
//! and never `todo!()`. See ARCHITECTURE.md's build order: those verbs land in
//! later slices, not stubbed 40-deep now.

use camino::Utf8PathBuf;
use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::action::execute::{accept_revision, finish, start, status as execute_status};
use crate::action::feature::{
    close, create, delete, deliver, demote, integrate, promote, rebase, reparent, review, status,
    view,
};
use crate::action::hall::InitInput;
use crate::action::plan::approve as plan_approve;
use crate::action::plan::{create as plan_create, show as plan_show, status as plan_status};
use crate::action::provider::add as provider_add;
use crate::action::repo::{add, pull, remove, setup as repo_setup, upstream as repo_upstream};
use crate::action::session::{
    connect as session_connect, conversion as session_conversion, relay as session_relay,
    start as session_start, stop as session_stop,
};
use crate::action::skill::{
    add as skill_add, create as skill_create, detach as skill_detach, remove as skill_remove,
    update as skill_update,
};
use crate::action::sync::SyncInput;

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
    /// Manage SPDD plans.
    #[command(subcommand)]
    Plan(PlanCommand),
    /// Manage skills.
    #[command(subcommand)]
    Skill(SkillCommand),
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

/// The `ivar feature` surface: one branch across the repos it has promoted.
#[derive(Debug, Subcommand)]
pub enum FeatureCommand {
    /// Create a feature: name, branch, no repos promoted yet. A subfeature
    /// is created with `--parent <feature>`, which derives its base from the
    /// parent's branch; `--via`/`--strategy` persist the feature's own
    /// integration-policy override.
    Create(FeatureCreateArgs),
    /// List features and how far each got.
    List,
    /// Promote a repo onto a feature's branch and materialise its worktree.
    /// A branch that already exists is adopted as-is; one that does not is
    /// created off the repo's effective base.
    Promote(FeaturePromoteArgs),
    /// Remove a repo from a feature. Its worktree stays on disk.
    Demote(FeatureDemoteArgs),
    /// Show one feature in detail: every promoted repo and its state, and —
    /// with `--recursive` — its whole subtree's health.
    Status(FeatureStatusArgs),
    /// Integrate a child into its immediate parent, leaves first: each
    /// promoted repo's work lands on the parent's branch, durably and
    /// resumably. `--via`/`--strategy` override the resolved policy for the
    /// run; after the first receipt the policy is frozen.
    Integrate(FeatureIntegrateArgs),
    /// Move a still-pristine child under a different parent, updating its
    /// parent and derived base in one record write. Refused once any
    /// promotion, plan, execution, session, receipt, close record, or
    /// descendant exists.
    Reparent(FeatureReparentArgs),
    /// Manage a feature's Run Receipt lifecycle.
    #[command(subcommand)]
    Execute(ExecuteCommand),
    /// Preview, then push, a feature's promoted repos. `--preview` prints the
    /// side-effect-free summary (with its fingerprint) and pushes nothing;
    /// applying with `--fingerprint` is refused if the state has drifted.
    Deliver(FeatureDeliverArgs),
    /// Close a feature: stop its executor sessions, remove its execution
    /// state, and record the outcome on plan.md's frontmatter. Idempotent —
    /// closing an already-closed feature is a no-op.
    Close(FeatureCloseArgs),
    /// Delete a feature: its worktrees, its directory under `.ivar/`, and its
    /// plans. Refuses if anything under the feature directory is not
    /// removable, and preserves the feature record for retry if a teardown
    /// step fails.
    Delete(FeatureDeleteArgs),
    /// Rebase every promoted repo's worktree onto its effective base. A dirty
    /// worktree is skipped; a conflict is aborted and reported.
    Rebase(FeatureRebaseArgs),
    /// Write a VSCode workspace opening the feature: promoted repos on the
    /// feature branch, everyone else on their default branch.
    Review(FeatureReviewArgs),
    /// Open an interactive multi-shell view over the feature's promoted
    /// repos — one shell per repo, each running in its worktree.
    View(FeatureViewArgs),
    /// Delete features whose branches have been merged into their default
    /// branches.
    Prune,
}

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

/// Arguments for `ivar feature integrate`.
#[derive(Debug, Args)]
pub struct FeatureIntegrateArgs {
    /// The child feature to integrate.
    pub feature: String,
    /// The via override for this run: `pr` or `local`. Ignored once the
    /// first receipt froze the policy.
    #[arg(long)]
    pub via: Option<String>,
    /// The strategy override for this run: `squash`, `merge`, or `rebase`.
    /// Ignored once the first receipt froze the policy.
    #[arg(long)]
    pub strategy: Option<String>,
}

/// Arguments for `ivar feature reparent`.
#[derive(Debug, Args)]
pub struct FeatureReparentArgs {
    /// The child feature to move.
    pub child: String,
    /// The new parent feature. The child's `base` is rewritten to the new
    /// parent's branch in the same record write.
    #[arg(long)]
    pub parent: String,
}

/// Arguments for `ivar feature promote`.
#[derive(Debug, Args)]
pub struct FeaturePromoteArgs {
    /// The feature to promote into.
    pub feature: String,
    /// The repo to promote onto the feature's branch.
    pub repo: String,
    /// Override the branch a new worktree starts from, for this repo only.
    /// Defaults to the feature's declared base, or the repo's default branch.
    #[arg(long)]
    pub base: Option<String>,
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
    /// Render the feature's whole subtree — itself and every descendant, in
    /// deterministic pre-order — with each feature's derived state, repos,
    /// and blockers.
    #[arg(long)]
    pub recursive: bool,
}

/// The `ivar feature execute` Run Receipt lifecycle.
#[derive(Debug, Subcommand)]
pub enum ExecuteCommand {
    /// Start a new run, resume a blocked run, or restart a non-terminal run.
    Start(ExecuteStartArgs),
    /// Record a coordinator's structured completion report.
    Finish(ExecuteFinishArgs),
    /// Show the current receipt, a receipt by id, or complete history.
    Status(ExecuteStatusArgs),
    /// Accept an approved plan revision for a diverged run.
    AcceptRevision(ExecuteAcceptRevisionArgs),
}

/// Arguments for `ivar feature execute start`.
#[derive(Debug, Args)]
pub struct ExecuteStartArgs {
    pub feature: String,
    #[arg(long)]
    pub plan: String,
    #[arg(long, conflicts_with = "restart")]
    pub resume: bool,
    #[arg(long, conflicts_with = "resume")]
    pub restart: bool,
}

/// Arguments for `ivar feature execute finish`.
#[derive(Debug, Args)]
pub struct ExecuteFinishArgs {
    pub feature: String,
    #[arg(long)]
    pub plan: String,
    #[arg(long)]
    pub report_json: String,
    #[arg(long)]
    pub outcome: String,
}

/// Arguments for `ivar feature execute status`.
#[derive(Debug, Args)]
pub struct ExecuteStatusArgs {
    pub feature: String,
    #[arg(long)]
    pub history: bool,
    #[arg(long)]
    pub run: Option<String>,
}

/// Arguments for `ivar feature execute accept-revision`.
#[derive(Debug, Args)]
pub struct ExecuteAcceptRevisionArgs {
    pub feature: String,
    #[arg(long)]
    pub plan: String,
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

/// Arguments for `ivar feature close`.
#[derive(Debug, Args)]
pub struct FeatureCloseArgs {
    /// The feature to close.
    pub name: String,
    /// How the feature ended: `delivered` or `abandoned`.
    #[arg(long)]
    pub outcome: String,
}

/// Arguments for `ivar feature delete`.
#[derive(Debug, Args)]
pub struct FeatureDeleteArgs {
    /// The feature to delete.
    pub name: String,
}

/// Arguments for `ivar feature rebase`.
#[derive(Debug, Args)]
pub struct FeatureRebaseArgs {
    /// The feature to rebase.
    pub name: String,
    /// Collapse the base: rebase every promoted repo onto this branch, and
    /// record it as the declared base for each repo that lands there. The
    /// verb for once a feature's own base has landed.
    #[arg(long)]
    pub onto: Option<String>,
}

/// Arguments for `ivar feature review`.
#[derive(Debug, Args)]
pub struct FeatureReviewArgs {
    /// The feature to open.
    pub name: String,
}

/// Arguments for `ivar feature view`.
#[derive(Debug, Args)]
pub struct FeatureViewArgs {
    /// The feature to view.
    pub name: String,
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
    /// Stop a session — tear down its view dir and end any running harness.
    /// Omitting the session stops *every* session in the hall.
    Stop(SessionStopArgs),
    /// Remove dead sessions: view dirs that exist but hold no readable
    /// `state.json`. A session with a readable record is never touched.
    Prune,
    /// Relay session info: four-line output contract for external consumers.
    Relay(SessionRelayArgs),
}

/// Arguments for `ivar session start`.
#[derive(Debug, Args)]
pub struct SessionStartArgs {
    /// The feature to open a session for. Omit for a discovery session: no
    /// feature bound, every repo read-only on its default branch.
    pub feature: Option<String>,
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

/// Arguments for `ivar session stop`.
#[derive(Debug, Args)]
pub struct SessionStopArgs {
    /// The session to stop — its id, or a unique prefix of one.
    ///
    /// Omitting it stops **every** session in the hall: every discovery
    /// session and every feature's sessions, not just this feature's and not
    /// just the most recent. Pass `$IVAR_SESSION_ID` to stop only your own.
    pub session: Option<String>,
}

/// Arguments for `ivar session relay`.
///
/// A thin alias over `session start --relay`: the same feature under a
/// different provider. It carries no logic of its own — see the session
/// relay action — so its surface mirrors start's relay flags.
#[derive(Debug, Args)]
pub struct SessionRelayArgs {
    /// The feature to relay a session for.
    pub feature: String,
    /// The provider to relay to. Required — relay must switch providers.
    #[arg(long)]
    pub provider: String,
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
    /// Requires the gate upstream of it to be approved
    /// first, and records a fingerprint of the artifact's content.
    Approve(PlanApproveArgs),
    /// Declare a revision of an approved gate, marking it — and every gate
    /// downstream — as needing revision.
    Invalidate(PlanInvalidateArgs),
    /// Show approval gate status for a plan file.
    Status(PlanStatusArgs),
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
    /// The gate: `requirements`, `analysis`, or `plan`.
    pub gate: String,
}

/// Arguments for `ivar plan invalidate`.
#[derive(Debug, Args)]
pub struct PlanInvalidateArgs {
    /// The feature whose gate to invalidate.
    pub feature: String,
    /// The gate: `requirements`, `analysis`, or `plan`.
    pub gate: String,
}

/// Arguments for `ivar plan status`.
#[derive(Debug, Args)]
pub struct PlanStatusArgs {
    /// Path to the plan file (plan.md or similar).
    pub plan_path: String,
}

/// The `ivar skill` surface: the hall's shared skills directory.
#[derive(Debug, Subcommand)]
pub enum SkillCommand {
    /// List the skills in the hall's shared skills directory.
    List,
    /// Scaffold a new skill: a folder with a SKILL.md.
    Create(SkillCreateArgs),
    /// Install an external skill from a git repo.
    Add(SkillAddArgs),
    /// Update external skills to their tracked ref.
    Update(SkillUpdateArgs),
    /// Remove a skill from the hall's shared skills directory.
    Remove(SkillRemoveArgs),
    /// Convert an external skill into an authored (local) skill.
    Detach(SkillDetachArgs),
    /// Materialise hall skills to native targets for other tools.
    Sync,
    /// Show skill installation state — which are external, authored, or stale.
    Status,
    /// Health diagnostics for skills: find broken links, missing refs, and
    /// suggest fix_actions.
    Doctor,
}

/// The `ivar provider` surface: which harnesses a hall knows about.
#[derive(Debug, Subcommand)]
pub enum ProviderCommand {
    /// List the hall's providers and the default one.
    List,
    /// Register a new provider by name.
    Add(ProviderAddArgs),
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

/// Arguments for `ivar skill add`.
#[derive(Debug, Args)]
pub struct SkillAddArgs {
    /// The git repo URL or path to install the skill from.
    pub repo: String,
    /// A sub-path inside the repo that holds the skill folder.
    #[arg(long)]
    pub path: Option<String>,
    /// A git ref (branch, tag, or sha) to pin the skill to.
    #[arg(long)]
    pub r#ref: Option<String>,
}

/// Arguments for `ivar skill update`.
#[derive(Debug, Args)]
pub struct SkillUpdateArgs {
    /// Which external skills to update; updates all when omitted.
    pub skills: Vec<String>,
}

/// Arguments for `ivar skill remove`.
#[derive(Debug, Args)]
pub struct SkillRemoveArgs {
    /// The skill's id to remove.
    pub skill: String,
}

/// Arguments for `ivar skill detach`.
#[derive(Debug, Args)]
pub struct SkillDetachArgs {
    /// The external skill's id to convert into an authored skill.
    pub skill: String,
}

/// Arguments for `ivar provider add`.
#[derive(Debug, Args)]
pub struct ProviderAddArgs {
    /// The provider's name (e.g. `claude-code`, `opencode`).
    pub name: String,
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
            feature,
            repo,
            base,
        }
    }
}

impl From<FeatureDemoteArgs> for demote::DemoteInput {
    fn from(args: FeatureDemoteArgs) -> Self {
        let FeatureDemoteArgs { feature, repo } = args;
        Self { feature, repo }
    }
}

impl From<FeatureStatusArgs> for status::StatusInput {
    fn from(args: FeatureStatusArgs) -> Self {
        let FeatureStatusArgs { feature, recursive } = args;
        Self { feature, recursive }
    }
}

impl From<FeatureIntegrateArgs> for integrate::IntegrateInput {
    fn from(args: FeatureIntegrateArgs) -> Self {
        let FeatureIntegrateArgs {
            feature,
            via,
            strategy,
        } = args;
        Self {
            feature,
            via,
            strategy,
        }
    }
}

impl From<FeatureReparentArgs> for reparent::ReparentInput {
    fn from(args: FeatureReparentArgs) -> Self {
        let FeatureReparentArgs { child, parent } = args;
        Self { child, parent }
    }
}

impl From<ExecuteStartArgs> for start::StartInput {
    fn from(args: ExecuteStartArgs) -> Self {
        let ExecuteStartArgs {
            feature,
            plan,
            resume,
            restart,
        } = args;
        Self {
            feature,
            plan,
            resume,
            restart,
        }
    }
}

impl From<ExecuteFinishArgs> for finish::FinishInput {
    fn from(args: ExecuteFinishArgs) -> Self {
        let ExecuteFinishArgs {
            feature,
            plan,
            report_json,
            outcome,
        } = args;
        Self {
            feature,
            plan,
            report_json,
            outcome,
        }
    }
}

impl From<ExecuteStatusArgs> for execute_status::StatusInput {
    fn from(args: ExecuteStatusArgs) -> Self {
        let ExecuteStatusArgs {
            feature,
            history,
            run,
        } = args;
        Self {
            feature,
            history,
            run,
        }
    }
}

impl From<ExecuteAcceptRevisionArgs> for accept_revision::AcceptRevisionInput {
    fn from(args: ExecuteAcceptRevisionArgs) -> Self {
        let ExecuteAcceptRevisionArgs { feature, plan } = args;
        Self { feature, plan }
    }
}

impl From<FeatureDeliverArgs> for deliver::DeliverInput {
    fn from(args: FeatureDeliverArgs) -> Self {
        let FeatureDeliverArgs {
            name,
            preview,
            fingerprint,
        } = args;
        Self {
            feature: name,
            preview,
            fingerprint,
        }
    }
}

impl From<FeatureCloseArgs> for close::CloseInput {
    fn from(args: FeatureCloseArgs) -> Self {
        let FeatureCloseArgs { name, outcome } = args;
        Self { name, outcome }
    }
}

impl From<FeatureDeleteArgs> for delete::DeleteInput {
    fn from(args: FeatureDeleteArgs) -> Self {
        let FeatureDeleteArgs { name } = args;
        Self { name }
    }
}

impl From<FeatureRebaseArgs> for rebase::RebaseInput {
    fn from(args: FeatureRebaseArgs) -> Self {
        let FeatureRebaseArgs { name, onto } = args;
        Self { name, onto }
    }
}

impl From<FeatureReviewArgs> for review::ReviewInput {
    fn from(args: FeatureReviewArgs) -> Self {
        let FeatureReviewArgs { name } = args;
        Self { name }
    }
}

impl From<FeatureViewArgs> for view::ViewInput {
    fn from(args: FeatureViewArgs) -> Self {
        let FeatureViewArgs { name } = args;
        Self { feature: name }
    }
}

impl From<SessionStartArgs> for session_start::StartInput {
    fn from(args: SessionStartArgs) -> Self {
        let SessionStartArgs {
            feature,
            resume,
            provider,
            detached,
            relay,
        } = args;
        Self {
            feature,
            resume,
            provider,
            detached,
            relay,
        }
    }
}

impl From<SessionConnectArgs> for session_connect::ConnectInput {
    fn from(args: SessionConnectArgs) -> Self {
        let SessionConnectArgs {
            session_id,
            feature,
        } = args;
        Self {
            session_id,
            feature,
        }
    }
}

impl From<SessionConvertArgs> for session_conversion::ConvertInput {
    fn from(args: SessionConvertArgs) -> Self {
        let SessionConvertArgs {
            session_id,
            feature,
        } = args;
        Self {
            session_id,
            feature,
        }
    }
}

impl From<SessionStopArgs> for session_stop::StopInput {
    fn from(args: SessionStopArgs) -> Self {
        let SessionStopArgs { session } = args;
        Self { session }
    }
}

impl From<SessionRelayArgs> for session_relay::RelayInput {
    fn from(args: SessionRelayArgs) -> Self {
        let SessionRelayArgs { feature, provider } = args;
        Self { feature, provider }
    }
}

impl From<ProviderAddArgs> for provider_add::AddInput {
    fn from(args: ProviderAddArgs) -> Self {
        let ProviderAddArgs { name } = args;
        Self { name }
    }
}

impl From<PlanCreateArgs> for plan_create::CreateInput {
    fn from(args: PlanCreateArgs) -> Self {
        let PlanCreateArgs { feature } = args;
        Self { feature }
    }
}

impl From<PlanShowArgs> for plan_show::ShowInput {
    fn from(args: PlanShowArgs) -> Self {
        let PlanShowArgs { feature, artifact } = args;
        Self { feature, artifact }
    }
}

impl From<PlanApproveArgs> for plan_approve::ApproveInput {
    fn from(args: PlanApproveArgs) -> Self {
        let PlanApproveArgs { feature, gate } = args;
        Self { feature, gate }
    }
}

impl From<PlanInvalidateArgs> for plan_approve::InvalidateInput {
    fn from(args: PlanInvalidateArgs) -> Self {
        let PlanInvalidateArgs { feature, gate } = args;
        Self { feature, gate }
    }
}

impl From<PlanStatusArgs> for plan_status::StatusInput {
    fn from(args: PlanStatusArgs) -> Self {
        let PlanStatusArgs { plan_path } = args;
        Self { plan_path }
    }
}

impl From<SkillCreateArgs> for skill_create::CreateInput {
    fn from(args: SkillCreateArgs) -> Self {
        let SkillCreateArgs { id, description } = args;
        Self { id, description }
    }
}

impl From<SkillAddArgs> for skill_add::AddInput {
    fn from(args: SkillAddArgs) -> Self {
        let SkillAddArgs { repo, path, r#ref } = args;
        Self {
            repo,
            path,
            ref_: r#ref,
        }
    }
}

impl From<SkillUpdateArgs> for skill_update::UpdateInput {
    fn from(args: SkillUpdateArgs) -> Self {
        let SkillUpdateArgs { skills } = args;
        Self { skills }
    }
}

impl From<SkillRemoveArgs> for skill_remove::RemoveInput {
    fn from(args: SkillRemoveArgs) -> Self {
        let SkillRemoveArgs { skill } = args;
        Self { skill }
    }
}

impl From<SkillDetachArgs> for skill_detach::DetachInput {
    fn from(args: SkillDetachArgs) -> Self {
        let SkillDetachArgs { skill } = args;
        Self { skill }
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
