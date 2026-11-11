#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Unit tests for the `memory` key of `ivar.json`.

use crate::domain::name::HallName;
use crate::domain::provider::Provider;
use crate::store::manifest::{Manifest, Providers, generate};

const BASE: &str = r#"{"version":5,"name":"acme","providers":{"available":["claude-code"],"default":"claude-code"},"repos":[],"integration":{"via":"local","strategy":"squash"}"#;

fn empty_manifest() -> Manifest {
    Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![],
    )
    .unwrap()
}

#[test]
fn memory_capture_defaults_to_on_and_is_absent_from_the_file() {
    let manifest = empty_manifest();
    assert!(manifest.memory_capture());
    assert!(
        !serde_json::to_string(&manifest)
            .unwrap()
            .contains("\"memory\"")
    );
}

#[test]
fn turning_capture_off_round_trips_and_survives_other_builders() {
    let off = empty_manifest().with_memory_capture(false).unwrap();
    assert!(!off.memory_capture());
    let json = serde_json::to_string(&off).unwrap();
    assert!(json.contains(r#""memory":{"capture":false}"#), "{json}");

    let after_other_builder = off.with_graph_path_added("brand").unwrap();
    assert!(!after_other_builder.memory_capture());

    let back = off.with_memory_capture(true).unwrap();
    assert!(!serde_json::to_string(&back).unwrap().contains("\"memory\""));
}

#[test]
fn an_unknown_key_under_memory_is_refused() {
    let json = format!(r#"{BASE},"memory":{{"capture":true}}}}"#);
    assert!(serde_json::from_str::<Manifest>(&json).is_ok());
    let json = format!(r#"{BASE},"memory":{{"capture":true,"typo":1}}}}"#);
    assert!(serde_json::from_str::<Manifest>(&json).is_err());
}

#[test]
fn the_schema_declares_memory_capture_as_an_optional_boolean() {
    let schema = generate();
    assert_eq!(
        schema.pointer("/properties/memory/anyOf/0/$ref"),
        Some(&serde_json::json!("#/$defs/MemorySettings"))
    );
    assert_eq!(
        schema.pointer("/$defs/MemorySettings/properties/capture/type"),
        Some(&serde_json::json!("boolean"))
    );
    let required = schema
        .pointer("/required")
        .and_then(|v| v.as_array())
        .unwrap();
    assert!(!required.iter().any(|v| v == "memory"));
    assert_eq!(
        schema.pointer("/$defs/MemorySettings/properties/handoff_ttl_days/minimum"),
        Some(&serde_json::json!(1))
    );
    let memory_required = schema
        .pointer("/$defs/MemorySettings/required")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(memory_required.is_empty(), "{memory_required:?}");
}

#[test]
fn handoff_ttl_defaults_to_seven_days_and_is_absent_from_the_file() {
    let manifest = empty_manifest();
    assert_eq!(manifest.memory_handoff_ttl_days(), 7);
    assert!(
        !serde_json::to_string(&manifest)
            .unwrap()
            .contains("handoff_ttl_days")
    );
}

#[test]
fn handoff_ttl_alone_parses_and_keeps_capture_on() {
    let json = format!(r#"{BASE},"memory":{{"handoff_ttl_days":3}}}}"#);
    let manifest: Manifest = serde_json::from_str(&json).unwrap();
    assert_eq!(manifest.memory_handoff_ttl_days(), 3);
    assert!(manifest.memory_capture());
}

#[test]
fn a_zero_handoff_ttl_is_refused() {
    let json = format!(r#"{BASE},"memory":{{"handoff_ttl_days":0}}}}"#);
    assert!(serde_json::from_str::<Manifest>(&json).is_err());
}

#[test]
fn toggling_capture_keeps_the_handoff_ttl() {
    let json = format!(r#"{BASE},"memory":{{"handoff_ttl_days":3}}}}"#);
    let manifest: Manifest = serde_json::from_str(&json).unwrap();

    let off = manifest.with_memory_capture(false).unwrap();
    assert_eq!(off.memory_handoff_ttl_days(), 3);
    let back = off.with_memory_capture(true).unwrap();
    assert_eq!(back.memory_handoff_ttl_days(), 3);
    let text = serde_json::to_string(&back).unwrap();
    assert!(
        text.contains(r#""memory":{"handoff_ttl_days":3}"#),
        "{text}"
    );
}

// -- auto_commit and auto_push -------------------------------------------------

#[test]
fn auto_commit_and_auto_push_default_to_on_and_are_absent_from_the_file() {
    let manifest = empty_manifest();

    assert!(manifest.auto_commit());
    assert!(manifest.auto_push());
    let json = serde_json::to_string(&manifest).unwrap();
    assert!(
        !json.contains("auto_commit") && !json.contains("auto_push"),
        "{json}"
    );
}

#[test]
fn either_switch_turned_off_parses_round_trips_and_survives_other_builders() {
    let json = format!(r#"{BASE},"auto_push":false}}"#);
    let manifest: Manifest = serde_json::from_str(&json).unwrap();
    assert!(manifest.auto_commit());
    assert!(!manifest.auto_push());

    let json = format!(r#"{BASE},"auto_commit":false}}"#);
    let manifest: Manifest = serde_json::from_str(&json).unwrap();
    assert!(!manifest.auto_commit());
    assert!(manifest.auto_push());

    let rebuilt = manifest.with_graph_path_added("brand").unwrap();
    assert!(!rebuilt.auto_commit());
    let text = serde_json::to_string(&rebuilt).unwrap();
    assert!(text.contains(r#""auto_commit":false"#), "{text}");
    assert!(!text.contains("auto_push"), "{text}");
}

#[test]
fn auto_resolve_defaults_to_on_is_absent_from_the_file_and_survives_other_builders() {
    let manifest = empty_manifest();
    assert!(manifest.auto_resolve());
    assert!(
        !serde_json::to_string(&manifest)
            .unwrap()
            .contains("auto_resolve")
    );

    let json = format!(r#"{BASE},"auto_resolve":false}}"#);
    let manifest: Manifest = serde_json::from_str(&json).unwrap();
    assert!(!manifest.auto_resolve());
    assert!(manifest.auto_commit() && manifest.auto_push());

    let rebuilt = manifest.with_graph_path_added("brand").unwrap();
    assert!(!rebuilt.auto_resolve());
    let text = serde_json::to_string(&rebuilt).unwrap();
    assert!(text.contains(r#""auto_resolve":false"#), "{text}");

    let json = format!(r#"{BASE},"auto_resolve":true}}"#);
    let manifest: Manifest = serde_json::from_str(&json).unwrap();
    assert!(
        !serde_json::to_string(&manifest)
            .unwrap()
            .contains("auto_resolve")
    );
}

#[test]
fn a_switch_written_as_true_is_left_out_when_the_file_is_written_back() {
    let json = format!(r#"{BASE},"auto_commit":true,"auto_push":true}}"#);
    let manifest: Manifest = serde_json::from_str(&json).unwrap();

    let text = serde_json::to_string(&manifest).unwrap();

    assert!(!text.contains("auto_"), "{text}");
}

#[test]
fn a_switch_that_is_not_a_boolean_is_refused() {
    let json = format!(r#"{BASE},"auto_push":"no"}}"#);
    assert!(serde_json::from_str::<Manifest>(&json).is_err());
}

#[test]
fn the_schema_declares_both_switches_as_optional_booleans() {
    let schema = generate();
    for key in ["auto_commit", "auto_push"] {
        assert_eq!(
            schema.pointer(&format!("/properties/{key}/type")),
            Some(&serde_json::json!("boolean")),
            "{key}"
        );
        let required = schema
            .pointer("/required")
            .and_then(|v| v.as_array())
            .unwrap();
        assert!(!required.iter().any(|v| v == key), "{key} must be optional");
    }
}
