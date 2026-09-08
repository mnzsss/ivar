//! Extracted file atomic indexing transaction.

use rusqlite::params;

use super::GraphDb;
use super::types::{
    Result, edge_kind_to_str, now_timestamp, provenance_to_str, symbol_kind_to_str,
};
use crate::domain::graph::Span;
use crate::store::graph::extractor::ExtractedFile;

impl GraphDb {
    /// Indexes an extracted file's symbols and edges inside a transaction.
    ///
    /// Updates the `files` record, deletes any old symbols and edges for the file,
    /// inserts new symbols, resolves local intra-file targets and enclosing symbol IDs,
    /// and inserts new edges.
    pub fn index_extracted_file(
        &self,
        repo_id: &str,
        file_path: &str,
        content_hash: &str,
        mtime_ns: i64,
        size_bytes: i64,
        extracted: &ExtractedFile,
    ) -> Result<(usize, usize)> {
        self.conn.execute_batch("BEGIN IMMEDIATE;")?;

        let res = (|| -> Result<(usize, usize)> {
            let now = now_timestamp();
            let mut file_stmt = self.conn.prepare_cached(
                "INSERT INTO files (repo, path, content_hash, mtime_ns, size_bytes, indexed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(repo, path) DO UPDATE SET
                    content_hash = excluded.content_hash,
                    mtime_ns = excluded.mtime_ns,
                    size_bytes = excluded.size_bytes,
                    indexed_at = excluded.indexed_at
                 RETURNING id",
            )?;
            let file_id: i64 = file_stmt.query_row(
                params![repo_id, file_path, content_hash, mtime_ns, size_bytes, now],
                |row| row.get(0),
            )?;

            // Delete old symbols for this file (cascades to edges from symbols)
            self.conn
                .execute("DELETE FROM symbols WHERE file_id = ?1", params![file_id])?;

            // Delete old edges for this file
            self.conn
                .execute("DELETE FROM edges WHERE file_id = ?1", params![file_id])?;

            // Insert new symbols
            let mut sym_stmt = self.conn.prepare_cached(
                "INSERT INTO symbols (file_id, repo, name, kind, scope, signature, docstring, start_line, start_col, end_line, end_col, is_exported, complexity)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                 RETURNING id",
            )?;
            struct IndexedSymbolInfo {
                id: i64,
                span: Span,
                scope: Option<String>,
            }

            let mut syms_by_name: std::collections::HashMap<String, Vec<IndexedSymbolInfo>> =
                std::collections::HashMap::new();
            let mut sym_spans: Vec<(Span, i64, Option<String>)> =
                Vec::with_capacity(extracted.symbols.len());
            let num_symbols = extracted.symbols.len();
            for sym in &extracted.symbols {
                let kind_str = symbol_kind_to_str(&sym.kind);
                let is_exported = if sym.is_exported { 1 } else { 0 };
                let sym_id: i64 = sym_stmt.query_row(
                    params![
                        file_id,
                        repo_id,
                        &sym.name,
                        kind_str.as_ref(),
                        &sym.scope,
                        &sym.signature,
                        &sym.docstring,
                        sym.span.start_line as i64,
                        sym.span.start_col as i64,
                        sym.span.end_line as i64,
                        sym.span.end_col as i64,
                        is_exported,
                        sym.complexity.map(|c| c as i64),
                    ],
                    |row| row.get(0),
                )?;
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

                    // Ambiguous duplicate names without definitive evidence -> leave unresolved
                    None
                });

                edge_stmt.execute(params![
                    repo_id,
                    file_id,
                    from_symbol_id,
                    to_symbol_id,
                    &edge.to_name,
                    kind_str.as_ref(),
                    prov_str,
                    edge.line as i64,
                    edge.col as i64,
                    edge.confidence,
                ])?;
            }

            Ok((num_symbols, num_edges))
        })();

        match res {
            Ok(counts) => {
                self.conn.execute_batch("COMMIT;")?;
                Ok(counts)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK;");
                Err(e)
            }
        }
    }
}
