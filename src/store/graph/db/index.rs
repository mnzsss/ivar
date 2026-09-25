//! Extracted file atomic indexing transaction.

use rusqlite::params;

use super::GraphDb;
use super::row;
use super::types::{FileContentHit, Result, edge_kind_to_str, now_timestamp, provenance_to_str};
use crate::domain::graph::Span;
use crate::store::graph::extractor::ExtractedFile;

struct IndexedSymbolInfo {
    id: i64,
    span: Span,
    scope: Option<String>,
}

fn resolve_edge_endpoints(
    edge: &crate::domain::graph::Edge,
    sym_spans: &[(Span, i64, Option<String>)],
    syms_by_name: &std::collections::HashMap<String, Vec<IndexedSymbolInfo>>,
) -> (Option<i64>, Option<i64>) {
    // If from_symbol_id is not set, find smallest enclosing symbol in this file
    let enclosing_sym = sym_spans
        .iter()
        .filter(|(span, _, _)| {
            if edge.line < span.start_line || edge.line > span.end_line {
                return false;
            }
            if edge.line == span.start_line && edge.col < span.start_col {
                return false;
            }
            if edge.line == span.end_line && edge.col > span.end_col {
                return false;
            }
            true
        })
        .min_by_key(|(span, _, _)| {
            (
                span.end_line.saturating_sub(span.start_line),
                span.end_col.saturating_sub(span.start_col),
            )
        });

    let from_symbol_id = edge
        .from_symbol_id
        .or_else(|| enclosing_sym.map(|(_, id, _)| *id));
    let caller_scope = enclosing_sym.and_then(|(_, _, scope)| scope.as_deref());

    // Resolve to_symbol_id:
    // If already resolved, keep it.
    // Otherwise, if to_name matches symbols in this file:
    // - Single matching symbol -> resolve to its id.
    // - Multiple matching symbols -> use enclosing span / caller scope evidence:
    //   1) Check if caller scope matches symbol scope.
    //   2) Check if edge is inside symbol span (e.g. recursion / inner symbol).
    //   If still ambiguous (multiple candidates or none unique), leave unresolved (None).
    let to_symbol_id = edge.to_symbol_id.or_else(|| {
        let name = edge.to_name.as_deref()?;
        let candidates = syms_by_name.get(name)?;
        if let [candidate] = candidates.as_slice() {
            return Some(candidate.id);
        }

        // Multiple symbols share this name.
        // Try filtering by matching caller scope if present.
        if let Some(scope) = caller_scope {
            let scope_matches: Vec<_> = candidates
                .iter()
                .filter(|c| c.scope.as_deref() == Some(scope))
                .collect();
            if let [candidate] = scope_matches.as_slice() {
                return Some(candidate.id);
            }
        }

        // Try checking if the edge is located inside the symbol span
        let span_matches: Vec<_> = candidates
            .iter()
            .filter(|c| {
                if edge.line < c.span.start_line || edge.line > c.span.end_line {
                    return false;
                }
                if edge.line == c.span.start_line && edge.col < c.span.start_col {
                    return false;
                }
                if edge.line == c.span.end_line && edge.col > c.span.end_col {
                    return false;
                }
                true
            })
            .collect();
        if let [candidate] = span_matches.as_slice() {
            return Some(candidate.id);
        }

        None
    });

    (from_symbol_id, to_symbol_id)
}

impl GraphDb {
    /// Indexes an extracted file's symbols, edges, and content inside a transaction.
    ///
    /// Updates the `files` record, deletes any old symbols, edges, and FTS content for the file,
    /// inserts new FTS content, inserts new symbols, resolves local intra-file targets and enclosing symbol IDs,
    /// and inserts new edges.
    ///
    /// # Errors
    ///
    /// Returns [`GraphDbError`] if any statement in the transaction fails.
    #[allow(clippy::too_many_arguments)]
    pub fn index_extracted_file(
        &self,
        repo: &str,
        path: &str,
        hash: &str,
        mtime_ns: i64,
        size_bytes: i64,
        indexed_content: &str,
        content_truncated: bool,
        extracted: &ExtractedFile,
    ) -> Result<(usize, usize)> {
        self.in_transaction(|| -> Result<(usize, usize)> {
            let now = now_timestamp();
            let mut file_stmt = self.conn.prepare_cached(
                "INSERT INTO files (repo, path, content_hash, mtime_ns, size_bytes, content_truncated, indexed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(repo, path) DO UPDATE SET
                    content_hash = excluded.content_hash,
                    mtime_ns = excluded.mtime_ns,
                    size_bytes = excluded.size_bytes,
                    content_truncated = excluded.content_truncated,
                    indexed_at = excluded.indexed_at
                 RETURNING id",
            )?;
            let file_id: i64 = file_stmt.query_row(
                params![repo, path, hash, mtime_ns, size_bytes, content_truncated as i64, now],
                |row| row.get(0),
            )?;

            // Delete old FTS content for this file
            self.conn.execute(
                "DELETE FROM file_content_fts WHERE rowid = ?1",
                params![file_id],
            )?;

            // Insert new FTS content
            self.conn.execute(
                "INSERT INTO file_content_fts (rowid, content) VALUES (?1, ?2)",
                params![file_id, indexed_content],
            )?;

            // Delete old symbols for this file (cascades to edges from symbols)
            self.conn
                .execute("DELETE FROM symbols WHERE file_id = ?1", params![file_id])?;

            // Delete old edges for this file
            self.conn
                .execute("DELETE FROM edges WHERE file_id = ?1", params![file_id])?;

            // Insert new symbols
            let mut sym_stmt = self.conn.prepare_cached(
                "INSERT INTO symbols (file_id, repo, name, kind, scope, signature, docstring, start_line, start_col, end_line, end_col, is_exported, complexity, name_words)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                 RETURNING id",
            )?;

            let mut syms_by_name: std::collections::HashMap<String, Vec<IndexedSymbolInfo>> =
                std::collections::HashMap::new();
            let mut sym_spans: Vec<(Span, i64, Option<String>)> =
                Vec::with_capacity(extracted.symbols.len());
            let num_symbols = extracted.symbols.len();
            for sym in &extracted.symbols {
                let sym_id = row::insert_symbol(&mut sym_stmt, Some(file_id), repo, sym)?;
                syms_by_name
                    .entry(sym.name.clone())
                    .or_default()
                    .push(IndexedSymbolInfo {
                        id: sym_id,
                        span: sym.span,
                        scope: sym.scope.clone(),
                    });
                sym_spans.push((sym.span, sym_id, sym.scope.clone()));
            }

            // Insert edges
            let mut edge_stmt = self.conn.prepare_cached(
                "INSERT INTO edges (repo, file_id, from_symbol_id, to_symbol_id, to_name, kind, provenance, line, col, confidence)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )?;
            let num_edges = extracted.edges.len();
            for edge in &extracted.edges {
                let kind_str = edge_kind_to_str(&edge.kind);
                let prov_str = provenance_to_str(&edge.provenance);

                let (from_symbol_id, to_symbol_id) =
                    resolve_edge_endpoints(edge, &sym_spans, &syms_by_name);

                edge_stmt.execute(params![
                    repo,
                    file_id,
                    from_symbol_id,
                    to_symbol_id,
                    &edge.to_name,
                    kind_str.as_ref(),
                    prov_str,
                    i64::try_from(edge.line).unwrap_or(i64::MAX),
                    i64::try_from(edge.col).unwrap_or(i64::MAX),
                    edge.confidence,
                ])?;
            }

            Ok((num_symbols, num_edges))
        })
    }

    /// Full-text searches indexed file content joined through visible_files using SQLite FTS5 bm25.
    ///
    /// # Errors
    ///
    /// Returns [`GraphDbError`] if the query fails.
    pub fn search_file_content(
        &self,
        query: &str,
        repo: Option<&str>,
        limit: usize,
    ) -> Result<Vec<FileContentHit>> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let fts_query = format!("\"{}\"", query.replace('"', "\"\""));
        let sql = match repo {
            Some(_) => {
                "SELECT vf.id, vf.repo, vf.path, fts.rank, fts.content, vf.content_truncated
                 FROM file_content_fts fts
                 JOIN visible_files vf ON fts.rowid = vf.id
                 WHERE file_content_fts MATCH ?1 AND vf.repo = ?2
                 ORDER BY fts.rank, vf.repo, vf.path
                 LIMIT ?3"
            }
            None => {
                "SELECT vf.id, vf.repo, vf.path, fts.rank, fts.content, vf.content_truncated
                 FROM file_content_fts fts
                 JOIN visible_files vf ON fts.rowid = vf.id
                 WHERE file_content_fts MATCH ?1
                 ORDER BY fts.rank, vf.repo, vf.path
                 LIMIT ?2"
            }
        };

        let mut stmt = self.conn.prepare_cached(sql)?;
        let map_row = |row: &rusqlite::Row| -> rusqlite::Result<FileContentHit> {
            Ok(FileContentHit {
                file_id: row.get(0)?,
                repo: row.get(1)?,
                path: row.get(2)?,
                rank: row.get(3)?,
                indexed_content: row.get(4)?,
                content_truncated: row.get::<_, i64>(5)? != 0,
            })
        };
        let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut results = Vec::new();
        if let Some(r) = repo {
            let rows = stmt.query_map(params![fts_query, r, limit_i64], map_row)?;
            for row in rows {
                results.push(row?);
            }
        } else {
            let rows = stmt.query_map(params![fts_query, limit_i64], map_row)?;
            for row in rows {
                results.push(row?);
            }
        }
        Ok(results)
    }
}
