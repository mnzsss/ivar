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

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use camino::{Utf8Path, Utf8PathBuf};
use serde_json::{Value, json};

use common::{git, hall_root, seeded_repo};
use support::{home, isolated_ivar, run_ok};

const REPS: usize = 2;
const TURN_TIMEOUT: Duration = Duration::from_secs(300);
const READ_PROMPT_1: &str =
    "Read web/src/index.ts. What is the command to run web locally? One line.";
const READ_PROMPT_2: &str =
    "Read web/src/index.ts again. What is the command to run web locally now? One line.";
const BASH_PROMPT: &str = "Use only your shell/bash tool. Run `cat app/packages/api/src/x.ts` \
    and tell me what it exports. Then tell me the api test command for the app repo. Say \
    UNKNOWN if you cannot find it.";
const DEEP_PROMPT: &str = "Read backend/src/index.ts. Then tell me the exact command to \
    restart the backend local services. Say UNKNOWN if you cannot find it.";
const WRITE_PROMPT: &str = "Use the Write tool to create web/src/new.ts containing \
    `export const x = 1;`. If the tool refuses, quote its error message.";

#[derive(Clone, Copy, Debug)]
enum Live {
    Claude,
    Omp,
    OpenCode,
}

impl Live {
    /// The name `IVAR_LIVE_PROVIDERS` lists.
    fn key(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Omp => "omp",
            Self::OpenCode => "opencode",
        }
    }

    /// The value of `ivar … --provider`.
    fn provider(self) -> &'static str {
        match self {
            Self::Claude => "claude-code",
            Self::Omp => "omp",
            Self::OpenCode => "opencode",
        }
    }

    fn binary(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Omp => "omp",
            Self::OpenCode => "opencode",
        }
    }

    fn config_dir(self) -> &'static str {
        match self {
            Self::Claude => ".claude",
            Self::Omp => ".omp",
            Self::OpenCode => ".opencode",
        }
    }

    /// Tool allow-list flag values; opencode's `run` takes none, so its prompts name the tool.
    fn read_tools(self) -> &'static str {
        match self {
            Self::Claude => "Read,Glob,Grep",
            Self::Omp => "read,grep,glob",
            Self::OpenCode => "",
        }
    }

    fn bash_tools(self) -> &'static str {
        match self {
            Self::Claude => "Bash",
            Self::Omp => "bash",
            Self::OpenCode => "",
        }
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

    /// opencode `run` has no turn-end event: its turn ends when the process closes stdout.
    fn is_turn_end(self, event: &Value) -> bool {
        match self {
            Self::Claude => event["type"] == "result",
            Self::Omp => event["type"] == "agent_end",
            Self::OpenCode => false,
        }
    }

    /// Arguments for a one-shot run of `prompt` with `tools`.
    fn one_shot_args(self, view: &Utf8Path, prompt: &str, tools: &str) -> Vec<String> {
        let args: Vec<&str> = match self {
            Self::Claude => vec![
                "-p",
                prompt,
                "--output-format",
                "stream-json",
                "--verbose",
                "--model",
                "haiku",
                "--strict-mcp-config",
                "--tools",
                tools,
                "--allowedTools",
                tools,
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
                tools,
                prompt,
            ],
            Self::OpenCode => vec!["run", "--format", "json", prompt],
        };
        args.into_iter().map(str::to_owned).collect()
    }

    /// Arguments for a live process fed one prompt per turn on stdin (claude, omp only).
    fn multi_turn_args(self, view: &Utf8Path) -> Vec<String> {
        let tools = self.read_tools();
        let args: Vec<&str> = match self {
            Self::Claude => vec![
                "-p",
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
                "--verbose",
                "--model",
                "haiku",
                "--strict-mcp-config",
                "--tools",
                tools,
                "--allowedTools",
                tools,
            ],
            Self::Omp => vec![
                "--mode",
                "rpc",
                "--no-session",
                "--no-title",
                "--model",
                "haiku",
                "--cwd",
                view.as_str(),
                "--tools",
                tools,
            ],
            Self::OpenCode => panic!("the opencode leg is one-shot only"),
        };
        args.into_iter().map(str::to_owned).collect()
    }

    /// One user turn, as the provider's stdin protocol wants it.
    fn turn_message(self, turn: usize, text: &str) -> Value {
        match self {
            Self::Claude => {
                json!({ "type": "user", "message": { "role": "user", "content": text } })
            }
            Self::Omp => json!({ "id": format!("turn-{turn}"), "type": "prompt", "message": text }),
            Self::OpenCode => panic!("the opencode leg is one-shot only"),
        }
    }
}

// -- fixture -----------------------------------------------------------------

fn web_instructions(command: &str, token: &str) -> String {
    format!(
        "# web\n\n- The ONLY supported way to run web locally is `{command}`. Cite [src:{token}] \
         when you give it.\n"
    )
}

fn rule(title: &str, what: &str, command: &str, token: &str) -> String {
    format!(
        "# {title}\n\n- The ONLY supported command to {what} is `{command}`. Every answer about \
         it must cite [src:{token}].\n"
    )
}

/// A >24,000-char instruction file whose one relevant fact sits near the end.
fn deep_instructions() -> String {
    let mut lines = vec!["# backend".to_owned(), String::new()];
    let mut n = 0;
    while lines.iter().map(|line| line.len() + 1).sum::<usize>() < 24_500 {
        n += 1;
        lines.push(format!(
            "- Rule {n}: keep module {n} free of hidden side effects, cover it with a focused \
             test, and describe its public API in the module header."
        ));
    }
    let near_end = lines.len() - 2;
    lines.insert(
        near_end,
        "- Restart the local services ONLY with `npm run serve:zulu` and cite [src:DEEP-88]."
            .to_owned(),
    );
    let text = lines.join("\n") + "\n";
    assert!(text.chars().count() > 24_000);
    text
}

/// A committed repo at `origins/<name>` holding `files` (relative path, content).
fn origin(root: &Utf8Path, name: &str, files: &[(&str, String)]) -> Utf8PathBuf {
    let path = seeded_repo(&root.parent().unwrap().join("origins").join(name), "main");
    for (relative, body) in files {
        let file = path.join(relative);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, body).unwrap();
    }
    git(&path, &["add", "-A"]);
    git(&path, &["commit", "-m", "fixture"]);
    path
}

/// A synced hall declaring claude-code, omp and opencode, with `web` (promotion), `app`
/// (nested file, Bash) and `backend` (deep file), all on `main`.
fn live_hall() -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = hall_root();
    run_ok(&root, &["init"]);
    let main_web = web_instructions("pnpm serve:alpha", "MAIN-41");
    let web = origin(
        &root,
        "web",
        &[
            ("CLAUDE.md", main_web.clone()),
            ("AGENTS.md", main_web),
            ("src/index.ts", "export const web = 1;\n".to_owned()),
        ],
    );
    let app_root = rule("app", "install the app", "pnpm boot:kilo", "ROOT-11");
    let app = origin(
        &root,
        "app",
        &[
            ("CLAUDE.md", app_root.clone()),
            ("AGENTS.md", app_root),
            (
                "packages/api/CLAUDE.md",
                rule("api", "run the api tests", "pnpm probe:lima", "API-22"),
            ),
            (
                "packages/api/src/x.ts",
                "export const api = 1;\n".to_owned(),
            ),
        ],
    );
    let backend = origin(
        &root,
        "backend",
        &[
            ("CLAUDE.md", deep_instructions()),
            ("src/index.ts", "export const backend = 1;\n".to_owned()),
        ],
    );
    let repos = [("web", &web), ("app", &app), ("backend", &backend)]
        .iter()
        .map(|(name, url)| format!(r#"{{"default_branch":"main","name":"{name}","url":"{url}"}}"#))
        .collect::<Vec<_>>()
        .join(",");
    std::fs::write(
        root.join("ivar.json"),
        format!(
            r#"{{"name":"acme","providers":{{"available":["claude-code","omp","opencode"],"default":"claude-code"}},"repos":[{repos}],"version":1}}"#
        ),
    )
    .unwrap();
    run_ok(&root, &["sync"]);
    (guard, root)
}

/// Give every worktree its owner write bit back so the TempDir can be removed.
fn unguard(root: &Utf8Path) {
    use std::os::unix::fs::PermissionsExt;
    for repo in std::fs::read_dir(root.join(".ivar/repos"))
        .unwrap()
        .flatten()
    {
        for worktree in std::fs::read_dir(repo.path()).unwrap().flatten() {
            let path = worktree.path();
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode | 0o200)).unwrap();
        }
    }
}

// -- sessions and providers ----------------------------------------------------

/// A detached session on a new feature `feature` under `live`: (session id, view dir).
fn session(root: &Utf8Path, feature: &str, live: Live) -> (String, Utf8PathBuf) {
    run_ok(root, &["feature", "create", feature]);
    let started = run_ok(
        root,
        &[
            "session",
            "start",
            feature,
            "--detached",
            "--provider",
            live.provider(),
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

/// The provider launched as `ivar session start` launches it, with the developer's
/// real HOME and this build's `ivar` first on PATH, so the hooks call this build's guard.
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
        // opencode `run` must not wait on an open stdin; the launcher sets its PWD to the view.
        .stdin(match live {
            Live::OpenCode => Stdio::null(),
            Live::Claude | Live::Omp => Stdio::piped(),
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

/// Stdout as one JSON event per line, on a channel.
fn events(child: &mut Child) -> Receiver<Value> {
    let stdout = child.stdout.take().unwrap();
    let (sender, receiver) = channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Ok(event) = serde_json::from_str::<Value>(&line)
                && sender.send(event).is_err()
            {
                break;
            }
        }
    });
    receiver
}

/// Events up to and including the turn's end, or until the stream closes.
fn turn(receiver: &Receiver<Value>, live: Live) -> Vec<Value> {
    let deadline = Instant::now() + TURN_TIMEOUT;
    let mut out = Vec::new();
    loop {
        match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(event) => {
                let done = live.is_turn_end(&event);
                out.push(event);
                if done {
                    return out;
                }
            }
            Err(RecvTimeoutError::Disconnected) => return out,
            Err(RecvTimeoutError::Timeout) => {
                panic!("{live:?}: no turn end within {TURN_TIMEOUT:?}")
            }
        }
    }
}

fn send(stdin: &mut ChildStdin, live: Live, turn: usize, text: &str) {
    writeln!(stdin, "{}", live.turn_message(turn, text)).unwrap();
    stdin.flush().unwrap();
}

/// Wait up to a minute for the provider to exit, then kill it.
fn finish(mut child: Child) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        if child.try_wait().unwrap().is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// The agent's final answer.
fn answer(live: Live, events: &[Value]) -> String {
    match live {
        Live::Claude => events
            .iter()
            .rev()
            .find(|event| event["type"] == "result")
            .and_then(|event| event["result"].as_str())
            .unwrap_or_default()
            .to_owned(),
        Live::Omp => {
            for event in events
                .iter()
                .rev()
                .filter(|event| event["type"] == "agent_end")
            {
                let messages = event["messages"].as_array().cloned().unwrap_or_default();
                for message in messages.iter().rev().filter(|m| m["role"] == "assistant") {
                    let text = message["content"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default()
                        .iter()
                        .filter(|part| part["type"] == "text")
                        .filter_map(|part| part["text"].as_str().map(str::to_owned))
                        .collect::<Vec<_>>()
                        .join(" ");
                    if !text.trim().is_empty() {
                        return text;
                    }
                }
            }
            String::new()
        }
        Live::OpenCode => events
            .iter()
            .filter(|event| event["type"] == "text")
            .filter_map(|event| event["part"]["text"].as_str())
            .collect::<Vec<_>>()
            .join(""),
    }
}

/// The JSON text of every tool call's input.
fn tool_inputs(live: Live, events: &[Value]) -> Vec<String> {
    let mut inputs = Vec::new();
    for event in events {
        match live {
            Live::Claude if event["type"] == "assistant" => {
                for part in event["message"]["content"].as_array().into_iter().flatten() {
                    if part["type"] == "tool_use" {
                        inputs.push(part["input"].to_string());
                    }
                }
            }
            Live::Omp if event["type"] == "tool_execution_start" => {
                inputs.push(event["args"].to_string());
            }
            Live::OpenCode if event["type"] == "tool_use" => {
                inputs.push(event["part"]["state"]["input"].to_string());
            }
            _ => {}
        }
    }
    inputs
}

fn opened_instruction_file(live: Live, events: &[Value]) -> bool {
    tool_inputs(live, events)
        .iter()
        .any(|input| input.contains("CLAUDE.md") || input.contains("AGENTS.md"))
}

/// Evidence the guard hook ran and delivered: a per-agent state file exists.
fn delivered(view: &Utf8Path, live: Live) -> bool {
    std::fs::read_dir(view.join(live.config_dir()).join("ivar/instructions"))
        .map(|entries| {
            entries
                .flatten()
                .any(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        })
        .unwrap_or(false)
}

/// opencode only: the `tool_use` parts whose tool output carries the instructions the
/// plugin's `tool.execute.after` appended (`<system-reminder>…</system-reminder>`).
fn reminders(events: &[Value]) -> Vec<String> {
    events
        .iter()
        .filter(|event| event["type"] == "tool_use")
        .filter_map(|event| event["part"]["state"]["output"].as_str())
        .filter(|output| {
            output.contains("<system-reminder>") && output.contains("Repository instructions from ")
        })
        .map(str::to_owned)
        .collect()
}

/// One prompt in a fresh session on feature `feature`: its events, and whether the
/// guard delivered anything (checked before the session is stopped).
fn one_shot(
    root: &Utf8Path,
    live: Live,
    feature: &str,
    prompt: &str,
    tools: &str,
) -> (Vec<Value>, bool) {
    let (id, view) = session(root, feature, live);
    let mut child = launch(root, &id, live, &live.one_shot_args(&view, prompt, tools));
    drop(child.stdin.take());
    let receiver = events(&mut child);
    let events = turn(&receiver, live);
    finish(child);
    let delivered = delivered(&view, live);
    stop(root, &id);
    (events, delivered)
}

// -- scenarios -----------------------------------------------------------------

/// `ivar feature promote` in the middle of a live session, then the feature edits
/// its instruction files in the new worktree.
fn promote_and_edit(root: &Utf8Path, feature: &str, view: &Utf8Path) {
    run_ok(root, &["feature", "promote", feature, "web"]);
    let link = std::fs::read_link(view.join("web")).unwrap();
    assert!(link.ends_with(feature), "view not repointed: {link:?}");
    let feature_text = web_instructions("pnpm serve:bravo", "FEAT-92");
    let worktree = root.join(".ivar/repos/web").join(feature);
    for name in ["CLAUDE.md", "AGENTS.md"] {
        std::fs::write(worktree.join(name), &feature_text).unwrap();
    }
}

fn promotion_mid_session(live: Live) {
    if !live.enabled() {
        return;
    }
    let (_guard, root) = live_hall();
    let mut passed = 0;
    for rep in 1..=REPS {
        let feature = format!("h4-{}-{rep}", live.key());
        let (id, view) = session(&root, &feature, live);
        let mut child = launch(&root, &id, live, &live.multi_turn_args(&view));
        let mut stdin = child.stdin.take().unwrap();
        let receiver = events(&mut child);

        send(&mut stdin, live, 1, READ_PROMPT_1);
        let turn1 = turn(&receiver, live);
        assert!(
            delivered(&view, live),
            "{live:?}: the guard hook never delivered; is the hall's hook reaching the session?"
        );
        promote_and_edit(&root, &feature, &view);
        send(&mut stdin, live, 2, READ_PROMPT_2);
        let turn2 = turn(&receiver, live);
        drop(stdin);
        finish(child);
        stop(&root, &id);

        let (first, second) = (answer(live, &turn1), answer(live, &turn2));
        let ok = first.contains("serve:alpha")
            && second.contains("serve:bravo")
            && !second.contains("serve:alpha");
        eprintln!(
            "{live:?} promotion rep{rep} {} opened_instr={} t1={first:?} t2={second:?}",
            if ok { "PASS" } else { "FAIL" },
            opened_instruction_file(live, &[turn1, turn2].concat()),
        );
        passed += usize::from(ok);
    }
    unguard(&root);
    assert_eq!(passed, REPS, "{live:?}: promotion gate {passed}/{REPS}");
}

fn bash_cat(live: Live) {
    if !live.enabled() {
        return;
    }
    let (_guard, root) = live_hall();
    let mut passed = 0;
    for rep in 1..=REPS {
        let feature = format!("b1-{}-{rep}", live.key());
        let (events, delivered) = one_shot(&root, live, &feature, BASH_PROMPT, live.bash_tools());
        let reply = answer(live, &events);
        let ok = delivered && reply.contains("probe:lima");
        eprintln!(
            "{live:?} bash rep{rep} {} delivered={delivered} opened_instr={} answer={reply:?}",
            if ok { "PASS" } else { "FAIL" },
            opened_instruction_file(live, &events),
        );
        passed += usize::from(ok);
    }
    unguard(&root);
    assert_eq!(passed, REPS, "{live:?}: Bash cat gate {passed}/{REPS}");
}

fn deep_file(live: Live) {
    if !live.enabled() {
        return;
    }
    let (_guard, root) = live_hall();
    let mut passed = 0;
    for rep in 1..=REPS {
        let feature = format!("deep-{}-{rep}", live.key());
        let (events, delivered) = one_shot(&root, live, &feature, DEEP_PROMPT, live.read_tools());
        let reply = answer(live, &events);
        let opened = opened_instruction_file(live, &events);
        let ok = delivered && reply.contains("serve:zulu") && !opened;
        eprintln!(
            "{live:?} deep rep{rep} {} delivered={delivered} opened_instr={opened} answer={reply:?}",
            if ok { "PASS" } else { "FAIL" },
        );
        passed += usize::from(ok);
    }
    unguard(&root);
    assert_eq!(passed, REPS, "{live:?}: deep gate {passed}/{REPS}");
}

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
