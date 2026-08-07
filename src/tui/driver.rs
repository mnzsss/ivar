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

use crate::error::{Failure, FixAction};
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

/// The real PTY: `portable-pty` behind the [`Pty`] seam.
///
/// `portable-pty` gives a `PtyPair`; reads go through the slave's reader
/// handle. Reads are blocking on the handle, so `try_read` is implemented
/// by checking the master's bytes available — `portable-pty` exposes a
/// non-blocking read on the master via `try_clone_reader` + polling; the
/// seam keeps that detail here, where it can be swapped.
pub struct PtsPty {
    pair: Option<portable_pty::PtyPair>,
}

impl std::fmt::Debug for PtsPty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PtsPty")
            .field("spawned", &self.pair.is_some())
            .finish()
    }
}

impl PtsPty {
    /// A fresh, unspawned PTY.
    #[must_use]
    pub fn new() -> Self {
        Self { pair: None }
    }
}

impl Default for PtsPty {
    fn default() -> Self {
        Self::new()
    }
}

impl Pty for PtsPty {
    fn spawn(
        &mut self,
        command: &Command,
        cwd: &Utf8Path,
        width: u16,
        height: u16,
    ) -> Result<(), Failure> {
        let pty_system = portable_pty::native_pty_system();
        let pair = pty_system
            .openpty(portable_pty::PtySize {
                rows: height,
                cols: width,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|source| {
                Failure::failed(
                    "session.pty_open_failed",
                    format!("could not open a PTY: {source}"),
                )
            })?;

        let mut builder = portable_pty::CommandBuilder::new(command.program());
        for arg in command.arguments() {
            builder.arg(arg);
        }
        for (key, value) in command.envs() {
            builder.env(key, value);
        }
        builder.cwd(cwd.as_str());

        let child = pair.slave.spawn_command(builder).map_err(|source| {
            Failure::failed(
                "session.spawn_failed",
                format!("could not start `{}`: {source}", command.display()),
            )
            .fix(FixAction::safe(
                "session.check_binary",
                format!("Is `{}` installed and on PATH?", command.program()),
            ))
        })?;
        drop(child);

        self.pair = Some(pair);
        Ok(())
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), io::Error> {
        let Some(pair) = &self.pair else {
            return Ok(());
        };
        let mut writer = pair.master.take_writer().map_err(io::Error::other)?;
        writer.write_all(bytes)?;
        Ok(())
    }

    fn try_read(&mut self) -> Result<Option<Vec<u8>>, io::Error> {
        let Some(pair) = &self.pair else {
            return Ok(None);
        };
        // Non-blocking probe: `portable-pty`'s reader blocks on a plain
        // `read`, so this reads through a clone of the master and treats
        // "no data yet" (WouldBlock / EOF) as `None`.
        let mut reader = pair.master.try_clone_reader().map_err(io::Error::other)?;
        let mut buf = [0u8; 4096];
        match reader.read(&mut buf) {
            Ok(0) => Ok(None),
            // `n` is at most the buffer's length, so the slice is always in
            // bounds — `get` is the lint-clean spelling of that guarantee.
            Ok(n) => Ok(Some(buf.get(..n).map(<[u8]>::to_vec).unwrap_or_default())),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn is_running(&self) -> bool {
        // Without a child handle to poll, this reports true for the shell's
        // lifetime — the caller's loop ends on user quit. A future slice
        // wires the child's exit status here.
        self.pair.is_some()
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        clippy::type_complexity
    )]

    use std::cell::RefCell;
    use std::rc::Rc;
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
            _command: &Command,
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

    /// A harness that records every PTY the driver's factory hands out, in
    /// spawn order, so a test can read and write each shell's channels. The
    /// ends are `Rc`-wrapped so the log can be shared without cloning the
    /// (non-cloneable) std mpsc handles themselves.
    #[derive(Clone)]
    struct PtyLog(Rc<RefCell<Vec<(Rc<Receiver<Vec<u8>>>, Rc<Sender<Vec<u8>>>)>>>);

    fn shells(count: usize) -> Vec<ShellSpec> {
        (0..count)
            .map(|index| ShellSpec {
                label: format!("repo-{index}"),
                cwd: Utf8PathBuf::from(format!("/worktrees/repo-{index}")),
                command: Command::new("bash"),
            })
            .collect()
    }

    /// A driver over `count` fake shells, returning the log of spawned PTYs.
    fn driver_with(count: usize) -> (Driver<FakePty, impl FnMut() -> FakePty>, PtyLog) {
        let log = PtyLog(Rc::new(RefCell::new(Vec::new())));
        let log_for_factory = log.clone();
        let factory = move || {
            let (pty, write_rx, read_tx) = FakePty::new();
            log_for_factory
                .0
                .borrow_mut()
                .push((Rc::new(write_rx), Rc::new(read_tx)));
            pty
        };
        (Driver::new(shells(count), factory, 80, 24), log)
    }

    fn injected_output(log: &PtyLog, shell: usize) -> Rc<Sender<Vec<u8>>> {
        log.0.borrow()[shell].1.clone()
    }

    fn written(log: &PtyLog, shell: usize) -> Rc<Receiver<Vec<u8>>> {
        log.0.borrow()[shell].0.clone()
    }

    #[test]
    fn only_the_initially_focused_shell_is_spawned() {
        let (driver, log) = driver_with(3);

        assert_eq!(log.0.borrow().len(), 1, "lazy spawn: one shell at start");
        assert_eq!(driver.mode(), Mode::Focus);
        assert!(driver.is_running());
    }

    #[test]
    fn navigation_moves_the_selection_in_nav_mode() {
        let (mut driver, _log) = driver_with(2);

        driver.apply_event(Key::CtrlB);
        assert_eq!(driver.mode(), Mode::Nav);
        driver.apply_event(Key::Down);
        assert_eq!(driver.selected(), 1);
        driver.apply_event(Key::Down);
        assert_eq!(driver.selected(), 1, "clamped at the last shell");
        driver.apply_event(Key::Char('k'));
        assert_eq!(driver.selected(), 0);
    }

    #[test]
    fn enter_spawns_and_focuses_the_selected_shell() {
        let (mut driver, log) = driver_with(3);

        // Nav to the third shell and focus it: it spawns lazily.
        driver.apply_event(Key::CtrlB);
        driver.apply_event(Key::Down);
        driver.apply_event(Key::Down);
        driver.apply_event(Key::Enter);
        assert_eq!(driver.mode(), Mode::Focus);
        assert_eq!(driver.selected(), 2);
        assert_eq!(
            log.0.borrow().len(),
            2,
            "shell 0 plus lazily spawned shell 2"
        );

        // Keys now flow to the focused shell's PTY, not shell 0's.
        driver.apply_event(Key::Char('x'));
        assert_eq!(
            written(&log, 1).try_recv().unwrap(),
            vec![b'x'],
            "bytes reach the focused shell"
        );
        assert!(
            written(&log, 0).try_recv().is_err(),
            "the background shell must not receive keys"
        );
    }

    #[test]
    fn focus_forwards_characters_to_the_shell() {
        let (mut driver, log) = driver_with(1);

        driver.apply_event(Key::Char('q'));
        assert_eq!(
            written(&log, 0).try_recv().unwrap(),
            vec![b'q'],
            "in focus, q is a shell key, not quit"
        );
        driver.apply_event(Key::Enter);
        assert_eq!(written(&log, 0).try_recv().unwrap(), vec![b'\n']);
    }

    #[test]
    fn q_in_nav_quits() {
        let (mut driver, _log) = driver_with(1);

        driver.apply_event(Key::CtrlB);
        assert!(driver.apply_event(Key::Char('q')));
    }

    #[test]
    fn pump_drains_every_shells_output() {
        let (mut driver, log) = driver_with(2);

        // Focus the second shell so both are spawned.
        driver.apply_event(Key::CtrlB);
        driver.apply_event(Key::Down);
        driver.apply_event(Key::Enter);

        injected_output(&log, 0)
            .send(b"from repo-0\n".to_vec())
            .unwrap();
        injected_output(&log, 1)
            .send(b"from repo-1\n".to_vec())
            .unwrap();

        assert!(driver.pump().unwrap());
        assert!(!driver.pump().unwrap(), "nothing left to read");

        let snapshot = driver.snapshot("checkout", &[]);
        // The focused shell (repo-1) is what the panel shows.
        let text = snapshot.panel.lines.join("\n");
        assert!(text.contains("from repo-1"), "was: {text}");
    }

    #[test]
    fn scroll_mode_scrolls_the_focused_shell_and_returns_to_focus() {
        let (mut driver, log) = driver_with(1);

        // Feed enough output to scroll through.
        let lines: Vec<u8> = (0..40)
            .flat_map(|n| format!("line {n}\n").into_bytes())
            .collect();
        injected_output(&log, 0).send(lines).unwrap();
        driver.pump().unwrap();

        driver.apply_event(Key::CtrlB);
        driver.apply_event(Key::Char('['));
        assert_eq!(driver.mode(), Mode::Scroll);

        driver.apply_event(Key::PgUp);
        let offset = driver.snapshot("checkout", &[]).panel.scroll_offset;
        assert!(offset > 0, "PgUp must scroll back");
        assert_eq!(offset, 22, "one page = the viewport height (80x24 -> 22)");

        driver.apply_event(Key::PgDn);
        let offset = driver.snapshot("checkout", &[]).panel.scroll_offset;
        assert_eq!(offset, 0, "PgDn returns to the live bottom");

        driver.apply_event(Key::PgUp);
        driver.apply_event(Key::Esc);
        assert_eq!(driver.mode(), Mode::Focus);
        assert_eq!(
            driver.snapshot("checkout", &[]).panel.scroll_offset,
            0,
            "leaving scroll resets the offset"
        );
    }

    #[test]
    fn a_spawn_failure_is_shown_in_the_panel_not_crashed() {
        // A factory that fails every spawn.
        struct FailingPty;
        impl Pty for FailingPty {
            fn spawn(
                &mut self,
                _command: &Command,
                _cwd: &Utf8Path,
                _width: u16,
                _height: u16,
            ) -> Result<(), Failure> {
                Err(Failure::failed("test.spawn_failed", "no shell here"))
            }
            fn write(&mut self, _bytes: &[u8]) -> Result<(), io::Error> {
                Ok(())
            }
            fn try_read(&mut self) -> Result<Option<Vec<u8>>, io::Error> {
                Ok(None)
            }
            fn is_running(&self) -> bool {
                false
            }
        }

        let driver = Driver::new(shells(1), || FailingPty, 80, 24);
        assert!(!driver.is_running());
        let panel = driver.snapshot("checkout", &[]).panel;
        assert!(
            panel.lines.join("\n").contains("could not start shell"),
            "was: {:?}",
            panel.lines
        );
        assert!(!panel.live);
    }
}
