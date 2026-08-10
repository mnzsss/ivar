#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};

use super::*;
use crate::error::Status;
use crate::infra::fs;
use crate::test_support::utf8_temp_dir;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Widget {
    version: u32,
    name: String,
    #[serde(default)]
    color: String,
}

fn v0_to_v1(mut value: serde_json::Value) -> Result<serde_json::Value, String> {
    if let serde_json::Value::Object(map) = &mut value {
        map.entry("color")
            .or_insert_with(|| serde_json::Value::String("gray".to_owned()));
    }
    Ok(value)
}

fn v1_to_v2_rename_label_to_name(
    mut value: serde_json::Value,
) -> Result<serde_json::Value, String> {
    if let serde_json::Value::Object(map) = &mut value
        && let Some(label) = map.remove("label")
    {
        map.insert("name".to_owned(), label);
    }
    Ok(value)
}

fn always_fails(_value: serde_json::Value) -> Result<serde_json::Value, String> {
    Err("this step always fails".to_owned())
}

// -- absent is Ok(None) --------------------------------------------------

#[test]
fn absent_file_reads_inspects_and_migrates_as_ok_none() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join("missing.json");
    let store = Store::<Widget>::new(path, vec![], 1, Policy::Local);

    assert_eq!(store.read().unwrap(), None);
    assert_eq!(store.inspect().unwrap(), None);
    assert_eq!(store.migrate().unwrap(), None);
}

// -- unversioned data is version 0, not an error -------------------------

#[test]
fn missing_version_field_detects_as_zero() {
    let value = serde_json::json!({"name": "legacy"});
    assert_eq!(detect_version(&value), 0);
}

#[test]
fn non_numeric_version_field_detects_as_zero() {
    let value = serde_json::json!({"version": "not-a-number", "name": "legacy"});
    assert_eq!(detect_version(&value), 0);
}

#[test]
fn unversioned_data_is_refused_when_no_migration_reaches_the_current_version() {
    // An empty chain at current = 1 means this format never had a v0. A file
    // detected at v0 is therefore not one of ours, and must be refused as a
    // versioning failure — NOT deserialized. Getting this wrong is silent:
    // `run_migrations` is a no-op on an empty chain and `stamp_version` would
    // relabel the value as current, adopting a foreign file as ours.
    let (_dir, root) = utf8_temp_dir();
    let path = root.join("state.json");
    fs::write_text(&path, r#"{"nothing_widget_understands": true}"#).unwrap();

    let store = Store::<Widget>::new(path.clone(), vec![], 1, Policy::Local);
    let error = store.read().unwrap_err();

    match error {
        Error::NoMigrationPath { found, current, .. } => {
            assert_eq!((found, current), (0, 1));
        }
        other => panic!("expected NoMigrationPath, got {other:?}"),
    }

    // And the refusal left the file exactly as it found it.
    assert_eq!(
        fs::read_text(&path).unwrap().unwrap(),
        r#"{"nothing_widget_understands": true}"#
    );
}

#[test]
fn a_shape_valid_unversioned_file_is_still_refused_by_an_empty_chain() {
    // The dangerous case: the payload *would* deserialize cleanly, so nothing
    // downstream would notice the adoption. Only the version guard catches it.
    let (_dir, root) = utf8_temp_dir();
    let path = root.join("state.json");
    fs::write_text(&path, r#"{"name": "looks-fine"}"#).unwrap();

    let store = Store::<Widget>::new(path, vec![], 1, Policy::Local);

    assert!(matches!(
        store.read().unwrap_err(),
        Error::NoMigrationPath { .. }
    ));
}

// -- behaviour 1: newer-than-binary is a hard refusal, nothing mutated ---

#[test]
fn data_newer_than_the_binary_is_refused_on_read_and_the_file_is_untouched() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join("state.json");
    let original = r#"{"version":99,"name":"from-the-future","color":"red"}"#;
    fs::write_text(&path, original).unwrap();

    let store = Store::<Widget>::new(path.clone(), vec![], 1, Policy::Local);
    let error = store.read().unwrap_err();

    match error {
        Error::TooNew { found, highest, .. } => {
            assert_eq!(found, 99);
            assert_eq!(highest, 1);
        }
        other => panic!("expected Error::TooNew, got {other:?}"),
    }

    let bytes_after = fs::read_bytes(&path).unwrap().unwrap();
    assert_eq!(
        bytes_after,
        original.as_bytes(),
        "refusal must not touch the file"
    );
}

#[test]
fn data_newer_than_the_binary_is_refused_on_write_and_the_file_is_untouched() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join("state.json");
    let original = r#"{"version":99,"name":"from-the-future","color":"red"}"#;
    fs::write_text(&path, original).unwrap();

    let store = Store::<Widget>::new(path.clone(), vec![], 1, Policy::Local);
    let attempted = Widget {
        version: 1,
        name: "overwrite-attempt".to_owned(),
        color: "blue".to_owned(),
    };
    let error = store.write(&attempted).unwrap_err();

    assert!(matches!(error, Error::TooNew { .. }));
    let bytes_after = fs::read_bytes(&path).unwrap().unwrap();
    assert_eq!(
        bytes_after,
        original.as_bytes(),
        "refusal must not touch the file"
    );
}

#[test]
fn too_new_failure_names_both_versions_and_points_at_upgrading() {
    let error = Error::TooNew {
        path: Utf8PathBuf::from("/hall/ivar.json"),
        found: 7,
        highest: 3,
    };
    let failure: Failure = error.into();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "store.version_too_new");
    assert_eq!(
        failure.expected,
        Some("schema version 3 or older".to_owned())
    );
    assert_eq!(failure.actual, Some("schema version 7".to_owned()));
    assert_eq!(failure.fix_actions.len(), 1);
    assert!(!failure.fix_actions[0].safe);
}

// -- inspect is safe even on a too-new file ------------------------------

#[test]
fn inspect_reports_too_new_without_erroring_or_mutating() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join("state.json");
    let original = r#"{"version":99,"name":"from-the-future"}"#;
    fs::write_text(&path, original).unwrap();

    let store = Store::<Widget>::new(path.clone(), vec![], 1, Policy::Local);
    let inspection = store.inspect().unwrap().unwrap();

    assert_eq!(
        inspection,
        Inspection::TooNew {
            detected: 99,
            current: 1
        }
    );
    assert!(!inspection.needs_migration());
    let bytes_after = fs::read_bytes(&path).unwrap().unwrap();
    assert_eq!(bytes_after, original.as_bytes());
}

#[test]
fn inspect_reports_needs_migration_without_mutating() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join("state.json");
    let original = r#"{"name":"legacy"}"#;
    fs::write_text(&path, original).unwrap();

    let store = Store::<Widget>::new(
        path.clone(),
        vec![Migration::new(0, 1, v0_to_v1)],
        1,
        Policy::Local,
    );
    let inspection = store.inspect().unwrap().unwrap();

    assert_eq!(
        inspection,
        Inspection::NeedsMigration {
            detected: 0,
            current: 1
        }
    );
    assert!(inspection.needs_migration());
    let bytes_after = fs::read_bytes(&path).unwrap().unwrap();
    assert_eq!(bytes_after, original.as_bytes(), "inspect must never write");
}

#[test]
fn inspect_reports_current_when_already_at_current() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join("state.json");
    fs::write_text(&path, r#"{"version":1,"name":"already-here"}"#).unwrap();

    let store = Store::<Widget>::new(path, vec![Migration::new(0, 1, v0_to_v1)], 1, Policy::Local);
    assert_eq!(store.inspect().unwrap().unwrap(), Inspection::Current);
}

// -- behaviour 3: the chain is validated at construction -----------------

#[test]
#[should_panic(expected = "must start at version 0")]
fn chain_not_starting_at_zero_panics_at_construction() {
    let _ = Store::<Widget>::new(
        "irrelevant.json",
        vec![Migration::new(1, 2, v0_to_v1)],
        2,
        Policy::Local,
    );
}

#[test]
#[should_panic(expected = "gap or overlap")]
fn chain_with_a_gap_panics_at_construction() {
    let _ = Store::<Widget>::new(
        "irrelevant.json",
        vec![
            Migration::new(0, 1, v0_to_v1),
            Migration::new(2, 3, v1_to_v2_rename_label_to_name),
        ],
        3,
        Policy::Local,
    );
}

#[test]
#[should_panic(expected = "ends at v1")]
fn chain_not_ending_at_current_panics_at_construction() {
    let _ = Store::<Widget>::new(
        "irrelevant.json",
        vec![Migration::new(0, 1, v0_to_v1)],
        2,
        Policy::Local,
    );
}

#[test]
fn empty_chain_at_a_nonzero_current_is_not_a_malformed_chain() {
    // `ivar.json`'s own case: version 1 with no v0 predecessor.
    let (_dir, root) = utf8_temp_dir();
    let path = root.join("ivar.json");
    fs::write_text(&path, r#"{"version":1,"name":"acme"}"#).unwrap();

    let store = Store::<Widget>::new(path, vec![], 1, Policy::Committed);
    assert_eq!(
        store.read().unwrap(),
        Some(Widget {
            version: 1,
            name: "acme".to_owned(),
            color: String::new()
        })
    );
}

// -- a multi-step chain actually runs in order ---------------------------

#[test]
fn a_multi_step_chain_runs_every_applicable_step_in_order() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join("state.json");
    // v0: no version field, has `label` instead of `name`, no `color`.
    fs::write_text(&path, r#"{"label":"original"}"#).unwrap();

    let store = Store::<Widget>::new(
        path,
        vec![
            Migration::new(0, 1, v0_to_v1),
            Migration::new(1, 2, v1_to_v2_rename_label_to_name),
        ],
        2,
        Policy::Local,
    );

    let widget = store.read().unwrap().unwrap();
    assert_eq!(
        widget,
        Widget {
            version: 2,
            name: "original".to_owned(),
            color: "gray".to_owned(),
        }
    );
}

// -- behaviour 1 (continued): a mid-chain failure leaves the file alone --

#[test]
fn a_migration_failing_mid_chain_leaves_the_file_untouched() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join("state.json");
    let original = r#"{"label":"original"}"#;
    fs::write_text(&path, original).unwrap();

    let store = Store::<Widget>::new(
        path.clone(),
        vec![
            Migration::new(0, 1, v0_to_v1),
            Migration::new(1, 2, always_fails),
        ],
        2,
        Policy::Local,
    );

    let error = store.read().unwrap_err();
    match error {
        Error::MigrationFailed { from, to, .. } => {
            assert_eq!(from, 1);
            assert_eq!(to, 2);
        }
        other => panic!("expected Error::MigrationFailed, got {other:?}"),
    }

    let bytes_after = fs::read_bytes(&path).unwrap().unwrap();
    assert_eq!(
        bytes_after,
        original.as_bytes(),
        "a failed migration must not write"
    );
}

// -- behaviour 2: Local persists after migration, Committed does not -----

#[test]
fn local_read_persists_the_migrated_form() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join("state.json");
    fs::write_text(&path, r#"{"label":"original"}"#).unwrap();

    let store = Store::<Widget>::new(
        path.clone(),
        vec![
            Migration::new(0, 1, v0_to_v1),
            Migration::new(1, 2, v1_to_v2_rename_label_to_name),
        ],
        2,
        Policy::Local,
    );
    store.read().unwrap();

    let on_disk: serde_json::Value = json::read(&path).unwrap().unwrap();
    assert_eq!(on_disk.get("version"), Some(&serde_json::json!(2)));
    assert_eq!(on_disk.get("name"), Some(&serde_json::json!("original")));
}

#[test]
fn committed_read_migrates_in_memory_but_never_writes() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join("ivar.json");
    let original = r#"{"label":"original"}"#;
    fs::write_text(&path, original).unwrap();

    let store = Store::<Widget>::new(
        path.clone(),
        vec![
            Migration::new(0, 1, v0_to_v1),
            Migration::new(1, 2, v1_to_v2_rename_label_to_name),
        ],
        2,
        Policy::Committed,
    );

    let widget = store.read().unwrap().unwrap();
    assert_eq!(widget.version, 2);
    assert_eq!(widget.name, "original");

    let bytes_after = fs::read_bytes(&path).unwrap().unwrap();
    assert_eq!(
        bytes_after,
        original.as_bytes(),
        "Committed must never persist a migrated read"
    );
}

// -- Committed.write refuses an older on-disk version --------------------

#[test]
fn committed_write_refuses_when_on_disk_is_older_and_points_at_migrate() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join("ivar.json");
    let original = r#"{"version":1,"name":"original","color":"red"}"#;
    fs::write_text(&path, original).unwrap();

    let store = Store::<Widget>::new(
        path.clone(),
        vec![
            Migration::new(0, 1, v0_to_v1),
            Migration::new(1, 2, v1_to_v2_rename_label_to_name),
        ],
        2,
        Policy::Committed,
    );
    let attempted = Widget {
        version: 2,
        name: "new-value".to_owned(),
        color: "blue".to_owned(),
    };

    let error = store.write(&attempted).unwrap_err();
    match &error {
        Error::CommittedRefusesImplicitUpgrade {
            on_disk, current, ..
        } => {
            assert_eq!(*on_disk, 1);
            assert_eq!(*current, 2);
        }
        other => panic!("expected Error::CommittedRefusesImplicitUpgrade, got {other:?}"),
    }

    let bytes_after = fs::read_bytes(&path).unwrap().unwrap();
    assert_eq!(
        bytes_after,
        original.as_bytes(),
        "the refusal must not touch the file"
    );

    let failure: Failure = error.into();
    assert_eq!(failure.code, "store.committed_refuses_implicit_upgrade");
    assert_eq!(failure.fix_actions.len(), 1);
    assert!(!failure.fix_actions[0].safe);
    assert_eq!(
        failure.fix_actions[0].command,
        Some("ivar migrate".to_owned())
    );
}

#[test]
fn committed_write_succeeds_once_on_disk_is_already_current() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join("ivar.json");
    fs::write_text(&path, r#"{"version":2,"name":"original","color":"red"}"#).unwrap();

    let store = Store::<Widget>::new(
        path.clone(),
        vec![
            Migration::new(0, 1, v0_to_v1),
            Migration::new(1, 2, v1_to_v2_rename_label_to_name),
        ],
        2,
        Policy::Committed,
    );
    let updated = Widget {
        version: 2,
        name: "updated".to_owned(),
        color: "green".to_owned(),
    };

    store.write(&updated).unwrap();

    assert_eq!(store.read().unwrap(), Some(updated));
}

#[test]
fn committed_write_succeeds_on_a_brand_new_file() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join("ivar.json");

    let store = Store::<Widget>::new(path.clone(), vec![], 1, Policy::Committed);
    let value = Widget {
        version: 1,
        name: "acme".to_owned(),
        color: "purple".to_owned(),
    };

    store.write(&value).unwrap();
    assert_eq!(store.read().unwrap(), Some(value));
}

// -- migrate(): the explicit escape hatch --------------------------------

#[test]
fn migrate_advances_a_committed_file_that_a_plain_read_left_untouched() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join("ivar.json");
    fs::write_text(&path, r#"{"label":"original"}"#).unwrap();

    let store = Store::<Widget>::new(
        path.clone(),
        vec![
            Migration::new(0, 1, v0_to_v1),
            Migration::new(1, 2, v1_to_v2_rename_label_to_name),
        ],
        2,
        Policy::Committed,
    );

    // A plain read migrates only in memory.
    store.read().unwrap();
    let bytes_after_read = fs::read_bytes(&path).unwrap().unwrap();
    assert_eq!(bytes_after_read, br#"{"label":"original"}"#);

    // The explicit migrate advances the file itself.
    let migrated = store.migrate().unwrap().unwrap();
    assert_eq!(migrated.version, 2);
    assert_eq!(migrated.name, "original");

    let on_disk: serde_json::Value = json::read(&path).unwrap().unwrap();
    assert_eq!(on_disk.get("version"), Some(&serde_json::json!(2)));

    // Writing now succeeds, because the on-disk version is current.
    store.write(&migrated).unwrap();
}

#[test]
fn migrate_does_not_rewrite_a_file_already_at_current() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join("ivar.json");
    fs::write_text(&path, r#"{"version":1,"name":"already-current"}"#).unwrap();

    let store = Store::<Widget>::new(path.clone(), vec![], 1, Policy::Committed);
    store.migrate().unwrap();

    // Byte-identical: nothing needed to change, so nothing was rewritten
    // through the canonical writer (which would reorder/reformat).
    let bytes_after = fs::read_bytes(&path).unwrap().unwrap();
    assert_eq!(bytes_after, br#"{"version":1,"name":"already-current"}"#);
}

// -- round-tripping through infra::json ----------------------------------

#[test]
fn write_round_trips_through_the_canonical_writer() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join("ivar.json");
    let store = Store::<Widget>::new(path.clone(), vec![], 1, Policy::Committed);
    let value = Widget {
        version: 1,
        name: "acme".to_owned(),
        color: "teal".to_owned(),
    };

    store.write(&value).unwrap();

    let text = fs::read_text(&path).unwrap().unwrap();
    let expected = json::to_canonical_string(&serde_json::json!({
        "color": "teal",
        "name": "acme",
        "version": 1,
    }))
    .unwrap();
    assert_eq!(
        text, expected,
        "write must go through infra::json's canonical format"
    );

    assert_eq!(store.read().unwrap(), Some(value));
}

// -- InvalidName-style coverage: every variant has its own code ----------

#[test]
fn migration_failed_failure_names_the_step_and_the_reason() {
    let error = Error::MigrationFailed {
        path: Utf8PathBuf::from("/hall/.ivar/state.json"),
        from: 1,
        to: 2,
        reason: "missing field `name`".to_owned(),
    };
    let failure: Failure = error.into();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "store.migration_failed");
    assert_eq!(failure.actual, Some("missing field `name`".to_owned()));
}

#[test]
fn deserialize_failure_is_a_blocked_schema_mismatch() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join("state.json");
    fs::write_text(&path, r#"{"version":1}"#).unwrap();

    let store = Store::<Widget>::new(path, vec![], 1, Policy::Local);
    let error = store.read().unwrap_err();
    let failure: Failure = error.into();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "store.schema_mismatch");
}
