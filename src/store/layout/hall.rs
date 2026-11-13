use super::*;

impl Layout {
    /// `<hall>/.ivar/setups/` — committed setup scripts and session hooks.
    #[must_use]
    pub fn hall_setups(&self) -> Utf8PathBuf {
        self.ivar_dir().join("setups")
    }

    /// `<hall>/.ivar/setups/<repo>.sh` — the repo's setup script. Committed:
    /// a git worktree shares history but not untracked files, so a fresh
    /// worktree needs this to bootstrap `.env`, `node_modules`, and so on.
    #[must_use]
    pub fn setup_script(&self, repo: &RepoName) -> Utf8PathBuf {
        self.hall_setups().join(format!("{}.sh", repo.as_str()))
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
        self.hall_setups()
            .join(format!("{}.session.sh", repo.as_str()))
    }

    /// `<hall>/.ivar/secrets/` — where a setup script reads values git does not
    /// carry, and where Ivar stores managed MCP credentials in `mcp.env`.
    /// Gitignored by the same `.ivar/*` rule that covers the rest of local state.
    ///
    /// General per-repo secrets in this directory are hand-maintained and never
    /// written by `ivar`. This location is handed to setup scripts through
    /// `IVAR_SECRETS_DIR`. The one file managed directly by Ivar under this
    /// directory is [`Self::mcp_secrets_env`].
    #[must_use]
    pub fn secrets_dir(&self) -> Utf8PathBuf {
        self.ivar_dir().join("secrets")
    }

    /// `<hall>/.ivar/secrets/mcp.env` — local, gitignored storage for
    /// Ivar-managed MCP OAuth client credentials.
    ///
    /// Managed directly by `store::mcp_secrets` with owner-only permissions
    /// on Unix. Distinct from other files under `secrets/`, which remain
    /// hand-maintained.
    #[must_use]
    pub fn mcp_secrets_env(&self) -> Utf8PathBuf {
        self.secrets_dir().join("mcp.env")
    }

    /// `<hall>/agents/` — scheduled agent definitions. Committed.
    #[must_use]
    pub fn agents_dir(&self) -> Utf8PathBuf {
        self.root.join("agents")
    }

    /// `<hall>/agents/done/` — concluded Agent Tasks. Never a source of agents.
    #[must_use]
    pub fn agents_done_dir(&self) -> Utf8PathBuf {
        self.agents_dir().join("done")
    }

    /// `<hall>/agents/<name>.md` — one scheduled agent's definition.
    #[must_use]
    pub fn agent_file(&self, name: &str) -> Utf8PathBuf {
        self.agents_dir().join(format!("{name}.md"))
    }

    /// `<hall>/.ivar/agents/` — local state of scheduled agent runs.
    #[must_use]
    pub fn agent_state_dir(&self) -> Utf8PathBuf {
        self.ivar_dir().join("agents")
    }

    /// `<hall>/.ivar/agents/runs/<agent>/` — one agent's run receipts.
    #[must_use]
    pub fn agent_runs_dir(&self, agent: &str) -> Utf8PathBuf {
        self.agent_state_dir().join("runs").join(agent)
    }

    /// `<hall>/.ivar/agents/runs/<agent>/<run-id>.json` — one run's receipt.
    #[must_use]
    pub fn agent_run_receipt(&self, agent: &str, run_id: &str) -> Utf8PathBuf {
        self.agent_runs_dir(agent).join(format!("{run_id}.json"))
    }

    /// `<hall>/.ivar/agents/locks/<agent>.lock` — held while the agent runs.
    #[must_use]
    pub fn agent_lock(&self, agent: &str) -> Utf8PathBuf {
        self.agent_state_dir()
            .join("locks")
            .join(format!("{agent}.lock"))
    }

    /// `<hall>/.ivar/agents/locks/tools/<tool>.lock` — held while a host run of
    /// the tool holds a copy of its login, so such runs go one at a time.
    #[must_use]
    pub fn tool_lock(&self, tool: &str) -> Utf8PathBuf {
        self.agent_state_dir()
            .join("locks")
            .join("tools")
            .join(format!("{tool}.lock"))
    }

    /// `<hall>/.ivar/agents/state/<agent>/<tool>/` — a host agent's private
    /// copy of the tool's login and everything the tool writes beside it.
    #[must_use]
    pub fn agent_tool_state(&self, agent: &str, tool: &str) -> Utf8PathBuf {
        self.agent_state_dir().join("state").join(agent).join(tool)
    }

    /// `<hall>/.ivar/agents/tmp/<run-id>/` — one run's scratch directory.
    #[must_use]
    pub fn agent_run_scratch(&self, run_id: &str) -> Utf8PathBuf {
        self.agent_state_dir().join("tmp").join(run_id)
    }

    /// `<hall>/.ivar/secrets/agents.env` — values for agents' `secrets`.
    #[must_use]
    pub fn agent_secrets_env(&self) -> Utf8PathBuf {
        self.secrets_dir().join("agents.env")
    }

    /// `<hall>/.ivar/skills/` — hall-scoped skills. Committed.
    #[must_use]
    pub fn hall_skills(&self) -> Utf8PathBuf {
        self.ivar_dir().join("skills")
    }

    /// Local skills that never enter the hall's git repo.
    ///
    /// The `.gitignore` is `.ivar/*` + `!.ivar/skills/` + `!.ivar/setups/` +
    /// `!.ivar/scripts/`, so any new directory here is gitignored by default.
    #[must_use]
    pub fn hall_skills_local(&self) -> Utf8PathBuf {
        self.ivar_dir().join("skills-local")
    }

    /// `<hall>/.ivar/scripts/` — the team's hall scripts. Committed.
    /// `ivar` only reads it.
    #[must_use]
    pub fn hall_scripts(&self) -> Utf8PathBuf {
        self.ivar_dir().join("scripts")
    }

    /// `<hall>/.ivar/scripts-local/` — personal hall scripts. Ignored by the
    /// `.ivar/*` line, so it never enters the hall's git repo. `ivar` only
    /// reads it.
    #[must_use]
    pub fn hall_scripts_local(&self) -> Utf8PathBuf {
        self.ivar_dir().join("scripts-local")
    }

    /// `<hall>/.ivar/features/<feature>/` — the unit of work's local working
    /// documents: `requirements.md` · `analysis.md` · `plan.md` · `tasks/` ·
    /// `discovery.md`. Gitignored.
    #[must_use]
    pub fn plan_dir(&self, feature: &FeatureName) -> Utf8PathBuf {
        self.feature_dir(feature)
    }

    /// `<hall>/.ivar/features/<feature>/discovery.md` — why this work exists.
    /// ivar owns the front matter; the agent owns the prose. Gitignored.
    #[must_use]
    pub fn discovery_doc(&self, feature: &FeatureName) -> Utf8PathBuf {
        self.feature_dir(feature).join("discovery.md")
    }

    /// `<hall>/docs/memory/` — committed durable memory pages, one directory per kind.
    #[must_use]
    pub fn docs_memory_dir(&self) -> Utf8PathBuf {
        self.root.join("docs").join("memory")
    }

    #[must_use]
    pub fn page_dir(&self, kind: crate::domain::memory_page::PageKind) -> Utf8PathBuf {
        let base = if kind.is_durable() {
            self.docs_memory_dir()
        } else {
            self.ivar_dir().join("memory")
        };
        base.join(kind.as_str())
    }

    #[must_use]
    pub fn update_pages_dir(&self) -> Utf8PathBuf {
        self.page_dir(crate::domain::memory_page::PageKind::Update)
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

    /// `<hall>/<provider-dotdir>/skills/` — the provider-native skills directory.
    ///
    /// `ivar` owns only directories matching `ivar-*` inside it; every other
    /// directory belongs to the user and must survive every operation.
    #[must_use]
    pub fn skills_dir(&self, provider: &Provider) -> Utf8PathBuf {
        self.root().join(provider.skills_dir())
    }

    /// `<hall>/.opencode/plugins/` — the provider-native plugin directory.
    /// Returns `None` for providers that have no plugin system (Claude Code).
    #[must_use]
    pub fn plugins_dir(&self, provider: &Provider) -> Option<Utf8PathBuf> {
        provider.plugins_dir().map(|dir| self.root().join(dir))
    }

    /// Hall-root paths no session may write although they sit outside
    /// `.ivar/`, because each launches code outside any sandbox: the hall's
    /// git hooks and config, every provider's hook config (where `ivar guard`
    /// is wired) and MCP config, and the hall's `.env`.
    #[must_use]
    pub fn guard_protected_paths(&self) -> Vec<Utf8PathBuf> {
        let git_dir = self.root.join(".git");
        [
            git_dir.join("hooks"),
            git_dir.join("config"),
            self.root.join(".env"),
        ]
        .into_iter()
        .chain(
            Provider::ALL
                .iter()
                .flat_map(Provider::hook_config_paths)
                .map(|path| self.root.join(path)),
        )
        .chain(
            Provider::ALL
                .iter()
                .map(|provider| self.mcp_config(provider)),
        )
        .collect()
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
    /// the three negations below re-include exactly the children that are meant
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
    ///
    /// `.tmp/` is the scratch dir agents and scripts create at the hall root;
    /// it is ignored so working files never reach a diff.
    #[must_use]
    pub fn gitignore_lines() -> Vec<&'static str> {
        vec![
            ".ivar/*",
            "!.ivar/skills/",
            "!.ivar/setups/",
            "!.ivar/scripts/",
            ".claude/commands/ivar-*.md",
            ".opencode/commands/ivar-*.md",
            ".omp/commands/ivar-*.md",
            ".claude/skills/",
            ".opencode/skills/",
            ".omp/skills/",
            ".tmp/",
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

    /// `<hall>/.ivar/hall-sync.lock` — held while the hall's pending changes
    /// are committed and pushed, so two runs never interleave their git
    /// commands. Ignored by the `.ivar/*` line like the rest of local state.
    #[must_use]
    pub fn hall_sync_lock(&self) -> Utf8PathBuf {
        self.ivar_dir().join("hall-sync.lock")
    }

    /// `<hall>/.ivar/hall-sync.log` — what a detached `ivar hall sync` printed,
    /// for the sync a `SessionEnd` hook starts and cannot wait for. Ignored by
    /// the `.ivar/*` line like the rest of local state.
    #[must_use]
    pub fn hall_sync_log(&self) -> Utf8PathBuf {
        self.ivar_dir().join("hall-sync.log")
    }

    /// `<hall>/.ivar/graph.db` — the code graph, derived and disposable.
    #[must_use]
    pub fn graph_db(&self) -> Utf8PathBuf {
        self.ivar_dir().join("graph.db")
    }

    /// `<hall>/.ivar/graph.lock` — serialises writers of [`Self::graph_db`].
    #[must_use]
    pub fn graph_lock(&self) -> Utf8PathBuf {
        self.ivar_dir().join("graph.lock")
    }

    /// `<hall>/.ivar/memory.db` — the hall memory. Only `ivar memory clean`
    /// deletes it.
    #[must_use]
    pub fn memory_db(&self) -> Utf8PathBuf {
        self.ivar_dir().join("memory.db")
    }

    /// `<hall>/.ivar/memory.lock` — serialises writers of [`Self::memory_db`].
    /// Binaries older than the split take this file as their graph index
    /// lock, which is why the legacy move holds it too.
    #[must_use]
    pub fn memory_lock(&self) -> Utf8PathBuf {
        self.ivar_dir().join("memory.lock")
    }

    /// `<hall>/.ivar/spool/memory` — capture hooks append session JSONL files
    /// here; ignored by git through `.ivar/*`.
    #[must_use]
    pub fn memory_spool_dir(&self) -> Utf8PathBuf {
        self.ivar_dir().join("spool").join("memory")
    }
}
