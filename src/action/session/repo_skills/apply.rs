//! Make a session skills dir hold exactly a [`Plan`]: links for bare names,
//! renamed copies for prefixed ones, and a record of what ivar owns so only
//! those entries are ever removed.

use std::collections::BTreeSet;

use camino::{Utf8Path, Utf8PathBuf};

use crate::error::{Failure, Warning};
use crate::infra::{fs, hash, json};

use super::naming::{Placement, Plan};
use super::scan::RepoSkill;
use super::skill_md::rename_frontmatter;

pub(crate) const RECORD_FILE: &str = ".ivar-repo-skills.json";

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Owned {
    Link {
        name: String,
        target: Utf8PathBuf,
    },
    Copy {
        name: String,
        source: String,
        hash: String,
    },
}

#[derive(Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Record {
    pub entries: Vec<Owned>,
}

fn link_target(skill: &RepoSkill) -> Utf8PathBuf {
    Utf8PathBuf::from("../..")
        .join(&skill.repo)
        .join(&skill.source_rel)
}

fn owned_name(o: &Owned) -> &str {
    match o {
        Owned::Link { name, .. } | Owned::Copy { name, .. } => name,
    }
}

/// Whether `dest` still is exactly what `owned` says ivar put there.
fn still_ours(dest: &Utf8Path, owned: &Owned) -> Result<bool, Failure> {
    Ok(match owned {
        Owned::Link { target, .. } => {
            matches!(fs::read_symlink(dest)?, fs::SymlinkTarget::Target(t) if &t == target)
        }
        Owned::Copy { hash: recorded, .. } => {
            matches!(fs::read_symlink(dest)?, fs::SymlinkTarget::NotASymlink)
                && hash::tree(dest)? == *recorded
        }
    })
}

pub(crate) fn apply(
    view_dir: &Utf8Path,
    skills_dir: &Utf8Path,
    plan: &Plan,
) -> Result<Vec<Warning>, Failure> {
    let record_path = skills_dir.join(RECORD_FILE);
    let previous: Record = json::read(&record_path)?.unwrap_or_default();
    let mut warnings = Vec::new();
    let mut next = Record::default();
    let wanted: BTreeSet<&str> = plan.entries.iter().map(|p| p.dest_name.as_str()).collect();

    // 1. Drop what we own and no longer want; leave anything changed by hand.
    for owned in &previous.entries {
        let dest = skills_dir.join(owned_name(owned));
        if !wanted.contains(owned_name(owned)) && still_ours(&dest, owned)? {
            fs::remove_path(&dest)?;
        }
    }

    if !plan.entries.is_empty() {
        fs::ensure_dir(skills_dir)?;
    }
    for entry in &plan.entries {
        let dest = skills_dir.join(&entry.dest_name);
        let prior = previous
            .entries
            .iter()
            .find(|o| owned_name(o) == entry.dest_name);
        let occupied = !matches!(fs::read_symlink(&dest)?, fs::SymlinkTarget::Absent);
        let ours = match prior {
            Some(o) => still_ours(&dest, o)?,
            None => false,
        };
        if occupied && !ours {
            warnings.push(Warning::new(
                "skill.repo_skipped",
                format!("{}/{}", entry.skill.repo, entry.skill.source_rel),
                format!("skipped: {dest} exists and was not created by ivar (or was edited)"),
            ));
            // Dropped from the record: an edited copy is the user's from now on.
            continue;
        }
        let owned = match entry.placement {
            Placement::Bare => {
                let target = link_target(&entry.skill);
                if occupied && matches!(prior, Some(Owned::Copy { .. })) {
                    fs::remove_path(&dest)?;
                }
                fs::replace_symlink_if_changed(&target, &dest)?;
                Owned::Link {
                    name: entry.dest_name.clone(),
                    target,
                }
            }
            Placement::Prefixed => {
                let source = view_dir
                    .join(&entry.skill.repo)
                    .join(&entry.skill.source_rel);
                if occupied {
                    fs::remove_path(&dest)?;
                }
                fs::copy_dir(&source, &dest)?;
                let skill_md = dest.join("SKILL.md");
                let text = fs::read_text(&skill_md)?.unwrap_or_default();
                // write_atomic, not write_text: copied files keep the source's
                // cleared write bits (default-branch worktrees are read-only).
                fs::write_atomic(
                    &skill_md,
                    rename_frontmatter(&text, &entry.dest_name).as_bytes(),
                )?;
                Owned::Copy {
                    name: entry.dest_name.clone(),
                    source: format!("{}/{}", entry.skill.repo, entry.skill.source_rel),
                    hash: hash::tree(&dest)?,
                }
            }
        };
        next.entries.push(owned);
    }

    if next.entries.is_empty() && previous.entries.is_empty() {
        return Ok(warnings);
    }
    if next != previous {
        fs::ensure_dir(skills_dir)?;
        json::write_canonical(&record_path, &next)?;
    }
    Ok(warnings)
}
