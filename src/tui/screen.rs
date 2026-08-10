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

/// A text viewport of a terminal screen.
///
/// Deliberately lossy: the widget does not need cell colours for the agent
/// scrollback — it needs text, at a size, that fits the panel. Keeping the
/// seam to text is what makes the emulator swap cheap.
/// How many rows of scrollback the emulator keeps beyond the visible
/// viewport. The widget only ever renders the visible part; the scrollback
/// is what `vt100` needs so a long agent output does not truncate mid-line
/// before it scrolls into view.
const SCROLLBACK_ROWS: usize = 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screen {
    /// The current viewport, one string per row.
    rows: Vec<String>,
    /// The viewport size.
    width: u16,
    height: u16,
}

impl Screen {
    /// A blank screen at `width` × `height`.
    #[must_use]
    pub fn new(width: u16, height: u16) -> Self {
        Self {
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
        // The scrollback is what lets an agent's long output scroll back up;
        // the viewport rows below are what the widget renders.
        let mut parser = vt100::Parser::new(self.height, self.width, SCROLLBACK_ROWS);
        parser.process(bytes);
        self.rows = parser
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
        self.rows.resize(height as usize, String::new());
        for row in &mut self.rows {
            row.truncate(width as usize);
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/tui/screen.rs"]
mod tests;
