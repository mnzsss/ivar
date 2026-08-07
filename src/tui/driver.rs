//! All I/O, one module: PTY reads/writes, crossterm events, resize.
//!
//! ARCHITECTURE.md, seam 6: `driver.rs` owns every byte of I/O and exposes
//! explicit step methods the host loop calls — `refresh`, `apply_event`,
//! `apply_output_chunk` — and spawns no background tasks. It owns no
//! executor; the `session` action's host loop drives it.
//!
//! # The PTY seam
//!
//! A [`Pty`] is the outside world this driver talks to: something that
//! spawns a command, gives it a terminal, and yields bytes back. The real
//! implementation uses `portable-pty`; tests use an in-memory one — the
//! driver is generic over the seam, which is what keeps it testable without
//! a real terminal.

use std::io;

use camino::Utf8Path;

use crate::error::{Failure, FixAction};
use crate::tui::key_router::{Action, Direction, Key, Mode, move_selection, reduce};
use crate::tui::screen::Screen;
use crate::tui::widget::{Row, Snapshot};

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
        command: &crate::infra::proc::Command,
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

/// The driver's view of the world: the snapshot it renders, the screen it
/// renders into, and the PTY it talks to. Owns no executor, spawns no
/// tasks — every method is an explicit step the host loop calls.
pub struct Driver<P: Pty> {
    pty: P,
    mode: Mode,
    selected: usize,
    screen: Screen,
}

impl<P: Pty> std::fmt::Debug for Driver<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Driver")
            .field("mode", &self.mode)
            .field("selected", &self.selected)
            .field("screen", &self.screen)
            .finish_non_exhaustive()
    }
}

impl<P: Pty> Driver<P> {
    /// A driver over `pty`, with a blank screen at `width`×`height`.
    pub fn new(pty: P, width: u16, height: u16) -> Self {
        Self {
            pty,
            mode: Mode::Navigate,
            selected: 0,
            screen: Screen::new(width, height),
        }
    }

    /// Whether the agent process is still alive.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.pty.is_running()
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

    /// The agent's scrollback as text.
    #[must_use]
    pub fn agent_text(&self) -> String {
        self.screen.as_text()
    }

    /// Handle one key: reduce it, then perform whatever the action means.
    /// Returns `true` if the TUI should quit.
    pub fn apply_event(&mut self, key: Key, rows: &[Row]) -> bool {
        let row_count = rows.len();
        let (next_mode, action) = reduce(self.mode, key);
        self.mode = next_mode;

        match action {
            Action::Quit => true,
            Action::Up => {
                self.selected = move_selection(self.selected, Direction::Up, row_count);
                false
            }
            Action::Down => {
                self.selected = move_selection(self.selected, Direction::Down, row_count);
                false
            }
            Action::FocusAgent | Action::FocusNavigate => {
                self.selected = self.selected.clamp(0, row_count.saturating_sub(1));
                false
            }
            Action::WriteBytes(bytes) => {
                let _ = self.pty.write(&bytes);
                false
            }
            Action::None => false,
        }
    }

    /// Apply whatever output the PTY produced since the last call. Returns
    /// `false` when there is nothing more to read right now, `true` when
    /// output was consumed (so the host may want to re-render).
    pub fn pump(&mut self) -> Result<bool, io::Error> {
        match self.pty.try_read()? {
            Some(bytes) if !bytes.is_empty() => {
                self.screen.feed(&bytes);
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Resize the viewport and the PTY.
    pub fn resize(&mut self, width: u16, height: u16) {
        self.screen.resize(width, height);
    }

    /// The [`Snapshot`] the widget should render right now, built from the
    /// driver's state and the host-pushed `rows`/`detail`.
    pub fn snapshot(&self, root: &str, rows: &[Row], detail: &str) -> Snapshot {
        Snapshot {
            root: root.to_owned(),
            rows: rows.to_vec(),
            selected: self.selected,
            detail: detail.to_owned(),
            agent_scrollback: self.screen.as_text(),
            mode: self.mode,
        }
    }
}

/// Spawn `command` in `cwd` through `pty`, after checking the harness can
/// actually do what is being asked. `resume` is validated by the caller;
/// this only wires spawn-time facts (size) into the process.
pub fn spawn_agent(
    pty: &mut impl Pty,
    command: &crate::infra::proc::Command,
    cwd: &Utf8Path,
    width: u16,
    height: u16,
) -> Result<(), Failure> {
    pty.spawn(command, cwd, width, height)
}

/// The standard error for a PTY that died mid-session — the agent is gone,
/// and the fix action names the one thing a user can do: look at why.
pub fn agent_died() -> Failure {
    Failure::failed(
        "session.agent_died",
        "the agent process exited while the TUI was running",
    )
    .expected("the agent to stay alive for the session")
    .actual("the process terminated")
    .fix(FixAction::safe(
        "session.rerun",
        "Check the agent's exit message and start the session again.",
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::sync::mpsc;
    use std::sync::mpsc::{Receiver, Sender};

    use super::*;

    /// An in-memory PTY: written bytes land in a channel, read bytes come
    /// from a queue the test controls.
    struct FakePty {
        written: Sender<Vec<u8>>,
        output: Receiver<Vec<u8>>,
        running: bool,
    }

    impl FakePty {
        fn new() -> (Self, Receiver<Vec<u8>>, Sender<Vec<u8>>) {
            let (write_tx, write_rx) = mpsc::channel();
            let (read_tx, read_rx) = mpsc::channel();
            (
                Self {
                    written: write_tx,
                    output: read_rx,
                    running: true,
                },
                write_rx,
                read_tx,
            )
        }
    }

    impl Pty for FakePty {
        fn spawn(
            &mut self,
            _command: &crate::infra::proc::Command,
            _cwd: &Utf8Path,
            _width: u16,
            _height: u16,
        ) -> Result<(), Failure> {
            self.running = true;
            Ok(())
        }

        fn write(&mut self, bytes: &[u8]) -> Result<(), io::Error> {
            let _ = self.written.send(bytes.to_vec());
            Ok(())
        }

        fn try_read(&mut self) -> Result<Option<Vec<u8>>, io::Error> {
            Ok(self.output.try_recv().ok())
        }

        fn is_running(&self) -> bool {
            self.running
        }
    }

    fn rows() -> Vec<Row> {
        vec![
            Row {
                label: "api".to_owned(),
                status: "ready".to_owned(),
            },
            Row {
                label: "web".to_owned(),
                status: "pending".to_owned(),
            },
        ]
    }

    #[test]
    fn navigation_moves_the_selection() {
        let (pty, _write_rx, _read_tx) = FakePty::new();
        let mut driver = Driver::new(pty, 80, 24);
        let rows = rows();

        driver.apply_event(Key::Down, &rows);
        assert_eq!(driver.selected(), 1);
        driver.apply_event(Key::Down, &rows);
        assert_eq!(driver.selected(), 1, "clamped at the last row");
        driver.apply_event(Key::Up, &rows);
        assert_eq!(driver.selected(), 0);
    }

    #[test]
    fn agent_mode_keys_flow_to_the_pty() {
        let (pty, write_rx, _read_tx) = FakePty::new();
        let mut driver = Driver::new(pty, 80, 24);
        let rows = rows();

        // Enter focuses the agent; a typed key becomes a write.
        driver.apply_event(Key::Enter, &rows);
        assert_eq!(driver.mode(), Mode::Agent);
        driver.apply_event(Key::Char('x'), &rows);
        assert_eq!(write_rx.try_recv().unwrap(), vec![b'x']);
    }

    #[test]
    fn esc_returns_from_agent_to_navigation() {
        let (pty, _write_rx, _read_tx) = FakePty::new();
        let mut driver = Driver::new(pty, 80, 24);
        let rows = rows();

        driver.apply_event(Key::Enter, &rows);
        driver.apply_event(Key::Esc, &rows);
        assert_eq!(driver.mode(), Mode::Navigate);
    }

    #[test]
    fn pty_output_lands_in_the_screen() {
        let (pty, _write_rx, read_tx) = FakePty::new();
        let mut driver = Driver::new(pty, 80, 24);

        read_tx.send(b"hello from agent\n".to_vec()).unwrap();
        let consumed = driver.pump().unwrap();

        assert!(consumed);
        assert!(driver.agent_text().contains("hello from agent"));
    }

    #[test]
    fn q_quits() {
        let (pty, _write_rx, _read_tx) = FakePty::new();
        let mut driver = Driver::new(pty, 80, 24);
        let rows = rows();

        assert!(driver.apply_event(Key::Char('q'), &rows));
    }
}
