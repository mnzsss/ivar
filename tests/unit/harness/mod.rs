//! Unit tests for `crate::harness::mod`.
//!
//! Physically located here but compiled inside the library crate via `#[path]`
//! so `use super::*` reaches private parent items.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

/// The Feature Session View Dir `tick` materialises and spawns in — what
/// every `execute_command` call now has to carry.
fn view_dir() -> &'static Utf8Path {
    Utf8Path::new("/hall/.ivar/features/plan-issues/sessions/0d2a1b3c")
}

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
    assert!(caps.supports_questions);
    let opencode = Harness::OpenCode.capabilities();
    assert!(!opencode.supports_review);
}

/// `opencode run` bakes `{permission: "question", action: "deny"}` into every
/// session it creates, so the `question` tool never executes and the JSON
/// stream has no question envelope. The flag says so rather than leaving
/// `tick` to wait for a block that cannot arrive.
#[test]
fn opencode_cannot_ask_questions_headlessly() {
    assert!(!Harness::OpenCode.capabilities().supports_questions);
}

// -- execute_command --------------------------------------------------

#[test]
fn claude_code_execute_command_is_headless_and_streamed() {
    let command = Harness::ClaudeCode.execute_command("do the thing", view_dir(), None, None);

    assert_eq!(command.program(), "claude");
    assert_eq!(
        command.arguments(),
        [
            "-p",
            "do the thing",
            "--output-format",
            "stream-json",
            "--verbose",
            "--permission-mode",
            "bypassPermissions"
        ]
    );
}

/// A headless `claude -p` has nobody to answer a permission prompt, so every
/// question it would have asked is denied where it stands — and an executor
/// launched without a permission mode cannot write the files its own write
/// contract grants it, nor read the plan to find out what it was asked to do.
/// The arbiter of writes is ivar's execution guard, which runs as a
/// `PreToolUse` hook and is unaffected by this mode; the harness prompt is a
/// second gate with nobody behind it.
#[test]
fn claude_code_execute_command_leaves_no_permission_prompt_to_answer() {
    let command = Harness::ClaudeCode.execute_command("do the thing", view_dir(), None, None);
    let args = command.arguments();

    let mode = args
        .iter()
        .position(|arg| arg == "--permission-mode")
        .and_then(|index| args.get(index + 1))
        .expect("the headless invocation must set a permission mode");

    assert_eq!(mode, "bypassPermissions");
}

/// `-p` on the `opencode` CLI is `--password`, not the prompt, and without
/// `--format json` the output is prose for a human that `parse_opencode_line`
/// cannot read at all.
#[test]
fn opencode_execute_command_is_run_format_json() {
    let command = Harness::OpenCode.execute_command("do the thing", view_dir(), None, None);

    assert_eq!(command.program(), "opencode");
    assert_eq!(
        command.arguments(),
        ["run", "--dir", view_dir().as_str(), "--format", "json"]
    );
}

/// `opencode run` re-renders an argv message — wrapping anything containing a
/// space in literal quotes and escaping the quotes inside it — and reads stdin
/// verbatim. So the prompt goes on stdin and appears nowhere in the argv.
#[test]
fn the_opencode_prompt_travels_on_stdin_not_argv() {
    let command = Harness::OpenCode.execute_command("do the thing", view_dir(), None, None);

    assert_eq!(command.stdin_text(), Some("do the thing"));
    assert!(
        !command.arguments().iter().any(|a| a.contains("do the")),
        "prompt leaked into argv: {:?}",
        command.arguments()
    );
    assert!(!command.display().contains("-p "), "{}", command.display());
}

/// Nothing about a prompt's own text can turn it into a flag once it travels
/// on stdin — not a leading dash, not a newline, not a quote.
#[test]
fn an_opencode_prompt_that_looks_like_a_flag_is_still_just_the_prompt() {
    let command = Harness::OpenCode.execute_command(
        "--not-a-flag\nwith \"quotes\"",
        view_dir(),
        Some("opus"),
        Some("build"),
    );

    assert_eq!(
        command.arguments(),
        [
            "run",
            "--dir",
            view_dir().as_str(),
            "--format",
            "json",
            "--model",
            "opus",
            "--agent",
            "build",
        ]
    );
    assert_eq!(command.stdin_text(), Some("--not-a-flag\nwith \"quotes\""));
}

/// Claude Code takes its prompt as `-p`'s value and is fed nothing on stdin —
/// the two harnesses use different channels, and only one of them is written
/// to.
#[test]
fn claude_code_is_never_fed_on_stdin() {
    assert_eq!(
        Harness::ClaudeCode
            .execute_command("do the thing", view_dir(), None, None)
            .stdin_text(),
        None
    );
}

#[test]
fn model_is_appended_only_when_supplied() {
    let without = Harness::ClaudeCode.execute_command("p", view_dir(), None, None);
    assert!(!without.display().contains("--model"));

    let with = Harness::ClaudeCode.execute_command("p", view_dir(), Some("opus"), None);
    assert!(with.display().contains("--model opus"));
}

#[test]
fn agent_is_appended_only_when_supplied() {
    let without = Harness::OpenCode.execute_command("p", view_dir(), None, None);
    assert!(!without.display().contains("--agent"));

    let with = Harness::OpenCode.execute_command("p", view_dir(), None, Some("reviewer"));
    assert!(with.display().contains("--agent reviewer"));
}

/// The bug this builder undoes: `model` and `agent` are two distinct
/// flags, never collapsed into one. Passing both must produce both flags,
/// each carrying its own value — not one flag carrying either value.
#[test]
fn model_and_agent_are_distinct_flags_not_conflated() {
    let display = Harness::ClaudeCode
        .execute_command("p", view_dir(), Some("opus"), Some("reviewer"))
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
fn claude_code_execute_command_has_no_directory_flag() {
    // The claude CLI has neither --cwd nor --dir; its working directory is
    // set on the spawn (`proc::Command::cwd`), never on the argv. Giving it
    // one would put an unknown argument in front of a CLI that never reads
    // it.
    let display = Harness::ClaudeCode
        .execute_command("p", view_dir(), None, None)
        .display();

    assert!(!display.contains("--cwd"), "was: {display}");
    assert!(!display.contains("--dir"), "was: {display}");
    assert!(
        !display.contains(view_dir().as_str()),
        "the view dir reached claude's argv: {display}"
    );
}

/// The regression this flag exists for. `opencode run` reads its project
/// directory from `$PWD`, not from `getcwd`, so an executor spawned in a
/// session view dir by a process whose `PWD` still named the hall opened its
/// session *at the hall*: the hall's config, the hall's plugins — no
/// execution guard — and tool paths under the default-branch worktree
/// instead of the promoted one. `--dir` states the directory in the one
/// channel a child cannot inherit stale.
///
/// Asserted for two different paths, so the flag carries what the caller
/// passed rather than anything baked in.
#[test]
fn an_opencode_executor_names_its_view_dir_on_the_argv() {
    for dir in [
        view_dir(),
        Utf8Path::new("/hall/.ivar/features/other/sessions/9f"),
    ] {
        let command = Harness::OpenCode.execute_command("p", dir, None, None);
        let args = command.arguments();

        let named = args
            .iter()
            .position(|arg| arg == "--dir")
            .and_then(|index| args.get(index + 1))
            .expect("the OpenCode executor must name its project directory");

        assert_eq!(named, dir.as_str());
    }
}
