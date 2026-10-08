//! The session write guard: determines which files a session may write.
//!
pub use crate::domain::guard::{GuardDecision, GuardOutcome, ToolRequest};
use crate::domain::provider::Provider;
use crate::error::Failure;
use crate::store::layout::Layout;
use camino::Utf8PathBuf;

mod decide;
mod set;
mod target;
mod usage;

use decide::*;
pub(crate) use decide::{Resolution, decide};
pub(crate) use set::{WritableSet, canonicalize_lenient};
use target::*;
use usage::*;

/// What the guard worked out for one tool call, before any side effect.
struct Evaluation {
    request: ToolRequest,
    cwd: Option<Utf8PathBuf>,
    session_env: Option<crate::action::session::env::SessionEnv>,
    decision: GuardDecision,
}

/// Run the guard: parse stdin JSON, resolve the session, decide, attach the
/// repository instructions to an allow, and shape the output for the
/// provider.
///
/// `slice` is set only by Claude Code's extra hook entries. They return that
/// slice of the instructions and never a decision, and exit 0 whatever the
/// input: the decision belongs to the entry without `slice`.
///
/// # Errors
///
/// Without `slice`, returns [`Failure`] if `stdin_json` is not a valid hook
/// payload for `provider`.
pub fn guard(
    provider: Provider,
    stdin_json: &str,
    slice: Option<usize>,
) -> Result<GuardOutcome, Failure> {
    let ambient = std::env::var("IVAR_SESSION_ID").ok();
    if let Some(index) = slice {
        let context = evaluate(provider, stdin_json, ambient.as_deref())
            .ok()
            .and_then(|evaluation| instructions_for(provider, &evaluation, index));
        return Ok(crate::providers::render_context(
            provider,
            context.as_deref(),
        ));
    }

    let evaluation = evaluate(provider, stdin_json, ambient.as_deref())?;
    if matches!(evaluation.decision, GuardDecision::Allow)
        && is_graph_explore_tool(&evaluation.request.tool)
        && let Some(cwd) = evaluation.cwd.as_deref()
    {
        record_graph_call_at(cwd, evaluation.session_env.as_ref(), ambient.clone());
    }
    if let Some(pattern) = &evaluation.request.search_pattern
        && let Some(cwd) = evaluation.cwd.as_deref()
    {
        record_search_miss_at(cwd, evaluation.session_env.as_ref(), ambient, pattern);
    }

    let context = instructions_for(provider, &evaluation, 0);
    Ok(crate::providers::render_decision(
        provider,
        &evaluation.decision,
        context.as_deref(),
    ))
}

/// Parse, resolve and decide, with no side effect, so the slice entries can
/// share it without recording graph calls or search misses again. `ambient`
/// is the agent's `IVAR_SESSION_ID`, passed in so tests never touch process
/// env: it names the agent's own session when the cwd is outside its view dir
/// (see `SessionEnv::resolve_for_agent`), unless a write's target lies in a
/// live session's view dir (see `SessionEnv::resolve_for_write`).
fn evaluate(
    provider: Provider,
    stdin_json: &str,
    ambient: Option<&str>,
) -> Result<Evaluation, Failure> {
    let (tool_request, cwd) = crate::providers::parse_tool_request(provider, stdin_json)?;

    let targets: Vec<Utf8PathBuf> = tool_request
        .targets
        .iter()
        .filter_map(|t| resolve_target(cwd.as_deref(), t))
        .collect();

    // A write names the session it lands in: from a cwd outside every view
    // dir, the session owning the first absolute target beats the ambient
    // id. Reads, and relative targets, keep the agent's own session.
    let owner_target = tool_request
        .writes
        .then(|| tool_request.targets.iter().find(|t| t.is_absolute()))
        .flatten()
        .and_then(|t| resolve_target(cwd.as_deref(), t));

    let session_env = cwd
        .as_deref()
        .and_then(|cwd| {
            use crate::action::session::env::SessionEnv;
            match owner_target.as_deref() {
                Some(target) => SessionEnv::resolve_for_write(cwd, target, ambient).ok(),
                None => SessionEnv::resolve_for_agent(cwd, ambient).ok(),
            }
        })
        .flatten();
    let mut set = session_env.as_ref().and_then(resolve_writable_set);

    let mut ambiguous_features = None;

    if set.is_none()
        && tool_request.writes
        && let Some(first_abs) = tool_request.targets.iter().find(|t| t.is_absolute())
        && let Some(target_path) = resolve_target(cwd.as_deref(), first_abs)
    {
        match resolve_set_by_target(&target_path) {
            TargetResolution::Unique(s) | TargetResolution::SharedHall(s) => {
                set = Some(s);
            }
            TargetResolution::Ambiguous(features) => {
                ambiguous_features = Some(features);
            }
            TargetResolution::None => {}
        }
    }

    let mut relative_denial = None;
    if set.is_none()
        && tool_request.writes
        && let Some(first_rel) = tool_request
            .targets
            .iter()
            .find(|t| !t.is_absolute() && !has_uri_scheme(t.as_str()))
    {
        relative_denial = Some(relative_no_session_reason(first_rel, cwd.as_deref()));
    }

    let resolution = match (&set, ambiguous_features) {
        (Some(set), _) => Resolution::Resolved(set),
        (None, Some(features)) => Resolution::Ambiguous { features },
        (None, None) => {
            let (scoped, live_count) = match cwd
                .as_deref()
                .and_then(|c| Layout::discover(c).ok().flatten())
            {
                Some(layout) => {
                    let live = super::lookup::list_all(&layout)
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|s| s.state.is_some())
                        .count();
                    let scoped = targets
                        .first()
                        .map(|t| feature_scratch_dirs(&layout, t))
                        .unwrap_or_default();
                    (scoped, live)
                }
                None => (Vec::new(), 0),
            };
            Resolution::Unresolved {
                scoped_scratch_dirs: scoped,
                live_count,
            }
        }
    };

    let decision = if let Some(reason) = relative_denial {
        GuardDecision::Deny { reason }
    } else {
        decide(&resolution, &tool_request, &targets)
    };
    Ok(Evaluation {
        request: tool_request,
        cwd,
        session_env,
        decision,
    })
}

/// The repository instructions this call carries: only on an allow, only
/// inside a resolved session, and never an error. A failure here must not
/// change or block the decision (R-FAIL-OPEN), so every error becomes `None`.
fn instructions_for(provider: Provider, evaluation: &Evaluation, index: usize) -> Option<String> {
    if !matches!(evaluation.decision, GuardDecision::Allow) {
        return None;
    }
    let env = evaluation.session_env.as_ref()?;
    let cwd = evaluation.cwd.as_deref()?;
    let request = &evaluation.request;
    let paths = super::instructions::touched_paths(&request.input, cwd);
    let agent = request.agent.as_deref().unwrap_or("main");
    let delivered = match (provider, request.call_id.as_deref()) {
        (Provider::ClaudeCode, Some(call_id)) => super::instructions::deliver_slice(
            &env.view_dir,
            provider,
            agent,
            call_id,
            &paths,
            index,
        ),
        _ if index == 0 => super::instructions::deliver(&env.view_dir, provider, agent, &paths),
        _ => Ok(None),
    };
    delivered.ok().flatten()
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/session/guard/mod.rs"]
mod tests;
