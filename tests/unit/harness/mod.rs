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
    // The contract of seam 5: the flags say what the harness can do,
    // and nothing in this module ever guesses from the binary name.
    let caps = Harness::ClaudeCode.capabilities();
    assert!(caps.supports_resume);
    assert!(caps.interactive);
    let opencode = Harness::OpenCode.capabilities();
    assert!(!opencode.supports_review);
}

// -- execute_command --------------------------------------------------

#[test]
fn claude_code_execute_command_is_headless_and_streamed() {
    let command = Harness::ClaudeCode.execute_command("do the thing", None, None);

    assert_eq!(command.program(), "claude");
    assert_eq!(
        command.arguments(),
        [
            "-p",
            "do the thing",
            "--output-format",
            "stream-json",
            "--verbose"
        ]
    );
}

#[test]
fn opencode_execute_command_is_run_dash_p() {
    let command = Harness::OpenCode.execute_command("do the thing", None, None);

    assert_eq!(command.program(), "opencode");
    assert_eq!(command.arguments(), ["run", "-p", "do the thing"]);
}

#[test]
fn model_is_appended_only_when_supplied() {
    let without = Harness::ClaudeCode.execute_command("p", None, None);
    assert!(!without.display().contains("--model"));

    let with = Harness::ClaudeCode.execute_command("p", Some("opus"), None);
    assert!(with.display().contains("--model opus"));
}

#[test]
fn agent_is_appended_only_when_supplied() {
    let without = Harness::OpenCode.execute_command("p", None, None);
    assert!(!without.display().contains("--agent"));

    let with = Harness::OpenCode.execute_command("p", None, Some("reviewer"));
    assert!(with.display().contains("--agent reviewer"));
}

/// The bug this builder undoes: `model` and `agent` are two distinct
/// flags, never collapsed into one. Passing both must produce both flags,
/// each carrying its own value — not one flag carrying either value.
#[test]
fn model_and_agent_are_distinct_flags_not_conflated() {
    let display = Harness::ClaudeCode
        .execute_command("p", Some("opus"), Some("reviewer"))
        .display();

    assert!(display.contains("--model opus"), "was: {display}");
    assert!(display.contains("--agent reviewer"), "was: {display}");
    assert!(
        !display.contains("--model reviewer"),
        "agent leaked into --model: {display}"
    );
    assert!(
        !display.contains("--agent opus"),
        "model leaked into --agent: {display}"
    );
}

#[test]
fn claude_code_execute_command_has_no_cwd_flag() {
    // There is no --cwd flag on the claude CLI; the working directory is
    // set on the spawn (`proc::Command::cwd`), never on the argv.
    let display = Harness::ClaudeCode
        .execute_command("p", None, None)
        .display();

    assert!(!display.contains("--cwd"), "was: {display}");
}
