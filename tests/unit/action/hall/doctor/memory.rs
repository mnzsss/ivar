#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::action::Ctx;
use crate::action::sync::SyncInput;
use crate::test_support::seeded_hall;

fn codes(findings: &[Diagnosis]) -> Vec<&'static str> {
    findings.iter().map(|d| d.code).collect()
}

#[test]
fn a_synced_hall_with_capture_on_and_an_empty_spool_is_quiet() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    crate::action::sync::sync(&Ctx::new(root), &SyncInput::default()).unwrap();

    let manifest = Manifest::read(&layout).unwrap().unwrap();

    assert!(memory_capture_diagnoses(&layout, &manifest).is_empty());
}

#[test]
fn an_old_undrained_spool_is_an_orphan_with_the_drain_fix() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    std::fs::create_dir_all(layout.memory_spool_dir()).unwrap();
    let spool = layout.memory_spool_dir().join("old.jsonl");
    std::fs::write(&spool, "{}\n").unwrap();
    let long_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
    std::fs::File::options()
        .write(true)
        .open(&spool)
        .unwrap()
        .set_modified(long_ago)
        .unwrap();
    let manifest = Manifest::read(&layout).unwrap().unwrap();

    let findings = memory_capture_diagnoses(&layout, &manifest);

    let orphan = findings
        .iter()
        .find(|d| d.code == "memory.spool_orphan")
        .unwrap();
    assert!(orphan.fix.contains("ivar memory status"), "{}", orphan.fix);
}

#[test]
fn dropped_events_and_a_failed_drain_are_reported_with_their_fix() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    std::fs::create_dir_all(layout.memory_spool_dir()).unwrap();
    std::fs::write(layout.memory_spool_dir().join("s.dropped"), "xxx").unwrap();
    std::fs::write(
        layout.memory_spool_dir().join("_drain.json"),
        r#"{"last_drain_at":null,"last_error":"database is locked","drained_total":0,"dropped_total":0}"#,
    )
    .unwrap();
    let manifest = Manifest::read(&layout).unwrap().unwrap();

    let found = codes(&memory_capture_diagnoses(&layout, &manifest));

    assert!(found.contains(&"memory.spool_dropped"), "{found:?}");
    assert!(found.contains(&"memory.drain_failed"), "{found:?}");
}

fn write_drain_state(layout: &Layout, dropped_total: u64, last_drop_age_secs: u64) {
    std::fs::create_dir_all(layout.memory_spool_dir()).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    std::fs::write(
        layout.memory_spool_dir().join("_drain.json"),
        serde_json::json!({
            "dropped_total": dropped_total,
            "dropped_last_epoch_secs": now - last_drop_age_secs
        })
        .to_string(),
    )
    .unwrap();
}

#[test]
fn drops_already_acknowledged_by_a_drain_are_still_reported_for_seven_days() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    write_drain_state(&layout, 5, 3600);
    let manifest = Manifest::read(&layout).unwrap().unwrap();

    let findings = memory_capture_diagnoses(&layout, &manifest);

    let dropped = findings
        .iter()
        .find(|d| d.code == "memory.spool_dropped")
        .unwrap();
    assert!(dropped.what.contains("5 events"), "{}", dropped.what);
    assert!(dropped.fix.contains("ivar graph mcp"), "{}", dropped.fix);
}

#[test]
fn drops_acknowledged_more_than_seven_days_ago_no_longer_clutter_the_doctor() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    write_drain_state(&layout, 5, 8 * 24 * 3600);
    let manifest = Manifest::read(&layout).unwrap().unwrap();

    let found = codes(&memory_capture_diagnoses(&layout, &manifest));

    assert!(!found.contains(&"memory.spool_dropped"), "{found:?}");
}

#[test]
fn a_provider_without_the_capture_hook_is_reported_only_while_capture_is_on() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    std::fs::remove_file(root.join(".claude/settings.json")).ok();
    let manifest = Manifest::read(&layout).unwrap().unwrap();

    assert!(
        codes(&memory_capture_diagnoses(&layout, &manifest))
            .contains(&"provider.capture_hook_missing")
    );

    let off = manifest.with_memory_capture(false).unwrap();
    assert!(memory_capture_diagnoses(&layout, &off).is_empty());
}
