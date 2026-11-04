use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use camino::{Utf8Path, Utf8PathBuf};
use serde_json::Value;

use crate::live::{Live, TURN_TIMEOUT};
use crate::support::{home, isolated_ivar, run_ok};

/// A detached session on a new feature `feature` under `live`: (session id, view dir).
pub(crate) fn session(root: &Utf8Path, feature: &str, live: Live) -> (String, Utf8PathBuf) {
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

pub(crate) fn stop(root: &Utf8Path, id: &str) {
    let _ = isolated_ivar(&home(root))
        .current_dir(root)
        .args(["session", "stop", id])
        .output();
}

/// The provider launched as `ivar session start` launches it, with the developer's
/// real HOME and this build's `ivar` first on PATH, so the hooks call this build's guard.
pub(crate) fn launch(root: &Utf8Path, id: &str, live: Live, args: &[String]) -> Child {
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
pub(crate) fn events(child: &mut Child) -> Receiver<Value> {
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
pub(crate) fn turn(receiver: &Receiver<Value>, live: Live) -> Vec<Value> {
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

pub(crate) fn send(stdin: &mut ChildStdin, live: Live, turn: usize, text: &str) {
    writeln!(stdin, "{}", live.turn_message(turn, text)).unwrap();
    stdin.flush().unwrap();
}

/// Wait up to a minute for the provider to exit, then kill it.
pub(crate) fn finish(mut child: Child) {
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
pub(crate) fn answer(live: Live, events: &[Value]) -> String {
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
pub(crate) fn tool_inputs(live: Live, events: &[Value]) -> Vec<String> {
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

pub(crate) fn opened_instruction_file(live: Live, events: &[Value]) -> bool {
    tool_inputs(live, events)
        .iter()
        .any(|input| input.contains("CLAUDE.md") || input.contains("AGENTS.md"))
}

/// Evidence the guard hook ran and delivered: a per-agent state file exists.
pub(crate) fn delivered(view: &Utf8Path, live: Live) -> bool {
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
pub(crate) fn reminders(events: &[Value]) -> Vec<String> {
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
pub(crate) fn one_shot(
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
