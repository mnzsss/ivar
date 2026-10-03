//! Shared table builder and writer for tabular human output.

use crate::error::{HEADER, paint};
use crate::infra::term;
use comfy_table::presets::NOTHING;
use comfy_table::{ContentArrangement, Table};
use std::io::{self, Write};

/// Build a new `comfy_table::Table` with standard ivar formatting rules:
/// - preset `NOTHING` (no borders or divider lines)
/// - padding `(0, 2)` on all columns except the last which has `(0, 0)`
/// - `force_no_tty()` to prevent comfy-table from inserting raw ANSI codes
/// - `Dynamic` arrangement + `set_width` if `term::table_width()` is `Some`, else `Disabled`
#[must_use]
pub fn new(header: &[&str]) -> Table {
    let mut table = Table::new();
    table.load_style(NOTHING);
    table.force_no_tty();
    table.set_header(header);

    let col_count = header.len();
    for (i, col) in table.column_iter_mut().enumerate() {
        if i + 1 == col_count {
            col.set_padding((0, 0));
        } else {
            col.set_padding((0, 2));
        }
    }

    if let Some(width) = term::table_width() {
        table.set_content_arrangement(ContentArrangement::Dynamic);
        table.set_width(width);
    } else {
        table.set_content_arrangement(ContentArrangement::Disabled);
    }

    table
}

/// Write `table` to `w`: trims trailing whitespace from each line, paints
/// line 0 (the header) with [`HEADER`], and writes a trailing newline after each line.
///
/// # Errors
///
/// Returns [`io::Error`] if writing to `w` fails.
pub fn write(w: &mut impl Write, table: &Table) -> io::Result<()> {
    let raw = table.to_string();
    for (idx, line) in raw.lines().enumerate() {
        let trimmed = line.trim_end();
        if idx == 0 {
            writeln!(w, "{}", paint(HEADER, trimmed))?;
        } else {
            writeln!(w, "{trimmed}")?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/unit/infra/table.rs"]
mod tests;
