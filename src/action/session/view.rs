//! View Dir materialisation: the shared core for `session start`, `session
//! connect`, and `session convert`.
//!
//! A **View Dir** is the single directory an agent session works in: one
//! symlink per registered repo (promoted repos point at their feature
//! worktree; the rest at their base view — the root feature's declared base
//! branch worktree, a parent's feature worktree, or the read-only
//! default-branch worktree, see [`base_view`]), a real per-session harness
//! config dir, and — for feature sessions — the session bootstrap
//! instructions materialised.
//!
//! Materialisation never touches git: the base worktrees it links are
//! created and fast-forwarded beforehand by [`base_view::prepare`]
//! (`session start`/`connect`/`convert`), so `feature promote` and
//! `feature rename` re-link without a fetch.
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
//! For claude-code, `settings.json` inside the real config dir is a symlink to
//! the hall's own `.claude/settings.json` (the file only). Claude Code reads
//! project settings only from the session's cwd, so without it the hall's
//! hooks never reach a view. A link rather than a copy keeps the file under the
//! hall's write protection, so an agent cannot remove its own guard hook.
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
//! - a discovery session's file is the `HALL.md` content;
//! - a feature session's file is the session bootstrap block, two newlines,
//!   then the `HALL.md` content;
//! - either is followed by a generated `## Repository instructions` section
//!   listing `<view>/<repo>/<file>` for each linked repo whose root has an
//!   instruction file (provider-native name, else the other), in manifest
//!   order — omitted when none has one.
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

use crate::action::session::{base_view, instructions, repo_skills};
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
/// to its feature worktree (writable). Every other repo is symlinked to the
/// worktree [`base_view::resolve_on_disk`] names: the root feature's base
/// branch worktree or the default-branch worktree, both guarded read-only
/// (write bits cleared on the root; kernel-enforced under the Linux
/// sandbox), or the nearest promoting ancestor's feature worktree, whose
/// write bits are left alone — the session is kept out of it by the write
/// guard and the sandbox, not by chmod. For a **discovery session**
/// (`feature: None`), every repo is a read-only default-branch worktree.
///
/// `provider` is the session's own provider — what the session actually runs
/// (or ran) under — not the hall's default. It decides which config dir and
/// which instruction file the View Dir gets.
///
/// A repo whose worktree is missing is skipped with the rest still linked —
/// the session should still open for the repos that are there.
///
/// Each linked repo's own skills (`.omp/skills`, `.agents/skills`,
/// `.claude/skills`, `.opencode/skills`) are projected into the config dir's
/// `skills/`, linked through the repo symlink so promotion retargets them; a
/// name the hall or the user already uses is prefixed `<repo>--<name>` instead
/// of shadowing it.
pub(crate) fn materialise(
    layout: &Layout,
    manifest: &Manifest,
    feature: Option<&Feature>,
    provider: Provider,
    view_dir: &Utf8Path,
) -> Result<MaterialiseReport, Failure> {
    fs::ensure_dir(view_dir)?;

    // The scratch dir: where an agent's temporary and working files belong.
    // Inside the view dir, so it is already in the session's writable set and
    // `ivar session stop` takes it away with everything else. Created here
    // rather than on demand because the guard's denial message names it, and
    // a path named in a message has to exist.
    fs::ensure_dir(&Layout::session_scratch(view_dir))?;

    for repo in manifest.repos() {
        // `guard`: whether this session clears the worktree's write bits.
        // A promoted repo is the session's own (writable); an unpromoted one
        // is viewed at its base (see `base_view`), guarded read-only unless
        // that base is a parent's feature worktree — the parent writes there,
        // so its bits are never touched (Landlock and the guard hook still
        // keep this session out of it).
        let (worktree, guard) = match feature {
            Some(feature) if feature.is_promoted(repo.name()) => {
                (layout.repo_worktree(repo.name(), &feature.branch), false)
            }
            Some(feature) => {
                let (view, worktree) = base_view::resolve_on_disk(layout, repo, feature)?;
                (worktree, view.guards_read_only())
            }
            None => (
                layout.repo_worktree(repo.name(), repo.default_branch()),
                true,
            ),
        };
        if !fs::is_dir(&worktree)? {
            continue;
        }
        let link = view_dir.join(repo.name().as_str());
        // Replace only when the target changed: the view dir is re-materialised
        // on every connect, and an unchanged link must not be renamed (each
        // rename opens a transient resolution race — see `infra::fs`).
        fs::replace_symlink_if_changed(&worktree, &link)?;
        if guard {
            fs::clear_write_bits(&worktree)?;
        }
    }

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
    materialise_session_settings(layout, provider, view_dir, &mut report);
    materialise_repo_skills(layout, manifest, provider, view_dir, &mut report);
    materialise_session_instructions(layout, manifest, provider, feature, view_dir, &mut report)?;

    Ok(report)
}

/// Link the view's `.claude/settings.json` (the file only) to the hall's.
/// Claude Code reads project settings only from `<cwd>/.claude/settings.json`,
/// so without this the guard and its instruction slices never run in a
/// session. A link, not a copy: the hall file is protected from agent writes
/// (`Layout::guard_protected_paths`, the sandbox), so an agent cannot remove
/// its own guard hook. It also carries the user's hall-level settings.
/// Never fails the session: a missing hall file or a failed link is a warning.
fn materialise_session_settings(
    layout: &Layout,
    provider: Provider,
    view_dir: &Utf8Path,
    report: &mut MaterialiseReport,
) {
    if provider != Provider::ClaudeCode {
        return;
    }
    let hall_settings = layout.root().join(Provider::CLAUDE_SETTINGS);
    let link = view_dir.join(Provider::CLAUDE_SETTINGS);
    match fs::is_file(&hall_settings) {
        Ok(true) => {
            if let Err(error) = fs::replace_symlink_if_changed(&hall_settings, &link) {
                report.warnings.push(Warning::new(
                    "settings.view_unlinked",
                    link.as_str(),
                    format!("hall settings not linked; the guard will not run in this session: {error}"),
                ));
            }
        }
        Ok(false) => report.warnings.push(Warning::new(
            "settings.hall_missing",
            hall_settings.as_str(),
            "the hall has no `.claude/settings.json` (run `ivar sync`); the guard will not run in this session",
        )),
        Err(error) => report.warnings.push(Warning::new(
            "settings.hall_missing",
            hall_settings.as_str(),
            format!("the hall's `.claude/settings.json` is unreadable; the guard will not run in this session: {error}"),
        )),
    }
}

/// Project the skills each linked repo ships into the session's skills dir
/// (see `repo_skills`). Never fails the session: any error becomes a warning.
fn materialise_repo_skills(
    layout: &Layout,
    manifest: &Manifest,
    provider: Provider,
    view_dir: &Utf8Path,
    report: &mut MaterialiseReport,
) {
    let home = match crate::providers::user_home() {
        Ok(home) => Some(home),
        Err(failure) => {
            report.warnings.push(Warning::new(
                "skill.repo_home_unavailable",
                view_dir.as_str(),
                format!("user skill dirs not checked for name collisions: {failure}"),
            ));
            None
        }
    };
    match repo_skills::materialise(layout, manifest, provider, view_dir, home.as_deref()) {
        Ok(warnings) => report.warnings.extend(warnings),
        Err(failure) => report.warnings.push(Warning::new(
            "skill.repo_unreadable",
            view_dir.as_str(),
            format!("repo skills not projected: {failure}"),
        )),
    }
}

/// Write the provider-native instruction file (`CLAUDE.md` / `AGENTS.md`) at
/// the View Dir root, derived from the canonical `HALL.md`.
///
/// A discovery session's file is exactly the canonical content; a feature
/// session's file is the session bootstrap block followed by it. Both end
/// with the repository pointer section from [`repo_instructions_section`]
/// when a linked repo has an instruction file. The
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

    let content = match feature {
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

    let content = match repo_instructions_section(manifest, provider, view_dir) {
        Some(section) if content.is_empty() => section,
        Some(section) => format!("{}\n\n{section}", content.trim_end_matches('\n')),
        None => content,
    };

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

/// The generated `## Repository instructions` section: one absolute pointer,
/// through the view symlink, per linked repo whose root holds an instruction
/// file — the provider-native name, else the other — in manifest order.
/// `None` when no linked repo has one. The pointer goes through the symlink,
/// so it names the default branch's file before a promotion and the feature
/// worktree's after, and is regenerated with every materialisation.
fn repo_instructions_section(
    manifest: &Manifest,
    provider: Provider,
    view_dir: &Utf8Path,
) -> Option<String> {
    let pointers: Vec<String> = manifest
        .repos()
        .iter()
        .filter_map(|repo| {
            let root = view_dir.join(repo.name().as_str());
            // A path directly in the repo root: its chain is that root's file alone.
            let file = instructions::instruction_chain(
                view_dir,
                &root.join(provider.instruction_file()),
                provider,
            )
            .into_iter()
            .next()?;
            Some(format!("- `{}`: {file}\n", repo.name().as_str()))
        })
        .collect();
    if pointers.is_empty() {
        return None;
    }
    Some(format!(
        "## Repository instructions\n\nEach linked repository has its own instructions. They \
         are delivered to you as you work in the repository; read them in full before working \
         in it:\n\n{}",
        pointers.concat()
    ))
}
