//! The shipped workflow skills: provider-neutral skills materialised into each
//! harness's skill target directory.

use camino::{Utf8Path, Utf8PathBuf};

use crate::error::Failure;
use crate::infra::{fs, hash};

mod catalog;

pub use catalog::{ShippedSkill, catalog};

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
    /// The path to the skill directory or SKILL.md.
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
    fs::ensure_dir(skills_dir).map_err(|source| Error::Fs {
        path: skills_dir.to_owned(),
        source,
    })?;

    let mut changes = Vec::new();

    for skill in catalog() {
        let dir_name = skill.skill_dir_name();
        let target_dir = skills_dir.join(&dir_name);
        let skill_file = target_dir.join("SKILL.md");

        fs::ensure_dir(&target_dir).map_err(|source| Error::Fs {
            path: target_dir.clone(),
            source,
        })?;

        let existing = fs::read_bytes(&skill_file).map_err(|source| Error::Fs {
            path: skill_file.clone(),
            source,
        })?;

        let change = match existing {
            Some(bytes) if bytes == skill.content.as_bytes() => Change::Unchanged,
            Some(_) => {
                write_skill(&skill_file, skill.content)?;
                Change::Updated
            }
            None => {
                write_skill(&skill_file, skill.content)?;
                Change::Created
            }
        };

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
                fs::remove_path(&entry).map_err(|source| Error::Fs {
                    path: entry.clone(),
                    source,
                })?;
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
    if !fs::is_dir(skills_dir).map_err(|source| Error::Fs {
        path: skills_dir.to_owned(),
        source,
    })? {
        return Ok(Vec::new());
    }

    let mut changes = Vec::new();
    for entry in directory_entries(skills_dir)? {
        let dir_name = entry.file_name().unwrap_or("dir").to_owned();
        let Some(id) = ivar_id(&dir_name) else {
            continue;
        };
        fs::remove_path(&entry).map_err(|source| Error::Fs {
            path: entry.clone(),
            source,
        })?;
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

    let present_entries = if fs::is_dir(skills_dir).map_err(|source| Error::Fs {
        path: skills_dir.to_owned(),
        source,
    })? {
        directory_entries(skills_dir)?
    } else {
        Vec::new()
    };

    let mut present = Vec::new();
    for entry in present_entries {
        let name = entry.file_name().unwrap_or("").to_owned();
        let skill_file = entry.join("SKILL.md");
        let bytes = fs::read_bytes(&skill_file).map_err(|source| Error::Fs {
            path: skill_file.clone(),
            source,
        })?;
        present.push((name, entry, bytes));
    }

    for skill in catalog() {
        let dir_name = skill.skill_dir_name();
        let target_dir = skills_dir.join(&dir_name);
        let skill_file = target_dir.join("SKILL.md");

        let integrity = match present.iter().find(|(name, _, _)| name == &dir_name) {
            Some((_, _, _)) if !enabled => Some((skill_file, Integrity::Stale)),
            Some((_, _, Some(bytes))) if bytes == skill.content.as_bytes() => {
                Some((skill_file, Integrity::Current))
            }
            Some((_, _, _)) => Some((skill_file, Integrity::Modified)),
            None if enabled => Some((skill_file, Integrity::Missing)),
            None => None,
        };

        if let Some((path, integrity)) = integrity {
            inspections.push(Inspection {
                id: skill.id.to_owned(),
                path,
                integrity,
            });
        }
    }

    for (name, path, _) in &present {
        if let Some(id) = ivar_id(name)
            && catalog().iter().all(|s| s.id != id)
        {
            inspections.push(Inspection {
                id: id.to_owned(),
                path: path.join("SKILL.md"),
                integrity: Integrity::Obsolete,
            });
        }
    }

    Ok(inspections)
}

fn write_skill(path: &Utf8Path, content: &str) -> Result<(), Error> {
    fs::write_atomic(path, content.as_bytes()).map_err(|source| Error::Fs {
        path: path.to_owned(),
        source,
    })
}

fn directory_entries(dir: &Utf8Path) -> Result<Vec<Utf8PathBuf>, Error> {
    fs::read_dir(dir).map_err(|source| Error::Fs {
        path: dir.to_owned(),
        source,
    })
}

fn ivar_id(name: &str) -> Option<&str> {
    name.strip_prefix("ivar-")
}

#[cfg(test)]
#[path = "../../../tests/unit/harness/skills.rs"]
mod tests;
