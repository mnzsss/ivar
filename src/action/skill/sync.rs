//! `ivar skill sync` — materialise hall skills to native targets.
//!
//! Runs offline-only: reads declared skills from `.ivar/skills/`, builds a
//! pure sync plan (`domain::skill_sync::plan_with_options`), targets the hall's
//! available providers, removes orphans, executes each step via the
//! renderer (`store::render::render` / `store::render::remove`), and writes
//! the updated installation state (`store::skill::write`).
//!
//! Best-effort per target: a step that fails becomes a [`Warning`] and does
//! NOT abort the remaining steps. Idempotent — running twice produces no
//! further changes.

use std::time::{SystemTime, UNIX_EPOCH};

use camino::Utf8Path;
use serde::Serialize;

use crate::action::Ctx;
use crate::domain::skill::SkillRoot;
use crate::domain::skill_sync::{
    Action, InstallationEntry, PlanOptions, ProviderEntry, State, Step, Target, TargetId,
};
use crate::error::{Failure, Outcome, Report, Warning, WriteHuman};
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;
use crate::store::render::{self, Error as RenderError};
use crate::store::skill;

use super::super::{discover_hall, read_manifest};
use super::enumerate;

/// What `ivar skill sync` did.
#[derive(Debug, Clone, Serialize)]
pub struct SyncOutcome {
    /// The hall root this ran against.
    pub root: camino::Utf8PathBuf,
    /// Number of steps executed (create / update / remove).
    pub steps: u64,
}

impl WriteHuman for SyncOutcome {
    fn write_human(&self, w: &mut impl std::io::Write) -> std::io::Result<()> {
        writeln!(
            w,
            "Synced {} step{}.",
            self.steps,
            if self.steps == 1 { "" } else { "s" }
        )
    }
}

/// Materialise hall skills to native targets following `providers.available`
/// and remove orphaned targets of dropped providers.
///
/// Offline-only — no remote fetch. Returns a [`Report`] containing the number
/// of steps executed and any per-step warnings.
pub fn sync(ctx: &Ctx) -> Outcome<SyncOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let mut warnings = Vec::new();
    let steps = reconcile(&layout, &manifest, &mut warnings)?;
    let outcome = SyncOutcome {
        root: layout.root().to_path_buf(),
        steps,
    };
    Ok(if warnings.is_empty() {
        Report::new(outcome)
    } else {
        Report::with_warnings(outcome, warnings)
    })
}

pub(crate) fn reconcile(
    layout: &Layout,
    manifest: &Manifest,
    warnings: &mut Vec<Warning>,
) -> Result<u64, Failure> {
    let (skills, enumerate_warnings) =
        enumerate::enumerate_both(&layout.hall_skills(), &layout.hall_skills_local())?;
    warnings.extend(enumerate_warnings);

    let hall_state = read_state(layout, SkillRoot::Hall);
    let local_state = read_state(layout, SkillRoot::Local);
    let state = merge_states(&hall_state, &local_state);

    let available = super::available_targets(manifest);
    let targets = build_sync_targets(&skills, layout, &available);
    let mut steps = crate::domain::skill_sync::plan_with_options(
        &skills,
        &targets,
        &state,
        &available,
        &PlanOptions::default(),
    );
    // Recorded paths may be hall-relative (`.claude/skills/x`); a remove
    // must never resolve against the process cwd.
    for step in &mut steps {
        if step.action == Action::Remove {
            step.target = layout.root().join(&step.target);
        }
    }

    execute_steps(&steps, warnings);
    if !steps.is_empty() {
        persist_sync_state(layout, &state, &skills, &steps, &targets, warnings);
    }
    remove_emptied_skill_dirs(layout, &available, warnings);

    Ok(steps.iter().filter(|step| step.action != Action::Unchanged).count() as u64)
}

// -- helpers ------------------------------------------------------------------

fn build_sync_targets(
    skills: &[crate::domain::skill::Skill],
    layout: &Layout,
    available: &[TargetId],
) -> Vec<Target> {
    let mut targets = Vec::new();
    for skill in skills {
        for &target_id in available {
            // `target_path` is hall-relative (`.claude/skills/<id>`); the
            // renderer and planner operate on absolute paths, so join it onto
            // the hall root here.
            let relative = skill::target_path(target_id, skill.id.as_str());
            let path = layout.root().join(relative);
            let source_hash = compute_source_hash(&skill.dir);
            // The renderer symlinks the whole skill directory (step.source),
            // so verify the link against that same directory.
            let status = render::verify_status(&path, &skill.dir);
            targets.push(Target {
                id: target_id,
                skill: skill.id.clone(),
                path,
                source_path: skill.dir.clone(),
                source_hash,
                status,
            });
        }
    }
    targets
}

// Execute best-effort: failures become warnings, never abort.
fn execute_steps(steps: &[Step], warnings: &mut Vec<Warning>) {
    for step in steps {
        if let Err(e) = execute_step(step) {
            warnings.push(Warning::new(
                "skill.sync.step_failed",
                format!("{}@{}", step.skill, step.target),
                e.to_string(),
            ));
        }
    }
}

fn persist_sync_state(
    layout: &Layout,
    state: &State,
    skills: &[crate::domain::skill::Skill],
    steps: &[Step],
    targets: &[Target],
    warnings: &mut Vec<Warning>,
) {
    let mut new_state = state.clone();
    for step in steps {
        match step.action {
            Action::Create | Action::Update => {
                let hash = targets
                    .iter()
                    .find(|t| t.skill == step.skill && t.id == step.target_id)
                    .map(|t| t.source_hash.clone())
                    .unwrap_or_default();
                update_state_entry(&mut new_state, step, &hash);
            }
            Action::Remove => remove_provider_entry(&mut new_state, step),
            Action::Unchanged => {}
        }
    }
    // Split the merged state back out, one file per root. This is the
    // whole reason the states are separate: `.ivar/skills/` is un-ignored
    // by the hall's `.gitignore`, so anything recorded there is committed.
    // A personal skill's id must never reach it.
    for root in [SkillRoot::Hall, SkillRoot::Local] {
        let split = split_state(&new_state, skills, root);
        let path = skill::state_path(layout.root(), root);
        if split.installations.is_empty() {
            // Nothing left for this root — drop the file rather than leave
            // an empty one, so `read` answers `None` (nothing installed).
            let _ = fs::remove_path(&path);
            continue;
        }
        if let Err(e) = skill::write(layout.root(), root, &split) {
            warnings.push(Warning::new(
                "skill.sync.state_write_failed",
                path.to_string(),
                e.to_string(),
            ));
        }
    }
}

/// Read one root's recorded state, treating an unreadable file as empty.
///
/// A missing state file is the normal case for a hall with no personal
/// skills, and for every hall before its first sync.
fn read_state(layout: &Layout, root: SkillRoot) -> State {
    skill::read(layout.root(), root)
        .unwrap_or_default()
        .unwrap_or_default()
}

/// Merge two roots' states into the single view the planner expects.
///
/// Ids cannot clash: `enumerate_both` refuses an id declared in both roots,
/// so no entry can be shadowed here.
fn merge_states(hall: &State, local: &State) -> State {
    let mut merged = hall.clone();
    merged.installations.extend(
        local
            .installations
            .iter()
            .map(|(k, v)| (k.clone(), v.clone())),
    );
    merged
}

/// Take the entries belonging to one root out of the merged state.
///
/// A skill that is no longer declared in either root has no `SkillRoot` to
/// consult, so its entry is attributed to the root whose state file already
/// records it — that is where the delete has to be written.
fn split_state(merged: &State, skills: &[crate::domain::skill::Skill], root: SkillRoot) -> State {
    let mut out = State::default();
    for (id, entry) in &merged.installations {
        let owner = skills
            .iter()
            .find(|skill| skill.id.as_str() == id)
            .map(|skill| skill.root);
        if owner == Some(root) {
            out.installations.insert(id.clone(), entry.clone());
        }
    }
    out
}

/// Compute a deterministic hash for a source directory.
///
/// Uses the directory's last-modified time as a change indicator. Same
/// directory at the same time → same hash, which makes the planner report
/// `Unchanged` on the second run (idempotency).
fn compute_source_hash(dir: &Utf8Path) -> String {
    let meta = fs::stat(dir).ok().flatten();
    let mtime = meta
        .and_then(|m| m.modified().ok())
        .map(|t| t.duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos())
        .unwrap_or(0);
    format!("mtime:{mtime}")
}

/// Execute a single sync step.
fn execute_step(step: &Step) -> Result<(), RenderError> {
    match step.action {
        Action::Create | Action::Update => render::render(step),
        Action::Remove => render::remove(step),
        Action::Unchanged => Ok(()),
    }
}

/// Update the installation state for a skill after a Create or Update step.
fn update_state_entry(state: &mut State, step: &Step, source_hash: &str) {
    let iso = format!(
        "{}Z",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    );
    let provider = ProviderEntry {
        target_path: skill::target_path(step.target_id, step.skill.as_str()),
        rendered_hash: source_hash.to_owned(),
        linked_at: iso.clone(),
        mode: Some(step.mode),
    };
    let entry = state
        .installations
        .entry(step.skill.as_str().to_owned())
        .or_insert_with(|| InstallationEntry {
            source_path: step.source.clone(),
            source_hash: source_hash.to_owned(),
            installed_at: iso.clone(),
            commit_sha: None,
            providers: std::collections::HashMap::new(),
        });
    entry.source_path = step.source.clone();
    source_hash.clone_into(&mut entry.source_hash);
    entry.installed_at.clone_from(&iso);
    entry.providers.insert(step.target_id, provider);
}

/// Drop one provider's record; the skill's entry goes when none remain.
fn remove_provider_entry(state: &mut State, step: &Step) {
    let id = step.skill.as_str();
    let Some(entry) = state.installations.get_mut(id) else {
        return;
    };
    entry.providers.remove(&step.target_id);
    if entry.providers.is_empty() {
        state.installations.remove(id);
    }
}

/// Remove `<config>/skills` of a provider the hall no longer lists, only
/// when cleanup left it empty. Never its config dir or a sibling.
fn remove_emptied_skill_dirs(layout: &Layout, available: &[TargetId], warnings: &mut Vec<Warning>) {
    for target_id in [TargetId::Claude, TargetId::OpenCode, TargetId::Omp] {
        if available.contains(&target_id) {
            continue;
        }
        let Some(dir) = skill::target_path(target_id, "_").parent().map(|p| layout.root().join(p))
        else {
            continue;
        };
        if fs::is_real_dir(&dir).unwrap_or(false)
            && fs::read_dir(&dir).is_ok_and(|entries| entries.is_empty())
            && let Err(e) = fs::remove_path(&dir)
        {
            warnings.push(Warning::new(
                "skill.sync.dir_remove_failed",
                dir.to_string(),
                e.to_string(),
            ));
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/action/skill/sync.rs"]
mod tests;
