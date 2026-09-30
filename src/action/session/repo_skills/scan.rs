//! Which skills a repo ships, and which names the hall and the user already use.

use std::collections::{BTreeMap, BTreeSet};

use camino::{Utf8Path, Utf8PathBuf};

use crate::error::Warning;
use crate::infra::fs;

use super::skill_md::skill_name;

/// Where a repo (and the hall) keeps skills, in precedence order.
pub(crate) const SOURCE_DIRS: [&str; 4] = [
    ".omp/skills",
    ".agents/skills",
    ".claude/skills",
    ".opencode/skills",
];
/// User-level skill dirs every harness also loads.
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

#[derive(Debug, Default)]
pub(crate) struct Scan {
    pub skills: Vec<RepoSkill>,
    pub warnings: Vec<Warning>,
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
