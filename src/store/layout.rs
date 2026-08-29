//! Every path under a hall is computed here. Nowhere else.
//!
//! The predecessor scattered path construction across ten modules, and
//! consolidating seven dotdirs into one meant touching all of them. This module
//! is the answer to that: a `Layout` holds the hall root and hands out typed
//! paths, so the next reorganisation is one file.
//!
//! # The layout, as decided
//!
//! ```text
//! <hall>/
//!   ivar.json                          the manifest — COMMITTED, and outside .ivar/
//!                                      on purpose: it is the identity file and has
//!                                      to be visible in a pull request review.
//!   .ivar/
//!     state.json                       local hall state (gitignored)
//!     repos/<repo>/.bare/              the bare clone
//!     repos/<repo>/<branch>/           a worktree off that bare
//!     features/<feature>/              promotion records
//!     features/<feature>/planning/     approval-gate state (approvals.json)
//!     features/<feature>/execution/    the current Run Receipt (run.json)
//!     features/<feature>/execution/archive/runs/<run-id>.json
//!                                      terminal receipts, immutable
//!     features/<feature>/execution/archive/boards/<hash>.json
//!                                      raw normalized legacy boards, content-addressed
//!     features/<feature>/integration/  throwaway local-integration staging
//!                                      worktrees (candidate/source per repo)
//!     features/<feature>/sessions/<id>/  feature-session view dirs
//!     sessions/<id>/                   discovery-session view dirs
//!     secrets/                         per-repo secret material, hand-maintained
//!                                      (gitignored — see below)
//!     setups/<repo>.sh                 per-repo setup scripts — COMMITTED
//!     setups/<repo>.session.sh         per-repo session hooks — COMMITTED
//!     skills/                          hall-scoped skills — COMMITTED
//!   plans/<feature>/                   requirements.md · analysis.md · plan.md — COMMITTED
//!   .claude/  .opencode/               harness-dictated. These are the TARGET of the
//!                                      view dir's symlinks, not the source, which is
//!                                      why they stay at the root.
//! ```
//!
//! Note that children inside `.ivar/` carry no leading dot. One dotdir at the top,
//! plain names underneath.
//!
//! # Contract
//!
//! - `Layout::discover(from: &Utf8Path)` — walk **up** from a directory looking for
//!   `ivar.json`, the way `git` finds `.git`. Returns `Ok(None)` when there is no
//!   hall above it; that is the normal "you are not in a hall" case, not an error.
//!   The one way this fails is `from` itself not being resolvable (it does not
//!   exist, a component is unreadable, …) — resolved once, up front, which is also
//!   what keeps the walk from chasing a moving target forever.
//! - `Layout::at(root)` — for `init`, where the root is chosen rather than found.
//! - Accessors returning `Utf8PathBuf`, taking validated newtypes from
//!   [`crate::domain::name`] rather than `&str`: `manifest()`, `state()`,
//!   `repo_bare(&RepoName)`, `repo_worktree(&RepoName, &BranchName)`,
//!   `feature_dir(&FeatureName)`, `planning_dir(&FeatureName)`,
//!   `execution_dir(&FeatureName)`, `run_receipt(&FeatureName)`,
//!   `archived_run(&FeatureName, &RunId)`, `archived_board(&FeatureName, &str)`,
//!   `feature_session(&FeatureName, &SessionId)`, `discovery_session(&SessionId)`,
//!   `setup_script(&RepoName)`, `session_hook(&RepoName)`, `secrets_dir()`,
//!   `hall_skills()`, `plan_dir(&FeatureName)`,
//!   `harness_dir(&Provider)`, `commands_dir(&Provider)`.
//! - `gitignore_lines()` — the exact patterns the hall's `.gitignore` needs.
//!
//! # Why `secrets/` needs no gitignore line of its own
//!
//! The hall's `.gitignore` excludes `.ivar/*` and negates exactly the two
//! committed children. Anything else under `.ivar/` is therefore ignored by
//! construction, and `secrets/` is deliberately placed there rather than at the
//! hall root for precisely that reason: a secrets directory that depends on
//! someone remembering to add a line is a secrets directory that eventually
//! leaks. Adding a third negation would be the trap described below; not adding
//! one is the whole design.
//!
//! # The gitignore trap
//!
//! It must be `.ivar/*` plus `!.ivar/skills/` plus `!.ivar/setups/` — **never**
//! `.ivar/`. Git does not re-include a child of an excluded directory, so `.ivar/`
//! followed by negations silently drops the hall's committed children from
//! version control. Silently. Put a test on this; the failure mode is invisible
//! until a teammate clones and the skills (or the setup scripts) are not there.
//!
//! Two further lines — `.claude/commands/ivar-*.md` and
//! `.opencode/commands/ivar-*.md` — keep the shipped workflow commands `ivar`
//! materialises out of the hall's git history. They match only the reserved
//! `ivar-*` prefix, so a user's own commands remain committable.
//!
//! `docs/reference/on-disk-format.md` marks *two* children of `.ivar/` committed —
//! `skills/` and `setups/` — and `ARCHITECTURE.md`'s environment-contract section
//! independently calls `.ivar/setups/<repo>.sh` "committed" in its own words. The
//! "two traps" callout earlier in that same document names only `skills/` in its
//! `.gitignore` example, which undercounts by one line against its own later
//! text; `gitignore_lines()` here follows the on-disk-format contract (both
//! negated) rather than that abbreviated example.
//!
//! # No legacy names
//!
//! The manifest has already been renamed once (`workdir.json` → `hall.json` →
//! `ivar.json`). Do **not** port a fallback for either old name. A fresh
//! implementation should not be born carrying compatibility debt it never had.

use camino::{Utf8Path, Utf8PathBuf};

use crate::domain::feature::RunId;
use crate::domain::name::{BranchName, FeatureName, RepoName, SessionId};
use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction};

/// The manifest's filename. Never one of its two predecessors — see the module
/// doc comment.
const MANIFEST_FILE_NAME: &str = "ivar.json";

/// The one dotdir everything `ivar` manages lives under.
const IVAR_DIR: &str = ".ivar";

/// The current Run Receipt's filename, under a feature's execution directory.
const RUN_FILE: &str = "run.json";

/// Every path under a hall, computed from its root.
///
/// See the module doc comment for the full layout this hands out paths for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    root: Utf8PathBuf,
}

impl Layout {
    /// Build a `Layout` at a root that has already been chosen.
    ///
    /// This is `ivar init`'s case: there is nothing to find yet, because the
    /// hall does not exist on disk until this call's caller creates it.
    #[must_use]
    pub fn at(root: impl Into<Utf8PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Walk up from `from` looking for `ivar.json`, the way `git` finds `.git`.
    ///
    /// `Ok(None)` means there is no hall above `from` — the normal "you are not
    /// in a hall" case, not an error at this layer.
    ///
    /// `from` is resolved to an absolute, symlink-free path exactly once, up
    /// front; every step after that is pure string arithmetic on
    /// [`Utf8Path::parent`], which strictly shortens the path and therefore
    /// cannot loop forever. The only way this returns `Err` is that first
    /// resolution failing — `from` does not exist, or a component of it is not
    /// reachable.
    pub fn discover(from: &Utf8Path) -> Result<Option<Self>, DiscoverError> {
        let mut current =
            from.canonicalize_utf8()
                .map_err(|source| DiscoverError::Unresolvable {
                    path: from.to_path_buf(),
                    source,
                })?;

        loop {
            if current.join(MANIFEST_FILE_NAME).is_file() {
                return Ok(Some(Self::at(current)));
            }
            match current.parent() {
                Some(parent) => current = parent.to_path_buf(),
                None => return Ok(None),
            }
        }
    }

    /// The hall root — the directory `ivar.json` lives in.
    #[must_use]
    pub fn root(&self) -> &Utf8Path {
        &self.root
    }

    /// `<hall>/ivar.json` — the manifest. Committed, and outside `.ivar/` on
    /// purpose: it is the identity file and has to be visible in review.
    #[must_use]
    pub fn manifest(&self) -> Utf8PathBuf {
        self.root.join(MANIFEST_FILE_NAME)
    }

    /// `<hall>/.ivar/state.json` — local hall state. Gitignored.
    #[must_use]
    pub fn state(&self) -> Utf8PathBuf {
        self.ivar_dir().join("state.json")
    }

    /// `<hall>/.ivar/repos/<repo>/.bare/` — the bare clone every checkout of
    /// `repo` is a worktree off.
    #[must_use]
    pub fn repo_bare(&self, repo: &RepoName) -> Utf8PathBuf {
        self.repo_dir(repo).join(".bare")
    }

    /// `<hall>/.ivar/repos/<repo>/<branch>/` — a worktree off that bare, on
    /// `branch`. `branch` may itself contain `/` (`feat/auth-v2` is normal
    /// git), which nests further directories here — that is expected.
    #[must_use]
    pub fn repo_worktree(&self, repo: &RepoName, branch: &BranchName) -> Utf8PathBuf {
        self.repo_dir(repo).join(branch.as_str())
    }

    /// `<hall>/.ivar/features/<feature>/` — promotion records.
    #[must_use]
    pub fn feature_dir(&self, feature: &FeatureName) -> Utf8PathBuf {
        self.features_dir().join(feature.as_str())
    }

    /// `<hall>/.ivar/features/<feature>/execution/` — the feature's current
    /// Run Receipt and its archive. The legacy importer also reads a historical
    /// board from this directory.
    #[must_use]
    pub fn execution_dir(&self, feature: &FeatureName) -> Utf8PathBuf {
        self.feature_dir(feature).join("execution")
    }

    /// `<hall>/.ivar/features/<feature>/execution/run.json` — the current Run
    /// Receipt. At most one exists; a terminal receipt is archived and this
    /// file removed before the next run creates it.
    #[must_use]
    pub fn run_receipt(&self, feature: &FeatureName) -> Utf8PathBuf {
        self.execution_dir(feature).join(RUN_FILE)
    }

    /// `<hall>/.ivar/features/<feature>/execution/archive/` — everything the
    /// lifecycle has finished with, kept forever.
    #[must_use]
    pub fn run_archive_dir(&self, feature: &FeatureName) -> Utf8PathBuf {
        self.execution_dir(feature).join("archive")
    }

    /// `<hall>/.ivar/features/<feature>/execution/archive/runs/` — one file
    /// per terminal receipt, named by run id.
    #[must_use]
    pub fn run_archive_runs_dir(&self, feature: &FeatureName) -> Utf8PathBuf {
        self.run_archive_dir(feature).join("runs")
    }

    /// `<hall>/.ivar/features/<feature>/execution/archive/runs/<run-id>.json` —
    /// one archived receipt.
    ///
    /// Takes a [`RunId`], not a `&str`: the id becomes a path component, and
    /// `RunId`'s only constructor validates it as a UUID. Joining an
    /// unvalidated string here is what `status --run <id>` would turn into a
    /// traversal.
    #[must_use]
    pub fn archived_run(&self, feature: &FeatureName, run: &RunId) -> Utf8PathBuf {
        self.run_archive_runs_dir(feature)
            .join(format!("{run}.json"))
    }

    /// `<hall>/.ivar/features/<feature>/execution/archive/boards/` — the raw
    /// normalized execution boards that legacy import consumed.
    #[must_use]
    pub fn board_archive_dir(&self, feature: &FeatureName) -> Utf8PathBuf {
        self.run_archive_dir(feature).join("boards")
    }

    /// `<hall>/.ivar/features/<feature>/execution/archive/boards/<hash>.json` —
    /// one archived board, named by the SHA-256 of its own normalized bytes.
    ///
    /// Content-addressed on purpose: identical content lands on the same path,
    /// so re-running an interrupted import writes the same bytes to the same
    /// place, and different content can never overwrite an existing archive
    /// because it computes a different name.
    #[must_use]
    pub fn archived_board(&self, feature: &FeatureName, source_hash: &str) -> Utf8PathBuf {
        self.board_archive_dir(feature)
            .join(format!("{source_hash}.json"))
    }

    /// `<hall>/.ivar/features/<feature>/execution/inbox/` — one append-only
    /// JSONL file per workstream. This is the channel a human reply travels
    /// to a blocked workstream; it is deliberately outside `board.json`, which
    /// stays a single small document the plan owns.
    #[must_use]
    pub fn execution_inbox_dir(&self, feature: &FeatureName) -> Utf8PathBuf {
        self.execution_dir(feature).join("inbox")
    }

    /// `<hall>/.ivar/features/<feature>/execution/inbox/<workstream>.jsonl` —
    /// the append-only inbox of one workstream.
    #[must_use]
    pub fn execution_inbox(&self, feature: &FeatureName, workstream: &str) -> Utf8PathBuf {
        self.execution_inbox_dir(feature)
            .join(format!("{workstream}.jsonl"))
    }

    /// `<hall>/.ivar/features/<feature>/integration/<repo>/` — the staging
    /// area for one repo's local integration: a detached candidate worktree
    /// and a temporary source worktree, both throwaway.
    #[must_use]
    pub fn integration_dir(&self, feature: &FeatureName, repo: &RepoName) -> Utf8PathBuf {
        self.feature_dir(feature)
            .join("integration")
            .join(repo.as_str())
    }

    /// `<hall>/.ivar/features/<feature>/integration/<repo>/candidate` — the
    /// detached worktree a local integration builds and checks before the
    /// parent's branch is touched.
    #[must_use]
    pub fn integration_candidate(&self, feature: &FeatureName, repo: &RepoName) -> Utf8PathBuf {
        self.integration_dir(feature, repo).join("candidate")
    }

    /// `<hall>/.ivar/features/<feature>/integration/<repo>/source` — the
    /// temporary worktree the rebase strategy replays the child's source onto
    /// the parent in, before the parent fast-forwards to the result.
    #[must_use]
    pub fn integration_source(&self, feature: &FeatureName, repo: &RepoName) -> Utf8PathBuf {
        self.integration_dir(feature, repo).join("source")
    }

    /// `<hall>/.ivar/features/<feature>/planning/` — the feature's approval
    /// gate state (`approvals.json`). Local derived state: approvals are
    /// per-machine records of what this machine's human has reviewed, and
    /// belong in no teammate's clone.
    #[must_use]
    pub fn planning_dir(&self, feature: &FeatureName) -> Utf8PathBuf {
        self.feature_dir(feature).join("planning")
    }

    /// `<hall>/.ivar/features/<feature>/sessions/<id>/` — a feature-session
    /// view dir.
    #[must_use]
    pub fn feature_session(&self, feature: &FeatureName, session: &SessionId) -> Utf8PathBuf {
        self.feature_dir(feature)
            .join("sessions")
            .join(session.as_str())
    }

    /// `<hall>/.ivar/features/<feature>/sessions/` — every feature-session
    /// view dir. `feature close` removes the whole tree to stop a feature's
    /// live sessions in one step.
    #[must_use]
    pub fn feature_sessions_dir(&self, feature: &FeatureName) -> Utf8PathBuf {
        self.feature_dir(feature).join("sessions")
    }

    /// `<hall>/<feature>.code-workspace` — the VSCode workspace `feature
    /// review` writes. Lives at the hall root, next to the worktrees it opens,
    /// so the relative folder paths inside it resolve against this file.
    #[must_use]
    pub fn workspace_file(&self, feature: &FeatureName) -> Utf8PathBuf {
        self.root
            .join(format!("{}.code-workspace", feature.as_str()))
    }

    /// `<hall>/.ivar/sessions/<id>/` — a discovery-session view dir.
    #[must_use]
    pub fn discovery_session(&self, session: &SessionId) -> Utf8PathBuf {
        self.ivar_dir().join("sessions").join(session.as_str())
    }

    /// `<hall>/.ivar/setups/<repo>.sh` — the repo's setup script. Committed:
    /// a git worktree shares history but not untracked files, so a fresh
    /// worktree needs this to bootstrap `.env`, `node_modules`, and so on.
    #[must_use]
    pub fn setup_script(&self, repo: &RepoName) -> Utf8PathBuf {
        self.ivar_dir()
            .join("setups")
            .join(format!("{}.sh", repo.as_str()))
    }

    /// `<hall>/.ivar/setups/<repo>.session.sh` — the repo's session hook.
    /// Committed, and a sibling of [`Self::setup_script`] on purpose: the two
    /// belong to the same repo and are read by the same people.
    ///
    /// The setup script bootstraps a *worktree* and is gated by a receipt, so
    /// it runs about once. This runs on every `session start`, ungated, and is
    /// where per-session state belongs — the database or compose project a
    /// session must not share with its siblings.
    #[must_use]
    pub fn session_hook(&self, repo: &RepoName) -> Utf8PathBuf {
        self.ivar_dir()
            .join("setups")
            .join(format!("{}.session.sh", repo.as_str()))
    }

    /// `<hall>/.ivar/secrets/` — where a setup script reads values git does not
    /// carry. Hand-maintained, never written by `ivar`, and gitignored by the
    /// same `.ivar/*` rule that covers the rest of local state.
    ///
    /// `ivar` stores no secrets. This is a *location* handed to setup scripts
    /// through `IVAR_SECRETS_DIR`, the same posture `domain::mcp` takes: hold
    /// references, never values.
    #[must_use]
    pub fn secrets_dir(&self) -> Utf8PathBuf {
        self.ivar_dir().join("secrets")
    }

    /// `<hall>/.ivar/skills/` — hall-scoped skills. Committed.
    #[must_use]
    pub fn hall_skills(&self) -> Utf8PathBuf {
        self.ivar_dir().join("skills")
    }

    /// Local skills that never enter the hall's git repo.
    ///
    /// The `.gitignore` is `.ivar/*` + `!.ivar/skills/` + `!.ivar/setups/`,
    /// so any new directory here is gitignored by default.
    #[must_use]
    pub fn hall_skills_local(&self) -> Utf8PathBuf {
        self.ivar_dir().join("skills-local")
    }

    /// `<hall>/plans/<feature>/` — `requirements.md` · `analysis.md` ·
    /// `plan.md`. Committed.
    #[must_use]
    pub fn plan_dir(&self, feature: &FeatureName) -> Utf8PathBuf {
        self.root.join("plans").join(feature.as_str())
    }

    /// `<hall>/docs/updates/` — numbered delivery-relevant change records,
    /// including a feature's durable cleanup record (MNZS-379's convention).
    /// Committed.
    #[must_use]
    pub fn docs_updates_dir(&self) -> Utf8PathBuf {
        self.root.join("docs").join("updates")
    }

    /// `<hall>/.claude/` or `<hall>/.opencode/` — the harness-dictated config
    /// dir a session's view dir symlinks target. Lives at the hall root, not
    /// under `.ivar/`, because it belongs to the harness, not to `ivar`.
    ///
    /// The dotdir comes from [`Provider::config_dir`], not from interpolating
    /// [`Provider::id`] — `claude-code` maps to `.claude`, not `.claude-code`.
    /// See `domain::provider` for why that distinction is the whole point.
    #[must_use]
    pub fn harness_dir(&self, provider: &Provider) -> Utf8PathBuf {
        self.root().join(provider.config_dir())
    }

    /// `<hall>/.claude/commands/` or `<hall>/.opencode/commands/` — the
    /// provider-native directory holding project workflow commands. Lives at
    /// the hall root, next to the rest of the harness's config.
    ///
    /// `ivar` owns only files matching `ivar-*.md` inside it; every other
    /// file belongs to the user and must survive every operation. The
    /// subdirectory comes from [`Provider::commands_dir`] for the same reason
    /// [`Self::harness_dir`] uses `config_dir` — the mapping is not the
    /// identity function.
    #[must_use]
    pub fn commands_dir(&self, provider: &Provider) -> Utf8PathBuf {
        self.root().join(provider.commands_dir())
    }

    /// `<hall>/HALL.md` — the sole editable source of shared hall
    /// instructions. Committed, and mostly the user's: `harness::config`
    /// owns only the bytes between its two markers.
    ///
    /// Every provider's root alias is a relative symlink to this file — it is
    /// the canonical source, and the aliases are never sources themselves.
    #[must_use]
    pub fn hall_instructions(&self) -> Utf8PathBuf {
        self.root.join("HALL.md")
    }

    /// `<hall>/CLAUDE.md` or `<hall>/AGENTS.md` — the provider-native root
    /// alias that must point relatively to `HALL.md`.
    ///
    /// The filename comes from [`Provider::instruction_file`] for the same
    /// reason [`Self::harness_dir`] uses `config_dir` — the mapping is not the
    /// identity function, and guessing it writes config into a file the harness
    /// never reads. The alias is derived state: `ivar sync` keeps it a symlink
    /// to `HALL.md`, and sessions never read it — they read the canonical file.
    #[must_use]
    pub fn instruction_alias(&self, provider: &Provider) -> Utf8PathBuf {
        self.root.join(provider.instruction_file())
    }

    /// `<hall>/.mcp.json` or `<hall>/opencode.json` — where this harness's MCP
    /// server definitions materialise, at the hall root so every session in the
    /// hall discovers them by walk-up from its View Dir.
    ///
    /// The filename comes from [`Provider::mcp_config_path`] rather than being
    /// interpolated here — see [`Self::harness_dir`] for why the mapping is
    /// this module's job to get right exactly once.
    #[must_use]
    pub fn mcp_config(&self, provider: &Provider) -> Utf8PathBuf {
        self.root().join(provider.mcp_config_path())
    }

    /// `<hall>/.ivar/repos/` — the parent every repo's store dir sits under.
    ///
    /// Public because `sync` has to create it before the first clone lands, and
    /// deriving it from `repo_bare(...).parent().parent()` at the call site is
    /// exactly the path arithmetic outside this module that the module exists
    /// to prevent.
    #[must_use]
    pub fn repos_dir(&self) -> Utf8PathBuf {
        self.ivar_dir().join("repos")
    }

    /// `<hall>/.ivar/repos/<repo>/` — the whole store for one repo: the bare
    /// clone and every worktree off it.
    ///
    /// Public because deregister removes the entire tree in one step, and
    /// deriving it from `repo_bare(...).parent().parent()` at the call site is
    /// exactly the path arithmetic outside this module that the module exists
    /// to prevent.
    #[must_use]
    pub fn repo_dir(&self, repo: &RepoName) -> Utf8PathBuf {
        self.repos_dir().join(repo.as_str())
    }

    /// `<hall>/.ivar/features/` — every feature's directory.
    ///
    /// Public because deregister has to enumerate features to find the ones
    /// promoting a repo, and because a session TUI wants the same list.
    #[must_use]
    pub fn features_dir(&self) -> Utf8PathBuf {
        self.ivar_dir().join("features")
    }

    /// `<hall>/.ivar/sessions/` — every discovery-session view dir.
    ///
    /// Public because deregister has to enumerate live session view dirs to
    /// repair the symlinks it leaves dangling.
    #[must_use]
    pub fn discovery_sessions_dir(&self) -> Utf8PathBuf {
        self.ivar_dir().join("sessions")
    }

    /// The exact lines the hall's `.gitignore` needs.
    ///
    /// `.ivar/*` excludes every direct child of `.ivar/` as its own entry, and
    /// the two negations below re-include exactly the children that are meant
    /// to be committed. This only works because the exclusion is `.ivar/*` and
    /// not `.ivar/` — see the module doc comment's "gitignore trap" section for
    /// why the whole-directory form silently breaks the negations.
    ///
    /// Commands use a narrow `ivar-*.md` pattern because user command files
    /// remain committable; skills target dirs (.claude/skills/, .opencode/skills/)
    /// are ignored wholesale because every child is a derived target pointing
    /// into `.ivar/skills*`, and the symlinks are absolute/machine-local.
    /// The source of truth remains `.ivar/skills/` and `.ivar/skills-local/`,
    /// never the harness target dirs.
    #[must_use]
    pub fn gitignore_lines() -> Vec<&'static str> {
        vec![
            ".ivar/*",
            "!.ivar/skills/",
            "!.ivar/setups/",
            ".claude/commands/ivar-*.md",
            ".opencode/commands/ivar-*.md",
            ".claude/skills/",
            ".opencode/skills/",
        ]
    }

    /// `<hall>/.gitignore` — where [`Self::gitignore_lines`] belong.
    #[must_use]
    pub fn gitignore_path(&self) -> Utf8PathBuf {
        self.root.join(".gitignore")
    }

    /// `<hall>/.ivar/` — the one dotdir everything this tool manages lives
    /// under. Public because `init` has to create it, and deriving it anywhere
    /// else would put the dotdir's name in a second place. See the module doc
    /// comment: every path under a hall is computed here, nowhere else.
    #[must_use]
    pub fn ivar_dir(&self) -> Utf8PathBuf {
        self.root.join(IVAR_DIR)
    }
}

/// Why [`Layout::discover`] could not even start walking. This is the only
/// fallible operation in this module — everything else is pure path
/// arithmetic.
#[derive(Debug, thiserror::Error)]
pub enum DiscoverError {
    /// The starting path could not be resolved to an absolute, symlink-free
    /// form: it does not exist, or a component of it is not reachable.
    #[error("could not resolve `{path}`: {source}")]
    Unresolvable {
        path: Utf8PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl From<DiscoverError> for Failure {
    fn from(error: DiscoverError) -> Self {
        match error {
            DiscoverError::Unresolvable { path, source } => Failure::blocked(
                "layout.discover_unresolvable",
                format!("could not resolve `{path}` to look for a hall"),
            )
            .expected("a starting directory that exists and is reachable")
            .actual(source.to_string())
            .fix(FixAction::safe(
                "layout.check_starting_path",
                format!("Check that `{path}` exists and is reachable, then try again."),
            )),
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/store/layout.rs"]
mod tests;
