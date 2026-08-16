//! Parse a plan.md's `## Operations` section into per-workstream operation
//! lists and write contracts.
//!
//! # Why this is its own module
//!
//! `operations_from_plan` was originally private to
//! [`super::replan`], the only verb that needed it. The executor prompt
//! renderer ([`super::prompt`]) needs the same parse — which workstream owns
//! which operations, per the plan's own Operations section — to know what to
//! put in an agent's hands. Copy-pasting the parser into a second module
//! would let the two forks drift on what counts as a heading, a bullet, or
//! the write-contract marker, and the format has a sharp edge (the section
//! never ends once entered — see `replan`'s module doc) that is exactly the
//! kind of behaviour a fork silently loses. One parser, two callers.
//!
//! The relocation started as a pure move of `operations_from_plan` out of
//! `replan.rs`. [`operation_text`] and its helpers joined it for the same
//! reason, moved out of `prompt.rs`: they parse the plan's `## Operation
//! details` section the same way `operations_from_plan` parses `##
//! Operations`, and a prompt renderer that also carries ~130 lines of
//! Markdown parsing is a second fork of this module's own argument for
//! existing — two parsers that can silently disagree on what counts as a
//! heading, a bullet, or an entry boundary. One parser, every caller.

use std::collections::BTreeSet;

use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction};

/// One workstream's Operations as authored in a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlanWorkstream {
    /// The workstream's id — the subheading text under `Operations`.
    pub(crate) id: String,
    /// The operations, in order.
    pub(crate) operations: Vec<String>,
    /// The paths the workstream may touch.
    pub(crate) write_contract: Vec<String>,
    /// The provider named in the plan, if any — `None` means the plan does
    /// not pin one (a caller session default may supply it at prepare).
    pub(crate) provider: Option<Provider>,
    /// The model selector named in the plan, if any.
    pub(crate) model: Option<String>,
    /// The agent selector named in the plan, if any.
    pub(crate) agent: Option<String>,
}

/// Parse `text`'s Operations section. See [`super::replan`]'s module doc
/// comment for the exact format; a plan without an Operations section yields
/// an empty list, which makes every board workstream affected — the
/// conservative answer when the new plan carries no operations at all.
///
/// Scalar targeting lines (`provider:`, `model:`, `agent:`) may appear in a
/// workstream block alongside the operation bullets. An unknown `provider:`
/// id is refused naming the valid options; an empty `model:`/`agent:` value
/// is refused rather than dropped silently.
pub(crate) fn operations_from_plan(text: &str) -> Result<Vec<PlanWorkstream>, Failure> {
    let mut workstreams = Vec::new();
    let mut in_operations = false;
    let mut collecting_write_contract = false;
    let mut current: Option<PlanWorkstream> = None;

    for line in text.lines() {
        let trimmed = line.trim();

        if let Some(heading) = trimmed.strip_prefix('#') {
            let title = heading.trim_start_matches('#').trim();
            if title.eq_ignore_ascii_case("operations") {
                // The section (re)starts; whatever workstream was open ends.
                if let Some(workstream) = current.take() {
                    workstreams.push(workstream);
                }
                in_operations = true;
                collecting_write_contract = false;
                continue;
            }
            if !in_operations {
                continue;
            }
            // Any other heading inside the section starts a new workstream,
            // named by the heading text.
            if let Some(workstream) = current.take() {
                workstreams.push(workstream);
            }
            current = Some(PlanWorkstream {
                id: title.to_owned(),
                operations: Vec::new(),
                write_contract: Vec::new(),
                provider: None,
                model: None,
                agent: None,
            });
            collecting_write_contract = false;
            continue;
        }

        if !in_operations {
            continue;
        }
        let Some(workstream) = current.as_mut() else {
            continue;
        };
        if let Some(value) = trimmed.strip_prefix("provider:") {
            let value = value.trim();
            let provider = value.parse::<Provider>().map_err(|_| {
                Failure::blocked(
                    "execute.plan_provider_invalid",
                    format!(
                        "workstream `{}` names unknown provider `{value}`; \
                         valid providers are `claude-code` and `opencode`",
                        workstream.id
                    ),
                )
                .expected("`claude-code` or `opencode`")
                .actual(value)
                .fix(FixAction::safe(
                    "execute.plan_provider_fix",
                    "Set `provider:` to `claude-code` or `opencode`.",
                ))
            })?;
            workstream.provider = Some(provider);
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("model:") {
            workstream.model = non_empty_target("model", &workstream.id, value)?;
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("agent:") {
            workstream.agent = non_empty_target("agent", &workstream.id, value)?;
            continue;
        }
        if trimmed == "write_contract:" {
            collecting_write_contract = true;
            continue;
        }
        if let Some(bullet) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            let item = bullet.trim().to_owned();
            if collecting_write_contract {
                workstream.write_contract.push(item);
            } else {
                workstream.operations.push(item);
            }
        }
    }
    if let Some(workstream) = current {
        workstreams.push(workstream);
    }

    Ok(workstreams)
}

/// Refuse an empty `model:`/`agent:` value so a bare selector is an explicit
/// authoring mistake, not a silently dropped one.
fn non_empty_target(field: &str, workstream: &str, value: &str) -> Result<Option<String>, Failure> {
    let value = value.trim();
    if value.is_empty() {
        return Err(Failure::blocked(
            "execute.plan_target_empty",
            format!("workstream `{workstream}` has an empty `{field}:` value"),
        )
        .expected(format!(
            "a non-empty {field} selector, or no `{field}:` line"
        ))
        .actual("an empty value"));
    }
    Ok(Some(value.to_owned()))
}

/// One workstream's resolved targeting, ready to be written back into the
/// plan's `## Operations` section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedTarget {
    /// The workstream's id — the `### <id>` heading it targets.
    pub(crate) id: String,
    /// The provider the workstream will run on.
    pub(crate) provider: Provider,
    /// The model selector, if one was resolved.
    pub(crate) model: Option<String>,
    /// The agent selector, if one was resolved.
    pub(crate) agent: Option<String>,
}

/// Rewrite targeting metadata into `text`'s `## Operations` section.
///
/// Only a level-two `## Operations` heading opens the section, and only a
/// level-three `### <id>` heading whose text matches a target exactly is a
/// workstream block — prose headings of other levels are never rewritten.
/// For every target, the matching heading is followed by the resolved lines
/// in stable order — `provider`, `model`, `agent` — and any
/// `provider:`/`model:`/`agent:` lines already in that block are removed, so
/// the resolved value is the only one that survives. Everything else in the
/// plan is preserved byte for byte, and the file's final newline state is
/// kept. A target with no matching heading is refused with
/// `execute.plan_workstream_missing`.
pub(crate) fn write_targets(text: &str, targets: &[ResolvedTarget]) -> Result<String, Failure> {
    let mut out: Vec<String> = Vec::new();
    let mut in_operations = false;
    // Whether the current workstream block is being rewritten, so its own
    // targeting lines are dropped in favour of the resolved ones.
    let mut rewriting = false;
    let mut written: Vec<&str> = Vec::with_capacity(targets.len());

    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(title) = trimmed.strip_prefix("## ")
            && title.trim().eq_ignore_ascii_case("operations")
        {
            in_operations = true;
            rewriting = false;
            out.push(line.to_owned());
            continue;
        }
        if in_operations
            && let Some(title) = trimmed.strip_prefix("### ")
            && let Some(target) = targets.iter().find(|target| target.id == title.trim())
        {
            out.push(line.to_owned());
            push_target_lines(&mut out, target);
            written.push(target.id.as_str());
            rewriting = true;
            continue;
        }
        if trimmed.starts_with('#') {
            // Any other heading — including a `##` sibling or a `###` block
            // that is not a target — closes the current workstream block.
            rewriting = false;
        }

        if rewriting
            && (trimmed.starts_with("provider:")
                || trimmed.starts_with("model:")
                || trimmed.starts_with("agent:"))
        {
            // The resolved lines were already written after the heading; drop
            // the block's own copies.
            continue;
        }
        out.push(line.to_owned());
    }

    if let Some(missing) = targets
        .iter()
        .find(|target| !written.contains(&target.id.as_str()))
    {
        return Err(Failure::blocked(
            "execute.plan_workstream_missing",
            format!(
                "the plan has no `### {}` workstream heading to write targeting into",
                missing.id
            ),
        )
        .expected("a `### <workstream-id>` heading for every graph workstream")
        .actual(format!("no heading matches workstream `{}`", missing.id))
        .fix(FixAction::safe(
            "execute.fix_plan_or_graph",
            format!(
                "Add a `### {}` heading under the plan's Operations section, or remove the workstream from the graph.",
                missing.id
            ),
        )));
    }

    let mut result = out.join("\n");
    if text.ends_with('\n') {
        result.push('\n');
    }
    Ok(result)
}

/// Append a target's resolved lines, in stable order: provider, model, agent.
fn push_target_lines(out: &mut Vec<String>, target: &ResolvedTarget) {
    out.push(format!("provider: {}", target.provider));
    if let Some(model) = &target.model {
        out.push(format!("model: {model}"));
    }
    if let Some(agent) = &target.agent {
        out.push(format!("agent: {agent}"));
    }
}

/// Find `id`'s text in the plan's `## Operation details` section.
///
/// `None` means the plan has no `**<id>**` line at all. `Some(text)` means the
/// entry exists and `text` is what it says — **possibly empty**, which the
/// caller refuses; see [`super::prompt`]'s `empty_operation`. The two are kept
/// apart because they are different authoring mistakes and deserve different
/// refusals.
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
/// put after it. `render_body`, in `super::prompt`, writes `**<id>** —
/// <text>`, so leaving the marker in returns it twice.
///
/// # What the text keeps
///
/// [`assemble`] rebuilds the entry the way the author wrote it: a wrapped
/// paragraph is unwrapped onto one line, a blank line stays a paragraph break,
/// and a list item keeps its own line. Joining every line with a space — which
/// is what unwrapping alone does — turns a `tests` / `doneWhen` block into one
/// run-on sentence, so carrying the block and flattening it would trade a
/// silent drop for an unreadable prompt.
pub(super) fn operation_text(
    plan_text: &str,
    id: &str,
    declared: &BTreeSet<&str>,
) -> Option<String> {
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
/// comes back empty — which is what `super::prompt`'s `empty_operation`
/// refuses.
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
/// unwrapped into the line above it? The bullet markers are the same two
/// [`operations_from_plan`] strips a bullet on, so the two parses agree on
/// what a bullet is.
fn opens_a_list_item(line: &str) -> bool {
    line.starts_with("- ") || line.starts_with("* ")
}

/// Drop the separator an author writes between a marker and its text on the
/// same line, so `render_body` (in `super::prompt`) can put back exactly one.
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

#[cfg(test)]
#[path = "../../../tests/unit/action/execute/plan_ops.rs"]
mod tests;
