#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use crate::test_support::seeded_hall;

#[test]
fn memory_status_is_rendered_through_the_memory_dispatch() {
    let (_guard, root) = seeded_hall();
    let mut out = Vec::new();

    let code = dispatch_memory(
        MemoryCommand::Status,
        &Ctx::new(root),
        true,
        &mut out,
        &mut Vec::new(),
    );

    assert_eq!(code, ExitCode::SUCCESS);
    let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert!(value.get("schema_version").is_some(), "{value}");
}

#[test]
fn capture_reaching_the_dispatch_is_a_silent_success() {
    let (_guard, root) = seeded_hall();
    let mut out = Vec::new();

    let code = dispatch_memory(
        MemoryCommand::Capture { provider: None },
        &Ctx::new(root),
        false,
        &mut out,
        &mut Vec::new(),
    );

    assert_eq!(code, ExitCode::SUCCESS);
    assert!(out.is_empty());
}
