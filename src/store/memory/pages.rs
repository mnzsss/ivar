//! Writing and searching rows of the `pages`, `pages_fts` and `page_links` tables.

use std::collections::BTreeSet;

use rusqlite::types::Value;
use rusqlite::{OptionalExtension, Transaction, params};

use super::db::MemoryDb;
use crate::domain::memory_page::links::{Link, LinkKind, LinkTarget};
use crate::domain::memory_page::{DURABLE_ROOT, EPHEMERAL_ROOT, Page, PageKind, Tier};

const BM25: &str = "bm25(pages_fts, 10.0, 5.0, 1.0)";
const SNIPPET: &str = "snippet(pages_fts, 2, '', '', '...', 24)";
const FALLBACK_SNIPPET_CHARS: usize = 160;

#[derive(Debug, Clone)]
pub struct IndexedFile {
    pub path: String,
    pub size: u64,
    pub mtime_ns: i64,
    pub content_hash: String,
}

#[derive(Debug, Clone)]
pub struct PageFilter {
    pub kinds: Vec<PageKind>,
    pub tier: Option<Tier>,
    pub tag: Option<String>,
    pub include_expired: bool,
    pub now: String,
}

#[derive(Debug, Clone)]
pub struct PageHit {
    pub path: String,
    pub kind: PageKind,
    pub tier: Tier,
    pub title: String,
    pub pinned: bool,
    pub snippet: String,
    pub fts_rank: f64,
}

fn conversion_error(index: usize, text: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        index,
        rusqlite::types::Type::Text,
        format!("unknown value `{text}`").into(),
    )
}

fn hit_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PageHit> {
    let kind: String = row.get(1)?;
    let tier: String = row.get(2)?;
    Ok(PageHit {
        path: row.get(0)?,
        kind: PageKind::parse(&kind).ok_or_else(|| conversion_error(1, &kind))?,
        tier: Tier::parse(&tier).ok_or_else(|| conversion_error(2, &tier))?,
        title: row.get(3)?,
        pinned: row.get(4)?,
        snippet: row.get(5)?,
        fts_rank: row.get(6)?,
    })
}

/// # Errors
///
/// Returns the `SQLite` error when the query fails.
pub fn indexed_files(db: &MemoryDb) -> rusqlite::Result<Vec<IndexedFile>> {
    let mut stmt = db
        .conn()
        .prepare("SELECT path, size, mtime_ns, content_hash FROM pages ORDER BY path")?;
    stmt.query_map([], |row| {
        Ok(IndexedFile {
            path: row.get(0)?,
            size: u64::try_from(row.get::<_, i64>(1)?).unwrap_or(0),
            mtime_ns: row.get(2)?,
            content_hash: row.get(3)?,
        })
    })?
    .collect()
}

fn delete_rows(tx: &Transaction<'_>, path: &str) -> rusqlite::Result<()> {
    tx.execute(
        "DELETE FROM pages_fts WHERE rowid = (SELECT id FROM pages WHERE path = ?1)",
        [path],
    )?;
    tx.execute("DELETE FROM pages WHERE path = ?1", [path])?;
    tx.execute("DELETE FROM page_links WHERE from_path = ?1", [path])?;
    Ok(())
}

fn link_source(kind: LinkKind) -> &'static str {
    match kind {
        LinkKind::Wikilink => "wikilink",
        LinkKind::Markdown => "markdown",
        LinkKind::Relation => "relation",
    }
}

fn target_text(target: &LinkTarget) -> String {
    match target {
        LinkTarget::Slug(slug) => slug.clone(),
        LinkTarget::KindSlug(kind, slug) => format!("{}/{slug}", kind.as_str()),
    }
}

/// Replaces every row of `page` in one transaction; links stay unresolved until `resolve_links`.
///
/// # Errors
///
/// Returns the `SQLite` error; the previous rows are kept then.
pub fn upsert_page(
    db: &mut MemoryDb,
    page: &Page,
    file: &IndexedFile,
    indexed_at: &str,
    links: &[Link],
) -> rusqlite::Result<()> {
    let tx = db.conn_mut().transaction()?;
    delete_rows(&tx, &page.path)?;
    let tags_json = serde_json::to_string(&page.meta.tags)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(error.into()))?;
    tx.execute(
        "INSERT INTO pages
           (path, kind, tier, title, tags, pinned, expires_at, size, mtime_ns, content_hash, indexed_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![
            page.path,
            page.kind.as_str(),
            page.kind.tier().as_str(),
            page.title,
            tags_json,
            page.meta.pinned,
            page.meta.expires_at,
            i64::try_from(file.size).unwrap_or(i64::MAX),
            file.mtime_ns,
            file.content_hash,
            indexed_at
        ],
    )?;
    let id = tx.last_insert_rowid();
    tx.execute(
        "INSERT INTO pages_fts (rowid, title, tags, body) VALUES (?1,?2,?3,?4)",
        params![id, page.title, page.meta.tags.join(" "), page.body],
    )?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO page_links (from_path, source, relation_type, target_text)
             VALUES (?1,?2,?3,?4)",
        )?;
        for link in links {
            stmt.execute(params![
                page.path,
                link_source(link.kind),
                link.relation_type,
                target_text(&link.target)
            ])?;
        }
    }
    tx.commit()
}

/// # Errors
///
/// Returns the `SQLite` error; nothing is removed then.
pub fn remove_page(db: &mut MemoryDb, path: &str) -> rusqlite::Result<()> {
    let tx = db.conn_mut().transaction()?;
    delete_rows(&tx, path)?;
    tx.commit()
}

fn file_stem(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.strip_suffix(".md").unwrap_or(name)
}

fn unnumbered(stem: &str) -> &str {
    match stem.split_once('-') {
        Some((number, rest)) if number.len() == 3 && number.bytes().all(|b| b.is_ascii_digit()) => {
            rest
        }
        _ => stem,
    }
}

fn names_slug(path: &str, slug: &str) -> bool {
    let stem = file_stem(path);
    stem == slug || unnumbered(stem) == slug
}

fn resolve_target(paths: &[String], target_text: &str) -> Option<String> {
    let (kind, slug) = match target_text.split_once('/') {
        Some((kind, slug)) => (Some(kind), slug),
        None => (None, target_text),
    };
    let directory = kind.map(|kind| {
        let root = match PageKind::parse(kind) {
            Some(kind) if !kind.is_durable() => EPHEMERAL_ROOT,
            _ => DURABLE_ROOT,
        };
        format!("{root}{kind}/")
    });
    paths
        .iter()
        .find(|path| {
            names_slug(path, slug)
                && directory
                    .as_deref()
                    .is_none_or(|directory| path.starts_with(directory))
        })
        .cloned()
}

/// Recomputes `to_path` for every link against the pages currently indexed.
///
/// # Errors
///
/// Returns the `SQLite` error; no link is changed then.
pub fn resolve_links(db: &mut MemoryDb) -> rusqlite::Result<()> {
    let tx = db.conn_mut().transaction()?;
    let paths: Vec<String> = tx
        .prepare("SELECT path FROM pages ORDER BY path")?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let links: Vec<(i64, String)> = tx
        .prepare("SELECT rowid, target_text FROM page_links")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    {
        let mut update = tx.prepare("UPDATE page_links SET to_path = ?1 WHERE rowid = ?2")?;
        for (rowid, text) in links {
            update.execute(params![resolve_target(&paths, &text), rowid])?;
        }
    }
    tx.commit()
}

/// # Errors
///
/// Returns the `SQLite` error when the query fails or a stored kind or tier is unknown.
pub fn search_pages(
    db: &MemoryDb,
    fts_query: &str,
    filter: &PageFilter,
    limit: usize,
) -> rusqlite::Result<Vec<PageHit>> {
    let mut sql = format!(
        "SELECT p.path, p.kind, p.tier, p.title, p.pinned, {SNIPPET}, {BM25}
           FROM pages_fts
           JOIN pages p ON p.id = pages_fts.rowid
          WHERE pages_fts MATCH ?"
    );
    let mut args: Vec<Value> = vec![fts_query.to_owned().into()];
    if !filter.include_expired {
        sql.push_str(" AND (p.expires_at IS NULL OR p.expires_at >= ?)");
        args.push(filter.now.clone().into());
    }
    if !filter.kinds.is_empty() {
        let placeholders = vec!["?"; filter.kinds.len()].join(",");
        sql.push_str(" AND p.kind IN (");
        sql.push_str(&placeholders);
        sql.push(')');
        args.extend(
            filter
                .kinds
                .iter()
                .map(|kind| Value::from(kind.as_str().to_owned())),
        );
    }
    if let Some(tier) = filter.tier {
        sql.push_str(" AND p.tier = ?");
        args.push(tier.as_str().to_owned().into());
    }
    if let Some(tag) = &filter.tag {
        sql.push_str(" AND EXISTS (SELECT 1 FROM json_each(p.tags) WHERE value = ?)");
        args.push(tag.clone().into());
    }
    sql.push_str(" ORDER BY ");
    sql.push_str(BM25);
    sql.push_str(" LIMIT ?");
    args.push(i64::try_from(limit).unwrap_or(i64::MAX).into());
    let mut stmt = db.conn().prepare(&sql)?;
    stmt.query_map(rusqlite::params_from_iter(args), hit_from_row)?
        .collect()
}

/// Pages one link away from any of `paths`, in either direction, excluding `paths` themselves.
///
/// # Errors
///
/// Returns the `SQLite` error when the query fails.
pub fn linked_paths(db: &MemoryDb, paths: &[String]) -> rusqlite::Result<Vec<String>> {
    let mut stmt = db.conn().prepare(
        "SELECT to_path FROM page_links
          WHERE from_path = ?1 AND to_path IN (SELECT path FROM pages)
         UNION
         SELECT from_path FROM page_links WHERE to_path = ?1",
    )?;
    let mut found = BTreeSet::new();
    for path in paths {
        for linked in stmt.query_map([path], |row| row.get::<_, String>(0))? {
            found.insert(linked?);
        }
    }
    Ok(found
        .into_iter()
        .filter(|linked| !paths.contains(linked))
        .collect())
}

fn prefix_snippet(body: &str) -> String {
    body.chars().take(FALLBACK_SNIPPET_CHARS).collect()
}

/// Looks one page up; `snippet_of` is an FTS query whose match shapes the snippet.
///
/// # Errors
///
/// Returns the `SQLite` error when the query fails or a stored kind or tier is unknown.
pub fn page_by_path(
    db: &MemoryDb,
    path: &str,
    snippet_of: Option<&str>,
    live_at: Option<&str>,
) -> rusqlite::Result<Option<PageHit>> {
    let conn = db.conn();
    let Some(mut hit) = conn
        .query_row(
            "SELECT p.path, p.kind, p.tier, p.title, p.pinned, pages_fts.body, 0.0
               FROM pages p JOIN pages_fts ON pages_fts.rowid = p.id
              WHERE p.path = ?1
                AND (?2 IS NULL OR p.expires_at IS NULL OR p.expires_at >= ?2)",
            params![path, live_at],
            hit_from_row,
        )
        .optional()?
    else {
        return Ok(None);
    };
    let matched = match snippet_of {
        Some(query) => conn
            .query_row(
                &format!(
                    "SELECT {SNIPPET}, {BM25} FROM pages_fts
                      JOIN pages p ON p.id = pages_fts.rowid
                     WHERE pages_fts MATCH ?1 AND p.path = ?2"
                ),
                params![query, path],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?)),
            )
            .optional()?,
        None => None,
    };
    match matched {
        Some((snippet, rank)) => {
            hit.snippet = snippet;
            hit.fts_rank = rank;
        }
        None => hit.snippet = prefix_snippet(&hit.snippet),
    }
    Ok(Some(hit))
}

/// # Errors
///
/// Returns the `SQLite` error when the query fails.
pub fn count_by_kind(db: &MemoryDb) -> rusqlite::Result<Vec<(String, i64)>> {
    let mut stmt = db
        .conn()
        .prepare("SELECT kind, count(*) FROM pages GROUP BY kind ORDER BY kind")?;
    stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect()
}

#[cfg(test)]
#[path = "../../../tests/unit/store/memory/pages.rs"]
mod tests;
