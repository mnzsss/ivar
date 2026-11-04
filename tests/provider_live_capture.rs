//! Live capture scenarios: a real `claude`, `omp` or `opencode` one-shot run inside an
//! `ivar` session must leave the observations its capability matrix declares.
//!
//! Opt-in, never in CI: every test is `#[ignore]`d and also returns early unless
//! `IVAR_LIVE_PROVIDERS` lists its provider. Providers run with the developer's real HOME.
//!
//! ```bash
//! IVAR_LIVE_PROVIDERS=claude,omp,opencode \
//!   cargo test --profile e2e --test provider_live_capture -- --ignored --test-threads=1 --nocapture
//! IVAR_LIVE_T3=1 cargo test --profile e2e --test provider_live_capture t3_session_end -- --ignored --nocapture
//! ```

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

use std::collections::BTreeSet;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use camino::{Utf8Path, Utf8PathBuf};
use serde_json::Value;

use common::{hall_root, ivar};
use support::{home, isolated_ivar, run_ok};

const PROMPT: &str = "Read the file README.md and answer with its first word.";
const T3_THREAD_WAIT: Duration = Duration::from_secs(180);

#[derive(Clone, Copy, Debug)]
enum Live {
    Claude,
    Omp,
    OpenCode,
}

impl Live {
    fn key(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Omp => "omp",
            Self::OpenCode => "opencode",
        }
    }

    fn provider(self) -> &'static str {
        match self {
            Self::Claude => "claude-code",
            Self::Omp => "omp",
            Self::OpenCode => "opencode",
        }
    }

    fn binary(self) -> &'static str {
        self.key()
    }

    fn enabled(self) -> bool {
        let on = std::env::var("IVAR_LIVE_PROVIDERS")
            .is_ok_and(|list| list.split(',').any(|name| name.trim() == self.key()));
        if !on {
            eprintln!(
                "skipped: IVAR_LIVE_PROVIDERS does not list `{}`",
                self.key()
            );
        }
        on
    }

    fn one_shot_args(self, view: &Utf8Path) -> Vec<String> {
        let args: Vec<&str> = match self {
            Self::Claude => vec![
                "-p",
                PROMPT,
                "--output-format",
                "stream-json",
                "--verbose",
                "--model",
                "haiku",
                "--strict-mcp-config",
                "--tools",
                "Read",
                "--allowedTools",
                "Read",
            ],
            Self::Omp => vec![
                "-p",
                "--mode",
                "json",
                "--no-session",
                "--no-title",
                "--model",
                "haiku",
                "--cwd",
                view.as_str(),
                "--tools",
                "read",
                PROMPT,
            ],
            Self::OpenCode => vec!["run", "--format", "json", PROMPT],
        };
        args.into_iter().map(str::to_owned).collect()
    }
}

fn scratch_hall() -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = hall_root();
    run_ok(&root, &["init"]);
    std::fs::write(
        root.join("ivar.json"),
        r#"{"name":"acme","providers":{"available":["claude-code","omp","opencode","t3"],"default":"claude-code"},"repos":[],"version":1}"#,
    )
    .unwrap();
    run_ok(&root, &["sync"]);
    (guard, root)
}

fn session(root: &Utf8Path, feature: &str, provider: &str) -> (String, Utf8PathBuf) {
    run_ok(root, &["feature", "create", feature]);
    let started = run_ok(
        root,
        &[
            "session",
            "start",
            feature,
            "--detached",
            "--provider",
            provider,
        ],
    );
    (
        started["session_id"].as_str().unwrap().to_owned(),
        Utf8PathBuf::from(started["view_dir"].as_str().unwrap()),
    )
}

fn stop(root: &Utf8Path, id: &str) {
    let _ = isolated_ivar(&home(root))
        .current_dir(root)
        .args(["session", "stop", id])
        .output();
}

fn launch(root: &Utf8Path, id: &str, live: Live, args: &[String]) -> Child {
    let ivar = Utf8PathBuf::from(env!("CARGO_BIN_EXE_ivar"));
    let path = format!(
        "{}:{}",
        ivar.parent().unwrap(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut command = Command::new(ivar.as_std_path());
    for (key, _) in std::env::vars() {
        if key.starts_with("IVAR_") {
            command.env_remove(key);
        }
    }
    command
        .env("PATH", path)
        .env("IVAR_NO_UPDATE_CHECK", "1")
        .args(["session", "sandbox", "--session", id, "--", live.binary()])
        .args(args)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

fn finish(mut child: Child) {
    let deadline = Instant::now() + Duration::from_secs(300);
    while Instant::now() < deadline {
        if child.try_wait().unwrap().is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn one_shot(root: &Utf8Path, id: &str, view: &Utf8Path, live: Live) {
    finish(launch(root, id, live, &live.one_shot_args(view)));
}

fn captured_events(root: &Utf8Path) -> BTreeSet<String> {
    let out = ivar()
        .current_dir(root)
        .args(["--json", "memory", "status"])
        .output()
        .unwrap()
        .stdout;
    let value: Value = serde_json::from_slice(&out).unwrap();
    value["observations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["source_event"].as_str().unwrap().to_owned())
        .collect()
}

fn capture_scenario(live: Live) {
    if !live.enabled() {
        return;
    }
    let (_guard, root) = scratch_hall();
    let (id, view) = session(&root, "capture", live.provider());
    one_shot(&root, &id, &view, live);
    stop(&root, &id);
    let seen = captured_events(&root);
    for event in ["session.started", "session.prompted", "session.tool"] {
        assert!(
            seen.contains(event),
            "{live:?}: {event} missing from {seen:?}"
        );
    }
    if matches!(live, Live::Claude | Live::Omp) {
        assert!(
            seen.contains("session.ended"),
            "{live:?}: session.ended missing from {seen:?}"
        );
    } else {
        assert!(
            !seen.contains("session.ended"),
            "{live:?} declares no session end"
        );
    }
}

#[test]
#[ignore = "needs provider credentials: claude"]
fn claude_capture_records_the_declared_events() {
    capture_scenario(Live::Claude);
}

#[test]
#[ignore = "needs provider credentials: omp"]
fn omp_capture_records_the_declared_events() {
    capture_scenario(Live::Omp);
}

#[test]
#[ignore = "needs provider credentials: opencode"]
fn opencode_capture_records_the_declared_events() {
    capture_scenario(Live::OpenCode);
}

/// T3 threads live in the desktop app, so a human drives one prompt and closes the thread
/// while this test polls. It only reports; flipping the matrix stays a manual decision.
#[test]
#[ignore = "needs a running T3 Code desktop app and a human-driven thread"]
fn t3_session_end_is_confirmed_or_stays_undeclared() {
    if std::env::var("IVAR_LIVE_T3").is_err() {
        eprintln!("skipped: IVAR_LIVE_T3 is not set");
        return;
    }
    let (_guard, root) = scratch_hall();
    let (id, view) = session(&root, "capture-t3", "t3");
    eprintln!("open a T3 thread in {view}, send one prompt, then close the thread");
    let deadline = Instant::now() + T3_THREAD_WAIT;
    while Instant::now() < deadline && !captured_events(&root).contains("session.ended") {
        std::thread::sleep(Duration::from_secs(2));
    }
    stop(&root, &id);
    eprintln!(
        "t3 session.ended observed: {}",
        captured_events(&root).contains("session.ended")
    );
}
