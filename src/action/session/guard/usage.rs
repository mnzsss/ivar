//! Graph-call and search-miss recording from guard hook payloads.

use crate::domain::graph::{MissEvent, MissKind, UsageEvent, UsageSource};
use crate::store::layout::Layout;
use camino::Utf8Path;

/// Claude Code spells MCP tools `mcp__<server>__<tool>`; OpenCode and OMP
/// join the server (named `…graph`) and tool with a single `_`.
pub(super) fn is_graph_explore_tool(tool: &str) -> bool {
    tool == "graph_explore"
        || tool.ends_with("__graph_explore")
        || tool.ends_with("-graph_graph_explore")
        || tool.ends_with("_graph_graph_explore")
}

/// The MCP server runs once per hall and cannot tell which session called
/// it; the hook's payload cwd can, so the hook stamps the session.
pub(super) fn record_graph_call_at(
    cwd: &Utf8Path,
    session_env: Option<&crate::action::session::env::SessionEnv>,
    ambient_session: Option<String>,
) {
    let Some(session) =
        crate::action::graph::session::session_key_for(session_env, ambient_session)
    else {
        return;
    };
    let layout = match session_env {
        Some(env) => Some(Layout::at(env.hall.clone())),
        None => Layout::discover(cwd).ok().flatten(),
    };
    let Some(layout) = layout else { return };
    let db_path = layout.ivar_dir().join("memory.db");
    let Ok(db) = crate::store::graph::db::GraphDb::open_for_usage(db_path.as_std_path()) else {
        return;
    };
    let _ = db.record_usage(&UsageEvent {
        command: "graph_explore".to_owned(),
        source: UsageSource::Hook,
        duration_ms: 0,
        result_count: None,
        error: false,
        session: Some(session),
        query: None,
    });
}

/// How long after a graph call a search counts as a follow-up rather than an
/// unrelated later search.
pub(super) const FOLLOWUP_WINDOW_SECS: i64 = 120;

pub(super) fn record_search_miss_at(
    cwd: &Utf8Path,
    session_env: Option<&crate::action::session::env::SessionEnv>,
    ambient_session: Option<String>,
    pattern: &str,
) {
    let Some(session) =
        crate::action::graph::session::session_key_for(session_env, ambient_session)
    else {
        return;
    };
    let layout = match session_env {
        Some(env) => Some(Layout::at(env.hall.clone())),
        None => Layout::discover(cwd).ok().flatten(),
    };
    if let Some(layout) = layout {
        record_search_miss(&layout, &session, pattern);
    }
}

/// Best-effort classification of one search-tool call as `skipped` or
/// `followup`. Every failure is swallowed: the guard's decision is already
/// made, and nothing here may change it or its exit code.
pub(super) fn record_search_miss(layout: &Layout, session: &str, pattern: &str) {
    let db_path = layout.ivar_dir().join("memory.db");
    if !db_path.exists() {
        return;
    }
    let Ok(db) = crate::store::graph::db::GraphDb::open_for_usage(db_path.as_std_path()) else {
        return;
    };

    let miss = |kind, query| MissEvent {
        session: Some(session.to_owned()),
        kind,
        query,
        pattern: Some(pattern.to_owned()),
        reason: None,
    };
    let _ = match db.last_graph_call(session) {
        Ok(None) => db.record_miss(&miss(MissKind::Skipped, None)),
        Ok(Some(call))
            if crate::store::graph::db::types::now_timestamp() - call.ts
                <= FOLLOWUP_WINDOW_SECS
                && matches!(db.has_followup_for(&call), Ok(false)) =>
        {
            db.record_followup(&miss(MissKind::Followup, call.query.clone()), &call)
        }
        _ => Ok(()),
    };
}
