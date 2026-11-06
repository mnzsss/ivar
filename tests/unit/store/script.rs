#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::test_support::seeded_hall;

const GOOD: &str = "#!/bin/bash\n# description: d\n# usage: u\n";

fn put(dir: &Utf8Path, name: &str, content: &str) {
    crate::infra::fs::ensure_dir(dir).unwrap();
    crate::infra::fs::write_text(&dir.join(name), content).unwrap();
}

#[test]
fn missing_folders_contribute_nothing() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    assert!(scan(&layout).unwrap().is_empty());
}

#[test]
fn personal_first_then_team_alphabetical_with_duplicates_kept() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    put(&layout.hall_scripts(), "zeta.sh", GOOD);
    put(&layout.hall_scripts(), "alpha.sh", GOOD);
    put(&layout.hall_scripts(), "a.sh", GOOD);
    put(&layout.hall_scripts(), "a-b.sh", GOOD);
    put(&layout.hall_scripts_local(), "mid.py", GOOD);
    put(&layout.hall_scripts_local(), "alpha.sh", GOOD);

    let order: Vec<_> = scan(&layout)
        .unwrap()
        .iter()
        .map(|f| (f.scope, f.name().to_owned()))
        .collect();
    assert_eq!(
        order,
        [
            (ScriptScope::Personal, "alpha".to_owned()),
            (ScriptScope::Personal, "mid".to_owned()),
            (ScriptScope::Team, "a".to_owned()),
            (ScriptScope::Team, "a-b".to_owned()),
            (ScriptScope::Team, "alpha".to_owned()),
            (ScriptScope::Team, "zeta".to_owned()),
        ]
    );
}

#[test]
fn malformed_files_carry_their_problem_and_dotfiles_and_dirs_are_skipped() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    let dir = layout.hall_scripts();
    put(&dir, "bad.sh", "echo no header\n");
    put(&dir, "notes.txt", "hello\n");
    put(&dir, ".gitkeep", "");
    crate::infra::fs::ensure_dir(&dir.join("nested")).unwrap();

    let files = scan(&layout).unwrap();
    let problems: Vec<_> = files
        .iter()
        .map(|f| (f.file_name.as_str(), f.parsed.clone().unwrap_err()))
        .collect();
    assert_eq!(
        problems,
        [
            ("bad.sh", ScriptProblem::MissingDescription),
            ("notes.txt", ScriptProblem::UnsupportedExtension),
        ]
    );
}

#[test]
fn non_utf8_content_is_unreadable_not_fatal() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    crate::infra::fs::ensure_dir(&layout.hall_scripts()).unwrap();
    std::fs::write(layout.hall_scripts().join("bin.sh"), [0xff, 0xfe, 0x00]).unwrap();
    let files = scan(&layout).unwrap();
    assert_eq!(
        files.first().unwrap().parsed.clone().unwrap_err(),
        ScriptProblem::Unreadable
    );
}
