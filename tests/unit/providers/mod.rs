// tests/unit/providers/mod.rs
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::domain::provider::Provider;
use crate::providers::{self, Capabilities};

#[test]
fn launch_contract_returns_correct_binary_and_capabilities_for_all_providers() {
    let claude = providers::launch_contract(Provider::ClaudeCode);
    assert_eq!(claude.binary, "claude");
    assert_eq!(
        claude.capabilities,
        Capabilities {
            supports_resume: true,
            supports_review: true,
            interactive: true,
        }
    );

    let opencode = providers::launch_contract(Provider::OpenCode);
    assert_eq!(opencode.binary, "opencode");
    assert_eq!(
        opencode.capabilities,
        Capabilities {
            supports_resume: true,
            supports_review: false,
            interactive: true,
        }
    );

    let omp = providers::launch_contract(Provider::Omp);
    assert_eq!(omp.binary, "omp");
    assert_eq!(
        omp.capabilities,
        Capabilities {
            supports_resume: true,
            supports_review: false,
            interactive: true,
        }
    );
}

#[test]
fn claude_code_fresh_start_and_resume_commands() {
    let fresh = providers::start_command(Provider::ClaudeCode, false).unwrap();
    assert_eq!(fresh.display(), "claude");

    let resumed = providers::start_command(Provider::ClaudeCode, true).unwrap();
    let display = resumed.display();
    assert!(display.starts_with("claude"), "was: {display}");
    assert!(display.contains("--continue"), "was: {display}");
}

#[test]
fn opencode_fresh_start_and_resume_commands() {
    let fresh = providers::start_command(Provider::OpenCode, false).unwrap();
    assert_eq!(fresh.display(), "opencode");

    let resumed = providers::start_command(Provider::OpenCode, true).unwrap();
    let display = resumed.display();
    assert!(display.starts_with("opencode"), "was: {display}");
    assert!(display.contains("--continue"), "was: {display}");
}

#[test]
fn omp_fresh_start_and_resume_commands() {
    let fresh = providers::start_command(Provider::Omp, false).unwrap();
    assert_eq!(fresh.display(), "omp");

    let resumed = providers::start_command(Provider::Omp, true).unwrap();
    let display = resumed.display();
    assert!(display.starts_with("omp"), "was: {display}");
    assert!(display.contains("--continue"), "was: {display}");
}
