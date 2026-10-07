use clap::Subcommand;

mod delivery;
mod execution;
mod lifecycle;

pub use delivery::FeatureDeliverArgs;
pub use execution::{
    ExecuteAcceptRevisionArgs, ExecuteCheckpointArgs, ExecuteCommand, ExecuteFinishArgs,
    ExecuteInterruptArgs, ExecuteStartArgs, ExecuteStatusArgs,
};
pub use lifecycle::{
    FeatureCleanupArgs, FeatureCloseArgs, FeatureCreateArgs, FeatureDeleteArgs, FeatureDemoteArgs,
    FeatureIntegrateArgs, FeaturePromoteArgs, FeatureRebaseArgs, FeatureRenameArgs,
    FeatureReparentArgs, FeatureStatusArgs, FeatureViewArgs, FeatureWorkspaceArgs,
};

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
    /// Rename a feature, its branch, or both — the one allowed identity
    /// transition. Every promoted repo's local branch and worktree, its
    /// remote branch when published, direct children, live sessions, and the
    /// feature directory all move together, durably and resumably: a
    /// mid-flight failure automatically reverses whatever already landed, and
    /// an interruption resumes on the next `ivar feature rename` invocation
    /// naming the same feature.
    Rename(FeatureRenameArgs),
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
    /// Rebase promoted repos' worktrees onto the remote tip of their base
    /// (`--offline`: the local ref); `--repo` selects repos. A dirty worktree
    /// is skipped; a conflict is aborted and reported.
    Rebase(FeatureRebaseArgs),
    /// Open an interactive multi-shell view over the feature's promoted
    /// repos — one shell per repo, each running in its worktree.
    View(FeatureViewArgs),
    /// Delete features whose branches have been merged into their default
    /// branches.
    Prune,
    /// Authorize and perform a feature's local teardown.
    Cleanup(FeatureCleanupArgs),
    /// Generate a multi-root VSCode workspace (.code-workspace) for a feature,
    /// opening promoted repos writable and every context repo read-only.
    Workspace(FeatureWorkspaceArgs),
}
