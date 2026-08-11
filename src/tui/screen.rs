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
//! that takes bytes and gives back a plain-text viewport. Everything else in
//! the TUI talks to that interface, never to `vt100` directly.

/// How many rows of scrollback the emulator keeps beyond the visible
/// viewport. The widget only ever renders the visible part; the scrollback
/// is what `vt100` needs so a long agent output does not truncate mid-line
/// before it scrolls into view.
const SCROLLBACK_ROWS: usize = 1000;

/// A text viewport of a terminal screen.
///
/// Deliberately lossy: the widget does not need cell colours for the agent
/// scrollback — it needs text, at a size, that fits the panel. Keeping the
/// seam to text is what makes the emulator swap cheap.
pub struct Screen {
    /// The emulator itself. It is *state*: a shell's output arrives in as
    /// many chunks as the PTY happened to deliver, and every chunk continues
    /// the same terminal — same cursor, same scroll region, same modes.
    /// Rebuilding it per chunk would show only the newest one.
    parser: vt100::Parser,
    /// The current viewport, one string per row — the emulator's screen after
    /// the last feed, cached so `rows` stays a cheap borrow.
    rows: Vec<String>,
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

    /// Re-read the viewport out of the emulator into [`Screen::rows`].
    fn recache(&mut self) {
        self.rows = self
            .parser
            .screen()
            .rows(0, self.width)
            .map(|row| row.trim_end().to_owned())
            .collect();
        // `rows()` yields `height` rows; be defensive about a short one.
        self.rows.resize(self.height as usize, String::new());
    }

    /// The current viewport rows.
    #[must_use]
    pub fn rows(&self) -> &[String] {
        &self.rows
    }

    /// The viewport as one joined string (with newlines), for the widget.
    #[must_use]
    pub fn as_text(&self) -> String {
        self.rows.join("\n")
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

#[cfg(test)]
#[path = "../../tests/unit/tui/screen.rs"]
mod tests;
