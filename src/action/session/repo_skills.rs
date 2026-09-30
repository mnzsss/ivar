//! Repo skills in session view dirs: find the skills a hall repo ships in
//! its provider skill dirs and decide the name each one gets in a session,
//! so a hall or user skill is never shadowed (see feature
//! `repo-skills-in-sessions`).

use std::collections::{BTreeMap, BTreeSet};

use camino::{Utf8Path, Utf8PathBuf};

use crate::error::{Failure, Warning};
use crate::infra::{frontmatter, fs, hash, json};

use crate::domain::provider::Provider;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;
pub(crate) const SOURCE_DIRS: [&str; 4] = [
    ".omp/skills",
    ".agents/skills",
    ".claude/skills",
    ".opencode/skills",
];
pub(crate) const PREFIX_SEPARATOR: &str = "--";
const USER_DIRS: [&str; 4] = [
    ".agents/skills",
    ".claude/skills",
    ".omp/agent/skills",
    ".config/opencode/skills",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepoSkill {
    pub repo: String,
    pub name: String,
    pub source_rel: Utf8PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Placement {
    Bare,
    Prefixed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Planned {
    pub skill: RepoSkill,
    pub dest_name: String,
    pub placement: Placement,
}

#[derive(Debug, Default)]
pub(crate) struct Plan {
    pub entries: Vec<Planned>,
    pub warnings: Vec<Warning>,
}

#[derive(Debug, Default)]
pub(crate) struct Scan {
    pub skills: Vec<RepoSkill>,
    pub warnings: Vec<Warning>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
struct NameOnly {
    name: Option<String>,
}

/// Extract the first top-level `name:` value from frontmatter lines, stripping
/// surrounding matching quotes (`'` or `"`).
fn extract_name_line(lines: impl Iterator<Item = impl AsRef<str>>) -> Option<String> {
    for line in lines {
        let line = line.as_ref();
        if let Some(rest) = line.strip_prefix("name:") {
            let val = rest.trim();
            let stripped = if (val.starts_with('"') && val.ends_with('"') && val.len() >= 2)
                || (val.starts_with('\'') && val.ends_with('\'') && val.len() >= 2)
            {
                val[1..val.len() - 1].trim()
            } else {
                val
            };
            if !stripped.is_empty() {
                return Some(stripped.to_owned());
            }
        }
    }
    None
}

/// The identity a harness gives `dir/SKILL.md`: its frontmatter `name`, or
/// the directory name when the frontmatter has none or cannot be parsed.
/// `Err` is returned only when the file cannot be read from disk (I/O error).
fn skill_name(dir: &Utf8Path) -> Result<Option<String>, String> {
    let path = dir.join("SKILL.md");
    let Some(text) = fs::read_text(&path).map_err(|e| e.to_string())? else {
        return Ok(None);
    };

    let fallback_dir = || dir.file_name().unwrap_or_default().to_owned();

    match frontmatter::split(&text) {
        Ok(split) => {
            if let Some(block) = split.frontmatter {
                if let Ok(meta) = serde_saphyr::from_str::<NameOnly>(block)
                    && let Some(n) = meta.name.filter(|n| !n.trim().is_empty())
                {
                    return Ok(Some(n));
                }
                let name = extract_name_line(block.lines()).unwrap_or_else(fallback_dir);
                Ok(Some(name))
            } else {
                Ok(Some(fallback_dir()))
            }
        }
        Err(_) => {
            // Unterminated fence: scan lines after the opening fence up to the
            // first blank or `#` line / end of file.
            let mut lines = text.lines();
            if let Some(first) = lines.next()
                && first.trim() == "---"
            {
                let block_lines = lines.take_while(|l| {
                    let t = l.trim();
                    !t.is_empty() && !t.starts_with('#')
                });
                let name = extract_name_line(block_lines).unwrap_or_else(fallback_dir);
                return Ok(Some(name));
            }
            Ok(Some(fallback_dir()))
        }
    }
}

pub(crate) fn scan_repo(repo: &str, repo_root: &Utf8Path) -> Scan {
    let mut scan = Scan::default();
    let mut seen: BTreeMap<String, Utf8PathBuf> = BTreeMap::new();
    for source_dir in SOURCE_DIRS {
        let dir = repo_root.join(source_dir);
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            // Loose files (README.md, state.json) are not skills.
            if !fs::is_dir(&entry).unwrap_or(false) {
                continue;
            }
            let rel = Utf8PathBuf::from(source_dir).join(entry.file_name().unwrap_or_default());
            match skill_name(&entry) {
                Ok(None) => {}
                Err(error) => scan.warnings.push(Warning::new(
                    "skill.repo_unreadable",
                    format!("{repo}/{rel}"),
                    format!("skipped: unreadable SKILL.md frontmatter ({error})"),
                )),
                Ok(Some(name)) => match seen.get(&name) {
                    Some(kept) => scan.warnings.push(Warning::new(
                        "skill.repo_duplicate",
                        format!("{repo}/{rel}"),
                        format!("skipped: `{name}` is already provided by {repo}/{kept}"),
                    )),
                    None => {
                        seen.insert(name.clone(), rel.clone());
                        scan.skills.push(RepoSkill {
                            repo: repo.to_owned(),
                            name,
                            source_rel: rel,
                        });
                    }
                },
            }
        }
    }
    scan.skills.sort_by(|a, b| a.name.cmp(&b.name));
    scan
}

pub(crate) fn reserved_dirs(hall_root: &Utf8Path, home: Option<&Utf8Path>) -> Vec<Utf8PathBuf> {
    let mut dirs: Vec<Utf8PathBuf> = SOURCE_DIRS.iter().map(|d| hall_root.join(d)).collect();
    if let Some(home) = home {
        dirs.extend(USER_DIRS.iter().map(|d| home.join(d)));
    }
    dirs
}

pub(crate) fn reserved_names(dirs: &[Utf8PathBuf]) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries {
            if let Some(dir_name) = entry.file_name() {
                names.insert(dir_name.to_owned());
            }
            if let Ok(Some(name)) = skill_name(&entry) {
                names.insert(name);
            }
        }
    }
    names
}

pub(crate) fn plan(skills: Vec<RepoSkill>, reserved: &BTreeSet<String>) -> Plan {
    let mut per_name: BTreeMap<&str, usize> = BTreeMap::new();
    for s in &skills {
        *per_name.entry(s.name.as_str()).or_default() += 1;
    }
    let shared: BTreeSet<String> = per_name
        .into_iter()
        .filter(|(_, n)| *n > 1)
        .map(|(k, _)| k.to_owned())
        .collect();

    let mut out = Plan::default();
    let mut taken = reserved.clone();
    let mut ordered = skills;
    ordered.sort_by(|a, b| {
        (a.name.as_str(), a.repo.as_str()).cmp(&(b.name.as_str(), b.repo.as_str()))
    });
    for skill in ordered {
        let subject = format!("{}/{}", skill.repo, skill.source_rel);
        if !reserved.contains(&skill.name) && !shared.contains(&skill.name) {
            taken.insert(skill.name.clone());
            out.entries.push(Planned {
                dest_name: skill.name.clone(),
                placement: Placement::Bare,
                skill,
            });
            continue;
        }
        let prefixed = format!("{}{PREFIX_SEPARATOR}{}", skill.repo, skill.name);
        if taken.contains(&prefixed) {
            out.warnings.push(Warning::new(
                "skill.repo_skipped",
                subject,
                format!(
                    "skipped: both `{}` and `{prefixed}` are already in use",
                    skill.name
                ),
            ));
            continue;
        }
        out.warnings.push(Warning::new(
            "skill.repo_prefixed",
            subject,
            format!(
                "`{}` is already in use; available as `{prefixed}`",
                skill.name
            ),
        ));
        taken.insert(prefixed.clone());
        out.entries.push(Planned {
            dest_name: prefixed,
            placement: Placement::Prefixed,
            skill,
        });
    }
    out.entries.sort_by(|a, b| a.dest_name.cmp(&b.dest_name));
    out
}

pub(crate) fn rename_frontmatter(source: &str, new_name: &str) -> String {
    let name_line = format!("name: {new_name}");
    let Ok(frontmatter::Split {
        frontmatter: Some(block),
        body,
    }) = frontmatter::split(source)
    else {
        return format!("---\n{name_line}\n---\n{source}");
    };
    let mut replaced = false;
    let lines: Vec<String> = block
        .lines()
        .map(|line| {
            if !replaced && line.starts_with("name:") {
                replaced = true;
                name_line.clone()
            } else {
                line.to_owned()
            }
        })
        .collect();
    let mut fm = lines.join("\n");
    if !replaced {
        fm = if fm.is_empty() {
            name_line
        } else {
            format!("{name_line}\n{fm}")
        };
    }
    format!("---\n{fm}\n---\n{body}")
}

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

pub(crate) fn materialise(
    layout: &Layout,
    manifest: &Manifest,
    provider: Provider,
    view_dir: &Utf8Path,
    home: Option<&Utf8Path>,
) -> Result<Vec<Warning>, Failure> {
    let mut warnings = Vec::new();
    let mut skills = Vec::new();
    for repo in manifest.repos() {
        let linked = view_dir.join(repo.name().as_str());
        // Only repos `view::materialise` actually linked (a missing worktree is skipped there).
        if !matches!(fs::read_symlink(&linked)?, fs::SymlinkTarget::Target(_)) {
            continue;
        }
        let scan = scan_repo(repo.name().as_str(), &linked);
        warnings.extend(scan.warnings);
        skills.extend(scan.skills);
    }
    let reserved = reserved_names(&reserved_dirs(layout.root(), home));
    let plan = plan(skills, &reserved);
    warnings.extend(plan.warnings.iter().cloned());
    warnings.extend(apply(
        view_dir,
        &view_dir.join(provider.skills_dir()),
        &plan,
    )?);
    Ok(warnings)
}
#[cfg(test)]
#[path = "../../../tests/unit/action/session/repo_skills.rs"]
mod tests;
