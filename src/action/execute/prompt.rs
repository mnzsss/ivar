//! Render the instruction an executor agent receives for one workstream.
//!
//! # What it does
//!
//! Composes, from the plan's text and a [`WorkstreamDef`]: which operations
//! the workstream owns (by id), each operation's text verbatim from the
//! plan's `## Operation details` section, the workstream's write contract
//! stated as a hard boundary, and enough orientation that the agent knows
//! other workstreams are running against the same repository at the same
//! time.
//!
//! # Where the prompt comes from
//!
//! Rendered from the plan, not authored in the execution graph. The graph
//! already names the `OP-*` ids a workstream owns, and the plan's fingerprint
//! already pins the revision those ids were approved against — an authored
//! prompt is a second copy of the same intent that can drift from the plan
//! silently, where a rendered one cannot drift without the plan fingerprint
//! changing first (which pauses the workstream — see
//! [`super::replan`]).
//!
//! # Refusing a missing operation
//!
//! A workstream's [`WorkstreamDef::operations`] names the ids it owns, but
//! that list is board data, copied in at `prepare` time; the plan is the
//! living source of truth an executor is bound by. Rendering checks the
//! claim against the plan twice: the id must appear under this workstream's
//! own heading in the plan's `## Operations` section (via
//! [`super::plan_ops::operations_from_plan`], extracted from `replan.rs` so
//! both parse the same way), and it must have a `**<id>**` entry in
//! `## Operation details` to supply the text. Either miss refuses with
//! [`Failure::blocked`] rather than rendering a prompt that quietly omits the
//! operation — an agent handed such a prompt would implement nothing for
//! that id and no one would notice until review.
//!
//! A third miss is checked because it once got through: an entry that exists
//! and says nothing. Requiring only that `**<id>**` *appear* is a gate that
//! passes `**OP-A**` followed by a blank line, and the prompt it lets out
//! reads `**OP-A** — **OP-A**` — the operation's name where its description
//! should be. The presence of the marker was never the thing worth checking;
//! the presence of the *text* is. See [`operation_text`].
//!
//! # Replies from a human
//!
//! A workstream that blocked on a question is relaunched from scratch by the
//! next `tick` — the child that asked is long gone. So the answers a human
//! gave it, kept in [`super::inbox`], are rendered into the prompt: without
//! them the relaunch is bit-for-bit the prompt that produced the question,
//! and the workstream asks it again, forever.
//!
//! # Signature
//!
//! [`render`] takes the plan text, a borrowed [`WorkstreamDef`], and the
//! replies already addressed to it — nothing else, and no filesystem. A
//! caller that already read the plan and already holds the workstream (e.g.
//! from a loaded [`crate::domain::feature::ExecutionBoard`]) renders without
//! touching the filesystem again.

use std::collections::BTreeSet;

use crate::domain::feature::WorkstreamDef;
use crate::error::{Failure, FixAction};

use super::plan_ops::operations_from_plan;

/// Render the executor prompt for `workstream`, using `plan_text` as the
/// source of truth for operation ownership and operation text, and `replies`
/// as the answers a human has already given this workstream (oldest first —
/// empty for the common case of a workstream that never blocked).
///
/// Blocked when `workstream` claims an operation id that the plan does not
/// back: either the id is absent from this workstream's own heading in the
/// plan's `## Operations` section, or it has no `**<id>**` entry in
/// `## Operation details`.
pub fn render(
    plan_text: &str,
    workstream: &WorkstreamDef,
    replies: &[String],
) -> Result<String, Failure> {
    let plan_workstreams = operations_from_plan(plan_text);
    let owned = plan_workstreams
        .iter()
        .find(|entry| entry.id == workstream.id);
    // Every operation id the plan declares, whoever owns it: the only text
    // that reliably marks where one entry in Operation details ends and the
    // next begins. See [`operation_text`]'s "Where an entry ends".
    let declared: BTreeSet<&str> = plan_workstreams
        .iter()
        .flat_map(|entry| entry.operations.iter().map(String::as_str))
        .collect();

    let mut details = Vec::with_capacity(workstream.operations.len());
    for id in &workstream.operations {
        let owns_it = owned.is_some_and(|entry| entry.operations.iter().any(|op| op == id));
        if !owns_it {
            return Err(missing_operation(
                workstream,
                id,
                format!(
                    "no `- {id}` bullet under a `### {}` heading in the plan's Operations section",
                    workstream.id
                ),
            ));
        }
        let text = operation_text(plan_text, id, &declared).ok_or_else(|| {
            missing_operation(
                workstream,
                id,
                format!("no `**{id}**` entry in the plan's Operation details"),
            )
        })?;
        if text.is_empty() {
            return Err(empty_operation(workstream, id));
        }
        details.push((id.as_str(), text));
    }

    Ok(render_body(workstream, &details, replies))
}

/// The "operation the plan does not back" refusal.
fn missing_operation(workstream: &WorkstreamDef, id: &str, actual: impl Into<String>) -> Failure {
    Failure::blocked(
        "execute.operation_missing_from_plan",
        format!(
            "workstream `{}` claims operation `{id}`, which the plan does not document",
            workstream.id
        ),
    )
    .expected("every operation id a workstream owns to be listed under its own heading in the plan's Operations section, with a matching entry in Operation details")
    .actual(actual)
    .fix(FixAction::safe(
        "execute.fix_plan_or_graph",
        "Add the operation to the plan (Operations section and Operation details), or remove it from this workstream's operations.",
    ))
}

/// The "operation entry with nothing in it" refusal — the marker is there,
/// the description is not.
fn empty_operation(workstream: &WorkstreamDef, id: &str) -> Failure {
    Failure::blocked(
        "execute.operation_text_empty",
        format!(
            "the plan's `**{id}**` entry has no text, so workstream `{}` cannot be told what `{id}` is",
            workstream.id
        ),
    )
    .expected("every `**OP-***` entry in the plan's Operation details to be followed by the text describing it, beside the marker or in the paragraph under it")
    .actual(format!(
        "`**{id}**` is followed by the next entry or the next heading — an executor handed this prompt would read `**{id}** — {id}` and nothing else"
    ))
    .fix(FixAction::safe(
        "execute.fix_plan_or_graph",
        format!("Write the description of `{id}` under its `**{id}**` entry in the plan's Operation details."),
    ))
}

/// Find `id`'s text in the plan's `## Operation details` section.
///
/// `None` means the plan has no `**<id>**` line at all. `Some(text)` means the
/// entry exists and `text` is what it says — **possibly empty**, which the
/// caller refuses; see [`empty_operation`]. The two are kept apart because
/// they are different authoring mistakes and deserve different refusals.
///
/// # Both shapes of an entry
///
/// Markdown gives an author two ways to write one, and the plans this renders
/// use both:
///
/// ```text
/// **OP-A** — beside the marker, wrapping
/// onto as many lines as it likes.
///
/// **OP-B**
///
/// Under the marker, after the blank line
/// that separates the two.
/// ```
///
/// Reading only the first shape is not a partial parse, it is a silent one:
/// the second shape yields the marker line alone, which renders as
/// `**OP-B** — **OP-B**` — an operation with a name and no description, which
/// is exactly what three workstreams were once launched with. So the blank
/// line under a bare marker is crossed, not treated as the end of the entry.
///
/// # Where an entry ends
///
/// At a Markdown heading, or at the marker of another operation **the plan
/// declares** — `declared` carries every id from the plan's own Operations
/// section, whoever owns it.
///
/// A blank line does *not* end it. It used to, and the drop was silent: plans
/// write an operation as a lead paragraph followed by a bulleted `dependsOn` /
/// `touches` / `tests` / `doneWhen` block, and the executor was handed the lead
/// paragraph alone — the acceptance criteria the operation existed to state
/// never reached the agent expected to meet them. Nothing refused, because a
/// non-empty lead paragraph is a valid entry; the run simply proceeded against
/// half its instructions. So an entry now runs to the next thing that is
/// unambiguously not part of it.
///
/// Asking the plan which ids exist, rather than reading the shape of the
/// text, is the whole point. The blunt rule that came first — any line opening
/// with `**token**` starts a new entry — truncated a description at `**410**`,
/// the HTTP status the operation existed to specify. Bold text mid-paragraph
/// is prose: a status code, a constant, an emphasised word. Only a declared id
/// is a boundary.
///
/// The one exception is the line immediately after a blank one, where anything
/// entry-shaped is an entry (see [`begins_an_entry`]) — that is what stops a
/// bare `**OP-A**` from swallowing the entry below it, including one the plan
/// forgot to declare.
///
/// The marker itself is stripped, along with whatever separator the author
/// put after it. [`render_body`] writes `**<id>** — <text>`, so leaving the
/// marker in returns it twice.
///
/// # What the text keeps
///
/// [`assemble`] rebuilds the entry the way the author wrote it: a wrapped
/// paragraph is unwrapped onto one line, a blank line stays a paragraph break,
/// and a list item keeps its own line. Joining every line with a space — which
/// is what unwrapping alone does — turns a `tests` / `doneWhen` block into one
/// run-on sentence, so carrying the block and flattening it would trade a
/// silent drop for an unreadable prompt.
fn operation_text(plan_text: &str, id: &str, declared: &BTreeSet<&str>) -> Option<String> {
    let marker = format!("**{id}**");
    let mut lines = plan_text.lines().peekable();
    while let Some(line) = lines.next() {
        let Some(rest) = line.trim().strip_prefix(marker.as_str()) else {
            continue;
        };
        let mut body = Vec::new();
        let beside_the_marker = strip_separator(rest);
        // Nothing beside the marker means the text is under it, past the blank
        // line Markdown puts between a lead-in and its paragraph — so the
        // entry opens as if a blank line had just been crossed.
        let mut after_blank = beside_the_marker.is_empty();
        if !after_blank {
            body.push(beside_the_marker.to_owned());
        }
        while let Some(next) = lines.peek() {
            let trimmed = next.trim();
            if trimmed.is_empty() {
                after_blank = true;
                body.push(String::new());
                lines.next();
                continue;
            }
            // The line after a blank one is the one place a bold token is an
            // entry on sight, so an entry the plan never declared still ends
            // this one rather than being swallowed into it. Mid-paragraph the
            // question is stricter — see [`interrupts_the_entry`].
            let ends = if after_blank {
                begins_an_entry(next)
            } else {
                interrupts_the_entry(next, declared)
            };
            if ends {
                break;
            }
            after_blank = false;
            body.push(trimmed.to_owned());
            lines.next();
        }
        return Some(assemble(&body));
    }
    None
}

/// Rebuild an entry's collected lines into the text an executor is handed.
///
/// Consecutive prose lines are unwrapped onto one line, a blank line becomes a
/// paragraph break, and a list item opens a line of its own. Leading and
/// trailing blanks fall away, so an entry that collected nothing but blanks
/// comes back empty — which is what [`empty_operation`] refuses.
fn assemble(body: &[String]) -> String {
    let mut out = String::new();
    let mut blank_pending = false;
    for line in body {
        if line.is_empty() {
            blank_pending = !out.is_empty();
            continue;
        }
        if out.is_empty() {
            out.push_str(line);
            continue;
        }
        out.push_str(if blank_pending {
            "\n\n"
        } else if opens_a_list_item(line) {
            "\n"
        } else {
            " "
        });
        out.push_str(line);
        blank_pending = false;
    }
    out
}

/// Does `line` open a Markdown list item — the one shape that must not be
/// unwrapped into the line above it? The bullet markers are the two
/// [`super::plan_ops`] reads, so the two parsers agree on what a bullet is.
fn opens_a_list_item(line: &str) -> bool {
    line.starts_with("- ") || line.starts_with("* ")
}

/// Drop the separator an author writes between a marker and its text on the
/// same line, so [`render_body`] can put back exactly one.
fn strip_separator(text: &str) -> &str {
    let text = text.trim();
    for separator in ["—", "–", ":", "-"] {
        if let Some(rest) = text.strip_prefix(separator) {
            return rest.trim();
        }
    }
    text
}

/// Does `line` look like the beginning of an entry — a heading, or a line
/// opening with `**token**`?
///
/// Only ever asked of the line immediately after a blank one, where anything
/// of that shape is an entry and nothing else can be. Asking it of a line
/// *inside* a paragraph is what truncated a description at `**410**`; that
/// question is [`interrupts_the_entry`]'s, and it has a stricter answer.
fn begins_an_entry(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with('#') || bold_token(trimmed).is_some()
}

/// Does `line`, reached mid-paragraph with no blank line before it, end the
/// entry being read?
///
/// Only two things do: a Markdown heading, and the marker of an operation the
/// plan actually **declares**. Every other bold token is prose — a status
/// code, a constant, an emphasised word — and treating it as a boundary drops
/// the rest of the description on the floor. The plan's own Operations section
/// is the authority on which ids exist, so this asks it instead of guessing
/// from the shape of the text.
fn interrupts_the_entry(line: &str, declared: &BTreeSet<&str>) -> bool {
    let trimmed = line.trim();
    if trimmed.starts_with('#') {
        return true;
    }
    bold_token(trimmed).is_some_and(|token| declared.contains(token))
}

/// The `token` in a line opening with `**token**`, when there is one and it
/// holds no whitespace — the shape every operation marker has.
fn bold_token(trimmed: &str) -> Option<&str> {
    let (token, _) = trimmed.strip_prefix("**")?.split_once("**")?;
    (!token.is_empty() && !token.contains(char::is_whitespace)).then_some(token)
}

/// Assemble the prompt body once every operation has been checked and its
/// text resolved.
fn render_body(
    workstream: &WorkstreamDef,
    details: &[(&str, String)],
    replies: &[String],
) -> String {
    let mut out = String::new();

    out.push_str(&format!(
        "# Executor instructions — workstream `{}`\n\n{}\n\n",
        workstream.id, workstream.title
    ));
    out.push_str(
        "You are one of several workstreams this plan's execution board runs in \
         parallel. Other workstreams are editing their own files in this repository \
         at the same time. Coordinate only through the operations and write contract \
         below — never by assuming you are the only agent at work.\n\n",
    );

    out.push_str("## Operations you own\n\n");
    for id in &workstream.operations {
        out.push_str(&format!("- {id}\n"));
    }
    out.push('\n');

    out.push_str("## Operation details\n\n");
    for (id, text) in details {
        out.push_str(&format!("**{id}** — {text}\n\n"));
    }

    out.push_str("## Write contract — hard boundary\n\n");
    out.push_str(
        "You may create or modify only the paths listed below. Touching any other \
         file violates the write contract. If you believe another file must change, \
         stop and report it rather than editing it.\n\n",
    );
    for path in &workstream.write_contract {
        out.push_str(&format!("- {path}\n"));
    }

    if !replies.is_empty() {
        out.push_str("\n## Answers from the human\n\n");
        out.push_str(
            "An earlier run of this workstream stopped to ask, and a human answered. \
             These answers are part of your instructions: read them before you start, \
             act on them, and do not ask the same question again.\n\n",
        );
        for (index, reply) in replies.iter().enumerate() {
            out.push_str(&format!("{}. {reply}\n", index + 1));
        }
    }

    out
}

#[cfg(test)]
#[path = "../../../tests/unit/action/execute/prompt.rs"]
mod tests;
