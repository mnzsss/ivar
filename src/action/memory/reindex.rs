//! Indexes `docs/memory/` and `.ivar/memory/` pages into `memory.db`. Pages are
//! files: the index is a cache that `rebuild` recreates, and `observations`
//! are never touched.

use std::collections::{BTreeMap, BTreeSet};
use std::io;

use rusqlite::Transaction;

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use crate::action::graph::split_failure;
use crate::action::{Ctx, discover_hall};
use crate::domain::memory_anchor::extract_candidates;
use crate::domain::memory_page::links::{extract_links, relation_links};
use crate::domain::session::rfc3339_now;
use crate::error::{Failure, Outcome, Report, WriteHuman};
use crate::infra::{fs, hash};
use crate::store::layout::Layout;
use crate::store::memory::anchors;
use crate::store::memory::db::MemoryDb;
use crate::store::memory::page_source::parse_page;
use crate::store::memory::pages::{self, IndexedFile};
use crate::store::memory::split::finish_legacy_move;

const STATE_FILE: &str = "_reindex.json";

#[derive(Debug, Default, Serialize)]
pub struct ReindexReport {
    pub indexed: usize,
    pub unchanged: usize,
    pub removed: usize,
    pub anchored: usize,
    pub invalid: Vec<String>,
}

impl WriteHuman for ReindexReport {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        write!(
            w,
            "Indexed {}, unchanged {}, removed {}",
            self.indexed, self.unchanged, self.removed
        )?;
        if !self.invalid.is_empty() {
            write!(w, ", skipped {} invalid", self.invalid.len())?;
        }
        writeln!(w)
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ReindexState {
    pub last_reindex_at: Option<String>,
    pub last_error: Option<String>,
    pub pages: usize,
}

#[derive(Debug, Clone)]
pub struct PageFile {
    pub rel: String,
    pub path: Utf8PathBuf,
}

#[must_use]
pub fn read_state(layout: &Layout) -> ReindexState {
    std::fs::read_to_string(layout.memory_spool_dir().join(STATE_FILE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn write_state(layout: &Layout, state: &ReindexState) {
    let dir = layout.memory_spool_dir();
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(bytes) = serde_json::to_vec_pretty(state) {
        let _ = fs::write_atomic(&dir.join(STATE_FILE), &bytes);
    }
}

fn collect(dir: &Utf8Path, found: &mut Vec<Utf8PathBuf>) {
    let Ok(entries) = dir.read_dir_utf8() else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.into_path();
        if path.is_dir() {
            collect(&path, found);
        } else if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
        {
            found.push(path);
        }
    }
}

/// Every Markdown file (any extension case) at any depth, so a nested file or an unknown kind
/// directory reaches validation instead of vanishing.
#[must_use]
pub fn walk_pages(layout: &Layout) -> Vec<PageFile> {
    let mut paths = Vec::new();
    collect(&layout.docs_memory_dir(), &mut paths);
    collect(&layout.ivar_dir().join("memory"), &mut paths);
    let mut files: Vec<PageFile> = paths
        .into_iter()
        .filter_map(|path| {
            let rel = path.strip_prefix(layout.root()).ok()?.to_string();
            Some(PageFile { rel, path })
        })
        .collect();
    files.sort_by(|a, b| a.rel.cmp(&b.rel));
    files
}

fn unchanged(
    row: &IndexedFile,
    size: u64,
    mtime_ns: i64,
    source: impl FnOnce() -> Option<String>,
) -> bool {
    row.size == size
        && (row.mtime_ns == mtime_ns
            || source().is_some_and(|text| hash::text(&text) == row.content_hash))
}

fn mtime_ns(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|elapsed| i64::try_from(elapsed.as_nanos()).ok())
        .unwrap_or(0)
}

fn anchor_page(
    tx: &Transaction<'_>,
    rel: &str,
    body: &str,
    attached: bool,
) -> rusqlite::Result<bool> {
    if !attached {
        anchors::set_pending(tx, rel, true)?;
        return Ok(false);
    }
    let found = anchors::resolve(tx, &extract_candidates(body))?;
    anchors::replace_anchors(tx, rel, &found)?;
    anchors::set_pending(tx, rel, false)?;
    Ok(true)
}

fn index(layout: &Layout, db: &mut MemoryDb, full: bool) -> Result<ReindexReport, String> {
    // ATTACH is refused once the transaction below is open.
    let attached = anchors::attach_indexed_graph(db.conn(), &layout.graph_db());
    let outcome = index_attached(layout, db, full, attached);
    if attached {
        anchors::detach_graph(db.conn());
    }
    outcome
}

fn index_attached(
    layout: &Layout,
    db: &mut MemoryDb,
    full: bool,
    attached: bool,
) -> Result<ReindexReport, String> {
    let sql = |error: rusqlite::Error| error.to_string();
    let tx = db.conn_mut().transaction().map_err(sql)?;
    let previous: BTreeMap<String, IndexedFile> = pages::indexed_files(&tx)
        .map_err(sql)?
        .into_iter()
        .map(|row| (row.path.clone(), row))
        .collect();
    let no_rows = BTreeMap::new();
    let known = if full {
        tx.execute_batch("DELETE FROM pages_fts; DELETE FROM page_links; DELETE FROM pages;")
            .map_err(sql)?;
        &no_rows
    } else {
        &previous
    };
    let mut report = ReindexReport::default();
    let mut kept = BTreeSet::new();
    let indexed_at = rfc3339_now();
    for file in walk_pages(layout) {
        let Ok(metadata) = std::fs::metadata(&file.path) else {
            kept.insert(file.rel);
            continue;
        };
        let size = metadata.len();
        let mtime = mtime_ns(&metadata);
        let read = || std::fs::read_to_string(&file.path).ok();
        if let Some(row) = known.get(&file.rel)
            && unchanged(row, size, mtime, read)
        {
            kept.insert(file.rel);
            report.unchanged += 1;
            continue;
        }
        let Some(text) = read() else {
            report.invalid.push(file.rel);
            continue;
        };
        let page = match parse_page(&file.rel, &text) {
            Ok(page) => page,
            Err(_) => {
                report.invalid.push(file.rel);
                continue;
            }
        };
        let mut links = extract_links(&page.body);
        links.extend(relation_links(&page.meta.relations));
        let indexed = IndexedFile {
            path: file.rel.clone(),
            size,
            mtime_ns: mtime,
            content_hash: hash::text(&text),
        };
        pages::upsert_page(&tx, &page, &indexed, &indexed_at, &links).map_err(sql)?;
        if anchor_page(&tx, &file.rel, &page.body, attached).map_err(sql)? {
            report.anchored += 1;
        }
        kept.insert(file.rel);
        report.indexed += 1;
    }
    for path in previous.keys().filter(|path| !kept.contains(*path)) {
        pages::remove_page(&tx, path).map_err(sql)?;
        report.removed += 1;
    }
    if attached {
        for path in anchors::pending_paths(&tx).map_err(sql)? {
            let Some(page) = std::fs::read_to_string(layout.root().join(&path))
                .ok()
                .and_then(|text| parse_page(&path, &text).ok())
            else {
                continue;
            };
            if anchor_page(&tx, &path, &page.body, true).map_err(sql)? {
                report.anchored += 1;
            }
        }
    }
    if report.indexed > 0 || report.removed > 0 {
        pages::resolve_links(&tx).map_err(sql)?;
    }
    tx.commit().map_err(sql)?;
    Ok(report)
}

fn record_error(layout: &Layout, error: &str) {
    let mut state = read_state(layout);
    state.last_error = Some(error.to_owned());
    write_state(layout, &state);
}

fn record(layout: &Layout, outcome: &Result<ReindexReport, String>) {
    let report = match outcome {
        Ok(report) => report,
        Err(error) => return record_error(layout, error),
    };
    let mut state = read_state(layout);
    let pages = report.indexed + report.unchanged;
    let nothing_to_record = report.indexed == 0
        && report.removed == 0
        && report.anchored == 0
        && state.last_error.is_none()
        && state.last_reindex_at.is_some()
        && state.pages == pages;
    if nothing_to_record {
        return;
    }
    state.last_reindex_at = Some(rfc3339_now());
    state.last_error = None;
    state.pages = pages;
    write_state(layout, &state);
}

fn finish(
    layout: &Layout,
    outcome: Result<ReindexReport, String>,
) -> Result<ReindexReport, Failure> {
    record(layout, &outcome);
    outcome.map_err(|error| {
        Failure::failed(
            "memory.reindex_failed",
            format!("could not index the memory pages: {error}"),
        )
    })
}

fn recorded<T>(layout: &Layout, locked: Result<T, impl Into<Failure>>) -> Result<T, Failure> {
    locked
        .map_err(Into::into)
        .inspect_err(|failure| record_error(layout, &failure.what))
}

/// The caller must hold `memory.lock`; the lock is not re-entrant.
///
/// # Errors
///
/// When the pages cannot be indexed; the error is recorded in `_reindex.json`.
pub fn refresh_locked(
    layout: &Layout,
    db: &mut MemoryDb,
    full: bool,
) -> Result<ReindexReport, Failure> {
    finish(layout, index(layout, db, full))
}

fn run(layout: &Layout, full: bool) -> Result<ReindexReport, Failure> {
    let outcome = MemoryDb::open(layout.memory_db().as_std_path())
        .map_err(|error| error.to_string())
        .and_then(|mut db| index(layout, &mut db, full));
    finish(layout, outcome)
}

/// Incremental, waiting for `memory.lock`.
///
/// # Errors
///
/// When the lock or the database cannot be used; the error is recorded in `_reindex.json`.
pub fn refresh(layout: &Layout) -> Result<ReindexReport, Failure> {
    let _lock = recorded(layout, fs::lock_exclusive(&layout.memory_lock()))?;
    run(layout, false)
}

/// Incremental, returning an empty report when another process holds
/// `memory.lock`.
///
/// # Errors
///
/// When the lock or the database cannot be used.
pub fn refresh_if_free(layout: &Layout) -> Result<ReindexReport, Failure> {
    let Some(_lock) = recorded(layout, fs::try_lock_exclusive(&layout.memory_lock()))? else {
        return Ok(ReindexReport::default());
    };
    run(layout, false)
}

/// Full rebuild: every page row is dropped and re-read; observations stay.
///
/// # Errors
///
/// When the lock or the database cannot be used.
pub fn rebuild(layout: &Layout) -> Result<ReindexReport, Failure> {
    let _lock = recorded(layout, fs::lock_exclusive(&layout.memory_lock()))?;
    run(layout, true)
}

/// # Errors
///
/// When no hall is found, or as [`rebuild`].
pub fn reindex(ctx: &Ctx) -> Outcome<ReindexReport> {
    let layout = discover_hall(ctx)?;
    finish_legacy_move(&layout).map_err(|error| split_failure(&layout, error))?;
    Ok(Report::new(rebuild(&layout)?))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/memory/reindex.rs"]
mod tests;
