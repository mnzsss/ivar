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
//! the presence of the *text* is. See [`super::plan_ops::operation_text`].
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

use super::plan_ops::{self, operations_from_plan};

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
    let plan_workstreams = operations_from_plan(plan_text)?;
    let owned = plan_workstreams
        .iter()
        .find(|entry| entry.id == workstream.id);
    // Every operation id the plan declares, whoever owns it: the only text
    // that reliably marks where one entry in Operation details ends and the
    // next begins. See [`super::plan_ops::operation_text`]'s "Where an entry
    // ends".
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
        let text = plan_ops::operation_text(plan_text, id, &declared).ok_or_else(|| {
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
        "You are an executor, not the feature coordinator. If you discover an \
         isolatable request outside the approved operations, stop and report \
         it. Do not create, reparent, promote, integrate, close, delete, or \
         otherwise mutate hall feature state; the coordinator creates the \
         child feature and announces it.\n\n",
    );
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
