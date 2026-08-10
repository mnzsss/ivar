//! All I/O, one module: PTY reads/writes, crossterm events, resize.
//!
//! ARCHITECTURE.md, seam 6: `driver.rs` owns every byte of I/O and exposes
//! explicit step methods the host loop calls — `pump`, `apply_event`,
//! `resize` — and spawns no background tasks. It owns no executor; the host
//! loop (`tui::master_detail`) drives it.
//!
//! # The PTY seam
//!
//! A [`Pty`] is the outside world this driver talks to: something that
//! spawns a command, gives it a terminal, and yields bytes back. The real
//! implementation is [`PtsPty`] over `portable-pty`; tests use an in-memory
//! one — the driver is generic over the seam, which is what keeps it
//! testable without a real terminal.
//!
//! # One driver, many shells
//!
//! A feature view is one shell per promoted repo, each running in its own
//! worktree. The driver owns them all as a [`Vec`] of [`Shell`]s, spawns
//! lazily (a shell starts the first time it is focused — one process per
//! repo, not N at start-up), and keeps every shell's output flowing so a
//! background shell's scrollback is current when the user switches back.

use std::io;

use camino::{Utf8Path, Utf8PathBuf};

use crate::error::Failure;
use crate::infra::proc::Command;
use crate::tui::key_router::{Action, Direction, Key, Mode, move_selection, reduce};
use crate::tui::screen::Screen;
use crate::tui::widget::{Panel, Row, Snapshot};

/// How many lines of plain scrollback each shell keeps for scroll mode,
/// beyond the emulator's live viewport. Bounds the memory a long-running
/// build or test run can accumulate.
const MAX_BUFFER_LINES: usize = 5000;

/// A spawned interactive process with a terminal.
///
/// This is the whole outside world the driver sees: spawn, write bytes,
/// read bytes, check liveness. The host loop owns the *rate* of reads; the
/// driver owns the *fact* of them.
pub trait Pty {
    /// Spawn `command` in `cwd` with a terminal of `width`×`height`.
    /// Returns when the process is running or the spawn failed.
    fn spawn(
        &mut self,
        command: &Command,
        cwd: &Utf8Path,
        width: u16,
        height: u16,
    ) -> Result<(), Failure>;

    /// Write bytes into the process's stdin (through the PTY).
    fn write(&mut self, bytes: &[u8]) -> Result<(), io::Error>;

    /// Read whatever output is available, without blocking. `Ok(None)` when
    /// the process exited and there is nothing left to read.
    fn try_read(&mut self) -> Result<Option<Vec<u8>>, io::Error>;

    /// Whether the process is still running.
    fn is_running(&self) -> bool;
}

/// What a feature view shell is: a command to spawn, and where. One per
/// promoted repo, pushed in by the action — the driver never reads the hall.
#[derive(Debug, Clone)]
pub struct ShellSpec {
    /// The shell's label — the repo name shown in the sidebar.
    pub label: String,
    /// The directory the shell runs in — the repo's feature worktree.
    pub cwd: Utf8PathBuf,
    /// The command to spawn (the user's shell).
    pub command: Command,
}

/// One shell's state inside the driver. The PTY is `None` until the shell's
/// first focus (lazy spawn).
struct Shell<P: Pty> {
    spec: ShellSpec,
    pty: Option<P>,
    /// The emulator's live viewport.
    screen: Screen,
    /// Plain text lines for scroll mode (naive decode of the same bytes the
    /// emulator interprets — scrollback is a "last N lines" approximation).
    buffer: Vec<String>,
    /// Lines scrolled back from the bottom; `0` is live.
    scroll_offset: usize,
    /// Set when a lazy spawn failed; the panel shows the message instead of
    /// a shell.
    spawn_error: Option<String>,
}

/// The driver's view of the world: the shells it spawned, the mode and
/// selection the key router drives, and the factory that builds a fresh
/// [`Pty`] on demand. Owns no executor — every method is an explicit step
/// the host loop calls.
pub struct Driver<P: Pty, F: FnMut() -> P> {
    shells: Vec<Shell<P>>,
    factory: F,
    mode: Mode,
    selected: usize,
    width: u16,
    height: u16,
}

impl<P: Pty, F: FnMut() -> P> std::fmt::Debug for Driver<P, F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Driver")
            .field("shell_count", &self.shells.len())
            .field("mode", &self.mode)
            .field("selected", &self.selected)
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl<P: Pty, F: FnMut() -> P> Driver<P, F> {
    /// A driver over `shells`, using `factory` to build each [`Pty`] on its
    /// shell's first focus. The initially selected shell (index 0) spawns
    /// right away so the TUI opens on a live shell; every other shell spawns
    /// when it is first focused.
    pub fn new(shells: Vec<ShellSpec>, factory: F, width: u16, height: u16) -> Self {
        let mut driver = Self {
            shells: shells
                .into_iter()
                .map(|spec| Shell {
                    spec,
                    pty: None,
                    screen: Screen::new(width, height),
                    buffer: Vec::new(),
                    scroll_offset: 0,
                    spawn_error: None,
                })
                .collect(),
            factory,
            mode: Mode::Focus,
            selected: 0,
            width,
            height,
        };
        driver.ensure_spawned(0);
        driver
    }

    /// Whether any spawned shell is still alive.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.shells
            .iter()
            .any(|shell| shell.pty.as_ref().is_some_and(|pty| pty.is_running()))
    }

    /// The current mode.
    #[must_use]
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// The current selection index.
    #[must_use]
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Handle one key: reduce it, then perform whatever the action means.
    /// Returns `true` if the TUI should quit.
    pub fn apply_event(&mut self, key: Key) -> bool {
        let (next_mode, action) = reduce(self.mode, key);
        self.mode = next_mode;

        match action {
            Action::Quit => true,
            Action::Up => {
                self.selected = move_selection(self.selected, Direction::Up, self.shells.len());
                false
            }
            Action::Down => {
                self.selected = move_selection(self.selected, Direction::Down, self.shells.len());
                false
            }
            Action::FocusShell => {
                self.ensure_spawned(self.selected);
                false
            }
            Action::EnterNav | Action::EnterScroll => false,
            Action::ExitScroll => {
                if let Some(shell) = self.shells.get_mut(self.selected) {
                    shell.scroll_offset = 0;
                }
                false
            }
            Action::ScrollUp => {
                self.scroll_by(true);
                false
            }
            Action::ScrollDown => {
                self.scroll_by(false);
                false
            }
            Action::WriteBytes(bytes) => {
                if let Some(shell) = self.shells.get_mut(self.selected)
                    && let Some(pty) = &mut shell.pty
                {
                    let _ = pty.write(&bytes);
                }
                false
            }
            Action::None => false,
        }
    }

    /// Apply whatever output every shell produced since the last call.
    /// Returns `false` when there is nothing more to read right now, `true`
    /// when output was consumed (so the host may want to re-render).
    pub fn pump(&mut self) -> Result<bool, io::Error> {
        let mut consumed = false;
        for shell in &mut self.shells {
            let bytes = match &mut shell.pty {
                Some(pty) => pty.try_read()?,
                None => None,
            };
            if let Some(bytes) = bytes.filter(|bytes| !bytes.is_empty()) {
                shell.screen.feed(&bytes);
                append_to_buffer(&mut shell.buffer, &bytes);
                consumed = true;
            }
        }
        Ok(consumed)
    }

    /// Resize every shell's viewport (and the PTYs they will spawn into).
    pub fn resize(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;
        for shell in &mut self.shells {
            shell.screen.resize(width, height);
        }
    }

    /// The [`Snapshot`] the widget should render right now, built from the
    /// driver's state and the host-pushed `rows`.
    pub fn snapshot(&self, title: &str, rows: &[Row]) -> Snapshot {
        Snapshot {
            title: title.to_owned(),
            rows: rows.to_vec(),
            selected: self.selected,
            mode: self.mode,
            panel: self.panel(),
        }
    }

    /// The right-hand [`Panel`]: the focused shell's viewport (live, with the
    /// block cursor) or its scrollback, or a message when it has not spawned.
    fn panel(&self) -> Panel {
        let Some(shell) = self.shells.get(self.selected) else {
            return Panel::empty();
        };
        if let Some(error) = &shell.spawn_error {
            return Panel {
                lines: vec![format!("could not start shell: {error}")],
                scroll_offset: 0,
                live: false,
            };
        }
        if shell.pty.is_none() {
            return Panel {
                lines: vec!["press enter in nav to start a shell here".to_owned()],
                scroll_offset: 0,
                live: false,
            };
        }
        match self.mode {
            Mode::Scroll if shell.scroll_offset > 0 => Panel {
                lines: shell.buffer.clone(),
                scroll_offset: shell.scroll_offset.min(shell.buffer.len()),
                live: false,
            },
            _ => Panel {
                lines: shell.screen.rows().to_vec(),
                scroll_offset: 0,
                live: self.mode == Mode::Focus,
            },
        }
    }

    /// Spawn shell `index`'s PTY, unless it already has one (or already tried
    /// and failed). A failure is recorded on the shell, never propagated —
    /// the TUI keeps running, and the panel shows why.
    fn ensure_spawned(&mut self, index: usize) {
        let Some(shell) = self.shells.get_mut(index) else {
            return;
        };
        if shell.pty.is_some() || shell.spawn_error.is_some() {
            return;
        }
        let mut pty = (self.factory)();
        match pty.spawn(
            &shell.spec.command,
            &shell.spec.cwd,
            self.width,
            self.height,
        ) {
            Ok(()) => shell.pty = Some(pty),
            Err(failure) => shell.spawn_error = Some(failure.to_string()),
        }
    }

    /// Move the focused shell's scroll offset by one page (a page is the
    /// viewport height), clamped to the buffer.
    fn scroll_by(&mut self, up: bool) {
        let Some(shell) = self.shells.get_mut(self.selected) else {
            return;
        };
        let page = usize::from(self.height.saturating_sub(2)).max(1);
        if up {
            shell.scroll_offset = (shell.scroll_offset + page).min(shell.buffer.len());
        } else {
            shell.scroll_offset = shell.scroll_offset.saturating_sub(page);
        }
    }
}

/// Append decoded `bytes` to a shell's plain-text scrollback, joining a
/// trailing partial line onto the previous one (the next chunk completes it).
fn append_to_buffer(buffer: &mut Vec<String>, bytes: &[u8]) {
    let text = String::from_utf8_lossy(bytes);
    let mut lines = text.split('\n');
    if let Some(first) = lines.next() {
        if let Some(last) = buffer.last_mut() {
            last.push_str(first);
        } else if !first.is_empty() {
            buffer.push(first.to_owned());
        }
    }
    for line in lines {
        buffer.push(line.to_owned());
    }
    if buffer.len() > MAX_BUFFER_LINES {
        let excess = buffer.len() - MAX_BUFFER_LINES;
        buffer.drain(..excess);
    }
}

pub use super::pty::PtsPty;

#[cfg(test)]
#[path = "../../tests/unit/tui/driver.rs"]
mod tests;
