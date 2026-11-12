#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::confirm::{SelectOption, fixed_select};
use crate::infra::fs;

#[test]
fn discovery_scans_fixture_directory_for_candidate_skills() {
    let temp = fs::TempDir::new().unwrap();
    let repo_root = temp.path().join("my-repo-12345");
    let foo_dir = repo_root.join("skills/foo");
    let bar_dir = repo_root.join("skills/bar");

    fs::ensure_dir(&foo_dir).unwrap();
    fs::ensure_dir(&bar_dir).unwrap();

    fs::write_text(
        &foo_dir.join("SKILL.md"),
        "---\nname: foo\ndescription: Foo skill\n---\nbody\n",
    )
    .unwrap();

    fs::write_text(
        &bar_dir.join("SKILL.md"),
        "---\nname: bar\ndescription: Bar skill\n---\nbody\n",
    )
    .unwrap();

    let candidates = discover_candidates(temp.path()).unwrap();
    assert_eq!(candidates.len(), 2);

    let foo_cand = candidates.iter().find(|c| c.id == "foo").unwrap();
    assert_eq!(foo_cand.path, "skills/foo");
    assert_eq!(foo_cand.description.as_deref(), Some("Foo skill"));

    let bar_cand = candidates.iter().find(|c| c.id == "bar").unwrap();
    assert_eq!(bar_cand.path, "skills/bar");
    assert_eq!(bar_cand.description.as_deref(), Some("Bar skill"));
}

#[test]
fn selection_seam_with_fixed_selects_chosen_index() {
    let options = vec![
        SelectOption {
            id: "alpha".to_owned(),
            description: Some("Alpha".to_owned()),
            path_if_any: "skills/alpha".to_owned(),
        },
        SelectOption {
            id: "beta".to_owned(),
            description: Some("Beta".to_owned()),
            path_if_any: "skills/beta".to_owned(),
        },
    ];

    let confirmer = fixed_select(true, vec![1]);
    let chosen = confirmer.select_many("Select skill", &options).unwrap();
    assert_eq!(chosen, vec![1]);
}

#[test]
fn in_candidate_duplicate_id_refusal() {
    let temp = fs::TempDir::new().unwrap();
    let repo_root = temp.path().join("my-repo-12345");
    let a_foo = repo_root.join("dir_a/foo");
    let b_foo = repo_root.join("dir_b/foo");

    fs::ensure_dir(&a_foo).unwrap();
    fs::ensure_dir(&b_foo).unwrap();

    fs::write_text(&a_foo.join("SKILL.md"), "---\nname: foo\n---\nbody\n").unwrap();
    fs::write_text(&b_foo.join("SKILL.md"), "---\nname: foo\n---\nbody\n").unwrap();

    let candidates = discover_candidates(temp.path()).unwrap();
    assert_eq!(candidates.len(), 2);
    assert_eq!(candidates[0].id, "foo");
    assert_eq!(candidates[1].id, "foo");
}

fn candidate(id: &str) -> CandidateSkill {
    CandidateSkill {
        id: id.to_owned(),
        path: format!("skills/{id}"),
        description: None,
        dir: Utf8PathBuf::from(format!("/tmp/{id}")),
    }
}

fn candidate_at(path: &str) -> CandidateSkill {
    let id = path.rsplit('/').next().unwrap();
    CandidateSkill {
        path: path.to_owned(),
        ..candidate(id)
    }
}

fn ext_at(path: &str) -> ExternalRef {
    ExternalRef {
        repo: "owner/repo".to_owned(),
        path: path.to_owned(),
        git_ref: String::new(),
    }
}

fn selected_paths(path: &str, candidates: Vec<CandidateSkill>) -> Vec<String> {
    select_candidates(&ext_at(path), candidates)
        .unwrap()
        .into_iter()
        .map(|c| c.path)
        .collect()
}

fn fixture_candidates() -> Vec<CandidateSkill> {
    vec![
        candidate_at("skills/core-skills/aws-cdk"),
        candidate_at("skills/core-skills/aws-auth"),
        candidate_at("skills-extra/other"),
        candidate_at("plugins/aws-agents/skills/agents-build"),
    ]
}

#[test]
fn path_naming_a_folder_selects_every_skill_beneath_it() {
    assert_eq!(
        selected_paths("skills", fixture_candidates()),
        ["skills/core-skills/aws-cdk", "skills/core-skills/aws-auth"]
    );
}

#[test]
fn path_naming_a_skill_selects_only_that_skill() {
    assert_eq!(
        selected_paths("skills/core-skills/aws-cdk", fixture_candidates()),
        ["skills/core-skills/aws-cdk"]
    );
}

#[test]
fn path_matching_no_folder_is_still_refused() {
    let err = select_candidates(&ext_at("skill"), fixture_candidates()).unwrap_err();
    assert_eq!(err.code, "skill.add.path_not_found");
}

#[test]
fn already_installed_candidates_are_skipped_not_refused() {
    let installed =
        |id: &str| Ok((id == "foo").then(|| Utf8PathBuf::from(".ivar/skills-local/foo")));
    let (to_install, skipped) =
        split_installed(vec![candidate("foo"), candidate("bar")], installed).unwrap();
    assert_eq!(
        to_install.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
        ["bar"]
    );
    assert_eq!(skipped.len(), 1);
    assert_eq!(skipped[0].id, "foo");
    assert_eq!(skipped[0].path, ".ivar/skills-local/foo");
}

#[test]
fn the_report_names_what_was_skipped() {
    let outcome = AddOutcome {
        root: Utf8PathBuf::from("/hall"),
        skills: Vec::new(),
        skipped: vec![SkippedSkill {
            id: "foo".to_owned(),
            path: Utf8PathBuf::from("/hall/.ivar/skills-local/foo"),
        }],
    };
    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("Skipped `foo`"), "{text}");
}
