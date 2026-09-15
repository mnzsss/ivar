//! The shipped workflow skills: provider-neutral skills materialised into each
//! harness's skill target directory.

use camino::{Utf8Path, Utf8PathBuf};

use crate::error::Failure;
use crate::infra::{fs, hash};

mod catalog;

pub use catalog::{ShippedSkill, SkillFile, catalog};

/// What happened to one skill during reconciliation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// The skill did not exist and now does.
    Created,
    /// The skill existed and its content changed.
    Updated,
    /// The skill was taken away.
    Removed,
    /// The skill was already in its target state.
    Unchanged,
}

/// The result of reconciling one skill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillChange {
    /// The skill's id.
    pub id: String,
    /// The directory name (e.g. `ivar-execute`).
    pub dir_name: String,
    /// What happened to it.
    pub change: Change,
}

/// The state of one skill directory as inspection found it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Integrity {
    /// Matches the catalog byte-for-byte.
    Current,
    /// The skill file was modified from the catalog content.
    Modified,
    /// The skill is missing from the directory.
    Missing,
    /// Present in a disabled provider's directory.
    Stale,
    /// An unknown skill in the `ivar-*` reserved namespace.
    Obsolete,
}

/// The report for one skill in the target directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inspection {
    /// The skill's id.
    pub id: String,
    /// The path to the skill directory.
    pub path: Utf8PathBuf,
    /// How the skill's integrity compares with its target state.
    pub integrity: Integrity,
}

/// Everything that can go wrong reconciling a skills directory.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not reconcile workflow skills at `{path}`: {source}")]
    Fs {
        path: Utf8PathBuf,
        #[source]
        source: fs::Error,
    },
    #[error("could not fingerprint legacy skill at `{path}`: {source}")]
    Hash {
        path: Utf8PathBuf,
        #[source]
        source: hash::Error,
    },
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        match error {
            Error::Fs { source, .. } => source.into(),
            Error::Hash { source, .. } => source.into(),
        }
    }
}

/// Bring `skills_dir` in line with the shipped catalog.
pub fn materialise(skills_dir: &Utf8Path) -> Result<Vec<SkillChange>, Error> {
    fs::ensure_dir(skills_dir).map_err(fs_error(skills_dir))?;

    let mut changes = Vec::new();

    for skill in catalog() {
        let dir_name = skill.skill_dir_name();
        let change = materialise_skill(&skills_dir.join(&dir_name), *skill)?;

        changes.push(SkillChange {
            id: skill.id.to_owned(),
            dir_name,
            change,
        });
    }

    // Clean up unrecognised `ivar-*` directories in the reserved namespace
    if let Ok(entries) = directory_entries(skills_dir) {
        for entry in entries {
            let name = entry.file_name().unwrap_or("dir").to_owned();
            if let Some(id) = ivar_id(&name) {
                if catalog().iter().any(|s| s.id == id) {
                    continue;
                }
                fs::remove_path(&entry).map_err(fs_error(&entry))?;
                changes.push(SkillChange {
                    id: id.to_owned(),
                    dir_name: name,
                    change: Change::Removed,
                });
            }
        }
    }

    Ok(changes)
}

/// Remove all shipped skills from `skills_dir`.
pub fn remove(skills_dir: &Utf8Path) -> Result<Vec<SkillChange>, Error> {
    if !fs::is_dir(skills_dir).map_err(fs_error(skills_dir))? {
        return Ok(Vec::new());
    }

    let mut changes = Vec::new();
    for entry in directory_entries(skills_dir)? {
        let dir_name = entry.file_name().unwrap_or("dir").to_owned();
        let Some(id) = ivar_id(&dir_name) else {
            continue;
        };
        fs::remove_path(&entry).map_err(fs_error(&entry))?;
        changes.push(SkillChange {
            id: id.to_owned(),
            dir_name,
            change: Change::Removed,
        });
    }

    Ok(changes)
}

/// Inspect the state of shipped skills in `skills_dir`.
pub fn inspect(skills_dir: &Utf8Path, enabled: bool) -> Result<Vec<Inspection>, Error> {
    let mut inspections = Vec::new();

    let present_entries = if fs::is_dir(skills_dir).map_err(fs_error(skills_dir))? {
        directory_entries(skills_dir)?
    } else {
        Vec::new()
    };

    for skill in catalog() {
        let target_dir = skills_dir.join(skill.skill_dir_name());
        let present = fs::is_dir(&target_dir).map_err(fs_error(&target_dir))?;
        let real_dir = fs::is_real_dir(&target_dir).map_err(fs_error(&target_dir))?;
        let integrity = match (present, enabled) {
            (true, false) => Integrity::Stale,
            (false, true) => Integrity::Missing,
            (false, false) => continue,
            (true, true) if real_dir && skill_is_intact(&target_dir, *skill)? => Integrity::Current,
            (true, true) => Integrity::Modified,
        };
        inspections.push(Inspection {
            id: skill.id.to_owned(),
            path: target_dir,
            integrity,
        });
    }

    for path in &present_entries {
        let name = path.file_name().unwrap_or("");
        if let Some(id) = ivar_id(name)
            && catalog().iter().all(|s| s.id != id)
        {
            inspections.push(Inspection {
                id: id.to_owned(),
                path: path.clone(),
                integrity: Integrity::Obsolete,
            });
        }
    }

    Ok(inspections)
}

fn skill_is_intact(target_dir: &Utf8Path, skill: ShippedSkill) -> Result<bool, Error> {
    for file in skill.files {
        let path = target_dir.join(file.path);
        let bytes = fs::read_bytes(&path).map_err(fs_error(&path))?;
        if bytes.as_deref() != Some(file.content.as_bytes()) {
            return Ok(false);
        }
    }
    Ok(undeclared_files(target_dir, target_dir, skill)?.is_empty())
}

fn materialise_skill(target_dir: &Utf8Path, skill: ShippedSkill) -> Result<Change, Error> {
    let created = ensure_real_dir(target_dir)?;
    let mut written = false;
    for file in skill.files {
        let path = target_dir.join(file.path);
        if let Some(relative_parent) = Utf8Path::new(file.path).parent() {
            let mut parent = target_dir.to_owned();
            for component in relative_parent.components() {
                parent.push(component);
                ensure_real_dir(&parent)?;
            }
        }
        let existing = fs::read_bytes(&path).map_err(fs_error(&path))?;
        if existing.as_deref() != Some(file.content.as_bytes()) {
            write_skill(&path, file.content)?;
            written = true;
        }
    }
    let undeclared = undeclared_files(target_dir, target_dir, skill)?;
    for path in &undeclared {
        fs::remove_file(path).map_err(fs_error(path))?;
    }
    let pruned_dirs = remove_empty_subdirs(target_dir)?;
    Ok(if created {
        Change::Created
    } else if written || pruned_dirs || !undeclared.is_empty() {
        Change::Updated
    } else {
        Change::Unchanged
    })
}

fn undeclared_files(
    root: &Utf8Path,
    dir: &Utf8Path,
    skill: ShippedSkill,
) -> Result<Vec<Utf8PathBuf>, Error> {
    let mut found = Vec::new();
    for entry in directory_entries(dir)? {
        // Never follow a symlink: it is an undeclared entry, unlinked as itself.
        let is_dir = fs::is_real_dir(&entry).map_err(fs_error(&entry))?;
        if is_dir {
            found.extend(undeclared_files(root, &entry, skill)?);
        } else if let Ok(relative) = entry.strip_prefix(root)
            && !skill
                .files
                .iter()
                .any(|file| Utf8Path::new(file.path) == relative)
        {
            found.push(entry);
        }
    }
    Ok(found)
}

/// Make `dir` a real directory, returning whether it had to be created.
fn ensure_real_dir(dir: &Utf8Path) -> Result<bool, Error> {
    if fs::is_real_dir(dir).map_err(fs_error(dir))? {
        return Ok(false);
    }
    // A symlink or file here would redirect writes outside the skill folder,
    // so it is unlinked as itself and replaced by a real directory.
    fs::remove_path(dir).map_err(fs_error(dir))?;
    fs::ensure_dir(dir).map_err(fs_error(dir))?;
    Ok(true)
}

fn remove_empty_subdirs(dir: &Utf8Path) -> Result<bool, Error> {
    let mut removed = false;
    for entry in directory_entries(dir)? {
        let is_dir = fs::is_real_dir(&entry).map_err(fs_error(&entry))?;
        if is_dir {
            removed |= remove_empty_subdirs(&entry)?;
            if directory_entries(&entry)?.is_empty() {
                fs::remove_path(&entry).map_err(fs_error(&entry))?;
                removed = true;
            }
        }
    }
    Ok(removed)
}

fn write_skill(path: &Utf8Path, content: &str) -> Result<(), Error> {
    fs::write_atomic(path, content.as_bytes()).map_err(fs_error(path))
}

fn directory_entries(dir: &Utf8Path) -> Result<Vec<Utf8PathBuf>, Error> {
    fs::read_dir(dir).map_err(fs_error(dir))
}

fn fs_error(path: &Utf8Path) -> impl FnOnce(fs::Error) -> Error + '_ {
    |source| Error::Fs {
        path: path.to_owned(),
        source,
    }
}

fn ivar_id(name: &str) -> Option<&str> {
    name.strip_prefix("ivar-")
}

#[cfg(test)]
#[path = "../../../tests/unit/harness/skills.rs"]
mod tests;
