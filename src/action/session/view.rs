//! View Dir materialisation: the shared core for `session start`, `session
//! connect`, and `session convert`.
//!
//! A **View Dir** is the single directory an agent session works in: one
//! symlink per registered repo (promoted repos point at their feature
//! worktree, the rest at their read-only default-branch worktree), a real
//! per-session harness config dir, and — for feature sessions — the
//! session bootstrap instructions materialised.
//!
//! # The harness config dir is real, never a symlink
//!
//! Per-session harness state such as `settings.json` lands inside
//! `<view_dir>/<config_dir>/`. This used to symlink that whole directory in
//! from the hall under the name `.config`: wrong on two counts. Claude Code
//! reads `.claude/`, never `.config/`, and nothing in this crate sets
//! `CLAUDE_CONFIG_DIR`, so the hall's standing config — including the shipped
//! `/ivar-*` commands — never reached a session's agent at all. A symlinked
//! directory would also send per-session `settings.json` into `hall/.claude`
//! itself.
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
//! # Instruction files are derived from `HALL.md`, never from an alias
//!
//! Every view dir receives the provider-native instruction file
//! (`CLAUDE.md` / `AGENTS.md`) at its root, derived from the hall's
//! canonical `HALL.md` — never from the root alias, whose bytes and target
//! are irrelevant here:
//!
//! - a discovery session's file is exactly the `HALL.md` content;
//! - a feature session's file is the session bootstrap block, two newlines,
//!   then the `HALL.md` content.
//!
//! The file is ephemeral — it dies with the View Dir — and regenerated on
//! every materialisation, so `session connect` repairs it. The hall's own
//! `HALL.md` is never modified. When `HALL.md` is absent or not a regular
//! file, the session still opens with a warning
//! (`instructions.canonical_unavailable`): a feature session receives only
//! its bootstrap, a discovery session receives no shared content. There is
//! no deliberate fallback to a legacy alias.
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
use crate::error::{Failure, Warning};
use crate::harness::config;
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

/// What view-dir materialisation found worth reporting without failing the
/// session: the canonical `HALL.md` being unavailable, for example.
#[derive(Debug, Default)]
pub(crate) struct MaterialiseReport {
    /// Anything that needs attention but must not stop the session opening.
    pub warnings: Vec<Warning>,
}

/// Materialise `view_dir` for `feature`/`provider`: one symlink per registered
/// repo, a real per-session harness config dir with the hall's `commands/`
/// symlinked back in, the provider-native instruction file derived from
/// `HALL.md`, and — for a feature session — the bootstrap instructions written.
///
/// For a **feature session** (`feature: Some`), a promoted repo is symlinked
/// to its feature worktree (writable); every other repo is symlinked to its
/// default-branch worktree and that worktree is held read-only by the kernel
/// (write bits cleared). For a **discovery session** (`feature: None`), every
/// repo is a read-only default-branch worktree.
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
) -> Result<MaterialiseReport, Failure> {
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
    // Project memory symlink if canonical memory root exists
    crate::domain::memory::project_memory_symlink(layout, view_dir)?;


    // The harness config dir — `.claude/` for claude-code, `.opencode/` for
    // opencode, `.omp/` for omp — is a real directory inside the view dir, never
    // a symlink to the hall's own (see the module doc for why). Surfaces
    // declared by the provider (commands, plus hooks for OMP) are symlinked
    // back in, so the hall's catalog and hooks reach the agent. It follows
    // the session's provider, not the hall's default — a relay must
    // materialise the config of the provider it relays to.
    let config_dir = view_dir.join(provider.config_dir());
    fs::ensure_dir(&config_dir)?;

    for projection in crate::providers::session_projections(provider) {
        let hall_source = layout.root().join(&projection.hall_source);
        if fs::is_dir(&hall_source)? {
            let dest_link = config_dir.join(&projection.config_relative_dest);
            if let Some(parent) = dest_link.parent() {
                fs::ensure_dir(parent)?;
            }
            fs::replace_symlink_if_changed(&hall_source, &dest_link)?;
        }
    }

    let mut report = MaterialiseReport::default();
    materialise_session_instructions(layout, manifest, provider, feature, view_dir, &mut report)?;

    Ok(report)
}

/// Write the provider-native instruction file (`CLAUDE.md` / `AGENTS.md`) at
/// the View Dir root, derived from the canonical `HALL.md`.
///
/// A discovery session's file is exactly the canonical content; a feature
/// session's file is the session bootstrap block followed by it. The
/// canonical file is read directly — never the root alias — and when it is
/// missing or not a regular file, the session still opens with a warning:
/// the feature session receives only its bootstrap, the discovery session
/// receives no shared content.
///
/// The file is ephemeral per-session state — it dies with the View Dir and is
/// regenerated on every materialisation, so `connect` repairs it — and the
/// hall's own file is never modified. Bytes are compared before writing, so
/// an unchanged file is not rewritten.
fn materialise_session_instructions(
    layout: &Layout,
    manifest: &Manifest,
    provider: Provider,
    feature: Option<&Feature>,
    view_dir: &Utf8Path,
    report: &mut MaterialiseReport,
) -> Result<(), Failure> {
    let target = view_dir.join(provider.instruction_file());

    // Only a regular `HALL.md` counts: a symlink or directory is not the
    // canonical state, and there is no fallback to a legacy alias.
    let canonical = layout.hall_instructions();
    let hall = match fs::read_symlink(&canonical)? {
        fs::SymlinkTarget::NotASymlink if fs::is_file(&canonical)? => {
            fs::read_text(&canonical)?.unwrap_or_default()
        }
        _ => String::new(),
    };

    if hall.is_empty() {
        report.warnings.push(Warning::new(
            "instructions.canonical_unavailable",
            "hall",
            "`HALL.md` is missing or not a regular file; the session opens without the hall's              shared instructions",
        ));
    }

    let base_content = match feature {
        Some(feature) => {
            let plan_rel = "../../plan.md";
            let block = config::session::build_session_block(&feature.name, plan_rel);
            if hall.is_empty() {
                block
            } else {
                format!("{block}\n\n{hall}")
            }
        }
        None => hall,
    };
    let mut hot_handoff_content = None;
    if let Some(feature) = feature {
        let claimed = crate::store::memory::handoff::claim_pending_handoffs(layout, &feature.name)?;
        if !claimed.is_empty() {
            let mut parts = Vec::new();
            for handoff in claimed {
                let mut section = format!("### Handoff from Session `{}`\n\n{}\n", handoff.source_session, handoff.summary.trim());
                if !handoff.open_tasks.is_empty() {
                    section.push_str("\n#### Open Tasks\n");
                    for task in &handoff.open_tasks {
                        section.push_str(&format!("- {task}\n"));
                    }
                }
                if !handoff.decisions.is_empty() {
                    section.push_str("\n#### Decisions\n");
                    for decision in &handoff.decisions {
                        section.push_str(&format!("- {decision}\n"));
                    }
                }
                if !handoff.modified_paths.is_empty() {
                    section.push_str("\n#### Modified Paths\n");
                    for path in &handoff.modified_paths {
                        section.push_str(&format!("- `{path}`\n"));
                    }
                }
                parts.push(section.trim_end().to_string());
            }
            hot_handoff_content = Some(parts.join("\n\n"));
        }
    }

    let memory_ctx = crate::domain::memory::render_memory_context(
        layout,
        manifest,
        feature.map(|f| &f.name),
        hot_handoff_content.as_deref(),
    )?;
    let memory_block = memory_ctx.render_block();
    let content = config::session::compose_instructions_with_memory(&base_content, &memory_block);
    if content.is_empty() {
        // Discovery with no canonical content: no shared instructions. A
        // stale file from an earlier materialisation is cleared.
        fs::remove_file(&target)?;
        return Ok(());
    }

    let needs_write = match fs::read_text(&target)? {
        Some(existing) => existing != content,
        None => true,
    };
    if needs_write {
        fs::write_text(&target, &content)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../../tests/unit/action/session/handoff_claim.rs"]
mod tests;
