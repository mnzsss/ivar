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
use ratatui::text::Line;

use crate::error::Failure;
use crate::infra::proc::Command;
use crate::tui::key_router::{
    Action, Direction, Key, Mode, move_selection, reduce, reduce_exited, reduce_wheel,
};
use crate::tui::screen::Screen;
use crate::tui::widget::{Panel, PanelState, Row, Snapshot};

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

    /// Tell the process its terminal changed size.
    ///
    /// A shell that is not told draws to the size it was spawned with, so
    /// every line wraps in the wrong place until it is restarted. This is
    /// the half of a resize the emulator cannot do on its own.
    fn resize(&mut self, width: u16, height: u16) -> Result<(), io::Error>;

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
    /// Plain text lines for scroll mode (the same bytes the emulator
    /// interprets, stripped of their escape sequences — scrollback is a
    /// "last N lines" approximation).
    buffer: Vec<String>,
    /// Where the plain-text decode left off, so an escape sequence split
    /// across two PTY chunks is still stripped whole.
    decode: Decode,
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
                    decode: Decode::Text,
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
    ///
    /// Which reducer runs depends on the focused shell: a dead one cannot
    /// take keys, so [`reduce_exited`] rebinds Focus to restart-or-quit
    /// rather than writing into a PTY that is gone.
    pub fn apply_event(&mut self, key: Key) -> bool {
        let (next_mode, action) = if self.focused_is_dead() {
            reduce_exited(self.mode, key)
        } else {
            reduce(self.mode, key)
        };
        self.mode = next_mode;
        self.perform(action)
    }

    /// Handle one mouse wheel notch. Never reaches the PTY: the terminal
    /// sends the wheel as arrow keys while the alternate screen is up, and
    /// forwarding those is what made scrolling type into the shell.
    pub fn scroll_wheel(&mut self, direction: Direction) {
        let (next_mode, action) = reduce_wheel(self.mode, direction);
        self.mode = next_mode;
        self.perform(action);
    }

    /// Perform one [`Action`]. Returns `true` if the TUI should quit.
    fn perform(&mut self, action: Action) -> bool {
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
            Action::ScrollLines(direction, lines) => {
                self.scroll_lines(direction, lines);
                false
            }
            Action::Restart => {
                self.respawn(self.selected);
                false
            }
            Action::WriteBytes(bytes) => {
                if let Some(shell) = self.shells.get_mut(self.selected) {
                    // Typing is a jump back to the live bottom: the output
                    // this key produces lands there, and a panel left
                    // scrolled back would hide it.
                    shell.scroll_offset = 0;
                    if let Some(pty) = &mut shell.pty {
                        let _ = pty.write(&bytes);
                    }
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
                append_to_buffer(&mut shell.buffer, &mut shell.decode, &bytes);
                consumed = true;
            }
        }
        Ok(consumed)
    }

    /// Resize every shell's viewport, and tell every already-spawned shell
    /// that its terminal changed size.
    ///
    /// Both halves matter: resizing only the emulator leaves the shell
    /// wrapping its lines at the old width, which is invisible here and very
    /// visible on screen.
    pub fn resize(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;
        for shell in &mut self.shells {
            shell.screen.resize(width, height);
            if let Some(pty) = &mut shell.pty {
                // A shell that refuses the resize is not worth tearing the
                // view down for; it just keeps the size it had.
                let _ = pty.resize(width, height);
            }
        }
    }

    /// The [`Snapshot`] the widget should render right now, built from the
    /// driver's state and the host-pushed `rows`.
    pub fn snapshot(&self, title: &str, rows: &[Row], prefix: &str) -> Snapshot {
        Snapshot {
            title: title.to_owned(),
            prefix: prefix.to_owned(),
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
                lines: vec![Line::raw(format!("could not start shell: {error}"))],
                scroll_offset: 0,
                state: PanelState::Exited,
            };
        }
        let Some(pty) = &shell.pty else {
            return Panel {
                lines: vec![Line::raw("press enter in nav to start a shell here")],
                scroll_offset: 0,
                state: PanelState::Scrolling,
            };
        };
        // Scrolled back is scrolled back, whatever the mode: the wheel
        // scrolls without leaving Focus, so the offset — not the mode — is
        // what decides which view the panel is.
        match shell.scroll_offset {
            offset if offset > 0 => Panel {
                // Scrollback is the plain-text approximation, so it is the one
                // view that has no colour to carry.
                lines: shell.buffer.iter().cloned().map(Line::raw).collect(),
                scroll_offset: offset.min(shell.buffer.len()),
                state: PanelState::Scrolling,
            },
            _ => Panel {
                lines: shell.screen.styled_rows().to_vec(),
                scroll_offset: 0,
                state: if !pty.is_running() {
                    // The last frame stays readable — it is the output the
                    // user came for — but the title says the shell is gone,
                    // and the cursor stops pretending to be a prompt.
                    PanelState::Exited
                } else if self.mode == Mode::Focus {
                    PanelState::Live
                } else {
                    PanelState::Scrolling
                },
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

    /// Give shell `index` a fresh process after its own exited: a new PTY
    /// and a blank screen, in the same worktree. The scrollback survives —
    /// it is the output the user came for, and losing it to a restart would
    /// be the worst moment to lose it.
    fn respawn(&mut self, index: usize) {
        let (width, height) = (self.width, self.height);
        let Some(shell) = self.shells.get_mut(index) else {
            return;
        };
        if shell.pty.as_ref().is_some_and(|pty| pty.is_running()) {
            return;
        }
        shell.pty = None;
        shell.spawn_error = None;
        shell.scroll_offset = 0;
        shell.screen = Screen::new(width, height);
        self.ensure_spawned(index);
    }

    /// Whether the focused shell can still take keys. A shell that never
    /// spawned is not dead — it has simply not started yet.
    fn focused_is_dead(&self) -> bool {
        self.shells.get(self.selected).is_some_and(|shell| {
            shell.spawn_error.is_some() || shell.pty.as_ref().is_some_and(|pty| !pty.is_running())
        })
    }

    /// Move the focused shell's scroll offset by one page (a page is the
    /// viewport height), clamped to the buffer.
    fn scroll_by(&mut self, up: bool) {
        let page = usize::from(self.height.saturating_sub(2)).max(1);
        let direction = if up { Direction::Up } else { Direction::Down };
        self.scroll_lines(direction, page);
    }

    /// Move the focused shell's scroll offset by `lines`, clamped to the
    /// buffer at the top and to the live bottom at the other end.
    fn scroll_lines(&mut self, direction: Direction, lines: usize) {
        let Some(shell) = self.shells.get_mut(self.selected) else {
            return;
        };
        shell.scroll_offset = match direction {
            Direction::Up => (shell.scroll_offset + lines).min(shell.buffer.len()),
            Direction::Down => shell.scroll_offset.saturating_sub(lines),
        };
    }
}

/// Where a plain-text decode left off. Escape sequences arrive split across
/// PTY chunks as often as not, so the parser has to be resumable — a state
/// machine, not a regex over one chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Decode {
    /// Ordinary text.
    Text,
    /// Just saw `ESC`; the next byte says which kind of sequence this is.
    Escape,
    /// Inside a CSI (`ESC [ … final`) — the colours, the cursor moves, all
    /// of it. Ends at the first byte in `0x40..=0x7e`.
    Csi,
    /// Inside an OSC (`ESC ] … BEL` or `… ESC \`) — window titles, and the
    /// shell integration marks a modern prompt emits.
    Osc,
    /// Saw `ESC` inside an OSC: a `\` ends the sequence, anything else is
    /// still OSC payload.
    OscEscape,
    /// A two-byte escape whose second byte carries no meaning here (charset
    /// selection, `ESC ( B` and friends).
    Skip,
    /// Just saw `\r`, and what it means depends on the next byte: `\r\n` is
    /// the line ending a PTY sends, while a lone `\r` rewrites the line.
    CarriageReturn,
}

/// Append `bytes` to a shell's scrollback as plain text: escape sequences
/// stripped, `\r` treated the way a terminal treats it (the line starts
/// over), and a trailing partial line joined onto the previous one.
///
/// The emulator renders the live viewport, so this is only what scroll mode
/// reads — and there it has to be *text*. Keeping the raw bytes put the
/// shell's own escape sequences on screen as `[32m` and `[A`, which is
/// exactly the noise scrolling back is meant to look past.
fn append_to_buffer(buffer: &mut Vec<String>, state: &mut Decode, bytes: &[u8]) {
    for ch in String::from_utf8_lossy(bytes).chars() {
        match *state {
            Decode::Text => feed_text(buffer, state, ch),
            // A lone `\r` rewrites the line it is on — progress bars and
            // spinners are one line, redrawn — but `\r\n` is just the line
            // ending, and clearing on it would empty every line there is.
            Decode::CarriageReturn => {
                *state = Decode::Text;
                if ch == '\n' {
                    buffer.push(String::new());
                } else {
                    if let Some(last) = buffer.last_mut() {
                        last.clear();
                    }
                    feed_text(buffer, state, ch);
                }
            }
            Decode::Escape => {
                *state = match ch {
                    '[' => Decode::Csi,
                    ']' => Decode::Osc,
                    '(' | ')' | '#' | '%' => Decode::Skip,
                    _ => Decode::Text,
                };
            }
            // A CSI ends at its final byte; everything before it is
            // parameters and intermediates.
            Decode::Csi => {
                if matches!(ch, '\x40'..='\x7e') {
                    *state = Decode::Text;
                }
            }
            Decode::Osc => match ch {
                '\x07' => *state = Decode::Text,
                '\x1b' => *state = Decode::OscEscape,
                _ => {}
            },
            Decode::OscEscape => {
                *state = if ch == '\\' {
                    Decode::Text
                } else {
                    Decode::Osc
                };
            }
            Decode::Skip => *state = Decode::Text,
        }
    }
    if buffer.len() > MAX_BUFFER_LINES {
        let excess = buffer.len() - MAX_BUFFER_LINES;
        buffer.drain(..excess);
    }
}

/// One character of ordinary text, with the control characters that mean
/// something to a line of text handled and the rest dropped.
fn feed_text(buffer: &mut Vec<String>, state: &mut Decode, ch: char) {
    match ch {
        '\x1b' => *state = Decode::Escape,
        '\n' => buffer.push(String::new()),
        '\r' => *state = Decode::CarriageReturn,
        '\t' => push_char(buffer, '\t'),
        // Every other control byte is an instruction to a terminal, not
        // text: BEL, backspace, the lot.
        _ if ch.is_control() => {}
        _ => push_char(buffer, ch),
    }
}

/// Append one character to the line the buffer is currently on, starting a
/// first line if there is none yet.
fn push_char(buffer: &mut Vec<String>, ch: char) {
    match buffer.last_mut() {
        Some(last) => last.push(ch),
        None => buffer.push(ch.to_string()),
    }
}

pub use super::pty::PtsPty;

#[cfg(test)]
#[path = "../../tests/unit/tui/driver.rs"]
mod tests;
