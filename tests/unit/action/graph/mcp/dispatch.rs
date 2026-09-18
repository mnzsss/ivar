#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::json;

use super::*;

#[test]
fn bounded_arg_clamps_above_the_limit() {
    let args = json!({"max_depth": 999_999_u64});
    assert_eq!(
        bounded_arg(&args, "max_depth", 5, MAX_DEPTH_LIMIT),
        MAX_DEPTH_LIMIT
    );
}

#[test]
fn bounded_arg_keeps_a_value_under_the_limit() {
    let args = json!({"max_hops": 3_u64});
    assert_eq!(bounded_arg(&args, "max_hops", 6, MAX_HOPS_LIMIT), 3);
}

#[test]
fn bounded_arg_falls_back_to_default_when_absent() {
    let args = json!({});
    assert_eq!(bounded_arg(&args, "max_depth", 5, MAX_DEPTH_LIMIT), 5);
}
