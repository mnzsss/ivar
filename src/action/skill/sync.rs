//! `ivar skill sync` — materialise hall skills to native targets.
//!
//! Runs offline-only: reads declared skills from `.ivar/skills/`, builds a
//! pure sync plan (`domain::skill_sync::plan`), executes each step via the
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
use crate::domain::name::RepoName;
use crate::domain::skill_sync::{
    Action, InstallationEntry, PlanOptions, ProviderEntry, State, Step, Target, TargetId,
};
use crate::error::{Failure, Outcome, Report, Warning, WriteHuman};
use crate::infra::fs;
use crate::store::render::{self, Error as RenderError};
use crate::store::skill;

use super::super::discover_hall;

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

/// Materialise hall skills to native targets (`.claude/skills`, `.opencode/skills`).
///
/// Offline-only — no remote fetch. Returns a [`Report`] containing the number
/// of steps executed and any per-step warnings.
pub fn sync(ctx: &Ctx) -> Outcome<SyncOutcome> {
    let layout = discover_hall(ctx)?;
    let hall_skills = layout.hall_skills();

    // Enumerate declared skills.
    let skills = enumerate_skills(&hall_skills)?;

    // Read current installation state.
    let state = skill::read(layout.root())
        .unwrap_or_default()
        .unwrap_or_default();

    // Build targets for both providers.
    let mut targets = Vec::new();
    for skill in &skills {
        for target_id in [TargetId::Claude, TargetId::OpenCode] {
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

    // Compute the sync plan. The planner is one-target-per-skill, so run it
    // once per provider (Claude, OpenCode) and concatenate — a skill
    // materialises to both native targets.
    let mut steps = Vec::new();
    for target_id in [TargetId::Claude, TargetId::OpenCode] {
        let provider_targets: Vec<Target> = targets
            .iter()
            .filter(|t| t.id == target_id)
            .cloned()
            .collect();
        steps.extend(crate::domain::skill_sync::plan_with_options(
            &skills,
            &provider_targets,
            &state,
            PlanOptions::default(),
        ));
    }
    steps.sort_by(|a, b| {
        (a.skill.as_str(), a.target.as_str()).cmp(&(b.skill.as_str(), b.target.as_str()))
    });

    // Execute best-effort: failures become warnings, never abort.
    let mut warnings = Vec::new();
    for step in &steps {
        if let Err(e) = execute_step(step) {
            warnings.push(Warning::new(
                "skill.sync.step_failed",
                format!("{}@{}", step.skill, step.target),
                e.to_string(),
            ));
        }
    }

    // Update state with all successful changes.
    if !steps.is_empty() {
        let mut new_state = state.clone();
        for step in &steps {
            match step.action {
                Action::Create | Action::Update => {
                    // Find the source hash from the target map.
                    let hash = targets
                        .iter()
                        .find(|t| t.skill == step.skill)
                        .map(|t| t.source_hash.clone())
                        .unwrap_or_default();
                    update_state_entry(&mut new_state, step, &hash);
                }
                Action::Remove => {
                    remove_state_entry(&mut new_state, &step.skill);
                }
                Action::Unchanged => {}
            }
        }
        if let Err(e) = skill::write(layout.root(), &new_state) {
            warnings.push(Warning::new(
                "skill.sync.state_write_failed",
                layout
                    .root()
                    .join(".ivar")
                    .join("skills")
                    .join("state.json")
                    .to_string(),
                e.to_string(),
            ));
        }
    }

    let executed: u64 = steps
        .iter()
        .filter(|step| step.action != Action::Unchanged)
        .count() as u64;

    let report = if warnings.is_empty() {
        Report::new(SyncOutcome {
            root: layout.root().to_path_buf(),
            steps: executed,
        })
    } else {
        Report::with_warnings(
            SyncOutcome {
                root: layout.root().to_path_buf(),
                steps: executed,
            },
            warnings,
        )
    };

    Ok(report)
}

// -- helpers ------------------------------------------------------------------

/// Enumerate skills from the hall skills directory.
fn enumerate_skills(hall_skills: &Utf8Path) -> Result<Vec<crate::domain::skill::Skill>, Failure> {
    let mut skills = Vec::new();
    if !fs::exists(hall_skills)? {
        return Ok(skills);
    }
    for entry in fs::read_dir(hall_skills)? {
        let file_name = entry.file_name().ok_or_else(|| {
            Failure::failed(
                "skill.bad_entry",
                format!("directory entry has no name: {:?}", entry),
            )
        })?;
        if let Ok(_id) = RepoName::new(file_name)
            && let Ok(Some(skill)) = skill::parse_skill(entry.clone())
        {
            skills.push(skill);
        }
    }
    skills.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(skills)
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
    let skill_id = step.skill.as_str().to_owned();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let iso = format!("{}Z", now.as_secs());

    let providers: std::collections::HashMap<TargetId, ProviderEntry> = {
        let mut map = std::collections::HashMap::new();
        map.insert(
            TargetId::Claude,
            ProviderEntry {
                target_path: skill::target_path(TargetId::Claude, step.skill.as_str()),
                rendered_hash: source_hash.to_owned(),
                linked_at: iso.clone(),
                mode: Some(step.mode),
            },
        );
        map.insert(
            TargetId::OpenCode,
            ProviderEntry {
                target_path: skill::target_path(TargetId::OpenCode, step.skill.as_str()),
                rendered_hash: source_hash.to_owned(),
                linked_at: iso.clone(),
                mode: Some(step.mode),
            },
        );
        map
    };

    match state.installations.get_mut(&skill_id) {
        Some(entry) => {
            entry.source_path = step.source.clone();
            entry.source_hash = source_hash.to_owned();
            entry.installed_at = iso.clone();
            entry.providers = providers;
        }
        None => {
            state.installations.insert(
                skill_id,
                InstallationEntry {
                    source_path: step.source.clone(),
                    source_hash: source_hash.to_owned(),
                    installed_at: iso,
                    commit_sha: None,
                    providers,
                },
            );
        }
    }
}

/// Remove a skill's entry from the installation state.
fn remove_state_entry(state: &mut State, skill: &RepoName) {
    state.installations.remove(skill.as_str());
}

#[cfg(test)]
#[path = "../../../tests/unit/action/skill/sync.rs"]
mod tests;
