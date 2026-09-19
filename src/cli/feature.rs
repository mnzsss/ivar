use camino::Utf8PathBuf;
use clap::{Args, Subcommand};

use crate::action::execute::{accept_revision, finish, start, status as execute_status};
use crate::action::feature::{
    close, create, delete, deliver, demote, integrate, promote, rebase, rename, reparent, status,
    view, workspace,
};
use crate::error::Failure;

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
    /// Rebase every promoted repo's worktree onto its effective base. A dirty
    /// worktree is skipped; a conflict is aborted and reported.
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

/// Arguments for `ivar feature workspace`.
#[derive(Debug, Args)]
pub struct FeatureWorkspaceArgs {
    /// The feature to generate a workspace for.
    pub feature: Option<String>,
    /// Which declared repos to include; includes all when omitted.
    pub repos: Vec<String>,
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
    pub feature: Option<String>,
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

/// The `ivar feature execute` Run Receipt lifecycle.
#[derive(Debug, Subcommand)]
pub enum ExecuteCommand {
    /// Start a new run, resume a blocked run, or restart a non-terminal run.
    Start(ExecuteStartArgs),
    /// Record a coordinator's structured completion report (see `--print-schema`).
    Finish(ExecuteFinishArgs),
    /// Show the current receipt, a receipt by id, or complete history.
    Status(ExecuteStatusArgs),
    /// Accept an approved plan revision for a diverged run.
    AcceptRevision(ExecuteAcceptRevisionArgs),
    /// Record an approved wave on the active run without editing the plan.
    Checkpoint(ExecuteCheckpointArgs),
    /// Abandon an active or blocked run, transitioning it to interrupted.
    Interrupt(ExecuteInterruptArgs),
}

/// Arguments for `ivar feature execute start`.
#[derive(Debug, Args)]
pub struct ExecuteStartArgs {
    pub feature: Option<String>,
    /// Plan file; defaults to `.ivar/features/<feature>/plan.md`.
    #[arg(long)]
    pub plan: Option<String>,
    #[arg(long, conflicts_with = "restart")]
    pub resume: bool,
    #[arg(long, conflicts_with = "resume")]
    pub restart: bool,
}

/// Arguments for `ivar feature execute finish`.
#[derive(Debug, Args)]
pub struct ExecuteFinishArgs {
    pub feature: Option<String>,
    /// Plan file; defaults to `.ivar/features/<feature>/plan.md`.
    #[arg(long)]
    pub plan: Option<String>,
    /// Path to the coordinator report JSON. Run with `--print-schema` for its shape.
    #[arg(long, required_unless_present = "print_schema")]
    pub report_json: Option<String>,
    /// How the run ended: succeeded, failed or blocked.
    #[arg(long, required_unless_present = "print_schema")]
    pub outcome: Option<String>,
    /// Print the coordinator report JSON schema and the accepted `--outcome` values, then exit.
    #[arg(long, conflicts_with_all = ["report_json", "outcome"])]
    pub print_schema: bool,
}

/// Arguments for `ivar feature execute status`.
#[derive(Debug, Args)]
pub struct ExecuteStatusArgs {
    pub feature: Option<String>,
    /// Plan file; defaults to `.ivar/features/<feature>/plan.md`.
    #[arg(long)]
    pub plan: Option<String>,
    #[arg(long, conflicts_with = "run")]
    pub history: bool,
    #[arg(long, conflicts_with = "history")]
    pub run: Option<String>,
}

/// Arguments for `ivar feature execute accept-revision`.
#[derive(Debug, Args)]
pub struct ExecuteAcceptRevisionArgs {
    pub feature: Option<String>,
    /// Plan file; defaults to `.ivar/features/<feature>/plan.md`.
    #[arg(long)]
    pub plan: Option<String>,
}

/// Arguments for `ivar feature execute checkpoint`.
#[derive(Debug, Args)]
pub struct ExecuteCheckpointArgs {
    pub feature: Option<String>,
    /// The 1-based wave number from `plan.md`.
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    pub wave: u32,
    /// Completed tasks, satisfied exit criteria, and deferred validation failures.
    #[arg(long)]
    pub summary: String,
}

/// Arguments for `ivar feature execute interrupt`.
#[derive(Debug, Args)]
pub struct ExecuteInterruptArgs {
    pub feature: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeatureDeliverArgs {
    /// The feature to deliver.
    pub feature: Option<String>,
    /// Print the delivery preview and push nothing.
    pub preview: bool,
    /// Land feature branches into default branches locally (fast-forward only).
    pub land: bool,
    /// The fingerprint from the preview the human approved; required to apply.
    pub fingerprint: Option<String>,
    /// Global metadata.
    pub global_metadata: deliver::PullRequestMetadata,
    /// Repository-scoped overrides.
    pub repo_overrides: Vec<deliver::RepoMetadataOverride>,
}

impl clap::Args for FeatureDeliverArgs {
    fn augment_args(cmd: clap::Command) -> clap::Command {
        cmd.arg(
            clap::Arg::new("feature")
                .help("The feature to deliver.")
                .required(false)
                .index(1),
        )
        .arg(
            clap::Arg::new("preview")
                .long("preview")
                .help("Print the delivery preview and push nothing.")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            clap::Arg::new("land")
                .long("land")
                .help("Land feature branches into default branches locally (fast-forward only).")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            clap::Arg::new("fingerprint")
                .long("fingerprint")
                .help("The fingerprint from the preview the human approved; required to apply. It covers `--name`, `--body` and `--draft`, so apply with the same values the preview used. Apply recomputes the preview and refuses when the fingerprint differs — the state has drifted since the preview.")
                .value_name("FINGERPRINT"),
        )
        .arg(
            clap::Arg::new("name")
                .long("name")
                .help("Pull request title. If placed before any `--repo`, applies globally; if placed after a `--repo`, applies to that repo. Part of the delivery fingerprint: pass the same value to the preview and the apply.")
                .value_name("TITLE")
                .action(clap::ArgAction::Append),
        )
        .arg(
            clap::Arg::new("body")
                .long("body")
                .help("Pull request body text, or a path to a `.md` / `.txt` file — either `./relative` or absolute. If placed before any `--repo`, applies globally; if placed after a `--repo`, applies to that repo. Part of the delivery fingerprint: pass the same value to the preview and the apply.")
                .value_name("BODY")
                .action(clap::ArgAction::Append),
        )
        .arg(
            clap::Arg::new("repo")
                .long("repo")
                .help("Scope following `--name`, `--body`, and `--draft` flags to this promoted repository.")
                .value_name("REPO")
                .action(clap::ArgAction::Append),
        )
        .arg(
            clap::Arg::new("draft")
                .long("draft")
                .help("Create or convert a pull request to a draft. If placed before any `--repo`, applies globally to all repos; if placed after a `--repo`, applies only to that repo. Incompatible with `--land`.")
                // The flag must be repeatable AND keep one parser index per
                // occurrence, because position decides global vs repository
                // scope. `SetTrue` rejects the second occurrence; `Count`
                // collapses every occurrence onto a single index. `Append`
                // taking no value records an index and `true` per occurrence.
                .num_args(0)
                .value_parser(clap::value_parser!(bool))
                .default_missing_value("true")
                .action(clap::ArgAction::Append),
        )
    }

    fn augment_args_for_update(cmd: clap::Command) -> clap::Command {
        Self::augment_args(cmd)
    }
}

impl clap::FromArgMatches for FeatureDeliverArgs {
    fn from_arg_matches(matches: &clap::ArgMatches) -> Result<Self, clap::Error> {
        let feature = matches.get_one::<String>("feature").cloned();
        let preview = matches.get_flag("preview");
        let land = matches.get_flag("land");
        let fingerprint = matches.get_one::<String>("fingerprint").cloned();

        let (global_metadata, repo_overrides) = Self::parse_metadata(matches)?;

        Ok(Self {
            feature,
            preview,
            land,
            fingerprint,
            global_metadata,
            repo_overrides,
        })
    }

    fn update_from_arg_matches(&mut self, matches: &clap::ArgMatches) -> Result<(), clap::Error> {
        *self = Self::from_arg_matches(matches)?;
        Ok(())
    }
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
}

/// Arguments for `ivar feature rebase`.
#[derive(Debug, Args)]
pub struct FeatureRebaseArgs {
    /// The feature to rebase.
    pub name: Option<String>,
    /// Collapse the base: rebase every promoted repo onto this branch, and
    /// record it as the declared base for each repo that lands there. The
    /// verb for once a feature's own base has landed.
    #[arg(long)]
    pub onto: Option<String>,
}

/// Arguments for `ivar feature view`.
#[derive(Debug, Args)]
pub struct FeatureViewArgs {
    /// The feature to view.
    pub name: Option<String>,
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
        } = args;
        Self {
            feature: feature.unwrap_or_default(),
            via,
            strategy,
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

impl From<ExecuteStartArgs> for start::StartInput {
    fn from(args: ExecuteStartArgs) -> Self {
        let ExecuteStartArgs {
            feature,
            plan,
            resume,
            restart,
        } = args;
        Self {
            feature: feature.unwrap_or_default(),
            plan,
            resume,
            restart,
        }
    }
}

/// `--report-json` and `--outcome` are optional to clap only so that
/// `--print-schema` can stand alone; every other invocation carries both.
/// Converting refuses rather than substituting empty strings, so a clap
/// surface that stops enforcing that says so instead of failing downstream.
impl TryFrom<ExecuteFinishArgs> for finish::FinishInput {
    type Error = Failure;

    fn try_from(args: ExecuteFinishArgs) -> Result<Self, Failure> {
        let ExecuteFinishArgs {
            feature,
            plan,
            report_json,
            outcome,
            print_schema: _,
        } = args;
        let (Some(report_json), Some(outcome)) = (report_json, outcome) else {
            return Err(Failure::blocked(
                "execute.finish_arguments_required",
                "`ivar feature execute finish` needs both `--report-json` and `--outcome`",
            ));
        };
        Ok(Self {
            feature: feature.unwrap_or_default(),
            plan,
            report_json,
            outcome,
        })
    }
}

impl From<ExecuteStatusArgs> for execute_status::StatusInput {
    fn from(args: ExecuteStatusArgs) -> Self {
        let ExecuteStatusArgs {
            feature,
            plan,
            history,
            run,
        } = args;
        Self {
            feature: feature.unwrap_or_default(),
            plan,
            history,
            run,
        }
    }
}

impl From<ExecuteAcceptRevisionArgs> for accept_revision::AcceptRevisionInput {
    fn from(args: ExecuteAcceptRevisionArgs) -> Self {
        let ExecuteAcceptRevisionArgs { feature, plan } = args;
        Self {
            feature: feature.unwrap_or_default(),
            plan,
        }
    }
}

fn set_delivery_metadata<T>(
    slot: &mut Option<T>,
    value: T,
    option: &str,
    repo: Option<&str>,
) -> Result<(), clap::Error> {
    if slot.is_some() {
        let scope = repo.map_or_else(String::new, |repo| format!(" in repository group `{repo}`"));
        return Err(clap::Error::raw(
            clap::error::ErrorKind::ArgumentConflict,
            format!("duplicate `--{option}`{scope}\n"),
        ));
    }
    *slot = Some(value);
    Ok(())
}

impl FeatureDeliverArgs {
    /// Reconstruct global metadata and ordered per-repo overrides from raw command line arguments.
    pub fn parse_metadata(
        matches: &clap::ArgMatches,
    ) -> Result<
        (
            deliver::PullRequestMetadata,
            Vec<deliver::RepoMetadataOverride>,
        ),
        clap::Error,
    > {
        enum DeliverOption {
            Name(String),
            Body(String),
            Repo(String),
            Draft,
        }

        let mut occurrences: Vec<(usize, DeliverOption)> = Vec::new();

        if let Some(indices) = matches.indices_of("name") {
            let values: Vec<&String> = matches
                .get_many::<String>("name")
                .map(|v| v.collect())
                .unwrap_or_default();
            for (idx, val) in indices.zip(values) {
                occurrences.push((idx, DeliverOption::Name(val.clone())));
            }
        }
        if let Some(indices) = matches.indices_of("body") {
            let values: Vec<&String> = matches
                .get_many::<String>("body")
                .map(|v| v.collect())
                .unwrap_or_default();
            for (idx, val) in indices.zip(values) {
                occurrences.push((idx, DeliverOption::Body(val.clone())));
            }
        }
        if let Some(indices) = matches.indices_of("repo") {
            let values: Vec<&String> = matches
                .get_many::<String>("repo")
                .map(|v| v.collect())
                .unwrap_or_default();
            for (idx, val) in indices.zip(values) {
                occurrences.push((idx, DeliverOption::Repo(val.clone())));
            }
        }
        // No `get_flag` guard: with `Append` the flag is not a bool-typed
        // value, and `indices_of` already returns `None` when it is absent.
        if let Some(indices) = matches.indices_of("draft") {
            for idx in indices {
                occurrences.push((idx, DeliverOption::Draft));
            }
        }

        occurrences.sort_by_key(|(idx, _)| *idx);

        let mut global_metadata = deliver::PullRequestMetadata::default();
        let mut repo_overrides: Vec<deliver::RepoMetadataOverride> = Vec::new();
        let mut current_repo: Option<(String, deliver::PullRequestMetadata)> = None;

        for (_, opt) in occurrences {
            match opt {
                DeliverOption::Repo(repo_name) => {
                    if let Some((r, meta)) = current_repo.take() {
                        repo_overrides.push(deliver::RepoMetadataOverride {
                            repo: r,
                            metadata: meta,
                        });
                    }
                    current_repo = Some((repo_name, deliver::PullRequestMetadata::default()));
                }
                DeliverOption::Name(title) => {
                    let (metadata, repo) = match &mut current_repo {
                        Some((repo, metadata)) => (metadata, Some(repo.as_str())),
                        None => (&mut global_metadata, None),
                    };
                    set_delivery_metadata(&mut metadata.title, title, "name", repo)?;
                }
                DeliverOption::Body(body) => {
                    let (metadata, repo) = match &mut current_repo {
                        Some((repo, metadata)) => (metadata, Some(repo.as_str())),
                        None => (&mut global_metadata, None),
                    };
                    set_delivery_metadata(&mut metadata.body, body, "body", repo)?;
                }
                DeliverOption::Draft => {
                    let (metadata, repo) = match &mut current_repo {
                        Some((repo, metadata)) => (metadata, Some(repo.as_str())),
                        None => (&mut global_metadata, None),
                    };
                    set_delivery_metadata(&mut metadata.draft, true, "draft", repo)?;
                }
            }
        }

        if let Some((r, meta)) = current_repo {
            repo_overrides.push(deliver::RepoMetadataOverride {
                repo: r,
                metadata: meta,
            });
        }

        Ok((global_metadata, repo_overrides))
    }
}

impl From<FeatureDeliverArgs> for deliver::DeliverInput {
    fn from(args: FeatureDeliverArgs) -> Self {
        let FeatureDeliverArgs {
            feature,
            preview,
            land,
            fingerprint,
            global_metadata,
            repo_overrides,
        } = args;

        Self {
            feature: feature.unwrap_or_default(),
            preview,
            land,
            fingerprint,
            global_metadata,
            repo_overrides,
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
        let FeatureDeleteArgs { name } = args;
        Self {
            name: name.unwrap_or_default(),
        }
    }
}

impl From<FeatureRebaseArgs> for rebase::RebaseInput {
    fn from(args: FeatureRebaseArgs) -> Self {
        let FeatureRebaseArgs { name, onto } = args;
        Self {
            name: name.unwrap_or_default(),
            onto,
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

impl From<FeatureViewArgs> for view::ViewInput {
    fn from(args: FeatureViewArgs) -> Self {
        let FeatureViewArgs { name } = args;
        Self {
            feature: name.unwrap_or_default(),
        }
    }
}
