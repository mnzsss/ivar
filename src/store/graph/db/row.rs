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
    let to_i64 = |value: usize| {
        i64::try_from(value).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
    };
    let start_line = to_i64(sym.span.start_line)?;
    let start_col = to_i64(sym.span.start_col)?;
    let end_line = to_i64(sym.span.end_line)?;
    let end_col = to_i64(sym.span.end_col)?;
    stmt.query_row(
        params![
            file_id,
            repo,
            &sym.name,
            kind_str.as_ref(),
            &sym.scope,
            &sym.signature,
            &sym.docstring,
            start_line,
            start_col,
            end_line,
            end_col,
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

    let span_field = |column: usize, value: i64| {
        usize::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(column, value))
    };

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
            span_field(8, start_line)?,
            span_field(9, start_col)?,
            span_field(10, end_line)?,
            span_field(11, end_col)?,
        ),
        is_exported: is_exported != 0,
        complexity: complexity
            .map(|c| u32::try_from(c).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(13, c)))
            .transpose()?,
    })
}
