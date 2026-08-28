//! The one place skills are read off disk, for every `ivar skill` verb.
//!
//! Four near-identical copies of this scan used to live in `sync`, `status`,
//! `doctor` and `list`. They are one function now, because the hall has two
//! skill roots and a per-caller copy would mean changing the same loop four
//! times for every future change to it.
//!
//! # Why the collision policy lives here, not in the planner
//!
//! `domain::skill_sync` is a pure reducer over a set of skills. A duplicate id
//! is a property of *enumeration* — of reading two directories that each claim
//! the same name — not of reconciling declared state with materialised state.
//! Detecting it here keeps the reducer unaware that roots exist at all.
//!
//! # Why a collision drops both copies
//!
//! A harness skills directory has exactly one slot per id, so the two roots
//! cannot both be honoured. Picking a winner would mean one root silently
//! shadows the other, and the user finds out much later that the agent has
//! been following the wrong skill. Refusing the pair is the loud option, and
//! the warning names both paths so the fix — rename one — is obvious.

use camino::Utf8Path;

use crate::domain::name::RepoName;
use crate::domain::skill::{Skill, SkillRoot};
use crate::error::{Failure, Warning};
use crate::infra::fs;
use crate::store::skill;

/// Read every skill in one root directory, sorted by id.
///
/// An absent directory yields an empty vec rather than an error: a hall
/// without a personal root is the normal case, not a broken one. An entry
/// whose name is not a valid [`RepoName`], or whose `SKILL.md` does not
/// parse, is skipped — one malformed folder does not blind the caller to the
/// rest.
pub fn enumerate(dir: &Utf8Path, root: SkillRoot) -> Result<Vec<Skill>, Failure> {
    let mut skills = Vec::new();
    if !fs::exists(dir)? {
        return Ok(skills);
    }
    for entry in fs::read_dir(dir)? {
        let file_name = entry.file_name().ok_or_else(|| {
            Failure::failed(
                "skill.bad_entry",
                format!("directory entry has no name: {entry:?}"),
            )
        })?;
        if let Ok(_id) = RepoName::new(file_name)
            && let Ok(Some(skill)) = skill::parse_skill(entry.clone(), root)
        {
            skills.push(skill);
        }
    }
    skills.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(skills)
}

/// Read both roots and return the skills that can be acted on, plus a warning
/// for every id the two roots disagree about.
///
/// An id present in both roots is dropped **from both** — see the module doc.
/// Every other skill is returned, so one collision never blocks the rest
/// (the best-effort contract `sync` states).
pub fn enumerate_both(
    hall_dir: &Utf8Path,
    local_dir: &Utf8Path,
) -> Result<(Vec<Skill>, Vec<Warning>), Failure> {
    let hall = enumerate(hall_dir, SkillRoot::Hall)?;
    let local = enumerate(local_dir, SkillRoot::Local)?;

    let mut warnings = Vec::new();
    let mut skills = Vec::new();

    for skill in hall.iter().chain(local.iter()) {
        let other = match skill.root {
            SkillRoot::Hall => &local,
            SkillRoot::Local => &hall,
        };
        let Some(twin) = other.iter().find(|candidate| candidate.id == skill.id) else {
            skills.push(skill.clone());
            continue;
        };
        // Warn once per pair, not once per copy: the hall side reports it.
        if skill.root == SkillRoot::Hall {
            let (hall_path, local_path) = (&skill.dir, &twin.dir);
            warnings.push(Warning::new(
                "skill.collision",
                skill.id.as_str().to_owned(),
                format!(
                    "declared in both roots — {hall_path} and {local_path}. \
                     Neither was materialised; rename one to resolve."
                ),
            ));
        }
    }

    skills.sort_by(|a, b| a.id.cmp(&b.id));
    Ok((skills, warnings))
}

/// Locate a skill by id across both roots, returning its directory and the
/// root that owns it.
///
/// No flag decides this: [`enumerate_both`] forbids one id from living in two
/// roots, so an id names at most one directory. The personal root is searched
/// first, matching the write default — a `create` with no flag lands there, so
/// a `remove` with no flag should find it there.
///
/// Returns `Ok(None)` when the id is in neither root; the caller owns the
/// error, because it knows which verb the user ran.
pub fn resolve(
    layout: &crate::store::layout::Layout,
    id: &str,
) -> Result<Option<(camino::Utf8PathBuf, SkillRoot)>, Failure> {
    for (dir, root) in [
        (layout.hall_skills_local(), SkillRoot::Local),
        (layout.hall_skills(), SkillRoot::Hall),
    ] {
        let candidate = dir.join(id);
        if fs::exists(&candidate)? {
            return Ok(Some((candidate, root)));
        }
    }
    Ok(None)
}

#[cfg(test)]
#[path = "../../../tests/unit/action/skill/enumerate.rs"]
mod tests;
