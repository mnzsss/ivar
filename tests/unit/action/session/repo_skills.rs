//! Unit tests for `crate::action::session::repo_skills`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeSet;

use camino::{Utf8Path, Utf8PathBuf};

use super::*;
use crate::infra::fs;
use crate::test_support::utf8_temp_dir;

fn write_skill(dir: &Utf8Path, frontmatter: &str) {
    fs::ensure_dir(dir).unwrap();
    fs::write_text(&dir.join("SKILL.md"), &format!("{frontmatter}\n# body\n")).unwrap();
}

fn skill(repo: &str, name: &str) -> RepoSkill {
    RepoSkill {
        repo: repo.to_owned(),
        name: name.to_owned(),
        source_rel: Utf8PathBuf::from(format!(".claude/skills/{name}")),
    }
}

fn names(set: &[&str]) -> BTreeSet<String> {
    set.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn scan_reads_every_provider_dir_and_ignores_top_level_skills() {
    let (_g, root) = utf8_temp_dir();
    write_skill(
        &root.join(".claude/skills/react-doctor"),
        "---\nname: react-doctor\ndescription: d\n---",
    );
    write_skill(&root.join(".agents/skills/lint"), "---\nname: lint\n---");
    write_skill(&root.join("skills/top-level"), "---\nname: top-level\n---");

    let scan = scan_repo("valhalla", &root);

    let found: Vec<_> = scan
        .skills
        .iter()
        .map(|s| (s.name.as_str(), s.source_rel.as_str()))
        .collect();
    assert_eq!(
        found,
        vec![
            ("lint", ".agents/skills/lint"),
            ("react-doctor", ".claude/skills/react-doctor")
        ]
    );
    assert!(scan.warnings.is_empty());
}

#[test]
fn scan_falls_back_to_the_dir_name_when_frontmatter_has_no_name() {
    let (_g, root) = utf8_temp_dir();
    write_skill(
        &root.join(".claude/skills/tauri"),
        "---\ndescription: no name here\n---",
    );

    let scan = scan_repo("mrunner", &root);

    assert_eq!(scan.skills[0].name, "tauri");
}

#[test]
fn scan_skips_malformed_frontmatter_with_a_warning() {
    let (_g, root) = utf8_temp_dir();
    write_skill(
        &root.join(".claude/skills/broken"),
        "---\nname: [unclosed\n---",
    );

    let scan = scan_repo("valhalla", &root);

    assert!(scan.skills.is_empty());
    assert_eq!(scan.warnings[0].code, "skill.repo_unreadable");
}

#[test]
fn scan_keeps_the_first_source_dir_for_a_duplicate_name() {
    let (_g, root) = utf8_temp_dir();
    write_skill(&root.join(".claude/skills/x"), "---\nname: x\n---");
    write_skill(&root.join(".omp/skills/x"), "---\nname: x\n---");

    let scan = scan_repo("valhalla", &root);

    assert_eq!(scan.skills.len(), 1);
    assert_eq!(scan.skills[0].source_rel, ".omp/skills/x");
    assert_eq!(scan.warnings[0].code, "skill.repo_duplicate");
}

#[test]
fn reserved_names_include_dir_names_and_frontmatter_names() {
    let (_g, root) = utf8_temp_dir();
    write_skill(&root.join("hall/skill"), "---\nname: seo-audit\n---");
    write_skill(&root.join("hall/commit"), "---\nname: commit\n---");

    let reserved = reserved_names(&[root.join("hall"), root.join("missing")]);

    assert_eq!(reserved, names(&["commit", "seo-audit", "skill"]));
}

#[test]
fn reserved_dirs_cover_hall_and_user_level_locations() {
    let dirs = reserved_dirs(Utf8Path::new("/hall"), Some(Utf8Path::new("/home/u")));

    let dirs: Vec<&str> = dirs.iter().map(|p| p.as_str()).collect();
    assert_eq!(
        dirs,
        vec![
            "/hall/.omp/skills",
            "/hall/.agents/skills",
            "/hall/.claude/skills",
            "/hall/.opencode/skills",
            "/home/u/.agents/skills",
            "/home/u/.claude/skills",
            "/home/u/.omp/agent/skills",
            "/home/u/.config/opencode/skills",
        ]
    );
    assert_eq!(reserved_dirs(Utf8Path::new("/hall"), None).len(), 4);
}

#[test]
fn plan_keeps_bare_names_that_nothing_else_uses() {
    let plan = plan(vec![skill("valhalla", "react-doctor")], &BTreeSet::new());

    assert_eq!(plan.entries[0].dest_name, "react-doctor");
    assert_eq!(plan.entries[0].placement, Placement::Bare);
    assert!(plan.warnings.is_empty());
}

#[test]
fn plan_prefixes_a_repo_skill_that_shadows_a_hall_or_user_skill() {
    let plan = plan(vec![skill("valhalla", "commit")], &names(&["commit"]));

    assert_eq!(plan.entries[0].dest_name, "valhalla--commit");
    assert_eq!(plan.entries[0].placement, Placement::Prefixed);
    assert_eq!(plan.warnings[0].code, "skill.repo_prefixed");
}

#[test]
fn plan_prefixes_both_sides_when_two_repos_ship_the_same_name() {
    let plan = plan(
        vec![skill("mrunner", "tauri"), skill("valhalla", "tauri")],
        &BTreeSet::new(),
    );

    let dests: Vec<_> = plan.entries.iter().map(|p| p.dest_name.as_str()).collect();
    assert_eq!(dests, vec!["mrunner--tauri", "valhalla--tauri"]);
}

#[test]
fn plan_skips_when_even_the_prefixed_name_is_taken() {
    let plan = plan(
        vec![skill("valhalla", "commit")],
        &names(&["commit", "valhalla--commit"]),
    );

    assert!(plan.entries.is_empty());
    assert_eq!(plan.warnings[0].code, "skill.repo_skipped");
}

#[test]
fn rename_frontmatter_rewrites_only_the_name_line() {
    let source = "---\nname: commit\nversion: \"1.1.0\"\ndisable-model-invocation: true\n---\n# body\nname: untouched\n";

    let renamed = rename_frontmatter(source, "valhalla--commit");

    assert_eq!(
        renamed,
        "---\nname: valhalla--commit\nversion: \"1.1.0\"\ndisable-model-invocation: true\n---\n# body\nname: untouched\n"
    );
}

#[test]
fn rename_frontmatter_adds_a_name_when_there_is_none() {
    assert_eq!(
        rename_frontmatter("---\ndescription: d\n---\nb\n", "r--x"),
        "---\nname: r--x\ndescription: d\n---\nb\n"
    );
    assert_eq!(
        rename_frontmatter("# no frontmatter\n", "r--x"),
        "---\nname: r--x\n---\n# no frontmatter\n"
    );
}
