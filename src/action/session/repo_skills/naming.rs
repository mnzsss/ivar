//! The name each repo skill gets in a session: bare when free, else
//! `<repo>--<name>`, never shadowing a hall or user skill.

use std::collections::{BTreeMap, BTreeSet};

use crate::error::Warning;

use super::scan::RepoSkill;

pub(crate) const PREFIX_SEPARATOR: &str = "--";

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
