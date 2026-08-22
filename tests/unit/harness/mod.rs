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

#[test]
fn claude_code_resumes_with_continue() {
    let command = Harness::ClaudeCode.start_command(true);
    let display = command.display();
    assert!(display.starts_with("claude"), "was: {display}");
    assert!(display.contains("--continue"), "was: {display}");
}

#[test]
fn a_fresh_start_has_no_extra_flags() {
    let display = Harness::ClaudeCode.start_command(false).display();
    assert_eq!(display, "claude");
}

#[test]
fn opencode_builds_its_own_command() {
    let display = Harness::OpenCode.start_command(false).display();
    assert!(display.starts_with("opencode"), "was: {display}");
}

#[test]
fn resume_is_supported_for_both_harnesses_today() {
    assert!(check_resume_supported(Harness::ClaudeCode).is_ok());
    assert!(check_resume_supported(Harness::OpenCode).is_ok());
}

#[test]
fn capabilities_are_explicit_not_inferred() {
    let claude = Harness::ClaudeCode.capabilities();
    assert!(claude.supports_resume);
    assert!(claude.supports_review);
    assert!(claude.interactive);

    assert!(!Harness::OpenCode.capabilities().supports_review);
}
