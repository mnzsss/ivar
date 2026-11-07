#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::action::script::approve::approve;
use crate::store::layout::Layout;
use crate::test_support::seeded_hall;

const CLEAN: &str = "#!/bin/bash\n# description: d\n# usage: u\n# env: TOKEN, URL\necho hi\n";

fn put(dir: &camino::Utf8Path, name: &str, content: &str) {
    fs::ensure_dir(dir).unwrap();
    fs::write_text(&dir.join(name), content).unwrap();
}

fn path() -> Utf8PathBuf {
    Utf8PathBuf::from(".ivar/scripts/a.sh")
}

#[test]
fn a_script_nobody_approved_is_unreviewed_and_the_check_does_not_create_the_database() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    put(&layout.hall_scripts(), "a.sh", CLEAN);
    let before = layout.memory_db().exists();

    let value = check(&Ctx::new(root), &path()).unwrap().value;

    assert_eq!(value.approval, "unreviewed");
    assert_eq!(value.reason, Some("no approval"));
    assert!(!value.is_approved());
    assert_eq!(value.env, ["TOKEN", "URL"]);
    assert_eq!(layout.memory_db().exists(), before, "a read never creates");
}

#[test]
fn approved_content_checks_as_approved_without_a_reason() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    put(&layout.hall_scripts(), "a.sh", CLEAN);
    let ctx = Ctx::new(root);
    approve(&ctx, &path()).unwrap();

    let value = check(&ctx, &path()).unwrap().value;

    assert_eq!(value.approval, "approved");
    assert_eq!(value.reason, None);
    assert!(value.is_approved());
    assert_eq!(value.sha256, crate::infra::hash::text(CLEAN));
}

#[test]
fn editing_the_file_makes_it_unreviewed_because_it_changed() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    put(&layout.hall_scripts(), "a.sh", CLEAN);
    let ctx = Ctx::new(root);
    approve(&ctx, &path()).unwrap();

    put(
        &layout.hall_scripts(),
        "a.sh",
        &format!("{CLEAN}rm -rf x\n"),
    );
    let value = check(&ctx, &path()).unwrap().value;

    assert_eq!(value.approval, "unreviewed");
    assert_eq!(value.reason, Some("changed since approval"));
}

#[test]
fn the_human_text_names_state_reason_and_env() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    put(&layout.hall_scripts(), "a.sh", CLEAN);
    put(
        &layout.hall_scripts(),
        "b.sh",
        "# description: d\n# usage: u\n",
    );
    let ctx = Ctx::new(root);
    approve(&ctx, &path()).unwrap();
    let text = |p: &str| {
        let mut out = Vec::new();
        check(&ctx, &Utf8PathBuf::from(p))
            .unwrap()
            .value
            .write_human(&mut out)
            .unwrap();
        String::from_utf8(out).unwrap()
    };

    assert_eq!(
        text(".ivar/scripts/a.sh"),
        "approved: .ivar/scripts/a.sh\nenv: TOKEN, URL\n"
    );
    assert_eq!(
        text(".ivar/scripts/b.sh"),
        "unreviewed: .ivar/scripts/b.sh (no approval)\nenv: none\n"
    );
}

#[test]
fn a_file_with_a_bad_header_still_checks_with_no_env() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    put(&layout.hall_scripts(), "a.sh", "# env: X\necho\n");

    let value = check(&Ctx::new(root), &path()).unwrap().value;

    assert!(value.env.is_empty());
    assert_eq!(value.approval, "unreviewed");
}

#[test]
fn a_missing_script_is_a_failure_not_unreviewed() {
    let (_guard, root) = seeded_hall();

    let failure = check(&Ctx::new(root), &path()).unwrap_err();

    assert_eq!(failure.code, "script.not_found");
}
