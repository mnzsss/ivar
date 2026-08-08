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
//!     features/<feature>/execution/    the feature execution board
//!     features/<feature>/sessions/<id>/  feature-session view dirs
//!     sessions/<id>/                   discovery-session view dirs
//!     setups/<repo>.sh                 per-repo setup scripts — COMMITTED
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
//!   `execution_dir(&FeatureName)`,
//!   `feature_session(&FeatureName, &SessionId)`, `discovery_session(&SessionId)`,
//!   `setup_script(&RepoName)`, `hall_skills()`, `plan_dir(&FeatureName)`,
//!   `harness_dir(&Provider)`.
//! - `gitignore_lines()` — the exact patterns the hall's `.gitignore` needs.
//!
//! # The gitignore trap
//!
//! It must be `.ivar/*` plus `!.ivar/skills/` plus `!.ivar/setups/` — **never**
//! `.ivar/`. Git does not re-include a child of an excluded directory, so `.ivar/`
//! followed by negations silently drops the hall's committed children from
//! version control. Silently. Put a test on this; the failure mode is invisible
//! until a teammate clones and the skills (or the setup scripts) are not there.
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

use crate::domain::name::{BranchName, FeatureName, RepoName, SessionId};
use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction};

/// The manifest's filename. Never one of its two predecessors — see the module
/// doc comment.
const MANIFEST_FILE_NAME: &str = "ivar.json";

/// The one dotdir everything `ivar` manages lives under.
const IVAR_DIR: &str = ".ivar";

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

    /// `<hall>/.ivar/features/<feature>/execution/` — the feature execution
    /// board.
    #[must_use]
    pub fn execution_dir(&self, feature: &FeatureName) -> Utf8PathBuf {
        self.feature_dir(feature).join("execution")
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

    /// `<hall>/.ivar/skills/` — hall-scoped skills. Committed.
    #[must_use]
    pub fn hall_skills(&self) -> Utf8PathBuf {
        self.ivar_dir().join("skills")
    }

    /// `<hall>/plans/<feature>/` — `requirements.md` · `analysis.md` ·
    /// `plan.md`. Committed.
    #[must_use]
    pub fn plan_dir(&self, feature: &FeatureName) -> Utf8PathBuf {
        self.root.join("plans").join(feature.as_str())
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

    /// `<hall>/CLAUDE.md` or `<hall>/AGENTS.md` — the Markdown file this
    /// harness reads for standing instructions, and where `ivar sync` keeps its
    /// managed block.
    ///
    /// Committed, and mostly the user's: `harness::config` owns only the bytes
    /// between its two markers. The filename comes from
    /// [`Provider::instruction_file`] for the same reason
    /// [`Self::harness_dir`] uses `config_dir` — the mapping is not the
    /// identity function, and guessing it writes config into a file the harness
    /// never reads.
    #[must_use]
    pub fn instruction_file(&self, provider: &Provider) -> Utf8PathBuf {
        self.root().join(provider.instruction_file())
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
    #[must_use]
    pub fn gitignore_lines() -> Vec<&'static str> {
        vec![".ivar/*", "!.ivar/skills/", "!.ivar/setups/"]
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
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use camino::Utf8PathBuf;

    use super::*;
    use crate::error::Status;
    use crate::test_support::canonical_temp_dir as hall_root;

    /// A fresh temp directory, canonicalised once so every path built from it
    /// in a test already matches what `Layout::discover`'s own internal
    /// canonicalisation will produce — tempdirs commonly sit behind a symlink
    /// (`/tmp` -> `/private/tmp` on macOS), and comparing a raw path against a
    /// canonicalised one would fail for reasons that have nothing to do with
    /// the behaviour under test.

    // -- discover -------------------------------------------------------------

    #[test]
    fn discover_finds_a_hall_at_the_starting_directory() {
        let (_guard, root) = hall_root();
        std::fs::write(root.join(MANIFEST_FILE_NAME), "{}").expect("write manifest");

        let layout = Layout::discover(&root)
            .expect("discover succeeds")
            .expect("hall is found");
        assert_eq!(layout.root(), root.as_path());
    }

    #[test]
    fn discover_finds_a_hall_from_several_levels_down() {
        let (_guard, root) = hall_root();
        std::fs::write(root.join(MANIFEST_FILE_NAME), "{}").expect("write manifest");
        let nested = root.join("a").join("b").join("c");
        std::fs::create_dir_all(&nested).expect("create nested dirs");

        let layout = Layout::discover(&nested)
            .expect("discover succeeds")
            .expect("hall is found");
        assert_eq!(layout.root(), root.as_path());
    }

    #[test]
    fn discover_returns_none_above_any_hall() {
        let (_guard, root) = hall_root();
        let nested = root.join("no-hall-anywhere-near-here");
        std::fs::create_dir_all(&nested).expect("create nested dir");

        let found = Layout::discover(&nested).expect("discover succeeds");
        assert!(found.is_none());
    }

    #[test]
    fn discover_does_not_mistake_a_directory_named_ivar_json_for_the_manifest() {
        let (_guard, root) = hall_root();
        std::fs::write(root.join(MANIFEST_FILE_NAME), "{}").expect("write the real manifest");
        let nested = root.join("inner");
        std::fs::create_dir_all(nested.join(MANIFEST_FILE_NAME))
            .expect("create a directory named ivar.json");

        let layout = Layout::discover(&nested)
            .expect("discover succeeds")
            .expect("hall is found one level up, past the directory");
        assert_eq!(layout.root(), root.as_path());
    }

    #[test]
    fn discover_fails_when_the_starting_path_does_not_exist() {
        let (_guard, root) = hall_root();
        let missing = root.join("does-not-exist");

        let error = Layout::discover(&missing).expect_err("missing path is unresolvable");
        assert!(matches!(error, DiscoverError::Unresolvable { .. }));
    }

    #[test]
    fn discover_error_converts_to_a_blocked_failure_pointing_at_the_path() {
        let (_guard, root) = hall_root();
        let missing = root.join("does-not-exist");
        let error = Layout::discover(&missing).expect_err("missing path is unresolvable");

        let failure: Failure = error.into();
        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "layout.discover_unresolvable");
        assert_eq!(failure.fix_actions.len(), 1);
        assert!(failure.fix_actions[0].safe);
    }

    // -- gitignore_lines --------------------------------------------------------

    /// `.ivar/` (the whole-directory form) is wrong, and the failure is
    /// invisible until someone goes looking for it: git prunes an *ignored
    /// directory* before it ever evaluates a negation for anything inside it,
    /// so `!.ivar/skills/` (or `!.ivar/setups/`) after a bare `.ivar/` would
    /// never re-include the hall's committed skills or setup scripts. No
    /// error, no warning — a teammate just clones the hall and those
    /// directories are not there. `.ivar/*` excludes each direct child as its
    /// own entry instead of the directory as a whole, which is exactly what
    /// lets the negations below reach in and un-ignore specific children.
    #[test]
    fn gitignore_lines_excludes_the_dotdir_per_entry_and_reincludes_committed_children() {
        assert_eq!(
            Layout::gitignore_lines(),
            vec![".ivar/*", "!.ivar/skills/", "!.ivar/setups/"]
        );
    }

    // -- path accessors -----------------------------------------------------

    #[test]
    fn accessors_compute_the_documented_paths() {
        let layout = Layout::at("/hall");
        let repo = RepoName::new("api").unwrap();
        let branch = BranchName::new("main").unwrap();
        let feature = FeatureName::new("checkout").unwrap();
        let session = SessionId::new("2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c").unwrap();

        assert_eq!(layout.manifest(), Utf8PathBuf::from("/hall/ivar.json"));
        assert_eq!(layout.state(), Utf8PathBuf::from("/hall/.ivar/state.json"));
        assert_eq!(
            layout.repo_bare(&repo),
            Utf8PathBuf::from("/hall/.ivar/repos/api/.bare")
        );
        assert_eq!(
            layout.repo_worktree(&repo, &branch),
            Utf8PathBuf::from("/hall/.ivar/repos/api/main")
        );
        assert_eq!(
            layout.feature_dir(&feature),
            Utf8PathBuf::from("/hall/.ivar/features/checkout")
        );
        assert_eq!(
            layout.execution_dir(&feature),
            Utf8PathBuf::from("/hall/.ivar/features/checkout/execution")
        );
        assert_eq!(
            layout.planning_dir(&feature),
            Utf8PathBuf::from("/hall/.ivar/features/checkout/planning")
        );
        assert_eq!(
            layout.feature_session(&feature, &session),
            Utf8PathBuf::from(
                "/hall/.ivar/features/checkout/sessions/2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c"
            )
        );
        assert_eq!(
            layout.feature_sessions_dir(&feature),
            Utf8PathBuf::from("/hall/.ivar/features/checkout/sessions")
        );
        assert_eq!(
            layout.workspace_file(&feature),
            Utf8PathBuf::from("/hall/checkout.code-workspace")
        );
        assert_eq!(
            layout.discovery_session(&session),
            Utf8PathBuf::from("/hall/.ivar/sessions/2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c")
        );
        assert_eq!(
            layout.setup_script(&repo),
            Utf8PathBuf::from("/hall/.ivar/setups/api.sh")
        );
        assert_eq!(
            layout.hall_skills(),
            Utf8PathBuf::from("/hall/.ivar/skills")
        );
        assert_eq!(layout.repos_dir(), Utf8PathBuf::from("/hall/.ivar/repos"));
        assert_eq!(
            layout.repo_dir(&repo),
            Utf8PathBuf::from("/hall/.ivar/repos/api")
        );
        assert_eq!(
            layout.features_dir(),
            Utf8PathBuf::from("/hall/.ivar/features")
        );
        assert_eq!(
            layout.discovery_sessions_dir(),
            Utf8PathBuf::from("/hall/.ivar/sessions")
        );
        assert_eq!(
            layout.instruction_file(&Provider::ClaudeCode),
            Utf8PathBuf::from("/hall/CLAUDE.md")
        );
        assert_eq!(
            layout.instruction_file(&Provider::OpenCode),
            Utf8PathBuf::from("/hall/AGENTS.md")
        );
        assert_eq!(
            layout.mcp_config(&Provider::ClaudeCode),
            Utf8PathBuf::from("/hall/.mcp.json")
        );
        assert_eq!(
            layout.mcp_config(&Provider::OpenCode),
            Utf8PathBuf::from("/hall/opencode.json")
        );
        assert_eq!(
            layout.plan_dir(&feature),
            Utf8PathBuf::from("/hall/plans/checkout")
        );
    }

    /// The regression guard for the bug `domain::provider` exists to make
    /// impossible: `Provider::ClaudeCode` maps to `.claude`, not `.claude-code`.
    /// Written as an explicit literal, not parameterised over the id, so this
    /// test cannot pass by accident if `harness_dir` goes back to interpolating
    /// `provider.id()`.
    #[test]
    fn harness_dir_maps_claude_code_to_dot_claude_not_dot_claude_code() {
        let layout = Layout::at("/hall");

        assert_eq!(
            layout.harness_dir(&Provider::ClaudeCode),
            Utf8PathBuf::from("/hall/.claude")
        );
    }

    #[test]
    fn repo_worktree_nests_a_slash_containing_branch_name() {
        let layout = Layout::at("/hall");
        let repo = RepoName::new("api").unwrap();
        let branch = BranchName::new("feat/auth-v2").unwrap();

        assert_eq!(
            layout.repo_worktree(&repo, &branch),
            Utf8PathBuf::from("/hall/.ivar/repos/api/feat/auth-v2")
        );
    }
}
