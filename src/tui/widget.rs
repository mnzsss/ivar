//! Pure projection of a snapshot into a `Buffer` — the referentially
//! transparent heart of the TUI (ARCHITECTURE.md, seam 6).
//!
//! This module never awaits, never opens anything, never reads the clock.
//! It takes a [`Snapshot`] and a [`Rect`] and returns a `Buffer`; two
//! renders of the same inputs produce byte-identical cells, which is what
//! makes it testable headless against ratatui's `TestBackend`.
//!
//! # The snapshot
//!
//! A [`Snapshot`] is the entire state the TUI can show, pushed *into* the
//! driver by the host loop — the widget never fetches. It is deliberately
//! small: a sidebar of rows (the promoted repos), the right-hand panel (the
//! focused shell's emulator viewport), and the input mode that decides focus indicators.
//!
//! # The panel
//!
//! [`Panel`] is the shell's output plus where in it the user is looking:
//! `lines` are all available lines (styled — the emulator's colours survive
//! the trip, see `screen`), `scroll_offset` says how far above the bottom the
//! view sits (`0` = the live viewport), and `state` says which of the three
//! things the panel currently is. The block cursor belongs only to
//! [`PanelState::Live`].

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph, Widget as _};

use super::key_router::Mode;

/// One row of the left-hand sidebar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The row's label.
    pub label: String,
    /// A one-word status, e.g. `ready` / `pending` / `failed`.
    pub status: String,
}

/// What the panel is showing right now. The three are exclusive, which is
/// why this is an enum and not a pair of flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelState {
    /// The shell's live viewport: the only state with a block cursor.
    Live,
    /// Scrollback, or a live viewport the user is not focused on.
    Scrolling,
    /// The shell has exited. Its last output stays readable, but nothing
    /// more is coming and keystrokes have nowhere to go.
    Exited,
}

/// The focused shell's output and the user's position in it.
#[derive(Debug, Clone, PartialEq)]
pub struct Panel {
    /// The emulator's visible rows, including styled history when scrolled.
    pub lines: Vec<Line<'static>>,
    /// Lines scrolled back from the bottom. `0` is the live view.
    pub scroll_offset: usize,
    /// What the panel is.
    pub state: PanelState,
    /// Emulator cursor, in panel-local cells; absent when hidden.
    pub cursor: Option<(u16, u16)>,
    /// Inclusive panel-local stream selection.
    pub selection: Option<((u16, u16), (u16, u16))>,
    /// Keyboard selection position before an anchor is set.
    pub caret: Option<(u16, u16)>,
}

impl Panel {
    /// A blank panel.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            lines: Vec::new(),
            scroll_offset: 0,
            state: PanelState::Scrolling,
            cursor: None,
            selection: None,
            caret: None,
        }
    }
}

/// Everything the feature view renders.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    /// The title — the feature name (or hall root, for a session).
    pub title: String,
    /// How the configured prefix key is spelled in the hints, e.g. `ctrl+o`.
    pub prefix: String,
    /// The sidebar rows — promoted repos (or features, for a session).
    pub rows: Vec<Row>,
    /// Which row is selected.
    pub selected: usize,
    /// Which input mode is active.
    pub mode: Mode,
    /// The right-hand panel.
    pub panel: Panel,
}

/// How the terminal splits: sidebar at 30% width, the shell panel at the
/// rest. One function so the host loop and the renderer cannot disagree
/// about how big the panel is.
#[must_use]
pub fn split(area: Rect) -> (Rect, Rect) {
    let sidebar_width = (area.width * 30) / 100;
    (
        Rect::new(area.x, area.y, sidebar_width, area.height),
        Rect::new(
            area.x + sidebar_width,
            area.y,
            area.width.saturating_sub(sidebar_width),
            area.height,
        ),
    )
}

/// The size, in cells, of the box a shell actually draws into for a terminal
/// of `area` — the panel minus its border.
///
/// This is the size the PTY and the emulator must be given. Handing them the
/// whole terminal instead is what makes a shell wrap its lines in the wrong
/// column: it believes it has room the panel does not have.
#[must_use]
pub fn panel_size(area: Rect) -> (u16, u16) {
    let (_, panel) = split(area);
    let inner = Block::default().borders(Borders::ALL).inner(panel);
    (inner.width.max(1), inner.height.max(1))
}

/// Render `snapshot` into `buf` (which must cover `area`).
///
/// The selected row is highlighted; the panel's block cursor is drawn only
/// when the panel is [`PanelState::Live`].
pub fn render(snapshot: &Snapshot, area: Rect, buf: &mut Buffer) {
    let (sidebar_area, panel_area) = split(area);
    render_sidebar(snapshot, sidebar_area, buf);
    render_panel(snapshot, panel_area, buf);
}

fn render_sidebar(snapshot: &Snapshot, area: Rect, buf: &mut Buffer) {
    let prefix = &snapshot.prefix;
    let hint = match snapshot.mode {
        Mode::Focus => format!("focus — {prefix} nav"),
        Mode::Nav => "nav — j/k move, v select, enter focus, q quit".to_owned(),
        Mode::Scroll => "scroll — pgup/pgdn, v select, q/esc focus".to_owned(),
        Mode::Select => "select — hjkl move, v anchor, y copy, esc cancel".to_owned(),
    };
    let title = format!(" {} — {hint} ", snapshot.title);
    let block = Block::default().borders(Borders::ALL).title(title);

    let mut lines: Vec<Line> = Vec::new();
    for (index, row) in snapshot.rows.iter().enumerate() {
        let mut style = Style::default();
        if index == snapshot.selected {
            style = style.add_modifier(Modifier::REVERSED);
        }
        lines.push(Line::styled(
            format!("  {}  {}", row.label, row.status),
            style,
        ));
    }

    Paragraph::new(lines).block(block).render(area, buf);
}

fn render_panel(snapshot: &Snapshot, area: Rect, buf: &mut Buffer) {
    let selected = snapshot
        .rows
        .get(snapshot.selected)
        .map(|row| row.label.as_str())
        .unwrap_or("?");
    let live = snapshot.panel.state == PanelState::Live;
    let state = match snapshot.panel.state {
        PanelState::Live => "live",
        PanelState::Scrolling => "scroll",
        PanelState::Exited => "exited",
    };
    // A dead shell is the one panel state with nothing to type into, so it
    // is the one that has to say what the keys do instead.
    let title = match snapshot.panel.state {
        PanelState::Exited => format!(" {selected} — {state} — enter restarts, q quits "),
        _ => format!(" {selected} — {state} "),
    };
    let block = Block::default().borders(Borders::ALL).title(title);

    let inner = block.inner(area);
    Paragraph::new(snapshot.panel.lines.clone())
        .block(block)
        .render(area, buf);
    if let Some((start, end)) = snapshot.panel.selection {
        let (start, end) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        for row in 0..inner.height {
            for col in 0..inner.width {
                if (row, col) >= start
                    && (row, col) <= end
                    && let Some(cell) = buf.cell_mut((inner.x + col, inner.y + row))
                {
                    cell.set_style(cell.style().add_modifier(Modifier::REVERSED));
                }
            }
        }
    }
    let cursor = if live { snapshot.panel.cursor } else { None };
    if let Some((row, col)) = snapshot.panel.caret.or(cursor)
        && row < inner.height
        && col < inner.width
        && let Some(cell) = buf.cell_mut((inner.x + col, inner.y + row))
    {
        cell.set_style(cell.style().add_modifier(Modifier::REVERSED));
    }
}

#[cfg(test)]
#[path = "../../tests/unit/tui/widget.rs"]
mod tests;
