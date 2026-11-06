//! Page anchors: resolving citations against the graph base layer and reading them back.

use std::collections::{BTreeMap, BTreeSet};

use camino::Utf8Path;
use rusqlite::{Connection, Transaction, params};

use crate::domain::memory_anchor::{
    Anchor, AnchorView, Candidate, MEMORY_DOCS_PREFIX, RelatedPage,
};
use crate::domain::memory_page::Tier;
use crate::domain::memory_rank::tier_multiplier;

const BASE_REPO: &str = "instr(f.repo, '/') = 0";

const BROKEN: &str =
    "(NOT EXISTS (SELECT 1 FROM graph.files f WHERE f.repo = a.repo AND f.path = a.path)
    OR (a.symbol IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM graph.symbols s JOIN graph.files f ON f.id = s.file_id
         WHERE f.repo = a.repo AND f.path = a.path AND s.name = a.symbol)))";

/// Attaches `graph_db` read-only as `graph`; returns whether it is readable.
/// `ATTACH` is refused inside an open transaction, so callers attach before `BEGIN`.
pub fn attach_graph(conn: &Connection, graph_db: &Utf8Path) -> bool {
    if !graph_db.exists() {
        return false;
    }
    let escaped = graph_db
        .as_str()
        .replace('%', "%25")
        .replace('?', "%3f")
        .replace('#', "%23");
    if conn
        .execute(
            "ATTACH DATABASE ?1 AS graph",
            [format!("file:{escaped}?mode=ro")],
        )
        .is_err()
    {
        return false;
    }
    let readable = conn
        .query_row("SELECT count(*) FROM graph.files", [], |r| {
            r.get::<_, i64>(0)
        })
        .is_ok();
    if !readable {
        detach_graph(conn);
    }
    readable
}

pub fn detach_graph(conn: &Connection) {
    let _ = conn.execute_batch("DETACH DATABASE graph");
}

fn path_hits(
    tx: &Transaction<'_>,
    repo: Option<&str>,
    path: &str,
) -> rusqlite::Result<Vec<(String, String)>> {
    let mut hits = Vec::new();
    let mut collect = |sql: &str, args: &[&dyn rusqlite::ToSql]| -> rusqlite::Result<()> {
        let mut stmt = tx.prepare(sql)?;
        for row in stmt.query_map(args, |r| Ok((r.get(0)?, r.get(1)?)))? {
            hits.push(row?);
        }
        Ok(())
    };
    let by_repo = "SELECT f.repo, f.path FROM graph.files f WHERE f.repo = ?1 AND f.path = ?2";
    if let Some(repo) = repo {
        collect(by_repo, &[&repo, &path])?;
        return Ok(hits);
    }
    collect(
        &format!("SELECT f.repo, f.path FROM graph.files f WHERE f.path = ?1 AND {BASE_REPO}"),
        &[&path],
    )?;
    if let Some((first, rest)) = path.split_once('/') {
        collect(&format!("{by_repo} AND {BASE_REPO}"), &[&first, &rest])?;
    }
    Ok(hits)
}

/// Returns the `(repo, path)` pairs defining `name`, optionally within `scope`.
fn symbol_hits(
    tx: &Transaction<'_>,
    scope: Option<&str>,
    name: &str,
) -> rusqlite::Result<Vec<(String, String)>> {
    let sql = format!(
        "SELECT DISTINCT f.repo, f.path FROM graph.symbols s JOIN graph.files f ON f.id = s.file_id
          WHERE s.name = ?1 AND (?2 IS NULL OR s.scope = ?2) AND {BASE_REPO}"
    );
    let mut stmt = tx.prepare(&sql)?;
    let rows = stmt.query_map(params![name, scope], |r| Ok((r.get(0)?, r.get(1)?)))?;
    rows.collect()
}

/// Resolves candidates against the graph base layer; needs `graph` attached.
/// An ambiguous citation survives only when another resolved path anchor disambiguates it.
///
/// # Errors
///
/// Returns the `SQLite` error from the graph lookups.
pub fn resolve(tx: &Transaction<'_>, candidates: &[Candidate]) -> rusqlite::Result<Vec<Anchor>> {
    let mut resolved = BTreeSet::new();
    let mut ambiguous_paths = Vec::new();
    let mut ambiguous_symbols = Vec::new();
    for candidate in candidates {
        match candidate {
            Candidate::Path { repo, path } => {
                let mut hits = path_hits(tx, repo.as_deref(), path)?;
                hits.sort();
                hits.dedup();
                match hits.as_slice() {
                    [] => {}
                    [(repo, path)] => {
                        resolved.insert(Anchor {
                            repo: repo.clone(),
                            path: path.clone(),
                            symbol: None,
                        });
                    }
                    _ => ambiguous_paths.push(hits),
                }
            }
            Candidate::Symbol { scope, name } => {
                let hits = symbol_hits(tx, scope.as_deref(), name)?;
                match hits.as_slice() {
                    [] => {}
                    [(repo, path)] => {
                        resolved.insert(Anchor {
                            repo: repo.clone(),
                            path: path.clone(),
                            symbol: Some(name.clone()),
                        });
                    }
                    _ => ambiguous_symbols.push((name.clone(), hits)),
                }
            }
        }
    }
    let path_repos = |resolved: &BTreeSet<Anchor>| -> BTreeSet<(String, String)> {
        resolved
            .iter()
            .filter(|a| a.symbol.is_none())
            .map(|a| (a.repo.clone(), a.path.clone()))
            .collect()
    };
    let known = path_repos(&resolved);
    for hits in ambiguous_paths {
        let kept: Vec<_> = hits
            .into_iter()
            .filter(|(repo, _)| known.iter().any(|(r, _)| r == repo))
            .collect();
        if let [(repo, path)] = kept.as_slice() {
            resolved.insert(Anchor {
                repo: repo.clone(),
                path: path.clone(),
                symbol: None,
            });
        }
    }
    let known = path_repos(&resolved);
    for (name, hits) in ambiguous_symbols {
        for (repo, path) in hits {
            if known.contains(&(repo.clone(), path.clone())) {
                resolved.insert(Anchor {
                    repo,
                    path,
                    symbol: Some(name.clone()),
                });
            }
        }
    }
    Ok(resolved.into_iter().collect())
}

/// # Errors
///
/// Returns the `SQLite` error; the caller decides whether to roll `tx` back.
pub fn replace_anchors(
    tx: &Transaction<'_>,
    page_path: &str,
    anchors: &[Anchor],
) -> rusqlite::Result<()> {
    tx.execute("DELETE FROM page_anchors WHERE page_path = ?1", [page_path])?;
    let mut insert = tx.prepare(
        "INSERT INTO page_anchors (page_path, repo, path, symbol) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for a in anchors {
        insert.execute(params![page_path, a.repo, a.path, a.symbol])?;
    }
    Ok(())
}

/// # Errors
///
/// Returns the `SQLite` error.
pub fn set_pending(tx: &Transaction<'_>, page_path: &str, pending: bool) -> rusqlite::Result<()> {
    tx.execute(
        "UPDATE pages SET anchors_pending = ?2 WHERE path = ?1",
        params![page_path, pending],
    )?;
    Ok(())
}

/// # Errors
///
/// Returns the `SQLite` error.
pub fn pending_paths(tx: &Transaction<'_>) -> rusqlite::Result<Vec<String>> {
    let mut stmt = tx.prepare("SELECT path FROM pages WHERE anchors_pending = 1 ORDER BY path")?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    rows.collect()
}

/// # Errors
///
/// Returns the `SQLite` error.
pub fn pending_count(conn: &Connection) -> rusqlite::Result<usize> {
    conn.query_row(
        "SELECT count(*) FROM pages WHERE anchors_pending = 1",
        [],
        |r| r.get(0),
    )
}

/// Anchors of a page, broken ones first. Without the graph attached none is reported broken.
///
/// # Errors
///
/// Returns the `SQLite` error.
pub fn anchors_for(
    conn: &Connection,
    page_path: &str,
    graph_attached: bool,
    limit: usize,
) -> rusqlite::Result<Vec<AnchorView>> {
    let broken = if graph_attached { BROKEN } else { "0" };
    let sql = format!(
        "SELECT a.repo, a.path, a.symbol, {broken} FROM page_anchors a WHERE a.page_path = ?1"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([page_path], |r| {
        Ok(AnchorView {
            repo: r.get(0)?,
            path: r.get(1)?,
            symbol: r.get(2)?,
            broken: r.get(3)?,
        })
    })?;
    let mut views = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    views.sort_by(|a, b| {
        b.broken
            .cmp(&a.broken)
            .then_with(|| (&a.repo, &a.path, &a.symbol).cmp(&(&b.repo, &b.path, &b.symbol)))
    });
    views.truncate(limit);
    Ok(views)
}

fn anchor_tokens(path: &str, symbol: Option<&str>) -> BTreeSet<String> {
    let mut tokens: BTreeSet<String> = path.split('/').map(str::to_lowercase).collect();
    if let Some(name) = path.rsplit('/').next()
        && let Some((stem, _)) = name.rsplit_once('.')
    {
        tokens.insert(stem.to_lowercase());
    }
    if let Some(symbol) = symbol {
        tokens.insert(symbol.to_lowercase());
    }
    tokens
}

/// Pages whose anchors match query terms, as `(page_path, terms_matched, anchors_matched)`, best first.
///
/// # Errors
///
/// Returns the `SQLite` error.
pub fn match_terms(
    conn: &Connection,
    terms: &[String],
) -> rusqlite::Result<Vec<(String, usize, usize)>> {
    let terms: BTreeSet<String> = terms.iter().map(|t| t.to_lowercase()).collect();
    let mut stmt = conn.prepare("SELECT page_path, path, symbol FROM page_anchors")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
        ))
    })?;
    let mut pages: BTreeMap<String, (BTreeSet<String>, usize)> = BTreeMap::new();
    for row in rows {
        let (page, path, symbol) = row?;
        let tokens = anchor_tokens(&path, symbol.as_deref());
        let matched: Vec<_> = terms
            .iter()
            .filter(|t| tokens.contains(*t))
            .cloned()
            .collect();
        if matched.is_empty() {
            continue;
        }
        let entry = pages.entry(page).or_default();
        entry.0.extend(matched);
        entry.1 += 1;
    }
    let mut out: Vec<_> = pages
        .into_iter()
        .map(|(p, (t, a))| (p, t.len(), a))
        .collect();
    out.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then(b.2.cmp(&a.2))
            .then_with(|| a.0.cmp(&b.0))
    });
    Ok(out)
}

/// Anchors whose file or symbol vanished from the graph; needs `graph` attached.
///
/// # Errors
///
/// Returns the `SQLite` error.
pub fn broken_anchors(conn: &Connection) -> rusqlite::Result<Vec<(String, Anchor)>> {
    let sql = format!(
        "SELECT a.page_path, a.repo, a.path, a.symbol FROM page_anchors a WHERE {BROKEN}
          ORDER BY a.page_path, a.repo, a.path, a.symbol"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get(0)?,
            Anchor {
                repo: r.get(1)?,
                path: r.get(2)?,
                symbol: r.get(3)?,
            },
        ))
    })?;
    rows.collect()
}

/// Durable pages anchored to any of `files` or `symbols`, most anchors first.
///
/// # Errors
///
/// Returns the `SQLite` error.
pub fn related_pages(
    conn: &Connection,
    files: &[(String, String)],
    symbols: &[String],
    limit: usize,
) -> rusqlite::Result<Vec<RelatedPage>> {
    let mut stmt = conn.prepare(
        "SELECT p.path, p.kind, p.title, p.tier, a.repo, a.path, a.symbol
           FROM page_anchors a JOIN pages p ON p.path = a.page_path
          WHERE a.page_path LIKE ?1
            AND (p.expires_at IS NULL OR p.expires_at > strftime('%Y-%m-%dT%H:%M:%SZ','now'))",
    )?;
    let rows = stmt.query_map([format!("{MEMORY_DOCS_PREFIX}%")], |r| {
        Ok((
            RelatedPage {
                path: r.get(0)?,
                kind: r.get(1)?,
                title: r.get(2)?,
                anchors: 0,
            },
            r.get::<_, String>(3)?,
            (r.get::<_, String>(4)?, r.get::<_, String>(5)?),
            r.get::<_, Option<String>>(6)?,
        ))
    })?;
    let mut pages: BTreeMap<String, (RelatedPage, f64)> = BTreeMap::new();
    for row in rows {
        let (page, tier, file, symbol) = row?;
        let hit = files.contains(&file) || symbol.is_some_and(|s| symbols.contains(&s));
        if !hit {
            continue;
        }
        let weight = tier_multiplier(Tier::parse(&tier).unwrap_or(Tier::Working));
        pages
            .entry(page.path.clone())
            .or_insert((page, weight))
            .0
            .anchors += 1;
    }
    let mut out: Vec<_> = pages.into_values().collect();
    out.sort_by(|a, b| {
        b.0.anchors
            .cmp(&a.0.anchors)
            .then(b.1.total_cmp(&a.1))
            .then_with(|| a.0.path.cmp(&b.0.path))
    });
    Ok(out.into_iter().take(limit).map(|(p, _)| p).collect())
}

#[cfg(test)]
#[path = "../../../tests/unit/store/memory/anchors.rs"]
mod tests;
