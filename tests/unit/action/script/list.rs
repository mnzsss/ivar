#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::test_support::seeded_hall;

fn put(dir: &camino::Utf8Path, name: &str, content: &str) {
    fs::ensure_dir(dir).unwrap();
    fs::write_text(&dir.join(name), content).unwrap();
}

#[test]
fn a_fresh_hall_has_no_scripts() {
    let (_guard, root) = seeded_hall();
    let report = list(&Ctx::new(root)).unwrap();
    assert!(report.is_clean());
    assert!(report.value.scripts.is_empty());
    assert!(report.value.invalid.is_empty());
}

#[test]
fn valid_scripts_list_with_scope_header_fields_and_unknown_approval() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    put(
        &layout.hall_scripts(),
        "deploy.sh",
        "#!/bin/bash\n# description: Deploy.\n# usage: deploy.sh <env>\n# env: TOKEN, URL\n",
    );
    put(
        &layout.hall_scripts_local(),
        "mine.py",
        "# description: Mine.\n# usage: mine.py\n",
    );
    put(&layout.hall_scripts(), "bad.sh", "echo hi\n");

    let value = list(&Ctx::new(root)).unwrap().value;

    let names: Vec<_> = value
        .scripts
        .iter()
        .map(|s| (s.name.as_str(), s.scope))
        .collect();
    assert_eq!(
        names,
        [
            ("mine", ScriptScope::Personal),
            ("deploy", ScriptScope::Team)
        ]
    );
    let deploy = value.scripts.get(1).unwrap();
    assert_eq!(deploy.path, ".ivar/scripts/deploy.sh");
    assert_eq!(deploy.env, ["TOKEN", "URL"]);
    assert!(value.scripts.iter().all(|s| s.approval == "unknown"));
    assert_eq!(value.invalid.len(), 1);
    assert_eq!(
        value.invalid.first().unwrap().code,
        "script.missing_description"
    );
}

#[test]
fn the_human_surface_names_valid_and_invalid_scripts() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    put(
        &layout.hall_scripts(),
        "deploy.sh",
        "# description: Deploy.\n# usage: deploy.sh\n",
    );
    put(&layout.hall_scripts(), "notes.txt", "x");
    let report = list(&Ctx::new(root)).unwrap();

    let mut out = Vec::new();
    report.value.write_human(&mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("deploy"), "{text}");
    assert!(text.contains("team"), "{text}");
    assert!(text.contains("unknown"), "{text}");
    assert!(text.contains(".ivar/scripts/notes.txt"), "{text}");
}

#[test]
fn listing_never_touches_the_scripts() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let content = "# description: d\n# usage: u\n";
    put(&layout.hall_scripts(), "a.sh", content);
    let path = layout.hall_scripts().join("a.sh");
    let before = std::fs::metadata(&path).unwrap().modified().unwrap();
    list(&Ctx::new(root)).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), content);
    assert_eq!(
        std::fs::metadata(&path).unwrap().modified().unwrap(),
        before
    );
}
