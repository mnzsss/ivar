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
//! focused shell's buffer), and the input mode that decides focus indicators.
//!
//! # The panel
//!
//! [`Panel`] is the shell's text plus where in it the user is looking:
//! `lines` are all available lines, `scroll_offset` says how far above the
//! bottom the view sits (`0` = the live viewport), and `live` says whether
//! the block cursor belongs in the panel at all — it only ever appears at
//! the live bottom.

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

/// The focused shell's text and the user's position in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Panel {
    /// Every available line, oldest first (the emulator's live viewport, or
    /// the driver's line buffer when scrolling).
    pub lines: Vec<String>,
    /// Lines scrolled back from the bottom. `0` is the live view.
    pub scroll_offset: usize,
    /// Whether this is the live viewport — the only state that shows the
    /// block cursor.
    pub live: bool,
}

impl Panel {
    /// A blank panel.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            lines: Vec::new(),
            scroll_offset: 0,
            live: false,
        }
    }
}

/// Everything the feature view renders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// The title — the feature name (or hall root, for a session).
    pub title: String,
    /// The sidebar rows — promoted repos (or features, for a session).
    pub rows: Vec<Row>,
    /// Which row is selected.
    pub selected: usize,
    /// Which input mode is active.
    pub mode: Mode,
    /// The right-hand panel.
    pub panel: Panel,
}

/// Render `snapshot` into `buf` (which must cover `area`).
///
/// Layout: sidebar at 30% width, the shell panel at 70%. The selected row is
/// highlighted; the panel's block cursor is drawn only when the panel is
/// [`Panel::live`].
pub fn render(snapshot: &Snapshot, area: Rect, buf: &mut Buffer) {
    let sidebar_width = (area.width * 30) / 100;
    let sidebar_area = Rect::new(area.x, area.y, sidebar_width, area.height);
    let panel_area = Rect::new(
        area.x + sidebar_width,
        area.y,
        area.width - sidebar_width,
        area.height,
    );

    render_sidebar(snapshot, sidebar_area, buf);
    render_panel(snapshot, panel_area, buf);
}

/// The window of `panel` that fits `available` rows.
///
/// Live (`scroll_offset == 0`): the emulator's rows are top-aligned with
/// blank padding below, so trim the padding and bottom-align the content the
/// way a real terminal does — the shell's last line always sits on the
/// bottom row, with the block cursor after it.
///
/// Scrolled (`scroll_offset > 0`): a window over the line buffer ending
/// `scroll_offset` lines above the bottom, blank-padded below when the top
/// of the buffer is reached.
///
/// Always returns exactly `available` rows.
#[must_use]
pub fn window(panel: &Panel, available: usize) -> Vec<String> {
    let available = available.max(1);
    if panel.scroll_offset == 0 {
        let mut lines = panel.lines.clone();
        while lines.last().is_some_and(|line| line.is_empty()) {
            lines.pop();
        }
        let mut windowed: Vec<String> = lines.into_iter().rev().take(available).collect();
        windowed.reverse();
        let mut padded = vec![String::new(); available.saturating_sub(windowed.len())];
        padded.append(&mut windowed);
        padded
    } else {
        let end = panel.lines.len().saturating_sub(panel.scroll_offset);
        let start = end.saturating_sub(available);
        let mut windowed: Vec<String> = panel
            .lines
            .iter()
            .skip(start)
            .take(available)
            .cloned()
            .collect();
        windowed.resize(available, String::new());
        windowed
    }
}

fn render_sidebar(snapshot: &Snapshot, area: Rect, buf: &mut Buffer) {
    let hint = match snapshot.mode {
        Mode::Focus => "focus — ctrl+b nav",
        Mode::Nav => "nav — j/k move, enter focus, q quit",
        Mode::Scroll => "scroll — pgup/pgdn, q/esc focus",
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
    let state = if snapshot.panel.live {
        "live"
    } else {
        "scroll"
    };
    let title = format!(" {selected} — {state} ");
    let block = Block::default().borders(Borders::ALL).title(title);

    let inner = block.inner(area);
    let available = usize::from(inner.height).max(1);
    let lines = window(&snapshot.panel, available);
    let body: Vec<Line> = lines.iter().map(|line| Line::raw(line.as_str())).collect();
    Paragraph::new(body).block(block).render(area, buf);

    // The block cursor lives only at the live bottom: the row after the
    // shell's last line, so it reads as the prompt position.
    if snapshot.panel.live && inner.height > 0 {
        let y = inner.y + inner.height - 1;
        let last = lines.last().map(String::as_str).unwrap_or("");
        let cursor_x = (last.chars().count() as u16).min(inner.width.saturating_sub(1));
        if let Some(cell) = buf.cell_mut((inner.x + cursor_x, y)) {
            cell.set_style(Style::default().add_modifier(Modifier::REVERSED));
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn snapshot() -> Snapshot {
        Snapshot {
            title: "checkout".to_owned(),
            rows: vec![
                Row {
                    label: "api".to_owned(),
                    status: "ready".to_owned(),
                },
                Row {
                    label: "web".to_owned(),
                    status: "pending".to_owned(),
                },
            ],
            selected: 1,
            mode: Mode::Focus,
            panel: Panel {
                lines: vec!["$ git status".to_owned(), "clean".to_owned()],
                scroll_offset: 0,
                live: true,
            },
        }
    }

    fn render_to_buffer(snap: &Snapshot, width: u16, height: u16) -> ratatui::buffer::Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(snap, frame.area(), frame.buffer_mut()))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    // -- window ---------------------------------------------------------------

    #[test]
    fn live_window_bottom_aligns_short_output() {
        let panel = Panel {
            lines: vec!["a".to_owned(), "b".to_owned(), String::new(), String::new()],
            scroll_offset: 0,
            live: true,
        };

        assert_eq!(window(&panel, 4), vec!["", "", "a", "b"]);
    }

    #[test]
    fn live_window_shows_the_last_rows_when_output_overflows() {
        let panel = Panel {
            lines: (1..=6).map(|n| n.to_string()).collect(),
            scroll_offset: 0,
            live: true,
        };

        assert_eq!(window(&panel, 4), vec!["3", "4", "5", "6"]);
    }

    #[test]
    fn scroll_window_ends_scroll_offset_lines_above_the_bottom() {
        let panel = Panel {
            lines: (1..=10).map(|n| n.to_string()).collect(),
            scroll_offset: 3,
            live: false,
        };

        // Offset 3: the window ends 3 lines above the bottom (line "10"), so
        // the last shown line is "7" and the 4-row window spans "4"..="7".
        assert_eq!(window(&panel, 4), vec!["4", "5", "6", "7"]);
    }

    #[test]
    fn scroll_window_pads_at_the_bottom_near_the_top_of_the_buffer() {
        let panel = Panel {
            lines: vec!["a".to_owned(), "b".to_owned()],
            scroll_offset: 2,
            live: false,
        };

        assert_eq!(window(&panel, 4), vec!["a", "b", "", ""]);
    }

    // -- rendering ------------------------------------------------------------

    #[test]
    fn rendering_shows_sidebar_rows_and_the_panel_text_headlessly() {
        let buffer = render_to_buffer(&snapshot(), 80, 24);

        let text: String = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol().to_owned())
            .collect();
        assert!(text.contains("api"), "sidebar rows must render: {text}");
        assert!(text.contains("web"));
        assert!(text.contains("ready"));
        assert!(
            text.contains("git status"),
            "panel text must render: {text}"
        );
        assert!(text.contains("checkout"), "the title must render: {text}");
    }

    #[test]
    fn the_selected_row_is_rendered_reversed() {
        let buffer = render_to_buffer(&snapshot(), 80, 24);

        let mut found_reversed = false;
        for cell in buffer.content() {
            if cell.symbol() == "w" && cell.style().add_modifier.contains(Modifier::REVERSED) {
                found_reversed = true;
            }
        }
        assert!(
            found_reversed,
            "`web` is selected, so its cells must be reversed"
        );
    }

    #[test]
    fn the_block_cursor_appears_only_at_the_live_bottom() {
        // Live: the cursor cell sits in the panel's bottom row.
        let live = render_to_buffer(&snapshot(), 80, 24);
        let bottom_y = 22;
        let cursor = live
            .content()
            .iter()
            .enumerate()
            .filter(|(index, cell)| {
                cell.style().add_modifier.contains(Modifier::REVERSED) && index / 80 == bottom_y
            })
            .count();
        assert!(
            cursor >= 1,
            "live panel must show a block cursor at the bottom"
        );

        // Scrolled: no cursor anywhere in the panel.
        let mut scrolled = snapshot();
        scrolled.panel.live = false;
        scrolled.panel.scroll_offset = 1;
        scrolled.panel.lines.push("older".to_owned());
        let scrolled_buffer = render_to_buffer(&scrolled, 80, 24);
        let panel_start = 24; // the 30% sidebar leaves the panel at column 24
        let cursor_in_panel = scrolled_buffer
            .content()
            .iter()
            .enumerate()
            .any(|(index, cell)| {
                cell.style().add_modifier.contains(Modifier::REVERSED) && index % 80 >= panel_start
            });
        assert!(
            !cursor_in_panel,
            "a non-live panel must not show a block cursor"
        );
    }

    #[test]
    fn nav_mode_renders_the_mode_hint() {
        let mut snap = snapshot();
        snap.mode = Mode::Nav;
        let buffer = render_to_buffer(&snap, 80, 24);

        let title_row: String = (0..80)
            .map(|x| buffer[(x, 0)].symbol().to_owned())
            .collect();
        assert!(title_row.contains("nav"), "was: {title_row}");
    }
}
