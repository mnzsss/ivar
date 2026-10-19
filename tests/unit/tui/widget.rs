#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::text::Line;

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
            lines: vec![Line::raw("$ git status"), Line::raw("clean")],
            scroll_offset: 0,
            state: PanelState::Live,
            cursor: Some((0, 2)),
            selection: None,
            caret: None,
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

#[test]
fn live_panel_keeps_short_output_at_emulator_top() {
    let buffer = render_to_buffer(&snapshot(), 80, 24);
    assert_eq!(buffer[(25, 1)].symbol(), "$");
    assert_eq!(buffer[(25, 2)].symbol(), "c");
    assert_eq!(buffer[(25, 22)].symbol(), " ");
}

#[test]
fn live_cursor_uses_emulator_row_and_column_not_last_glyph() {
    let mut snap = snapshot();
    snap.panel.lines = vec![Line::raw("$ "), Line::raw("second")];
    snap.panel.cursor = Some((0, 2));
    let buffer = render_to_buffer(&snap, 80, 24);
    assert!(
        buffer[(27, 1)]
            .style()
            .add_modifier
            .contains(Modifier::REVERSED)
    );
    assert!(
        !buffer[(30, 22)]
            .style()
            .add_modifier
            .contains(Modifier::REVERSED)
    );
    snap.panel.cursor = None;
    assert!(
        !render_to_buffer(&snap, 80, 24)[(27, 1)]
            .style()
            .add_modifier
            .contains(Modifier::REVERSED)
    );
}

#[test]
fn drag_highlight_is_confined_to_shell_panel() {
    let mut snap = snapshot();
    snap.panel.selection = Some(((0, 0), (1, 2)));
    snap.panel.cursor = None;
    let buffer = render_to_buffer(&snap, 80, 24);
    assert!(
        buffer[(25, 1)]
            .style()
            .add_modifier
            .contains(Modifier::REVERSED)
    );
    assert!(
        buffer[(27, 2)]
            .style()
            .add_modifier
            .contains(Modifier::REVERSED)
    );
    assert!(
        !buffer[(24, 1)]
            .style()
            .add_modifier
            .contains(Modifier::REVERSED)
    );
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
fn the_block_cursor_appears_only_at_the_live_emulator_position() {
    let live = render_to_buffer(&snapshot(), 80, 24);
    let bottom_y = 1;
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
        "live panel must show a block cursor at the emulator position"
    );

    // Scrolled: no cursor anywhere in the panel.
    let mut scrolled = snapshot();
    scrolled.panel.state = PanelState::Scrolling;
    scrolled.panel.scroll_offset = 1;
    scrolled.panel.lines.push(Line::raw("older"));
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
