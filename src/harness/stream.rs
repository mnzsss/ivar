//! Provider-shaped JSON in, [`ExecutorEvent`] out.
//!
//! `claude -p ... --output-format stream-json` and `opencode run` each speak
//! their own line protocol on stdout: one JSON value per line, in a shape
//! that belongs to that provider and to no one else. Nothing above this
//! module should ever hold a [`serde_json::Value`] that came off a provider's
//! stdout — every function here takes a raw line and hands back
//! [`ExecutorEvent`]s, the one vocabulary the board understands. That is the
//! whole reason this module exists rather than a `match parsed["type"]` at
//! each call site: a provider's envelope shape is free to change without
//! anything outside this file noticing.
//!
//! # Claude Code: envelopes vs. content blocks
//!
//! The stream's TOP-LEVEL envelopes are `system`, `assistant`, `user`,
//! `rate_limit_event` and `result`. `text` and `tool_use` are *content
//! blocks* nested under `message.content[]` on an `assistant` envelope — they
//! never appear at the top level. Matching `parsed["type"] == "tool_use"`
//! against the envelope, which an early draft of this module did, matches
//! nothing; the check has to go one level down.
//!
//! One `assistant` envelope can carry several tool calls in its `content`
//! array (the model deciding to read three files in one turn), so parsing a
//! line yields a *list* of events, not one — [`parse_claude_line`] returns a
//! `Vec` even when the line is a single `tool_use`.
//!
//! # Assistant prose is not a question
//!
//! [`ExecutorEvent::QuestionAsked`] moves the whole workstream to `blocked`.
//! If ordinary assistant narration ("I'll start by reading the config...")
//! produced it, every run would stall on its first sentence. Only an
//! explicit `AskUserQuestion` tool call is a question; every other content
//! block — including plain `text` blocks — is either a `ToolUsed` or
//! nothing.
//!
//! # The native session id
//!
//! The id `--resume` accepts is the provider's own, not ivar's session id.
//! Claude Code announces it once, on the `system`/`init` envelope's
//! `session_id` field. [`parse_claude_line`] surfaces it as
//! [`ExecutorEvent::NativeSession`] the moment it appears.
//!
//! # Unparseable is skipped, not fatal
//!
//! A line that is not valid JSON (a stray log line, a partial write) yields
//! an empty `Vec` rather than an error. The alternative — failing the whole
//! run on one bad line — would let a provider's own logging noise take down
//! an otherwise-healthy session.
//!
//! # `Started`, `Completed`, `Failed`
//!
//! These three variants exist on [`ExecutorEvent`] because the orchestrator
//! that spawns the child and watches it exit (`tick`, a different
//! workstream) needs the same vocabulary this module produces from stdout —
//! one enum, not two. Neither provider's line protocol announces its own
//! start or exit, so no parser here ever constructs them; the orchestrator
//! builds `Started` once the child is spawned and `Completed`/`Failed` from
//! the child's exit status.

use serde_json::Value;

/// The event a provider's output — or the fact of its process exiting — is
/// reduced to. This is the one vocabulary the board deals in; provider-shaped
/// JSON never crosses out of this module (see the module doc comment).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutorEvent {
    /// The provider process was spawned.
    Started,
    /// A tool call happened. `path` is best-effort — only some tools carry
    /// one, and providers spell it differently (`file_path`, `path`,
    /// `notebook_path`).
    ToolUsed {
        /// The tool's name, verbatim from the provider.
        tool: String,
        /// The file the tool call touched, when the tool call names one.
        path: Option<String>,
    },
    /// The executor asked the human a question. Blocks the workstream — see
    /// the module doc comment on why this is never inferred from prose.
    QuestionAsked {
        /// The question text shown to the human.
        prompt: String,
    },
    /// The provider's own session id, as `--resume` (or its equivalent)
    /// accepts it. Not ivar's session id.
    NativeSession {
        /// The provider-native session identifier.
        id: String,
    },
    /// The process exited zero.
    Completed,
    /// The process exited non-zero, or otherwise failed to run to
    /// completion.
    Failed {
        /// A human-readable description of the failure.
        error: String,
    },
}

/// A string field, present and non-empty.
fn string_field<'a>(object: &'a serde_json::Map<String, Value>, key: &str) -> Option<&'a str> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

/// Best-effort file path for a tool call, across the tools that carry one.
fn tool_path(input: &serde_json::Map<String, Value>) -> Option<String> {
    string_field(input, "file_path")
        .or_else(|| string_field(input, "notebook_path"))
        .or_else(|| string_field(input, "path"))
        .map(str::to_owned)
}

/// Pull the human-facing text out of an `AskUserQuestion` tool call's input.
/// Claude Code's `AskUserQuestion` carries either a single `question` string
/// or a `questions` array of `{ question: string }`; either shape yields a
/// prompt.
fn ask_user_question_prompt(input: &serde_json::Map<String, Value>) -> Option<String> {
    if let Some(question) = string_field(input, "question") {
        return Some(question.to_owned());
    }

    let questions = input.get("questions")?.as_array()?;
    let texts: Vec<&str> = questions
        .iter()
        .filter_map(Value::as_object)
        .filter_map(|q| string_field(q, "question"))
        .collect();

    if texts.is_empty() {
        None
    } else {
        Some(texts.join("\n"))
    }
}

/// Claude Code announces its own session id on the `system`/`init` envelope.
/// That id — not ivar's session id — is what `--resume` accepts.
fn claude_native_session_id(object: &serde_json::Map<String, Value>) -> Option<String> {
    if object.get("type").and_then(Value::as_str) != Some("system") {
        return None;
    }
    if object.get("subtype").and_then(Value::as_str) != Some("init") {
        return None;
    }
    string_field(object, "session_id").map(str::to_owned)
}

/// Reduce one line of Claude Code's `--output-format stream-json` protocol to
/// zero or more [`ExecutorEvent`]s.
///
/// See the module doc comment for the shape this walks: top-level envelopes
/// are `system`, `assistant`, `user`, `rate_limit_event`, `result`; the
/// content blocks that matter (`tool_use`) live under
/// `assistant.message.content[]`. A line that fails to parse, or that
/// parses but is not a JSON object, yields no events — it is skipped, not
/// fatal.
#[must_use]
pub fn parse_claude_line(line: &str) -> Vec<ExecutorEvent> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let Ok(Value::Object(object)) = serde_json::from_str::<Value>(trimmed) else {
        return Vec::new();
    };

    if let Some(id) = claude_native_session_id(&object) {
        return vec![ExecutorEvent::NativeSession { id }];
    }

    if object.get("type").and_then(Value::as_str) != Some("assistant") {
        return Vec::new();
    }

    let Some(content) = object
        .get("message")
        .and_then(Value::as_object)
        .and_then(|message| message.get("content"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };

    let mut events = Vec::new();

    for block in content {
        let Some(block) = block.as_object() else {
            continue;
        };
        if block.get("type").and_then(Value::as_str) != Some("tool_use") {
            continue;
        }
        let Some(name) = string_field(block, "name") else {
            continue;
        };

        let empty_input = serde_json::Map::new();
        let input = block
            .get("input")
            .and_then(Value::as_object)
            .unwrap_or(&empty_input);

        if name == "AskUserQuestion" {
            if let Some(prompt) = ask_user_question_prompt(input) {
                events.push(ExecutorEvent::QuestionAsked { prompt });
            }
            continue;
        }

        events.push(ExecutorEvent::ToolUsed {
            tool: name.to_owned(),
            path: tool_path(input),
        });
    }

    events
}

/// Reduce one line of OpenCode's `run` event protocol to zero or one
/// [`ExecutorEvent`].
///
/// OpenCode's protocol is flatter than Claude Code's — no envelope/content
/// split — so a line maps to at most one event. A line that fails to parse,
/// or names a shape this module does not recognise, yields no events.
#[must_use]
pub fn parse_opencode_line(line: &str) -> Vec<ExecutorEvent> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let Ok(Value::Object(object)) = serde_json::from_str::<Value>(trimmed) else {
        return Vec::new();
    };

    match object.get("type").and_then(Value::as_str) {
        Some("tool_use") if object.get("name").and_then(Value::as_str) == Some("edit") => {
            vec![ExecutorEvent::ToolUsed {
                tool: "edit".to_owned(),
                path: string_field(&object, "file_path").map(str::to_owned),
            }]
        }
        Some("question") => {
            let prompt = string_field(&object, "text").unwrap_or_default().to_owned();
            vec![ExecutorEvent::QuestionAsked { prompt }]
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
#[path = "../../tests/unit/harness/stream.rs"]
mod tests;
