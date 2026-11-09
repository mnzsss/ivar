#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::action::Ctx;
use crate::action::script::approve::approve;
use crate::store;
use crate::test_support::seeded_hall;
use camino::Utf8PathBuf;

const SCRIPT: &str = "#!/bin/bash\n# description: d\n# usage: u\n# env: TOKEN, URL\necho hi\n";
const PATH: &str = ".ivar/scripts/a.sh";

fn hall(frontmatter: &str, script: &str) -> (tempfile::TempDir, Utf8PathBuf, Layout) {
    let (guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    fs::ensure_dir(&layout.hall_scripts()).unwrap();
    fs::write_text(&layout.hall_scripts().join("a.sh"), script).unwrap();
    fs::ensure_dir(&layout.agents_dir()).unwrap();
    fs::write_text(
        &layout.agent_file("nightly"),
        &format!(
            "---\nschedule: \"@daily\"\ntool: claude\nscripts: [{PATH}]\n{frontmatter}---\nGo.\n"
        ),
    )
    .unwrap();
    (guard, root, layout)
}

fn spec(layout: &Layout) -> AgentSpec {
    store::agent::load(layout, "nightly").unwrap()
}

#[test]
fn an_agent_with_no_scripts_needs_no_database() {
    let (_guard, _root, layout) = hall("", SCRIPT);
    let mut spec = spec(&layout);
    spec.scripts.clear();
    let before = layout.memory_db().exists();

    ensure_approved(&layout, &spec).unwrap();

    assert_eq!(layout.memory_db().exists(), before);
}

#[test]
fn a_never_approved_script_blocks_and_is_named() {
    let (_guard, _root, layout) = hall("", SCRIPT);

    let failure = ensure_approved(&layout, &spec(&layout)).unwrap_err();

    assert!(failure.what.starts_with(UNREVIEWED), "{}", failure.what);
    assert!(failure.what.contains(PATH), "{}", failure.what);
    assert!(failure.what.contains("no approval"), "{}", failure.what);
    assert!(
        failure.what.contains("ivar script approve"),
        "{}",
        failure.what
    );
}

#[test]
fn an_approved_script_passes_and_a_changed_one_blocks_again() {
    let (_guard, root, layout) = hall("", SCRIPT);
    approve(&Ctx::new(root), &Utf8PathBuf::from(PATH)).unwrap();

    ensure_approved(&layout, &spec(&layout)).unwrap();

    fs::write_text(&layout.hall_scripts().join("a.sh"), "echo changed\n").unwrap();
    let failure = ensure_approved(&layout, &spec(&layout)).unwrap_err();
    assert!(failure.what.contains(PATH), "{}", failure.what);
    assert!(
        failure.what.contains("changed since approval"),
        "{}",
        failure.what
    );
}

#[test]
fn every_unreviewed_script_is_named() {
    let (_guard, root, layout) = hall("", SCRIPT);
    fs::write_text(&layout.hall_scripts().join("b.sh"), SCRIPT).unwrap();
    approve(&Ctx::new(root), &Utf8PathBuf::from(PATH)).unwrap();
    let mut spec = spec(&layout);
    spec.scripts.push(".ivar/scripts/b.sh".to_owned());
    spec.scripts.push(".ivar/scripts/gone.sh".to_owned());

    let failure = ensure_approved(&layout, &spec).unwrap_err();

    assert!(
        !failure.what.contains("`.ivar/scripts/a.sh`"),
        "{}",
        failure.what
    );
    assert!(failure.what.contains("`.ivar/scripts/b.sh` (no approval)"));
    assert!(failure.what.contains("`.ivar/scripts/gone.sh` (missing)"));
}

#[test]
fn a_script_whose_env_is_not_passed_warns_and_one_that_is_passed_does_not() {
    let (_guard, _root, layout) = hall("secrets: [TOKEN]\n", SCRIPT);
    let warnings = env_warnings(&layout, &[spec(&layout)]);
    let [warning] = warnings.as_slice() else {
        panic!("{warnings:?}");
    };
    assert_eq!(warning.code, "agent.script_env_missing");
    assert_eq!(warning.subject, "nightly");
    assert!(warning.what.contains("`URL`"), "{}", warning.what);
    assert!(!warning.what.contains("`TOKEN`"), "{}", warning.what);
    assert!(warning.what.contains(PATH), "{}", warning.what);

    let (_guard, _root, layout) = hall("secrets: [TOKEN]\nruntime: host\nenv: {URL: x}\n", SCRIPT);
    assert!(env_warnings(&layout, &[spec(&layout)]).is_empty());
}

#[test]
fn a_script_with_no_env_header_or_a_bad_header_does_not_warn() {
    let (_guard, _root, layout) = hall("", "#!/bin/bash\n# description: d\n# usage: u\n");
    assert!(env_warnings(&layout, &[spec(&layout)]).is_empty());

    let (_guard, _root, layout) = hall("", "echo no header\n");
    assert!(env_warnings(&layout, &[spec(&layout)]).is_empty());
}
