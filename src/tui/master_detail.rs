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
use ratatui::layout::Rect;

use crate::error::Failure;
use crate::tui::driver::{Driver, PtsPty, ShellSpec};
use crate::tui::key_router::Key;
use crate::tui::widget::{Row, panel_size, render};

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
    let prefix = Prefix::from_env();
    // `ratatui::init` enables raw mode and the alternate screen and installs
    // a panic hook that restores both; `restore` undoes them. Keeping the
    // pair here makes the restore unconditional.
    let mut terminal = ratatui::init();
    let result = run_loop(&mut terminal, view, Rect::new(0, 0, width, height), &prefix);
    ratatui::restore();
    result
}

/// The chord that opens navigation.
///
/// `Ctrl+B` is what `tmux` uses, and what Orca binds — a view running inside
/// either never sees it. So the prefix is configuration, read once from
/// `IVAR_TUI_PREFIX` (`ctrl+<letter>` or `f<number>`, case-insensitive).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prefix {
    code: KeyCode,
    modifiers: KeyModifiers,
    label: String,
}

/// The prefix used when `IVAR_TUI_PREFIX` says nothing. `Ctrl+O` is close to
/// unbound in an interactive shell (readline's `operate-and-get-next`), which
/// is the point: the prefix is stolen from the shell for as long as the view
/// runs.
const DEFAULT_PREFIX: char = 'o';

impl Prefix {
    /// The configured prefix, or [`DEFAULT_PREFIX`] when the variable is
    /// unset or unparseable — a typo in an env var must not cost the user a
    /// way out of the TUI.
    #[must_use]
    pub fn from_env() -> Self {
        std::env::var("IVAR_TUI_PREFIX")
            .ok()
            .as_deref()
            .and_then(Self::parse)
            .unwrap_or_else(Self::default_prefix)
    }

    /// `Ctrl+O`.
    #[must_use]
    pub fn default_prefix() -> Self {
        Self {
            code: KeyCode::Char(DEFAULT_PREFIX),
            modifiers: KeyModifiers::CONTROL,
            label: format!("ctrl+{DEFAULT_PREFIX}"),
        }
    }

    /// Parse `ctrl+<letter>` or `f<number>`. `None` for anything else.
    #[must_use]
    pub fn parse(spec: &str) -> Option<Self> {
        let spec = spec.trim().to_ascii_lowercase();
        if let Some(rest) = spec.strip_prefix("ctrl+") {
            let mut chars = rest.chars();
            let letter = chars.next().filter(char::is_ascii_alphanumeric)?;
            if chars.next().is_some() {
                return None;
            }
            return Some(Self {
                code: KeyCode::Char(letter),
                modifiers: KeyModifiers::CONTROL,
                label: format!("ctrl+{letter}"),
            });
        }
        let number: u8 = spec.strip_prefix('f')?.parse().ok()?;
        (1..=12).contains(&number).then(|| Self {
            code: KeyCode::F(number),
            modifiers: KeyModifiers::NONE,
            label: format!("f{number}"),
        })
    }

    /// Whether `key` is this prefix.
    #[must_use]
    pub fn matches(&self, key: KeyEvent) -> bool {
        key.code == self.code && key.modifiers.contains(self.modifiers)
    }

    /// How the hints spell this key.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }
}

/// The event loop: poll for keys (with a timeout so shell output is pumped
/// while the user types nothing), forward them through the driver, pump
/// every shell, and render one frame whenever anything changed.
fn run_loop(
    terminal: &mut DefaultTerminal,
    view: FeatureView,
    area: Rect,
    prefix: &Prefix,
) -> Result<(), Failure> {
    // The shell draws inside the panel, not inside the terminal: sizing the
    // PTY to the whole screen is what makes its lines wrap in the wrong
    // column.
    let (width, height) = panel_size(area);
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
                    if let Some(key) = map_key(key, prefix) {
                        if driver.apply_event(key) {
                            break;
                        }
                        dirty = true;
                    }
                }
                Event::Resize(width, height) => {
                    let area = Rect::new(0, 0, width, height);
                    let (panel_width, panel_height) = panel_size(area);
                    driver.resize(panel_width, panel_height);
                    terminal
                        .resize(area)
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
            let snapshot = driver.snapshot(&view.title, &view.rows, prefix.label());
            terminal
                .draw(|frame| render(&snapshot, frame.area(), frame.buffer_mut()))
                .map_err(|source| io_failure("feature.tui_render_failed", source))?;
            dirty = false;
        }
    }

    Ok(())
}

/// Map a crossterm key press onto the [`Key`]s the router understands.
///
/// `prefix` is the configured chord that opens navigation. Everything else
/// is carried through as faithfully as the router can express it — in Focus
/// mode this is the entire keyboard the shell will ever see, so a key that
/// is not mapped here is a key that does not work in the view.
#[must_use]
pub fn map_key(key: KeyEvent, prefix: &Prefix) -> Option<Key> {
    // The prefix is matched first: it is the one key the shell never gets,
    // so a prefix of `ctrl+c` would mean the shell never gets `ctrl+c`.
    if prefix.matches(key) {
        return Some(Key::Prefix);
    }
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    match key.code {
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
