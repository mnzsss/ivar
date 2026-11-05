//! Consolidates closed provider conversations into `session` pages.

pub mod attribute;
pub mod render;
pub mod state;

use std::collections::BTreeSet;

use crate::action::memory::drain::drain_locked;
use crate::action::memory::mcp::pages::publish;
use crate::action::memory::reindex;
use crate::domain::memory_page::PageMeta;
use crate::domain::session::{epoch_secs_from_rfc3339, rfc3339_now};
use crate::error::Failure;
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::memory::conversations::{
    ConversationHead, ConversationRow, conversation_heads, conversation_rows, unattached_rows,
};
use crate::store::memory::db::MemoryDb;
use crate::store::memory::page_source::parse_page;
use crate::store::memory::retention::{SESSION_RETENTION_DAYS, cutoff_before};

use attribute::{IDLE_SECS, attribute};
use render::{Conversation, render};
use state::{ConsolidateState, Marker};

pub const PAGE_TTL_DAYS: u64 = SESSION_RETENTION_DAYS;
#[allow(clippy::cast_possible_wrap)]
const PAGE_TTL_SECS: i64 = PAGE_TTL_DAYS as i64 * 86_400;

#[derive(Debug, Clone, Copy)]
pub enum Scope<'a> {
    Idle,
    Session(&'a str),
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ConsolidateReport {
    pub written: usize,
    pub removed: usize,
    pub busy: bool,
}

/// Consolidates closed conversations, or reports `busy` instead of waiting
/// when another process holds `memory.lock`. Never creates `memory.db`.
///
/// # Errors
///
/// When the lock, the database or a page cannot be used.
pub fn consolidate_if_free(
    layout: &Layout,
    now_epoch_secs: u64,
) -> Result<ConsolidateReport, Failure> {
    if !layout.memory_db().exists() {
        return Ok(ConsolidateReport::default());
    }
    let Some(_lock) = fs::try_lock_exclusive(&layout.memory_lock())? else {
        return Ok(ConsolidateReport {
            busy: true,
            ..ConsolidateReport::default()
        });
    };
    let mut db = open(layout)?;
    consolidate_locked(layout, &mut db, now_epoch_secs, Scope::Idle)
}

/// Drains the spool and consolidates every conversation of `session_id`,
/// open or not, waiting for `memory.lock`. Never creates `memory.db`.
///
/// # Errors
///
/// When the lock, the database or a page cannot be used.
pub fn consolidate_session(
    layout: &Layout,
    session_id: &str,
    now_epoch_secs: u64,
) -> Result<ConsolidateReport, Failure> {
    if !layout.memory_db().exists() {
        return Ok(ConsolidateReport::default());
    }
    let _lock = fs::lock_exclusive(&layout.memory_lock())?;
    let _ = drain_locked(layout, usize::MAX);
    let mut db = open(layout)?;
    consolidate_locked(layout, &mut db, now_epoch_secs, Scope::Session(session_id))
}

#[must_use]
pub fn is_expired(meta: &PageMeta, last_at_epoch: i64, now_epoch_secs: i64) -> bool {
    if let Some(at) = meta.expires_at.as_deref().and_then(epoch_secs_from_rfc3339) {
        return at <= now_epoch_secs;
    }
    !meta.pinned && now_epoch_secs - last_at_epoch >= PAGE_TTL_SECS
}

/// The caller must hold `memory.lock`; the lock is not re-entrant.
///
/// # Errors
///
/// When the database or a page cannot be used; the error is recorded in
/// `_consolidate.json`.
pub fn consolidate_locked(
    layout: &Layout,
    db: &mut MemoryDb,
    now_epoch_secs: u64,
    scope: Scope<'_>,
) -> Result<ConsolidateReport, Failure> {
    let mut state = state::read_state(layout);
    match run(layout, db, now_epoch_secs, scope, &mut state) {
        Ok(report) => {
            state.last_run_at = Some(rfc3339_now());
            state.last_error = None;
            state.written_total += u64::try_from(report.written).unwrap_or(u64::MAX);
            state::write_state(layout, &state)
                .map_err(|error| Failure::failed("memory.consolidate_failed", error))?;
            Ok(report)
        }
        Err(error) => {
            state.last_error = Some(error.clone());
            let _ = state::write_state(layout, &state);
            Err(Failure::failed("memory.consolidate_failed", error))
        }
    }
}

fn open(layout: &Layout) -> Result<MemoryDb, Failure> {
    MemoryDb::open(layout.memory_db().as_std_path())
        .map_err(|error| Failure::failed("memory.consolidate_failed", error.to_string()))
}

fn epoch(at: &str) -> i64 {
    epoch_secs_from_rfc3339(at).unwrap_or(0)
}

fn run(
    layout: &Layout,
    db: &mut MemoryDb,
    now_epoch_secs: u64,
    scope: Scope<'_>,
    state: &mut ConsolidateState,
) -> Result<ConsolidateReport, String> {
    let sql = |error: rusqlite::Error| error.to_string();
    let since = cutoff_before(now_epoch_secs, PAGE_TTL_DAYS);
    let only = match scope {
        Scope::Session(id) => Some(id),
        Scope::Idle => None,
    };
    let heads = conversation_heads(db, &since, only).map_err(sql)?;
    let attached = attach(db, &heads).map_err(sql)?;
    let now = i64::try_from(now_epoch_secs).unwrap_or(i64::MAX);
    let mut report = ConsolidateReport::default();
    for (head, extra) in heads.iter().zip(&attached) {
        let newest = extra
            .iter()
            .map(|row| row.id)
            .max()
            .unwrap_or(0)
            .max(head.last_id);
        let stem = attribute::page_stem(&head.native_session_id);
        let seen = state
            .markers
            .get(&stem)
            .is_some_and(|marker| marker.last_id >= newest);
        let closed = matches!(scope, Scope::Session(_))
            || head.ended
            || now - epoch(&head.last_at) >= IDLE_SECS;
        if seen || !closed {
            continue;
        }
        let rows = conversation_rows(db, &head.native_session_id).map_err(sql)?;
        let rel = attribute::page_rel(&head.native_session_id);
        let path = layout.root().join(&rel);
        let current = std::fs::read_to_string(&path).ok();
        let preserved = current
            .as_deref()
            .and_then(|text| parse_page(&rel, text).ok())
            .map(|page| page.meta)
            .unwrap_or_default();
        let conversation = Conversation {
            head,
            rows: &rows,
            attached: extra,
        };
        match render(&conversation, &preserved) {
            Some(content) if current.as_deref() != Some(content.as_str()) => {
                publish(&path, &rel, &content, true)?;
                report.written += 1;
            }
            None if current.is_some() => {
                std::fs::remove_file(&path).map_err(|error| error.to_string())?;
                report.removed += 1;
            }
            _ => {}
        }
        state.markers.insert(
            stem,
            Marker {
                last_id: newest,
                last_at: head.last_at.clone(),
                session_id: head.session_id.clone(),
            },
        );
    }
    state.markers.retain(|_, marker| marker.last_at >= since);
    if report.written + report.removed > 0 {
        reindex::refresh_locked(layout, db, false).map_err(|failure| failure.what)?;
    }
    Ok(report)
}

fn attach(
    db: &MemoryDb,
    heads: &[ConversationHead],
) -> rusqlite::Result<Vec<Vec<ConversationRow>>> {
    let mut attached = vec![Vec::new(); heads.len()];
    let sessions: BTreeSet<&str> = heads
        .iter()
        .map(|head| head.session_id.as_str())
        .filter(|id| !id.is_empty())
        .collect();
    for session in sessions {
        let members: Vec<usize> = heads
            .iter()
            .enumerate()
            .filter(|(_, head)| head.session_id == session)
            .map(|(index, _)| index)
            .collect();
        let windows: Vec<(i64, i64)> = members
            .iter()
            .filter_map(|&index| heads.get(index))
            .map(|head| (epoch(&head.first_at), epoch(&head.last_at)))
            .collect();
        for row in unattached_rows(db, session)? {
            let slot =
                attribute(&windows, epoch(&row.occurred_at)).and_then(|slot| members.get(slot));
            if let Some(rows) = slot.and_then(|&index| attached.get_mut(index)) {
                rows.push(row);
            }
        }
    }
    Ok(attached)
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/memory/consolidate.rs"]
mod tests;

#[cfg(test)]
#[path = "../../../../tests/unit/action/memory/consolidate_reads.rs"]
mod reads_tests;
