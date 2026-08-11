//! The host loop that turns the driver into a running TUI.
//!
//! The other `tui` modules are pure or step-driven: `widget` projects a
//! [`Snapshot`], `key_router` reduces keys, `driver` owns the I/O as
//! explicit steps. This module is where those steps actually *run* — it
//! initialises the terminal (raw mode, alternate screen), owns the event
//! loop (`poll` keys, pump every shell's output, render one frame), and
//! restores the terminal on the way out, whatever the exit path.
//!
//! It holds **no `store` and no `action` state** — the layering table says
//! `tui` may not reach them, so the caller pushes in a ready-made
//! [`FeatureView`] (shells to spawn, rows to list) and reads nothing back.

use std::io;
use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;

use crate::error::Failure;
use crate::tui::driver::{Driver, PtsPty, ShellSpec};
use crate::tui::key_router::Key;
use crate::tui::widget::{Row, render};

/// Everything the feature-view host loop needs, pushed in by the action.
#[derive(Debug, Clone)]
pub struct FeatureView {
    /// The title — the feature name.
    pub title: String,
    /// The sidebar rows — promoted repos, with their statuses.
    pub rows: Vec<Row>,
    /// The shells to spawn — one per promoted repo, each in its worktree.
    pub shells: Vec<ShellSpec>,
}

/// Run the interactive loop: init the terminal, pump and render until the
/// user quits, then restore the terminal. Cleanup (leaving raw mode and the
/// alternate screen) runs even when the loop errors.
pub fn run(view: FeatureView) -> Result<(), Failure> {
    let (width, height) = crossterm::terminal::size().unwrap_or((80, 24));
    // `ratatui::init` enables raw mode and the alternate screen and installs
    // a panic hook that restores both; `restore` undoes them. Keeping the
    // pair here makes the restore unconditional.
    let mut terminal = ratatui::init();
    let result = run_loop(&mut terminal, view, width, height);
    ratatui::restore();
    result
}

/// The event loop: poll for keys (with a timeout so shell output is pumped
/// while the user types nothing), forward them through the driver, pump
/// every shell, and render one frame whenever anything changed.
fn run_loop(
    terminal: &mut DefaultTerminal,
    view: FeatureView,
    width: u16,
    height: u16,
) -> Result<(), Failure> {
    let mut driver = Driver::new(view.shells, PtsPty::new, width, height);
    let mut dirty = true;

    loop {
        if crossterm::event::poll(Duration::from_millis(50))
            .map_err(|source| io_failure("feature.tui_poll_failed", source))?
        {
            let event = crossterm::event::read()
                .map_err(|source| io_failure("feature.tui_read_failed", source))?;
            match event {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if let Some(key) = map_key(key) {
                        if driver.apply_event(key) {
                            break;
                        }
                        dirty = true;
                    }
                }
                Event::Resize(width, height) => {
                    driver.resize(width, height);
                    terminal
                        .resize(ratatui::layout::Rect::new(0, 0, width, height))
                        .map_err(|source| io_failure("feature.tui_resize_failed", source))?;
                    dirty = true;
                }
                _ => {}
            }
        }

        if driver
            .pump()
            .map_err(|source| io_failure("feature.tui_pump_failed", source))?
        {
            dirty = true;
        }

        if dirty {
            let snapshot = driver.snapshot(&view.title, &view.rows);
            terminal
                .draw(|frame| render(&snapshot, frame.area(), frame.buffer_mut()))
                .map_err(|source| io_failure("feature.tui_render_failed", source))?;
            dirty = false;
        }
    }

    Ok(())
}

/// Map a crossterm key press onto the [`Key`]s the router understands.
/// `Ctrl+B` is the prefix key. Everything else is carried through as
/// faithfully as the router can express it — in Focus mode this is the entire
/// keyboard the shell will ever see, so a key that is not mapped here is a key
/// that does not work in the view.
#[must_use]
pub fn map_key(key: KeyEvent) -> Option<Key> {
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    match key.code {
        KeyCode::Char('b') if control => Some(Key::CtrlB),
        KeyCode::Char(c) if control => Some(Key::Ctrl(c)),
        KeyCode::Char(c) if alt => Some(Key::Alt(c)),
        KeyCode::Char(c) => Some(Key::Char(c)),
        // Erasing is not optional: without this, nothing typed can be
        // corrected.
        KeyCode::Backspace => Some(Key::Backspace),
        KeyCode::Delete => Some(Key::Delete),
        KeyCode::Tab => Some(Key::Tab),
        KeyCode::BackTab => Some(Key::BackTab),
        KeyCode::Enter => Some(Key::Enter),
        KeyCode::Esc => Some(Key::Esc),
        KeyCode::Up => Some(Key::Up),
        KeyCode::Down => Some(Key::Down),
        KeyCode::Left => Some(Key::Left),
        KeyCode::Right => Some(Key::Right),
        KeyCode::Home => Some(Key::Home),
        KeyCode::End => Some(Key::End),
        KeyCode::Insert => Some(Key::Insert),
        KeyCode::PageUp => Some(Key::PgUp),
        KeyCode::PageDown => Some(Key::PgDn),
        _ => None,
    }
}

/// A terminal I/O failure, named for the step that hit it.
fn io_failure(code: &'static str, source: io::Error) -> Failure {
    Failure::failed(code, format!("terminal I/O error: {source}"))
}

#[cfg(test)]
#[path = "../../tests/unit/tui/master_detail.rs"]
mod tests;
