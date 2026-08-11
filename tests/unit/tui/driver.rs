#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::type_complexity
)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};

use super::*;
use crate::tui::widget::PanelState;

/// A panel's text, for assertions that do not care about styling.
fn panel_text(panel: &Panel) -> String {
    panel
        .lines
        .iter()
        .map(ratatui::text::Line::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

/// An in-memory PTY: written bytes land in a channel, read bytes come
/// from a queue the test controls.
struct FakePty {
    written: Sender<Vec<u8>>,
    output: Receiver<Vec<u8>>,
    /// Shared so a test can end the shell from outside, the way a real one
    /// ends on its own.
    running: Rc<Cell<bool>>,
    /// Every size this PTY was told about, in order.
    resized: Rc<RefCell<Vec<(u16, u16)>>>,
}

impl FakePty {
    fn new() -> (Self, PtyHandle) {
        let (write_tx, write_rx) = mpsc::channel();
        let (read_tx, read_rx) = mpsc::channel();
        let running = Rc::new(Cell::new(true));
        let resized = Rc::new(RefCell::new(Vec::new()));
        (
            Self {
                written: write_tx,
                output: read_rx,
                running: Rc::clone(&running),
                resized: Rc::clone(&resized),
            },
            PtyHandle {
                written: Rc::new(write_rx),
                output: Rc::new(read_tx),
                running,
                resized,
            },
        )
    }
}

/// The test's end of one [`FakePty`]: what it was told, and control over
/// whether it is still alive.
struct PtyHandle {
    written: Rc<Receiver<Vec<u8>>>,
    output: Rc<Sender<Vec<u8>>>,
    running: Rc<Cell<bool>>,
    resized: Rc<RefCell<Vec<(u16, u16)>>>,
}

impl Pty for FakePty {
    fn spawn(
        &mut self,
        _command: &Command,
        _cwd: &Utf8Path,
        _width: u16,
        _height: u16,
    ) -> Result<(), Failure> {
        self.running.set(true);
        Ok(())
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), io::Error> {
        let _ = self.written.send(bytes.to_vec());
        Ok(())
    }

    fn try_read(&mut self) -> Result<Option<Vec<u8>>, io::Error> {
        Ok(self.output.try_recv().ok())
    }

    fn resize(&mut self, width: u16, height: u16) -> Result<(), io::Error> {
        self.resized.borrow_mut().push((width, height));
        Ok(())
    }

    fn is_running(&self) -> bool {
        self.running.get()
    }
}

/// A harness that records every PTY the driver's factory hands out, in
/// spawn order, so a test can read and write each shell's channels. The
/// ends are `Rc`-wrapped so the log can be shared without cloning the
/// (non-cloneable) std mpsc handles themselves.
#[derive(Clone)]
struct PtyLog(Rc<RefCell<Vec<PtyHandle>>>);

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
        let (pty, handle) = FakePty::new();
        log_for_factory.0.borrow_mut().push(handle);
        pty
    };
    (Driver::new(shells(count), factory, 80, 24), log)
}

fn injected_output(log: &PtyLog, shell: usize) -> Rc<Sender<Vec<u8>>> {
    log.0.borrow()[shell].output.clone()
}

fn written(log: &PtyLog, shell: usize) -> Rc<Receiver<Vec<u8>>> {
    log.0.borrow()[shell].written.clone()
}

/// Every size shell `shell` was told about, in order.
fn resizes(log: &PtyLog, shell: usize) -> Vec<(u16, u16)> {
    log.0.borrow()[shell].resized.borrow().clone()
}

/// End shell `shell`, the way a real one ends when the user types `exit`.
fn stop(log: &PtyLog, shell: usize) {
    log.0.borrow()[shell].running.set(false);
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

    driver.apply_event(Key::Prefix);
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
    driver.apply_event(Key::Prefix);
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

    driver.apply_event(Key::Prefix);
    assert!(driver.apply_event(Key::Char('q')));
}

#[test]
fn pump_drains_every_shells_output() {
    let (mut driver, log) = driver_with(2);

    // Focus the second shell so both are spawned.
    driver.apply_event(Key::Prefix);
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

    let snapshot = driver.snapshot("checkout", &[], "ctrl+o");
    // The focused shell (repo-1) is what the panel shows.
    let text = panel_text(&snapshot.panel);
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

    driver.apply_event(Key::Prefix);
    driver.apply_event(Key::Char('['));
    assert_eq!(driver.mode(), Mode::Scroll);

    driver.apply_event(Key::PgUp);
    let offset = driver
        .snapshot("checkout", &[], "ctrl+o")
        .panel
        .scroll_offset;
    assert!(offset > 0, "PgUp must scroll back");
    assert_eq!(offset, 22, "one page = the viewport height (80x24 -> 22)");

    driver.apply_event(Key::PgDn);
    let offset = driver
        .snapshot("checkout", &[], "ctrl+o")
        .panel
        .scroll_offset;
    assert_eq!(offset, 0, "PgDn returns to the live bottom");

    driver.apply_event(Key::PgUp);
    driver.apply_event(Key::Esc);
    assert_eq!(driver.mode(), Mode::Focus);
    assert_eq!(
        driver
            .snapshot("checkout", &[], "ctrl+o")
            .panel
            .scroll_offset,
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
        fn resize(&mut self, _width: u16, _height: u16) -> Result<(), io::Error> {
            Ok(())
        }
        fn is_running(&self) -> bool {
            false
        }
    }

    let driver = Driver::new(shells(1), || FailingPty, 80, 24);
    assert!(!driver.is_running());
    let panel = driver.snapshot("checkout", &[], "ctrl+o").panel;
    assert!(
        panel_text(&panel).contains("could not start shell"),
        "was: {:?}",
        panel_text(&panel)
    );
    assert_eq!(panel.state, PanelState::Exited);
}

/// A resize has two halves, and the invisible one is the one that matters:
/// the emulator can be told the new size all it likes, but if the shell is
/// not told, it keeps wrapping its lines at the old width.
#[test]
fn resizing_tells_every_spawned_shell_its_new_size() {
    let (mut driver, log) = driver_with(2);

    // Only shell 0 is spawned at this point (lazy spawn).
    driver.resize(100, 40);
    assert_eq!(resizes(&log, 0), vec![(100, 40)]);

    // Focus the second shell: it spawns at the current size, and follows
    // every resize after that.
    driver.apply_event(Key::Prefix);
    driver.apply_event(Key::Down);
    driver.apply_event(Key::Enter);
    driver.resize(60, 20);

    assert_eq!(resizes(&log, 0), vec![(100, 40), (60, 20)]);
    assert_eq!(resizes(&log, 1), vec![(60, 20)]);
}

/// A shell that has exited must say so. Leaving the panel in `Live` shows a
/// block cursor at a prompt that is not there any more.
#[test]
fn an_exited_shell_is_marked_in_the_panel() {
    let (driver, log) = driver_with(1);
    assert_eq!(
        driver.snapshot("checkout", &[], "ctrl+o").panel.state,
        PanelState::Live,
        "a running shell in Focus is live"
    );

    stop(&log, 0);

    let panel = driver.snapshot("checkout", &[], "ctrl+o").panel;
    assert_eq!(panel.state, PanelState::Exited);
    assert!(!driver.is_running());
}
