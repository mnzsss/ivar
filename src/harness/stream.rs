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
//! # OpenCode: one envelope, one part
//!
//! `opencode run --format json` is an envelope protocol too, just a
//! shallower one. Every line is
//! `{ "type": <t>, "timestamp": <ms>, "sessionID": "ses_…", …payload }`, and
//! the payload for everything that matters is a single `part` object. The
//! whole set of `t` is `step_start`, `step_finish`, `text`, `reasoning`
//! (emitted only under `--thinking`), `tool_use` and `error`.
//!
//! A `tool_use` line looks like:
//!
//! ```text
//! {"type":"tool_use","timestamp":…,"sessionID":"ses_…","part":{
//!   "type":"tool","tool":"read","callID":"call_…","id":"prt_…",
//!   "state":{"status":"completed","input":{"filePath":"a.txt","limit":1},
//!            "output":"…","metadata":{…},"title":"…","time":{…}}}}
//! ```
//!
//! So the tool's name is `part.tool` and its arguments are
//! `part.state.input` — **not** `name` and `file_path` at the top level,
//! which is what an earlier draft of this module matched against and which
//! matches nothing. The path key is `filePath`, camelCase, on every OpenCode
//! tool that names a file (`read`, `edit`, `write`, `patch`).
//!
//! OpenCode emits `tool_use` only once the call has settled — `state.status`
//! is `completed` or `error`, never `running` — so a tool call appears once,
//! after the fact. The `error` case is kept rather than filtered: a denied
//! write (this hall's own execution guard refusing one) is a thing that
//! happened, and the journal should say so.
//!
//! # OpenCode cannot ask
//!
//! OpenCode *has* a `question` tool and a `question.asked` server event, and
//! neither is reachable from `opencode run`. The `run` subcommand creates its
//! session with `{permission: "question", action: "deny", pattern: "*"}` (plus
//! the same for `plan_enter`/`plan_exit`), so the tool's permission assertion
//! fails before it executes; and `run`'s JSON writer only ever emits the six
//! types listed above, so even `permission.asked` — which it handles inline,
//! auto-rejecting — never reaches stdout as JSON.
//!
//! The consequence is that [`ExecutorEvent::QuestionAsked`] is unreachable for
//! OpenCode, and [`parse_opencode_line`] never constructs one. A question the
//! model wants to ask comes out as ordinary prose in a `text` part, which the
//! "Assistant prose is not a question" rule above already says must stay
//! non-blocking. This is declared, not inferred: `Capabilities`'
//! `supports_questions` is false for OpenCode, and `tick` records it on the
//! journal at launch so a run that never blocks is explained rather than
//! merely quiet.
//!
//! # The native session id
//!
//! The id `--resume` (or `--session`) accepts is the provider's own, not
//! ivar's session id. Claude Code announces it once, on the `system`/`init`
//! envelope's `session_id` field. OpenCode instead stamps `sessionID` on
//! *every* line, so [`parse_opencode_line`] emits
//! [`ExecutorEvent::NativeSession`] for every line it parses and the drain
//! loop in `tick`'s `launch` keeps only the first — the parser stays a pure
//! function of one line, which is the property that makes it testable.
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
    /// The run changed at least one path its own write contract allows — the
    /// positive evidence a workstream needs to claim it did anything.
    ///
    /// Emitted by the post-run audit, never parsed from a provider's stream:
    /// a tool call is a statement of intent, and what this reports is an
    /// effect read back off the filesystem. See
    /// `action::execute::tick::launch`'s `audit_run`.
    Produced {
        /// The contracted paths the run changed, `<repo>/<path>`.
        paths: Vec<String>,
    },
    /// The process exited zero.
    Completed {
        /// Whether the post-run audit had anything to observe — `false` when
        /// the feature has no promoted worktree, so nothing the run did could
        /// be read back off disk either way.
        ///
        /// It is the difference between "this workstream produced nothing"
        /// and "there was no place for it to produce anything", which look
        /// identical from the change set alone. Refusing a workstream on the
        /// second would be refusing it for the absence of an oracle rather
        /// than for evidence, which is the inverse of the default-deny
        /// discipline the guard states: deny on what you *saw*, never on what
        /// you could not look at.
        audited: bool,
    },
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
///
/// Both providers' spellings live in one list because the two vocabularies do
/// not collide: Claude Code writes `file_path` / `notebook_path`, OpenCode
/// writes `filePath`, and `path` is a fallback either might use.
fn tool_path(input: &serde_json::Map<String, Value>) -> Option<String> {
    string_field(input, "file_path")
        .or_else(|| string_field(input, "filePath"))
        .or_else(|| string_field(input, "notebook_path"))
        .or_else(|| string_field(input, "path"))
        .map(str::to_owned)
}

/// The human-readable half of an OpenCode `error` envelope.
///
/// The payload is `{"error": {"name": …, "data": {"message": …}}}` — a named
/// error, with the sentence a human wants one level in. `name` is the fallback
/// for a variant carrying no `data`; anything else yields nothing, and the
/// child's own non-zero exit reports the failure instead.
fn opencode_error_message(object: &serde_json::Map<String, Value>) -> Option<String> {
    let error = object.get("error").and_then(Value::as_object)?;
    error
        .get("data")
        .and_then(Value::as_object)
        .and_then(|data| string_field(data, "message"))
        .or_else(|| string_field(error, "name"))
        .map(str::to_owned)
}

/// The [`ExecutorEvent::ToolUsed`] a `tool_use` envelope describes: the tool's
/// name at `part.tool`, the file it touched at `part.state.input`. `None` for
/// an envelope missing either the part or the name.
fn opencode_tool_used(object: &serde_json::Map<String, Value>) -> Option<ExecutorEvent> {
    let part = object.get("part").and_then(Value::as_object)?;
    let tool = string_field(part, "tool")?;
    let path = part
        .get("state")
        .and_then(Value::as_object)
        .and_then(|state| state.get("input"))
        .and_then(Value::as_object)
        .and_then(tool_path);

    Some(ExecutorEvent::ToolUsed {
        tool: tool.to_owned(),
        path,
    })
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

/// Reduce one line of `opencode run --format json`'s event protocol to zero
/// or more [`ExecutorEvent`]s.
///
/// See the module doc's "OpenCode: one envelope, one part" for the shape this
/// walks. Three things follow from it, and each is load-bearing:
///
/// - Every line carries `sessionID`, so every parsed line yields a
///   [`ExecutorEvent::NativeSession`]; the caller keeps the first.
/// - A tool call is `part.tool` plus `part.state.input`, one level down —
///   never `name`/`file_path` at the top level.
/// - No line is ever a [`ExecutorEvent::QuestionAsked`]. `opencode run`
///   cannot ask; see "OpenCode cannot ask".
///
/// A line that fails to parse, or that names a type this module does not
/// recognise (`step_start`, `step_finish`, `text`, `reasoning`), yields
/// nothing beyond the session id — it is skipped, not fatal.
#[must_use]
pub fn parse_opencode_line(line: &str) -> Vec<ExecutorEvent> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let Ok(Value::Object(object)) = serde_json::from_str::<Value>(trimmed) else {
        return Vec::new();
    };

    let mut events = Vec::new();

    if let Some(id) = string_field(&object, "sessionID") {
        events.push(ExecutorEvent::NativeSession { id: id.to_owned() });
    }

    match object.get("type").and_then(Value::as_str) {
        Some("tool_use") => events.extend(opencode_tool_used(&object)),
        Some("error") => events
            .extend(opencode_error_message(&object).map(|error| ExecutorEvent::Failed { error })),
        _ => {}
    }

    events
}

#[cfg(test)]
#[path = "../../tests/unit/harness/stream.rs"]
mod tests;
