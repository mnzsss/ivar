//! Shared row-mapping helper for the 14-column symbol projection.

use rusqlite::params;

use crate::domain::graph::{Span, Symbol};
use crate::store::graph::schema::name_words;

use super::types::{parse_symbol_kind, symbol_kind_to_str};

/// Inserts one symbol row via the shared 14-column `symbols` insert statement,
/// returning its generated id.
pub(super) fn insert_symbol(
    stmt: &mut rusqlite::Statement<'_>,
    file_id: Option<i64>,
    repo: &str,
    sym: &Symbol,
) -> rusqlite::Result<i64> {
    let kind_str = symbol_kind_to_str(&sym.kind);
    let is_exported = if sym.is_exported { 1 } else { 0 };
    stmt.query_row(
        params![
            file_id,
            repo,
            &sym.name,
            kind_str.as_ref(),
            &sym.scope,
            &sym.signature,
            &sym.docstring,
            i64::try_from(sym.span.start_line).unwrap_or(i64::MAX),
            i64::try_from(sym.span.start_col).unwrap_or(i64::MAX),
            i64::try_from(sym.span.end_line).unwrap_or(i64::MAX),
            i64::try_from(sym.span.end_col).unwrap_or(i64::MAX),
            is_exported,
            sym.complexity.map(i64::from),
            name_words(&sym.name),
        ],
        |row| row.get(0),
    )
}

pub(crate) fn symbol_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Symbol> {
    let id: i64 = row.get(0)?;
    let file_id: i64 = row.get(1)?;
    let repo: String = row.get(2)?;
    let name: String = row.get(3)?;
    let kind_raw: String = row.get(4)?;
    let scope: Option<String> = row.get(5)?;
    let signature: Option<String> = row.get(6)?;
    let docstring: Option<String> = row.get(7)?;
    let start_line: i64 = row.get(8)?;
    let start_col: i64 = row.get(9)?;
    let end_line: i64 = row.get(10)?;
    let end_col: i64 = row.get(11)?;
    let is_exported: i64 = row.get(12)?;
    let complexity: Option<i64> = row.get(13)?;

    Ok(Symbol {
        id: Some(id),
        file_id: Some(file_id),
        repo,
        name,
        kind: parse_symbol_kind(&kind_raw),
        scope,
        signature,
        docstring,
        span: Span::new(
            usize::try_from(start_line).unwrap_or(usize::MAX),
            usize::try_from(start_col).unwrap_or(usize::MAX),
            usize::try_from(end_line).unwrap_or(usize::MAX),
            usize::try_from(end_col).unwrap_or(usize::MAX),
        ),
        is_exported: is_exported != 0,
        complexity: complexity.map(|c| u32::try_from(c).unwrap_or(u32::MAX)),
    })
}
