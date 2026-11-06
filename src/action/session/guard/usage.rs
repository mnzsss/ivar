//! Graph-call and search-miss recording from guard hook payloads.

use crate::action::session::env::SessionEnv;
use crate::domain::graph::truncate_for_storage;
use crate::domain::graph::{MissEvent, MissKind, UsageEvent, UsageSource};
use crate::domain::guard::ToolRequest;
use crate::store::layout::Layout;
use crate::store::memory::usage::{self, MemoryMiss, MemoryMissKind, QueryRecord};
use camino::{Utf8Path, Utf8PathBuf};

const IVAR_SERVER_SUFFIXES: [&str; 2] = ["-ivar_graph_explore", "_ivar_graph_explore"];
pub(super) const LEGACY_SERVER_SUFFIXES: [&str; 2] =
    ["-graph_graph_explore", "_graph_graph_explore"];

/// Claude Code spells MCP tools `mcp__<server>__<tool>`; OpenCode and OMP
/// join the server (named `…ivar`, formerly `…graph`) and tool with a single `_`.
pub(super) fn is_graph_explore_tool(tool: &str) -> bool {
    tool == "graph_explore"
        || tool.ends_with("__graph_explore")
        || IVAR_SERVER_SUFFIXES
            .iter()
            .chain(&LEGACY_SERVER_SUFFIXES)
            .any(|suffix| tool.ends_with(suffix))
}

/// The MCP server runs once per hall and cannot tell which session called
/// it; the hook's payload cwd can, so the hook stamps the session.
pub(super) fn record_graph_call_at(
    cwd: &Utf8Path,
    session_env: Option<&crate::action::session::env::SessionEnv>,
    ambient_session: Option<String>,
) {
    let Some((layout, session)) = session_layout(cwd, session_env, ambient_session) else {
        return;
    };
    let db_path = layout.graph_db();
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
    if let Some((layout, session)) = session_layout(cwd, session_env, ambient_session) {
        record_search_miss(&layout, &session, pattern);
    }
}

/// Best-effort classification of one search-tool call as `skipped` or
/// `followup`. Every failure is swallowed: the guard's decision is already
/// made, and nothing here may change it or its exit code.
pub(super) fn record_search_miss(layout: &Layout, session: &str, pattern: &str) {
    let db_path = layout.graph_db();
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

fn session_layout(
    cwd: &Utf8Path,
    session_env: Option<&SessionEnv>,
    ambient_session: Option<String>,
) -> Option<(Layout, String)> {
    let session = crate::action::graph::session::session_key_for(session_env, ambient_session)?;
    let layout = match session_env {
        Some(env) => Layout::at(env.hall.clone()),
        None => Layout::discover(cwd).ok().flatten()?,
    };
    Some((layout, session))
}

const MEMORY_QUERY_SUFFIXES: [&str; 3] =
    ["__memory_query", "-ivar_memory_query", "_ivar_memory_query"];
const MEMORY_DIRS: [&str; 2] = ["docs/memory", ".ivar/memory"];

pub(super) fn is_memory_query_tool(tool: &str) -> bool {
    tool == "memory_query"
        || MEMORY_QUERY_SUFFIXES
            .iter()
            .any(|suffix| tool.ends_with(suffix))
}

pub(super) fn memory_read_pattern(request: &ToolRequest, cwd: &Utf8Path) -> Option<String> {
    if request.writes {
        return None;
    }
    let path_arg = request
        .input
        .get("path")
        .and_then(serde_json::Value::as_str)
        .map(Utf8PathBuf::from);
    request
        .targets
        .iter()
        .cloned()
        .chain(path_arg)
        .map(|p| if p.is_absolute() { p } else { cwd.join(p) })
        .map(|p| p.to_string())
        .chain(request.search_pattern.clone())
        .find(|candidate| MEMORY_DIRS.iter().any(|dir| under_dir(candidate, dir)))
}

fn under_dir(candidate: &str, dir: &str) -> bool {
    candidate == dir
        || candidate.starts_with(&format!("{dir}/"))
        || candidate.contains(&format!("/{dir}/"))
        || candidate.ends_with(&format!("/{dir}"))
}

/// Never creates the file and gives up on a busy database after 50 ms: the
/// guard's decision and latency must not depend on recording.
fn open_memory(layout: &Layout) -> Option<rusqlite::Connection> {
    let conn = rusqlite::Connection::open_with_flags(
        layout.memory_db().as_std_path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    conn.busy_timeout(std::time::Duration::from_millis(50))
        .ok()?;
    Some(conn)
}

pub(super) fn record_memory_call_at(
    cwd: &Utf8Path,
    session_env: Option<&SessionEnv>,
    ambient_session: Option<String>,
    query: Option<&str>,
) {
    let Some((layout, session)) = session_layout(cwd, session_env, ambient_session) else {
        return;
    };
    let Some(conn) = open_memory(&layout) else {
        return;
    };
    let _ = usage::record_query(
        &conn,
        &QueryRecord {
            source: "hook".into(),
            session: Some(session),
            query: query.map(truncate_for_storage).unwrap_or_default(),
            filters: "{}".into(),
            page_count: 0,
            observation_count: 0,
            duration_ms: 0,
            error: false,
        },
    );
}

pub(super) fn record_memory_read_at(
    cwd: &Utf8Path,
    session_env: Option<&SessionEnv>,
    ambient_session: Option<String>,
    pattern: &str,
) {
    let Some((layout, session)) = session_layout(cwd, session_env, ambient_session) else {
        return;
    };
    let Some(conn) = open_memory(&layout) else {
        return;
    };
    let miss = |kind, usage_id, query| MemoryMiss {
        kind,
        session: Some(session.clone()),
        query,
        filters: None,
        pattern: Some(truncate_for_storage(pattern)),
        usage_id,
        fallback: false,
    };
    let _ = match usage::last_query(&conn, &session) {
        Ok(Some(last))
            if crate::store::graph::db::types::now_timestamp() - last.ts
                <= FOLLOWUP_WINDOW_SECS =>
        {
            if matches!(usage::has_followup_for(&conn, last.usage_id), Ok(false)) {
                usage::record_miss(
                    &conn,
                    &miss(MemoryMissKind::Followup, Some(last.usage_id), last.query),
                )
            } else {
                Ok(())
            }
        }
        Ok(_) => usage::record_miss(&conn, &miss(MemoryMissKind::Skipped, None, None)),
        Err(_) => Ok(()),
    };
}
