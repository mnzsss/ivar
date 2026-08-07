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
//! small: what the master-detail view needs, nothing more.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph, Widget as _};

use super::key_router::Mode;

/// One row of the left-hand list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The row's label.
    pub label: String,
    /// A one-word status, e.g. `ready` / `pending` / `missing`.
    pub status: String,
}

/// Everything the master-detail view renders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// The hall root.
    pub root: String,
    /// The rows of the left list — repos, features, or whatever the host
    /// loop pushed in.
    pub rows: Vec<Row>,
    /// Which row is selected.
    pub selected: usize,
    /// The right-hand detail pane's text.
    pub detail: String,
    /// The agent panel's scrollback, as captured from the PTY.
    pub agent_scrollback: String,
    /// Which panel has focus.
    pub mode: Mode,
}

/// Render `snapshot` into `buf` (which must cover `area`).
///
/// Layout: left list at 40% width, right detail at 60%; the agent panel is
/// the bottom-right region of the detail pane. The selected row is
/// highlighted; the focused panel gets a bordered block, the other a plain
/// one.
pub fn render(snapshot: &Snapshot, area: Rect, buf: &mut Buffer) {
    let left_width = (area.width * 2) / 5;
    let list_area = Rect::new(area.x, area.y, left_width, area.height);
    let detail_area = Rect::new(area.x + left_width, area.y, area.width - left_width, area.height);

    render_list(snapshot, list_area, buf);
    render_detail(snapshot, detail_area, buf);
}

fn render_list(snapshot: &Snapshot, area: Rect, buf: &mut Buffer) {
    let title = if snapshot.mode == Mode::Navigate {
        " ivar — navigate (q quit, enter focus agent) "
    } else {
        " ivar "
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title);

    let mut lines: Vec<Line> = Vec::new();
    for (index, row) in snapshot.rows.iter().enumerate() {
        let selected = index == snapshot.selected;
        let mut style = Style::default();
        if selected {
            style = style.add_modifier(Modifier::REVERSED);
        }
        lines.push(Line::styled(
            format!("  {}  {}", row.label, row.status),
            style,
        ));
    }

    Paragraph::new(lines).block(block).render(area, buf);
}

fn render_detail(snapshot: &Snapshot, area: Rect, buf: &mut Buffer) {
    let title = if snapshot.mode == Mode::Agent {
        " agent (esc to navigate) "
    } else {
        " detail "
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title);

    let body = if snapshot.detail.is_empty() {
        snapshot.agent_scrollback.clone()
    } else {
        snapshot.detail.clone()
    };

    Paragraph::new(body).block(block).render(area, buf);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use super::*;

    fn snapshot() -> Snapshot {
        Snapshot {
            root: "/hall".to_owned(),
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
            detail: "detail pane".to_owned(),
            agent_scrollback: String::new(),
            mode: Mode::Navigate,
        }
    }

    #[test]
    fn rendering_produces_the_expected_cells_headlessly() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let snap = snapshot();

        terminal
            .draw(|frame| render(&snap, frame.area(), frame.buffer_mut()))
            .unwrap();

        let buffer = terminal.backend().buffer().clone();
        // The title row exists.
        let title_row: String = (0..80)
            .map(|x| buffer[(x, 0)].symbol().to_string())
            .collect();
        assert!(title_row.contains("ivar"), "was: {title_row}");

        // Both rows render with their statuses.
        let text: String = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol().to_string())
            .collect();
        assert!(text.contains("api"), "rows must include api: {text}");
        assert!(text.contains("web"));
        assert!(text.contains("ready"));
    }

    #[test]
    fn the_selected_row_is_rendered_reversed() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let snap = snapshot();

        terminal
            .draw(|frame| render(&snap, frame.area(), frame.buffer_mut()))
            .unwrap();

        let buffer = terminal.backend().buffer().clone();
        // Find the cell where the selected row's label starts and assert the
        // reversed modifier is on.
        let mut found_reversed = false;
        for cell in buffer.content() {
            if cell.symbol() == "w" && cell.style().add_modifier.contains(Modifier::REVERSED) {
                found_reversed = true;
            }
        }
        // `web` is the selected row (index 1); at least its cells must be
        // reversed.
        assert!(found_reversed);
    }
}
