#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::store::layout::Layout;
use crate::test_support::seeded_hall;

fn write_skill_in(dir: &camino::Utf8Path, id: &str) {
    let skill_dir = dir.join(id);
    fs::ensure_dir(&skill_dir).unwrap();
    fs::write_text(
        &skill_dir.join("SKILL.md"),
        &format!("---\nname: {id}\ndescription: The {id} skill\n---\n\nBody.\n"),
    )
    .unwrap();
}

#[test]
fn an_absent_directory_yields_no_skills() {
    let (_guard, root) = seeded_hall();
    let missing = Layout::at(root.clone()).hall_skills_local();

    let skills = enumerate(&missing, SkillRoot::Local).unwrap();

    assert!(
        skills.is_empty(),
        "a hall without a personal root is normal, not an error"
    );
}

#[test]
fn skills_come_back_sorted_by_id() {
    let (_guard, root) = seeded_hall();
    let dir = Layout::at(root.clone()).hall_skills();
    fs::ensure_dir(&dir).unwrap();
    write_skill_in(&dir, "zebra");
    write_skill_in(&dir, "alpha");
    write_skill_in(&dir, "middle");

    let skills = enumerate(&dir, SkillRoot::Hall).unwrap();

    let ids: Vec<&str> = skills.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["alpha", "middle", "zebra"]);
}

#[test]
fn every_skill_carries_the_root_it_came_from() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let local_dir = layout.hall_skills_local();
    fs::ensure_dir(&local_dir).unwrap();
    write_skill_in(&local_dir, "private");

    let skills = enumerate(&local_dir, SkillRoot::Local).unwrap();

    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].root, SkillRoot::Local);
}

#[test]
fn an_entry_that_is_not_a_valid_id_is_skipped_not_an_error() {
    let (_guard, root) = seeded_hall();
    let dir = Layout::at(root.clone()).hall_skills();
    fs::ensure_dir(&dir).unwrap();
    write_skill_in(&dir, "valid");
    // A name `RepoName` refuses. It must not blind the caller to `valid`.
    fs::ensure_dir(&dir.join("not a valid name")).unwrap();

    let skills = enumerate(&dir, SkillRoot::Hall).unwrap();

    let ids: Vec<&str> = skills.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["valid"]);
}

#[test]
fn both_roots_merge_when_no_id_collides() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let (hall_dir, local_dir) = (layout.hall_skills(), layout.hall_skills_local());
    fs::ensure_dir(&hall_dir).unwrap();
    fs::ensure_dir(&local_dir).unwrap();
    write_skill_in(&hall_dir, "shared");
    write_skill_in(&local_dir, "private");

    let (skills, warnings) = enumerate_both(&hall_dir, &local_dir).unwrap();

    let ids: Vec<&str> = skills.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["private", "shared"]);
    assert!(warnings.is_empty());
}

#[test]
fn a_colliding_id_is_dropped_from_both_roots_with_one_warning() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let (hall_dir, local_dir) = (layout.hall_skills(), layout.hall_skills_local());
    fs::ensure_dir(&hall_dir).unwrap();
    fs::ensure_dir(&local_dir).unwrap();
    write_skill_in(&hall_dir, "clash");
    write_skill_in(&local_dir, "clash");

    let (skills, warnings) = enumerate_both(&hall_dir, &local_dir).unwrap();

    assert!(
        skills.is_empty(),
        "neither copy may be materialised — one slot per id"
    );
    assert_eq!(warnings.len(), 1, "one warning per pair, not per copy");

    let warning = &warnings[0];
    assert_eq!(warning.code, "skill.collision");
    assert_eq!(warning.subject, "clash");
    // Both absolute paths must appear, or the user reads this as
    // "my skill vanished" rather than "rename one of these two".
    assert!(
        warning.what.contains(hall_dir.join("clash").as_str()),
        "warning must name the hall path: {}",
        warning.what
    );
    assert!(
        warning.what.contains(local_dir.join("clash").as_str()),
        "warning must name the local path: {}",
        warning.what
    );
}

#[test]
fn a_collision_does_not_block_the_other_skills() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let (hall_dir, local_dir) = (layout.hall_skills(), layout.hall_skills_local());
    fs::ensure_dir(&hall_dir).unwrap();
    fs::ensure_dir(&local_dir).unwrap();
    write_skill_in(&hall_dir, "clash");
    write_skill_in(&local_dir, "clash");
    write_skill_in(&hall_dir, "innocent");
    write_skill_in(&local_dir, "bystander");

    let (skills, warnings) = enumerate_both(&hall_dir, &local_dir).unwrap();

    let ids: Vec<&str> = skills.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["bystander", "innocent"]);
    assert_eq!(warnings.len(), 1);
}
