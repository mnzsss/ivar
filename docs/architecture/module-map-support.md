# Module map: domain, store and adapters

Part of the [architecture overview](../../ARCHITECTURE.md).

The first part of the map is in [module-map.md](module-map.md).

```
  domain/          pure types and invariants. No I/O, no git, no clap.
    name.rs        validated newtypes: HallName, RepoName, FeatureName, BranchName…
    feature/       a facade over focused files: feature.rs (the promotion
                   record and FeatureBoard — declared here as `promotion`, via
                   `#[path]`, because a module cannot share the name of the
                   directory that contains it), delivery.rs (guards and the
                   delivery preview), approval.rs (the SPDD gates), run.rs
                   (Run Receipts), integration.rs (the pure nested-integration
                   vocabulary: via/strategy/policy resolution, receipts and
                   verification evidence, and the derived integration-state
                   classifier), base.rs (effective_base, the
                   declared-base-or-default-branch fallback a promotion
                   resolves against), and cleanup.rs (the side-effect-free
                   cleanup preview and eligibility classifier). Children are
                   derived by scanning `Feature.parent` — no feature stores a
                   child list, and no lifecycle field is persisted.
    plan.rs        plan frontmatter as data: the repos a plan declares it edits
    plan_commands.rs  the shell commands a plan's fenced blocks would run
                   (`cd`, `mkdir`, `npm|pnpm run <script>`), scanned so
                   `plan approve` can refuse the ones that cannot resolve
    session.rs     session state and identity
    discovery.rs   pure discovery types (DiscoveryDoc, status, frontmatter)
    provider.rs    which harnesses exist, and their capability flags
    health.rs      hall health derivation (uninitialized/operational/stale/degraded)
    skill.rs skill_sync.rs  skills, and the sync plan over them
    mcp.rs         MCP server definitions as data
    agent/         scheduled agents: spec.rs (frontmatter), cron.rs (cron to
                   systemd OnCalendar), tool.rs, receipt.rs, unit.rs (systemd
                   unit text), container.rs (docker argv) and images.rs (the
                   image tags pinned to this binary's version)

  store/           on-disk persistence. Owns file layout, nothing else.
    versioned/     the version-detect / migrate / refuse-if-newer machine in
                   mod.rs, with its Error and Failure conversion in error.rs
    manifest/      ivar.json — committed, NEVER auto-migrates. model.rs owns
                   the data and invariants, persistence.rs the read/write/plan/
                   migrate, error.rs the Error and its Failure conversion,
                   schema.rs the published schema/<version>.json, generated
                   from the same types so the two cannot disagree.
    layout.rs      every path under a hall is computed here, nowhere else —
                   including `.ivar/features/<feature>/`, the canonical
                   `HALL.md`, and each provider's root alias (`CLAUDE.md` /
                   `AGENTS.md`)
    mcp_secrets.rs .ivar/secrets/mcp.env — durable local MCP OAuth credentials,
                   owner-only on Unix, never committed.
    gitignore.rs   the hall's .gitignore: append the needed lines, never clobber
    setup_receipt.rs  what a worktree's setup script did last time. The one file
                   NOT under layout: it lives in git's admin dir, so it dies
                   with the worktree it describes.
    feature.rs session.rs  the per-feature and per-session state files
    feature/       run.rs (the current Run Receipt, the immutable run/board
                   archives, and single-run lock) with legacy.rs (the
                   permanent v0->v3 board migration chain)
    discovery.rs   discovery doc parsing and rendering (frontmatter I/O)
    skill.rs       the lockfile and skill state
    agent.rs       agent definitions and run receipts under .ivar/agents/
    agent_secrets.rs  .ivar/secrets/agents.env, mode 600, read per agent
    render.rs      materialising a skill: symlink or copy, and verify

  git/             the only module that knows git exists
    mod.rs         the Git trait, and the real implementation over the two below
    error.rs       git::Error and its Failure conversion
    read.rs        git2: refs, HEAD, worktree list, ahead/behind, status, blobs
    exec.rs        the git binary: clone --bare, worktree add/rm, branch, fetch,
                   push, rebase, checkout — plus six pure local reads
                   (worktree_dirty, changed_paths, head_commit,
                   paths_committed_since, diff_worktree, commits_ahead) that
                   stay here rather than in read.rs. The ADR-0001 §3 split
                   ("reads go through git2, writes and network go through the
                   binary") is a rule with this one named exception: each of
                   these six parses `git`'s porcelain -z output, which is far
                   cheaper than the equivalent git2 walk for a plain local
                   read, and none of them touches a remote.
    protect.rs     the pre-commit hook that refuses a commit on a default
                   branch, and the worktree-local config that selects it. Apart
                   from exec.rs because it configures a repository rather than
                   operating on one.
    credential.rs  git credential-helper protocol, for the token fallback

  providers/       provider adapters and native facts
    mod.rs         the closed Provider dispatch facade: launch contracts, start
                   commands, MCP server docs, managed artifacts, instruction surfaces,
                   session projections, tool request parsing, guard outcomes,
                   credential installation, and login commands.
    claude_code/   Claude Code facts: launch flags, .mcp.json generation, hook
                   script and settings.json managed artifacts, JSON guard outcome.
    opencode/      OpenCode facts: launch flags, opencode.json generation, plugin
                   materialisation, shell.env guard outcome, auth storage adapter.
    omp/           OMP facts: launch flags, mcp.json generation with auth blocks,
                   pre-hook script, JSON block/reason guard outcome, auth-broker
                   adapter.

  harness/         shared provider orchestration
    mod.rs         shared orchestration module definitions.
    commands.rs    the shipped-command reconciliation: materialise/remove/
                   inspect against a session's harness config directory. It
                   is the module file *for* `commands/` — a sibling of the
                   directory, not a file inside it — where `commands/catalog.rs`
                   owns ShippedCommand + the COMMANDS data and the *.md
                   sources live, compiled in with include_str!. No command
                   content or file reconciliation lives in `bin/ivar.rs` or
                   `action`.
    config/        shared config materialisation: instructions.rs owns
                   the canonical `HALL.md` managed block, provider
                   root aliases (relative symlinks to `HALL.md`), and shared
                   `AGENTS.md` ownership; mcp.rs orchestrates MCP document
                   construction and secret resolution; session.rs builds the
                   session bootstrap block.
    opencode_auth.rs  read and write access to OpenCode's MCP OAuth credential
                   store (`mcp-auth.json`) for Figma tokens
  tui/             ratatui. Sync render, explicit drive.
    screen.rs      the Screen seam over vt100 — the emulator swap point
    widget.rs      pure deterministic projection of a snapshot into a Buffer
    driver.rs      all I/O: pty reads, resize, event folding. Owns no executor.
    scrollback.rs  the plain-text scrollback decoder driver.rs feeds: escape
                   sequences stripped from PTY bytes, kept off the emulator
                   seam so a vt100 swap never has to think about it
    pty.rs         the concrete PtsPty adapter over portable-pty, behind the
                   Pty trait the driver is generic over
    key_router.rs  pure reducer: (mode, key) -> (mode, action)
    master_detail.rs the shell-view layout, and the one event loop in the crate

  infra/           adapters to the outside world
    fs/            the filesystem primitive set. Nothing else touches std::fs.
                   io.rs owns reads/writes/directories/metadata/removal,
                   symlink.rs owns create/replace/read plus SymlinkTarget,
                   guard.rs owns the read-only guard (chmod, mode, write bits)
                   and `LiftedGuard` RAII scope guard,
                   and the facade owns the shared Error.
    json.rs        write_canonical — the ONLY on-disk JSON writer
    frontmatter.rs split + parse + emit YAML frontmatter. The YAML swap point.
    hash.rs        sha256 of a file, and of a tree
    proc/          subprocess spawn: `capture` and `inherit` in mod.rs, the
                   incremental line-protocol runner (`stream`/`Stream`) a
                   provider process needs in streaming.rs, and the Linux
                   /proc working-directory attribution in cwd.rs.
    systemd.rs     systemctl --user and the user unit directory
    docker.rs      the docker CLI an agent run drives
    webhook.rs     the HTTP POST behind an agent's `notify`
    oauth.rs       OAuth 2.0 PKCE and code-for-token exchange primitives
    figma.rs       Figma MCP client pre-registration and dynamic OAuth discovery
    http_callback.rs  transient loopback listener for OAuth authorization code
    github.rs      GitHub token lookup (and the credential-helper wiring that
                   derives it from `gh`/`$GITHUB_TOKEN` on each call). PR
                   operations themselves are not a trait seam: tests fake the
                   `gh` executable (tests/support/fake_gh.rs) — on PATH for an
                   integration binary, through `proc`'s test-only per-thread
                   program redirect for an in-process unit test — and the
                   shared command boundary lives in
                   `action/feature/pull_requests/mod.rs`.
    term.rs        colour, NO_COLOR, is-a-tty, width. Decides *whether* to
                   colour.
    progress.rs    the transient stderr line a long verb reports through.
                   Silent by default; bin/ivar.rs is the only thing that
                   builds a live one. Never part of an Outcome.

  error.rs         Failure · Status · FixAction · Warning · Report · style roles.
                   Style roles and paint() live here because the layout of a failure does,
                   and colour must not become a second copy of that layout.
```
