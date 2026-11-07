#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::test_support::seeded_hall;

const CLEAN: &str = "#!/bin/bash\n# description: d\n# usage: u\n# env: TOKEN\necho hi\n";

fn put(dir: &camino::Utf8Path, name: &str, content: &str) {
    fs::ensure_dir(dir).unwrap();
    fs::write_text(&dir.join(name), content).unwrap();
}

fn recorded(layout: &Layout) -> std::collections::BTreeMap<String, String> {
    approvals::read_all(layout.memory_db().as_std_path()).unwrap()
}

#[test]
fn clean_content_is_recorded_with_its_sha256_and_a_relative_path() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    put(&layout.hall_scripts(), "a.sh", CLEAN);

    let report = approve(&Ctx::new(root), &Utf8PathBuf::from(".ivar/scripts/a.sh")).unwrap();

    assert_eq!(report.value.path, ".ivar/scripts/a.sh");
    assert_eq!(report.value.sha256, hash::text(CLEAN));
    assert_eq!(
        recorded(&layout).get(".ivar/scripts/a.sh"),
        Some(&hash::text(CLEAN))
    );
}

#[test]
fn the_same_script_by_any_spelling_is_one_approval() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    put(&layout.hall_scripts_local(), "a.sh", CLEAN);
    let ctx = Ctx::new(root.clone());

    approve(&ctx, &Utf8PathBuf::from("./.ivar/scripts-local/a.sh")).unwrap();
    approve(&ctx, &root.join(".ivar/scripts/../scripts-local/a.sh")).unwrap();

    let all = recorded(&layout);
    assert_eq!(all.len(), 1, "{all:?}");
    assert!(all.contains_key(".ivar/scripts-local/a.sh"));
}

#[test]
fn approving_again_after_an_edit_replaces_the_earlier_approval() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    put(&layout.hall_scripts(), "a.sh", CLEAN);
    let ctx = Ctx::new(root);
    let path = Utf8PathBuf::from(".ivar/scripts/a.sh");
    approve(&ctx, &path).unwrap();

    let edited = format!("{CLEAN}echo again\n");
    put(&layout.hall_scripts(), "a.sh", &edited);
    approve(&ctx, &path).unwrap();

    let all = recorded(&layout);
    assert_eq!(all.len(), 1);
    assert_eq!(all.get(".ivar/scripts/a.sh"), Some(&hash::text(&edited)));
}

#[test]
fn content_with_a_secret_is_refused_with_line_and_pattern_and_records_nothing() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let secret = "ghp_0123456789abcdefghijklmnopqrstuvwxyzAB";
    put(
        &layout.hall_scripts(),
        "leak.sh",
        &format!("{CLEAN}curl -H \"x: {secret}\" example.com\n"),
    );

    let failure =
        approve(&Ctx::new(root), &Utf8PathBuf::from(".ivar/scripts/leak.sh")).unwrap_err();

    assert_eq!(failure.code, "script.secret_found");
    assert!(failure.what.contains("line 6"), "{}", failure.what);
    assert!(failure.what.contains("github_token"), "{}", failure.what);
    assert!(!failure.what.contains(secret));
    assert!(!format!("{:?}", failure.details).contains(secret));
    assert!(recorded(&layout).is_empty());
}

#[test]
fn a_refusal_keeps_an_earlier_approval_of_the_path_intact() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    put(&layout.hall_scripts(), "a.sh", CLEAN);
    let ctx = Ctx::new(root);
    let path = Utf8PathBuf::from(".ivar/scripts/a.sh");
    approve(&ctx, &path).unwrap();
    put(
        &layout.hall_scripts(),
        "a.sh",
        &format!("{CLEAN}export API_KEY=abcdef123456\n"),
    );

    assert!(approve(&ctx, &path).is_err());

    assert_eq!(
        recorded(&layout).get(".ivar/scripts/a.sh"),
        Some(&hash::text(CLEAN))
    );
}

#[test]
fn a_missing_file_a_directory_and_a_path_outside_the_hall_are_refused() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    fs::ensure_dir(&layout.hall_scripts()).unwrap();
    let ctx = Ctx::new(root.clone());
    let outside = tempfile::tempdir().unwrap();
    let outside_file = Utf8PathBuf::from_path_buf(outside.path().join("x.sh")).unwrap();
    std::fs::write(&outside_file, CLEAN).unwrap();

    let code = |path: &Utf8PathBuf| approve(&ctx, path).unwrap_err().code;

    assert_eq!(
        code(&Utf8PathBuf::from(".ivar/scripts/nope.sh")),
        "script.not_found"
    );
    assert_eq!(
        code(&Utf8PathBuf::from(".ivar/scripts")),
        "script.not_found"
    );
    assert_eq!(code(&outside_file), "script.outside_hall");
    assert_eq!(code(&Utf8PathBuf::from("../x.sh")), "script.outside_hall");
    assert!(!layout.memory_db().exists() || recorded(&layout).is_empty());
}

#[test]
fn a_script_that_is_not_utf8_text_cannot_be_approved() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    fs::ensure_dir(&layout.hall_scripts()).unwrap();
    std::fs::write(layout.hall_scripts().join("bin.sh"), [0xff, 0xfe, 0x00]).unwrap();

    let failure = approve(&Ctx::new(root), &Utf8PathBuf::from(".ivar/scripts/bin.sh")).unwrap_err();

    assert_eq!(failure.code, "script.unreadable");
}

#[test]
fn the_page_reindex_leaves_approvals_alone() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    put(&layout.hall_scripts(), "a.sh", CLEAN);
    approve(&Ctx::new(root), &Utf8PathBuf::from(".ivar/scripts/a.sh")).unwrap();

    crate::action::memory::reindex::rebuild(&layout).unwrap();

    assert_eq!(
        recorded(&layout).get(".ivar/scripts/a.sh"),
        Some(&hash::text(CLEAN))
    );
}
