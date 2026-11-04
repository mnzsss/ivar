use super::*;

impl Layout {
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

    /// `<hall>/.ivar/feedback/` — local feedback reports and proposals.
    #[must_use]
    pub fn feedback_dir(&self) -> Utf8PathBuf {
        self.ivar_dir().join("feedback")
    }

    /// `<hall>/.ivar/feedback/<id>.md` — a specific feedback report or proposal.
    #[must_use]
    pub fn feedback_doc(&self, id: &str) -> Utf8PathBuf {
        self.feedback_dir().join(format!("{id}.md"))
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

    /// `<hall>/.ivar/features/<feature>/review/comments.json` — local review
    /// comments on the feature's changes.
    #[must_use]
    pub fn review_comments_file(&self, feature: &FeatureName) -> Utf8PathBuf {
        self.feature_dir(feature)
            .join("review")
            .join("comments.json")
    }

    /// `<hall>/.ivar/features/<feature>/<feature>.code-workspace` — multi-root
    /// workspace file opening promoted repos writable and context repos read-only.
    #[must_use]
    pub fn feature_workspace(&self, feature: &FeatureName) -> Utf8PathBuf {
        self.feature_dir(feature)
            .join(format!("{feature}.code-workspace"))
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

    /// `<hall>/.ivar/sessions/<id>/` — a discovery-session view dir.
    #[must_use]
    pub fn discovery_session(&self, session: &SessionId) -> Utf8PathBuf {
        self.ivar_dir().join("sessions").join(session.as_str())
    }

    /// `<view_dir>/.tmp/` — a session scratch directory, for an agent
    /// temporary files.
    ///
    /// Takes the view dir rather than a session identity, because that
    /// is the one input: both session kinds answer the same way, and
    /// the hall root does not enter the answer. Associated rather than
    /// a method for the same reason — callers holding only a view dir
    /// need it.
    ///
    /// Inside the view dir, so it is already in the session writable set
    /// and is torn down with the view dir by `ivar session stop`.
    #[must_use]
    pub fn session_scratch(view_dir: &Utf8Path) -> Utf8PathBuf {
        view_dir.join(crate::domain::session::SCRATCH_DIR)
    }
}
