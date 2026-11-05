# Module map: entrypoint, CLI and actions

Part of the [architecture overview](../../ARCHITECTURE.md).

## Module map

Describes the tree as it is. When a file moves, this moves with it — a map that
has drifted is worse than none, because it is the thing a newcomer trusts.

```
src/
  bin/ivar.rs      entrypoint: parse argv, dispatch, render, set exit code. No logic.
  lib.rs           crate attrs, module tree, the layering rule as a doc comment.

  app/             the composition root: dispatch and rendering. Only layer
                   allowed to reach every other one.
    run.rs         turns a parsed `Cli` into an exit code: matches the command,
                   converts args into an action's `Input`, renders the outcome.
    respond.rs     rendering an action's `Outcome`/`Report`/`Failure` as bytes.
    graph_dispatch.rs  the `ivar graph` subcommand tree's dispatch.
    agent_dispatch.rs  the `ivar agent` subcommand tree's dispatch.

  cli/             clap derive types ONLY — structs, enums, doc comments.
    root.rs        root entries and command-group wiring; focused command
                   modules own their argument types and conversions.
    feature/       feature subcommands in mod.rs, lifecycle arguments and
                   conversions in lifecycle.rs, run commands in execution.rs,
                   and ordered delivery metadata parsing in delivery.rs.

  action/          one function per leaf command. The unit of behaviour.
    hall/          init · status · doctor · migrate · cleanup — one file per
                   verb (init.rs, status.rs, doctor.rs, migrate.rs, cleanup.rs);
                   the facade owns the shared discovery/read/prompt helpers.
    sync/          sync — big enough to own a directory: the public verb and
                   report types live in mod.rs, repo materialisation in
                   repo.rs, provider/config reconciliation in providers.rs,
                   setup-script execution in setup.rs, and the one-time
                   `graph` to `ivar` MCP entry rename in mcp_rename.rs.
    repo/          add · list · remove · pull · setup · upstream, where pull/
                   splits the sweep in mod.rs from diagnosis.rs — whether a
                   diverged branch is safe to reset, and which blocker to name
    discovery/     create · list · show · amend · close — a unit of work's
                   working brief (.ivar/features/<name>/discovery.md) before
                   it earns execution.
    feature/       create · list · promote · demote · status · reparent ·
                   close · delete · rebase · view · prune · cleanup, plus
                   deliver/ — the preview fingerprint, push, and pull-request
                   phases split across mod.rs, repos.rs, and preview.rs,
                   with land/ (mod.rs, execute.rs, preflight.rs) for fast-forward
                   merge and push — and integrate/ — orchestration and locking
                   in mod.rs, result types in types.rs, validation in preflight.rs,
                   per-repo receipt reuse/resume in repos.rs, persistence and
                   staging cleanup in receipts.rs, with local.rs and pr.rs
                   applying the selected integration path. Nested-subfeature machinery
                   lives in private focused modules: relations.rs (the
                   child-derived tree, receipt freshness, and descendant
                   blockers), lifecycle.rs (the shared plan-frontmatter close
                   seam), mutation.rs (the scoped whole-child/per-promotion
                   mutation guards), verification.rs (the ordered executable
                   checks), reparent.rs, and pull_requests/ (shared lookup,
                   mutation, merge observation and sibling-PR operations).
    execute/       feature execute: start · finish · status · accept-revision ·
                   checkpoint (checkpoint.rs records an approved wave on the
                   active run without touching the plan fingerprint), the
                   provider-neutral Run Receipt lifecycle
    session/       start · connect · conversion · stop · prune · relay, plus
                   view (the shared View Dir materialisation: repo symlinks,
                   per-session harness config, the projected plan, the
                   bootstrap instructions) and
                   lookup (shared id-prefix resolution)
    plan/          create · list · show · approve · status
    provider/      add · list
    agent/         sync · status · run · logs · feedback · notify telegram
                   setup for scheduled agents, plus requirements.rs (the
                   systemd and Docker checks doctor reports), notify.rs (the
                   run summary to a webhook or Telegram), notify_setup.rs (the
                   interactive Telegram setup) and feature_access.rs (the `access: feature` worktree, host
                   commit and deliver preview fingerprint).
    skill/         create · add · update · remove · detach · sync · status ·
                   list · doctor
    memory/        status · clean · query · reindex · lint, plus capture, drain and
                   sweep for observations. reindex.rs walks `docs/memory/` and
                   `.ivar/memory/` into `pages` (size and mtime, confirmed by
                   hash); query.rs fuses page FTS and link neighbours; lint.rs
                   checks the page rules on disk. mcp/ holds the `memory_*` tool
                   schemas and handlers that `ivar graph mcp` routes to: pages.rs
                   writes, patches and deletes, confine.rs confines paths,
                   observations.rs reads the capture log.
    mcp/           auth — big enough to own a directory: the public verb,
                   input/outcome types, and resolution live in auth/mod.rs,
                   Figma pre-registration and the secret handoff in
                   auth/preregister.rs, and provider command construction and
                   dispatch in auth/dispatch.rs.
```
