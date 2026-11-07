#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::store::memory::db::MemoryDb;
use crate::test_support::seeded_hall;

fn accept(layout: &Layout, intent: &str, artifact: &str) -> i64 {
    let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    db.conn()
        .execute(
            "INSERT INTO automation_decisions (kind, intent, form, artifact, created_at, updated_at)
             VALUES ('accepted', ?1, 'script', ?2, 'then', 'then')",
            rusqlite::params![intent, artifact],
        )
        .unwrap();
    db.conn().last_insert_rowid()
}

#[test]
fn a_hall_without_decisions_has_no_finding_and_gets_no_database() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);

    assert!(automation_diagnoses(&layout).is_empty());
    assert!(!layout.memory_db().exists());
}

#[test]
fn an_accepted_decision_whose_artifact_is_missing_is_named_with_its_fix() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    let present = accept(&layout, "changelog", ".ivar/scripts/changelog.sh");
    let missing = accept(&layout, "release", ".ivar/scripts/release.sh");
    std::fs::create_dir_all(layout.root().join(".ivar/scripts").as_std_path()).unwrap();
    std::fs::write(
        layout
            .root()
            .join(".ivar/scripts/changelog.sh")
            .as_std_path(),
        "",
    )
    .unwrap();

    let findings = automation_diagnoses(&layout);

    let [finding] = findings.as_slice() else {
        panic!("{findings:?}");
    };
    assert_eq!(finding.code, "automation.artifact_missing");
    assert!(finding.what.contains(".ivar/scripts/release.sh"));
    assert!(finding.what.contains(&format!("decision {missing}")));
    assert!(!finding.what.contains(&format!("decision {present} ")));
    assert!(
        finding
            .fix
            .contains(&format!("ivar automation undo {missing}"))
    );
}

#[test]
fn a_decline_has_no_artifact_to_lose() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    db.conn()
        .execute(
            "INSERT INTO automation_decisions (kind, intent, created_at, updated_at)
             VALUES ('declined', 'x', 'then', 'then')",
            [],
        )
        .unwrap();
    drop(db);

    assert!(automation_diagnoses(&layout).is_empty());
}

#[test]
fn an_artifact_that_climbs_out_of_the_hall_counts_as_missing() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    accept(&layout, "x", "../elsewhere.sh");

    assert_eq!(automation_diagnoses(&layout).len(), 1);
}
