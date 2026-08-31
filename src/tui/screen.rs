//! The terminal-emulator seam — the swap point for how PTY output becomes
//! screen text.
//!
//! ARCHITECTURE.md's module map: `screen.rs` is "the Screen seam over vt100 —
//! the emulator swap point". The agent's PTY writes raw ANSI bytes; something
//! has to turn those into the cells the widget can render. This module owns
//! that something.
//!
//! `vt100` does the heavy lifting (the `vt100` crate is already a dependency,
//! ADR-0001 §2 notes its known gaps: no SGR 8/9 — tracked behind this seam so
//! `termwiz` stays a later swap). The seam is the *interface*: a [`Screen`]
//! that takes bytes and gives back a viewport, as text and as styled lines.
//! Everything else in the TUI talks to that interface, never to `vt100`
//! directly.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// How many rows of scrollback the emulator keeps beyond the visible
/// viewport. The widget only ever renders the visible part; the scrollback
/// is what `vt100` needs so a long agent output does not truncate mid-line
/// before it scrolls into view.
const SCROLLBACK_ROWS: usize = 1000;

/// A viewport of a terminal screen, as text and as styled lines.
///
/// ARCHITECTURE.md, seam 6: "in `ratatui` you own the cells, so a `vt100`
/// cell maps straight to a `Style` and the entire layer disappears." That
/// mapping lives here, because this is the module that knows `vt100` —
/// [`Screen::styled_rows`] is the whole of it, and swapping the emulator
/// means rewriting this file and nothing else.
pub struct Screen {
    /// The emulator itself. It is *state*: a shell's output arrives in as
    /// many chunks as the PTY happened to deliver, and every chunk continues
    /// the same terminal — same cursor, same scroll region, same modes.
    /// Rebuilding it per chunk would show only the newest one.
    parser: vt100::Parser,
    /// The current viewport, one string per row — the emulator's screen after
    /// the last feed, cached so `rows` stays a cheap borrow.
    rows: Vec<String>,
    /// The same viewport with the cells' colours and attributes kept. Cached
    /// alongside `rows` so a frame is a clone, not a re-walk of the grid.
    styled: Vec<Line<'static>>,
    /// The viewport size.
    width: u16,
    height: u16,
}

impl std::fmt::Debug for Screen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `vt100::Parser` is not `Debug`, and its innards are not what a
        // reader of this struct wants anyway.
        f.debug_struct("Screen")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("rows", &self.rows)
            .finish_non_exhaustive()
    }
}

impl Screen {
    /// A blank screen at `width` × `height`.
    #[must_use]
    pub fn new(width: u16, height: u16) -> Self {
        Self {
            // `vt100` panics on a zero-sized grid, and a zero-sized `Screen`
            // is a legal (if useless) thing to hold — `feed` refuses it. One
            // cell is the smallest grid that is not a panic.
            parser: vt100::Parser::new(height.max(1), width.max(1), SCROLLBACK_ROWS),
            rows: vec![String::new(); height as usize],
            styled: vec![Line::default(); height as usize],
            width,
            height,
        }
    }

    /// Feed raw terminal bytes into the emulator.
    ///
    /// Bytes are processed immediately; the viewport is whatever the
    /// emulator's screen shows afterwards. A screen with no rows (before any
    /// bytes arrive) stays blank.
    pub fn feed(&mut self, bytes: &[u8]) {
        if self.rows.is_empty() || self.width == 0 {
            return;
        }
        self.parser.process(bytes);
        self.recache();
    }

    /// Re-read the viewport out of the emulator into both caches.
    fn recache(&mut self) {
        let screen = self.parser.screen();
        self.rows = screen
            .rows(0, self.width)
            .map(|row| row.trim_end().to_owned())
            .collect();
        self.styled = (0..self.height)
            .map(|row| styled_row(screen, row, self.width))
            .collect();
        // `rows()` yields `height` rows; be defensive about a short one.
        self.rows.resize(self.height as usize, String::new());
        self.styled.resize(self.height as usize, Line::default());
    }

    /// The current viewport rows, as plain text.
    #[must_use]
    pub fn rows(&self) -> &[String] {
        &self.rows
    }

    /// The current viewport rows with their colours and attributes — what
    /// the panel renders, and the reason the view is not monochrome.
    #[must_use]
    pub fn styled_rows(&self) -> &[Line<'static>] {
        &self.styled
    }

    /// The viewport size.
    #[must_use]
    pub fn size(&self) -> (u16, u16) {
        (self.width, self.height)
    }

    /// Resize the viewport. The emulator reflows; content beyond the new
    /// size is dropped, content below is blank.
    pub fn resize(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;
        // The emulator reflows; re-reading it afterwards is what makes the
        // cached rows agree with the new size.
        self.parser
            .screen_mut()
            .set_size(height.max(1), width.max(1));
        self.recache();
    }
}

/// One row of the emulator's grid as a [`Line`], runs of equal style merged
/// into single spans.
fn styled_row(screen: &vt100::Screen, row: u16, width: u16) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut text = String::new();
    let mut current = Style::default();
    // A wide character owns two columns; the second is a continuation cell
    // that must not be emitted, or every following column shifts right.
    let mut skip_continuation = false;

    for col in 0..width {
        let cell = screen.cell(row, col);
        if skip_continuation {
            skip_continuation = false;
            continue;
        }
        skip_continuation = cell.is_some_and(vt100::Cell::is_wide);

        let (contents, style) = match cell {
            Some(cell) if cell.has_contents() => (cell.contents().to_owned(), cell_style(cell)),
            // An untouched cell is a blank at the default style, not a gap:
            // the panel is a grid, and columns have to line up.
            _ => (" ".to_owned(), Style::default()),
        };
        if style != current && !text.is_empty() {
            spans.push(Span::styled(std::mem::take(&mut text), current));
        }
        current = style;
        text.push_str(&contents);
    }
    if !text.is_empty() {
        spans.push(Span::styled(text, current));
    }

    trim_trailing_blanks(&mut spans);
    Line::from(spans)
}

/// Drop the run of unstyled blanks a grid row ends with. Without this every
/// row paints its full width, which is both wasteful and visible whenever the
/// panel's own background differs from the terminal's.
fn trim_trailing_blanks(spans: &mut Vec<Span<'static>>) {
    while let Some(last) = spans.last() {
        if last.style != Style::default() {
            return;
        }
        let trimmed = last.content.trim_end();
        if trimmed.is_empty() {
            spans.pop();
        } else {
            let trimmed = trimmed.to_owned();
            if let Some(last) = spans.last_mut() {
                last.content = trimmed.into();
            }
            return;
        }
    }
}

/// A `vt100` cell's colours and attributes as a `ratatui` [`Style`] — the
/// direct map ARCHITECTURE.md calls for.
fn cell_style(cell: &vt100::Cell) -> Style {
    let mut style = Style::default()
        .fg(colour(cell.fgcolor()))
        .bg(colour(cell.bgcolor()));
    if cell.bold() {
        style = style.add_modifier(Modifier::BOLD);
    }
    if cell.italic() {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if cell.underline() {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    if cell.inverse() {
        style = style.add_modifier(Modifier::REVERSED);
    }
    style
}

/// A `vt100` colour as a `ratatui` one. `Default` becomes `Reset` so the
/// terminal's own palette shows through instead of a guessed black or white.
fn colour(colour: vt100::Color) -> Color {
    match colour {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(index) => Color::Indexed(index),
        vt100::Color::Rgb(red, green, blue) => Color::Rgb(red, green, blue),
    }
}

#[cfg(test)]
#[path = "../../tests/unit/tui/screen.rs"]
mod tests;
