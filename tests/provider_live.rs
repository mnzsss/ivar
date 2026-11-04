//! Live provider evaluation: real `claude`, `omp` and `opencode` sessions receive the
//! linked repositories' instructions through `ivar guard`, launched exactly as `ivar
//! session start` launches them, against a scratch hall built with this build's `ivar`.
//!
//! Opt-in, never in CI: every test is `#[ignore]`d and also returns early unless
//! `IVAR_LIVE_PROVIDERS` lists its provider (`claude`, `omp`, `opencode`). Providers run
//! with the developer's real HOME (credentials); hall setup runs `ivar` with a scratch HOME.
//!
//! ```bash
//! IVAR_LIVE_PROVIDERS=claude,omp,opencode \
//!   cargo test --profile e2e --test provider_live -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! Gates, N = REPS per provider and scenario, every rep must pass:
//! - promotion: a session opened on main answers main's command; `ivar feature promote`
//!   and a feature edit mid-session; turn 2 answers the feature's command;
//! - Bash `cat`: a nested instruction file reached through a shell command;
//! - deep: a >24,000-char `CLAUDE.md` whose fact sits near the end, answered without the
//!   agent opening it (Claude through the slice entries, omp whole);
//! - Claude: a denied write is still blocked with the five slice entries present;
//! - opencode: a read in a linked repo gets the instructions appended to the tool output,
//!   and a write outside the writable set is blocked. Asserted from the tool output, never
//!   from the answer: the default model may answer poorly on long context.
//!
//! t3: `IVAR_LIVE_T3=1 cargo test --profile e2e --test provider_live t3_session_registers -- --ignored`
//! with the T3 Code desktop app open; `IVAR_LIVE_T3_CLOSED=1 … t3_session_start_with_the_desktop_app_closed`
//! with it closed. Manual guard check, once per release: in the T3 project that `ivar session
//! start --provider t3` opened, start a thread, ask it to write `.ivar/probe.txt`, and expect the
//! `PreToolUse` denial naming the writable set.

#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::print_stderr
)]

#[path = "support/integration.rs"]
mod common;

#[allow(dead_code)]
#[path = "personas/support.rs"]
mod support;

#[path = "provider_live/fixture.rs"]
mod fixture;
#[path = "provider_live/live.rs"]
mod live;
#[path = "provider_live/scenarios.rs"]
mod scenarios;
#[path = "provider_live/session.rs"]
mod session;
#[path = "provider_live/t3.rs"]
mod t3;

use camino::Utf8PathBuf;
use serde_json::Value;

use common::hall_root;
use support::run_ok;

use fixture::{live_hall, unguard};
use live::{Live, REPS, WRITE_PROMPT};
use scenarios::{bash_cat, deep_file, promotion_mid_session};
use session::{answer, one_shot, reminders};
use t3::{real_ivar, t3_command, t3_hall, wait_for_t3_project};

#[test]
#[ignore = "needs provider credentials: claude, omp"]
fn claude_sees_main_then_feature_instructions_across_a_promotion() {
    promotion_mid_session(Live::Claude);
}

#[test]
#[ignore = "needs provider credentials: claude, omp"]
fn omp_sees_main_then_feature_instructions_across_a_promotion() {
    promotion_mid_session(Live::Omp);
}

#[test]
#[ignore = "needs provider credentials: claude, omp"]
fn claude_bash_cat_delivers_the_nested_instructions() {
    bash_cat(Live::Claude);
}

#[test]
#[ignore = "needs provider credentials: claude, omp"]
fn omp_bash_cat_delivers_the_nested_instructions() {
    bash_cat(Live::Omp);
}

#[test]
#[ignore = "needs provider credentials: claude, omp"]
fn claude_answers_from_a_24k_file_through_the_slices() {
    deep_file(Live::Claude);
}

#[test]
#[ignore = "needs provider credentials: claude, omp"]
fn omp_answers_from_a_24k_file_delivered_whole() {
    deep_file(Live::Omp);
}

#[test]
#[ignore = "needs provider credentials: claude, omp"]
fn claude_denied_write_is_blocked_with_the_slice_entries_present() {
    let live = Live::Claude;
    if !live.enabled() {
        return;
    }
    let (_guard, root) = live_hall();
    let settings: Value =
        serde_json::from_str(&std::fs::read_to_string(root.join(".claude/settings.json")).unwrap())
            .unwrap();
    let guard_entries = settings["hooks"]["PreToolUse"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|entry| entry["hooks"].as_array().cloned().unwrap_or_default())
        .filter(|hook| {
            hook["command"]
                .as_str()
                .is_some_and(|command| command.starts_with("ivar guard --provider claude-code"))
        })
        .count();
    assert_eq!(
        guard_entries, 6,
        "the guard entry plus five slice entries: {settings}"
    );

    for rep in 1..=REPS {
        let feature = format!("deny-claude-{rep}");
        let (events, _delivered) = one_shot(&root, live, &feature, WRITE_PROMPT, "Read,Write");
        assert!(
            !root.join(".ivar/repos/web/main/src/new.ts").exists(),
            "rep{rep}: the denied write went through"
        );
        let denial_seen = events
            .iter()
            .filter(|event| event["type"] == "user")
            .any(|event| event.to_string().contains("is outside the writable set"));
        assert!(
            denial_seen,
            "rep{rep}: the guard's deny never reached the agent: {events:?}"
        );
    }
    unguard(&root);
}

/// The read goes through opencode's `tool.execute.before` (guard on stdout, kept by
/// `callID`) and `tool.execute.after` (appended to the tool output). The plugin's payload
/// `cwd` is empty, so the session only resolves through the `process.cwd()` fallback.
#[test]
#[ignore = "needs provider credentials: opencode"]
fn opencode_read_gets_the_instructions_appended_to_the_tool_output() {
    let live = Live::OpenCode;
    if !live.enabled() {
        return;
    }
    let (_guard, root) = live_hall();
    let mut passed = 0;
    for rep in 1..=REPS {
        let feature = format!("read-opencode-{rep}");
        let prompt = "Use the read tool to read web/src/index.ts, then say what it exports.";
        let (events, delivered) = one_shot(&root, live, &feature, prompt, live.read_tools());
        let reminders = reminders(&events);
        // opencode is native to AGENTS.md (R-FILE-CHOICE); web ships both.
        let ok = delivered
            && reminders.iter().any(|output| {
                output.contains("/web/AGENTS.md") && output.contains("pnpm serve:alpha")
            });
        eprintln!(
            "{live:?} read rep{rep} {} delivered={delivered} reminders={} answer={:?}",
            if ok { "PASS" } else { "FAIL" },
            reminders.len(),
            answer(live, &events),
        );
        passed += usize::from(ok);
    }
    unguard(&root);
    assert_eq!(passed, REPS, "{live:?}: read delivery gate {passed}/{REPS}");
}

/// Before Task 06 the plugin never loaded, so nothing blocked this write but Landlock.
/// The plugin's block is told apart by its error: `execSync` throws
/// `Command failed: ivar guard --provider opencode`.
#[test]
#[ignore = "needs provider credentials: opencode"]
fn opencode_write_outside_the_writable_set_is_blocked_by_the_plugin() {
    let live = Live::OpenCode;
    if !live.enabled() {
        return;
    }
    let (_guard, root) = live_hall();
    for rep in 1..=REPS {
        let feature = format!("deny-opencode-{rep}");
        let prompt = "Use the write tool to create web/src/new.ts containing \
            `export const x = 1;`. If the tool refuses, quote its error message.";
        let (events, _delivered) = one_shot(&root, live, &feature, prompt, live.read_tools());
        assert!(
            !root.join(".ivar/repos/web/main/src/new.ts").exists(),
            "rep{rep}: the denied write went through"
        );
        let blocked = events
            .iter()
            .filter(|event| event["type"] == "tool_use" && event["part"]["tool"] == "write")
            .any(|event| {
                event["part"]["state"]["status"] == "error"
                    && event.to_string().contains("ivar guard --provider opencode")
            });
        assert!(
            blocked,
            "rep{rep}: the plugin never blocked the write: {events:?}"
        );
    }
    unguard(&root);
}

#[test]
#[ignore = "needs a running T3 Code desktop app"]
fn t3_session_registers_and_unregisters_its_view_dir() {
    if std::env::var("IVAR_LIVE_T3").is_err() {
        return;
    }
    let (_guard, root, id, view_dir) = t3_hall();
    wait_for_t3_project(&view_dir);

    let stopped = real_ivar(&root, &["session", "stop", &id]);
    assert_eq!(stopped["ok"], true, "{stopped}");
    assert!(
        !stopped.to_string().contains("session.t3_project_left"),
        "an empty project is removed without --force: {stopped}"
    );

    let gone = t3_command(&["project", "remove", view_dir.as_str()]);
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&gone.stdout),
        String::from_utf8_lossy(&gone.stderr)
    );
    assert!(
        !gone.status.success() && output.contains("ProjectNotFoundError"),
        "the project must be gone after stop: {output}"
    );
    unguard(&root);
}

#[test]
#[ignore = "needs the T3 Code desktop app to be closed"]
fn t3_session_start_with_the_desktop_app_closed_keeps_the_session() {
    if std::env::var("IVAR_LIVE_T3_CLOSED").is_err() {
        return;
    }
    let (_guard, root) = hall_root();
    run_ok(&root, &["init"]);
    run_ok(&root, &["provider", "add", "t3"]);
    run_ok(&root, &["feature", "create", "t3-live"]);

    let started = real_ivar(&root, &["session", "start", "t3-live", "--provider", "t3"]);

    assert_eq!(started["ok"], true, "{started}");
    assert!(
        started.to_string().contains("session.t3_open_failed"),
        "the closed-app failure is a warning: {started}"
    );
    assert!(Utf8PathBuf::from(started["view_dir"].as_str().unwrap()).exists());
    unguard(&root);
}
