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
        prefix: "ctrl+o".to_owned(),
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
