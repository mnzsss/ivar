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
//! # Signature
//!
//! [`render`] takes the plan text and a borrowed [`WorkstreamDef`] — nothing
//! else. A caller that already read the plan and already holds the
//! workstream (e.g. from a loaded [`crate::domain::feature::ExecutionBoard`])
//! renders without touching the filesystem again.

use crate::domain::feature::WorkstreamDef;
use crate::error::{Failure, FixAction};

use super::plan_ops::operations_from_plan;

/// Render the executor prompt for `workstream`, using `plan_text` as the
/// source of truth for operation ownership and operation text.
///
/// Blocked when `workstream` claims an operation id that the plan does not
/// back: either the id is absent from this workstream's own heading in the
/// plan's `## Operations` section, or it has no `**<id>**` entry in
/// `## Operation details`.
pub fn render(plan_text: &str, workstream: &WorkstreamDef) -> Result<String, Failure> {
    let plan_workstreams = operations_from_plan(plan_text);
    let owned = plan_workstreams
        .iter()
        .find(|entry| entry.id == workstream.id);

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
        let text = operation_text(plan_text, id).ok_or_else(|| {
            missing_operation(
                workstream,
                id,
                format!("no `**{id}**` entry in the plan's Operation details"),
            )
        })?;
        details.push((id.as_str(), text));
    }

    Ok(render_body(workstream, &details))
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

/// Find `id`'s verbatim paragraph in the plan's `## Operation details`
/// section: a line starting with `**<id>**`, followed by any immediately
/// continuing (non-blank) lines, joined back into one line the way Markdown
/// reflows a wrapped paragraph.
fn operation_text(plan_text: &str, id: &str) -> Option<String> {
    let marker = format!("**{id}**");
    let mut lines = plan_text.lines().peekable();
    while let Some(line) = lines.next() {
        if !line.trim_start().starts_with(&marker) {
            continue;
        }
        let mut paragraph = vec![line.trim().to_owned()];
        while let Some(next) = lines.peek() {
            if next.trim().is_empty() {
                break;
            }
            paragraph.push(next.trim().to_owned());
            lines.next();
        }
        return Some(paragraph.join(" "));
    }
    None
}

/// Assemble the prompt body once every operation has been checked and its
/// text resolved.
fn render_body(workstream: &WorkstreamDef, details: &[(&str, String)]) -> String {
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

    out
}

#[cfg(test)]
#[path = "../../../tests/unit/action/execute/prompt.rs"]
mod tests;
