//! Unit tests for `crate::harness::mod`.
//!
//! Physically located here but compiled inside the library crate via `#[path]`
//! so `use super::*` reaches private parent items.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

#[test]
fn each_provider_maps_to_its_harness() {
    assert_eq!(
        Harness::for_provider(Provider::ClaudeCode).unwrap(),
        Harness::ClaudeCode
    );
    assert_eq!(
        Harness::for_provider(Provider::OpenCode).unwrap(),
        Harness::OpenCode
    );
}
