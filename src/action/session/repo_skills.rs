//! Repo skills in session view dirs: find the skills a hall repo ships in
//! its provider skill dirs and decide the name each one gets in a session,
//! so a hall or user skill is never shadowed (see feature
//! `repo-skills-in-sessions`).
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use camino::{Utf8Path, Utf8PathBuf};

use crate::error::Warning;
use crate::infra::{frontmatter, fs};

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

/// The identity a harness gives `dir/SKILL.md`: its frontmatter `name`, or
/// the directory name when the frontmatter has none. `Err` carries the
/// parse error text.
fn skill_name(dir: &Utf8Path) -> Result<Option<String>, String> {
    let path = dir.join("SKILL.md");
    let Some(text) = fs::read_text(&path).map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let meta: NameOnly = frontmatter::parse(&text).map_err(|e| e.to_string())?;
    Ok(Some(
        meta.name
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| dir.file_name().unwrap_or_default().to_owned()),
    ))
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

#[cfg(test)]
#[path = "../../../tests/unit/action/session/repo_skills.rs"]
mod tests;
