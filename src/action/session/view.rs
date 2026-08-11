//! View Dir materialisation: the shared core `session start`, `session
//! connect`, `session convert` and `execute tick` all run.
//!
//! A **View Dir** is the single directory an agent session works in: one
//! symlink per registered repo (promoted repos point at their feature
//! worktree, the rest at their read-only default-branch worktree), a real
//! per-session harness config dir, and — for feature sessions — the
//! feature's plan projected in and its bootstrap instructions materialised.
//!
//! # The harness config dir is real, never a symlink
//!
//! A later per-session write — the execute guard's `settings.json` — lands
//! inside `<view_dir>/<config_dir>/`. This used to symlink that whole
//! directory in from the hall under the name `.config`: wrong on two counts.
//! Claude Code reads `.claude/`, never `.config/`, and nothing in this crate
//! sets `CLAUDE_CONFIG_DIR`, so the hall's standing config — including the
//! shipped `/ivar-*` commands — never reached a session's agent at all. And
//! even fixed to the right name, a symlinked directory would send the
//! guard's per-session `settings.json` into `hall/.claude` itself, applying
//! one workstream's write guard to every session sharing the hall.
//!
//! A real directory keeps per-session state per-session. Only `commands/`
//! inside it is symlinked back to the hall — via [`Layout::commands_dir`],
//! not a hardcoded path, so the mapping from provider to dotdir stays in one
//! place — so the hall's shipped commands still reach the agent.
//!
//! The config dir follows the **session's own provider**, never the hall's
//! default: a relay from Claude Code to OpenCode materialises `.opencode/`
//! and OpenCode's commands, not the default provider's. That is what a relay
//! session actually launches with.
//!
//! # Only the active plan is projected
//!
//! A feature session gets `<view_dir>/plans/<feature>/` as a symlink to the
//! hall's committed `plans/<feature>/`, so the SPDD artifacts are reachable
//! through the session path the harness confines the agent to — and editable,
//! with writes landing in the hall's real plan directory. The link is
//! materialised even before the plan directory exists, so
//! `ivar plan create <feature>` run from inside the session makes the target
//! usable immediately. Plans of *other* features are never projected, and a
//! discovery session (no feature bound) gets no `plans/` at all.
//!
//! # Bootstrap instructions
//!
//! A feature session also receives the provider-native instruction file
//! (`CLAUDE.md` / `AGENTS.md`) at the View Dir root: the hall's standing
//! instructions plus a session bootstrap block that tells the agent how to
//! re-derive where the feature is in the SPDD cycle. The file is ephemeral —
//! it dies with the View Dir — and regenerated on every materialisation, so
//! `session connect` repairs it for sessions created before this existed. The
//! hall's own instruction file is never modified.
//!
//! # Idempotent, by comparison not by bookkeeping
//!
//! Every entry is replaced only when it changed ([`fs::replace_symlink_if_changed`],
//! byte comparison before writing the instruction file), so re-running this on
//! every `session connect` is a no-op when nothing drifted, and never renames a
//! symlink that already points at the right place (each rename opens a
//! transient resolution race — see `infra::fs`).

use camino::Utf8Path;

use crate::domain::feature::Feature;
use crate::domain::provider::Provider;
use crate::error::Failure;
use crate::harness::config;
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

/// Materialise `view_dir` for `feature`/`provider`: one symlink per registered
/// repo, a real per-session harness config dir with the hall's `commands/`
/// symlinked back in, and — for a feature session — the active plan projected
/// in and the bootstrap instructions written.
///
/// For a **feature session** (`feature: Some`), a promoted repo is symlinked
/// to its feature worktree (writable); every other repo is symlinked to its
/// default-branch worktree and that worktree is held read-only by the kernel
/// (write bits cleared). For a **discovery session** (`feature: None`), every
/// repo is a read-only default-branch worktree and no plan is projected.
///
/// `provider` is the session's own provider — what the session actually runs
/// (or ran) under — not the hall's default. It decides which config dir and
/// which instruction file the View Dir gets.
///
/// A repo whose worktree is missing is skipped with the rest still linked —
/// the session should still open for the repos that are there.
pub(crate) fn materialise(
    layout: &Layout,
    manifest: &Manifest,
    feature: Option<&Feature>,
    provider: Provider,
    view_dir: &Utf8Path,
) -> Result<(), Failure> {
    fs::ensure_dir(view_dir)?;

    for repo in manifest.repos() {
        let worktree = match feature {
            Some(feature) if feature.is_promoted(repo.name()) => {
                layout.repo_worktree(repo.name(), &feature.branch)
            }
            _ => layout.repo_worktree(repo.name(), repo.default_branch()),
        };
        if !fs::is_dir(&worktree)? {
            continue;
        }
        let link = view_dir.join(repo.name().as_str());
        // Replace only when the target changed: the view dir is re-materialised
        // on every connect, and an unchanged link must not be renamed (each
        // rename opens a transient resolution race — see `infra::fs`).
        fs::replace_symlink_if_changed(&worktree, &link)?;
        // A repo the session does not promote is held read-only by the kernel:
        // clear (or re-clear) the write bits on its default-branch worktree.
        if feature.is_none_or(|feature| !feature.is_promoted(repo.name())) {
            fs::clear_write_bits(&worktree)?;
        }
    }

    // The harness config dir — `.claude/` for claude-code, `.opencode/` for
    // opencode — is a real directory inside the view dir, never a symlink to
    // the hall's own (see the module doc for why). Only `commands/` is
    // symlinked back in, so the hall's shipped `/ivar-*` commands reach the
    // agent. It follows the session's provider, not the hall's default — a
    // relay must materialise the config of the provider it relays to.
    let config_dir = view_dir.join(provider.config_dir());
    fs::ensure_dir(&config_dir)?;
    let hall_commands = layout.commands_dir(&provider);
    if fs::is_dir(&hall_commands)? {
        let commands_link = config_dir.join("commands");
        fs::replace_symlink_if_changed(&hall_commands, &commands_link)?;
    }

    // Feature sessions: project the active plan and write the bootstrap
    // instructions. Discovery sessions get neither.
    if let Some(feature) = feature {
        project_plan(layout, feature, view_dir)?;
        materialise_session_instructions(layout, provider, feature, view_dir)?;
    }

    Ok(())
}

/// Project the session's feature plan into the view dir: a real `plans/`
/// directory with `plans/<feature>/` symlinked to the hall's committed plan
/// directory for that feature.
///
/// The link is created even when `plans/<feature>/` does not exist yet, so
/// `ivar plan create <feature>` run from inside the session makes the
/// projected plan usable immediately. Only the session's own feature is ever
/// linked — the hall's `plans/` directory is never projected wholesale.
fn project_plan(layout: &Layout, feature: &Feature, view_dir: &Utf8Path) -> Result<(), Failure> {
    let view_plans = view_dir.join("plans");
    fs::ensure_dir(&view_plans)?;
    let link = view_plans.join(feature.name.as_str());
    fs::replace_symlink_if_changed(&layout.plan_dir(&feature.name), &link)?;
    Ok(())
}

/// Write the provider-native instruction file (`CLAUDE.md` / `AGENTS.md`) at
/// the View Dir root: the session bootstrap block followed by the hall's
/// standing instructions, if any.
///
/// The file is ephemeral per-session state — it dies with the View Dir and is
/// regenerated on every materialisation, so `connect` repairs it — and the
/// hall's own instruction file is never modified. Bytes are compared before
/// writing, so an unchanged file is not rewritten.
fn materialise_session_instructions(
    layout: &Layout,
    provider: Provider,
    feature: &Feature,
    view_dir: &Utf8Path,
) -> Result<(), Failure> {
    let plan_rel = format!("plans/{}/plan.md", feature.name);
    let block = config::session::build_session_block(&feature.name, &plan_rel);

    let hall_file = layout.instruction_alias(&provider);
    let content = match fs::read_text(&hall_file)? {
        Some(hall) => format!("{block}\n\n{hall}"),
        None => block,
    };

    let target = view_dir.join(provider.instruction_file());
    let needs_write = match fs::read_text(&target)? {
        Some(existing) => existing != content,
        None => true,
    };
    if needs_write {
        fs::write_text(&target, &content)?;
    }
    Ok(())
}
