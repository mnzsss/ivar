#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use super::*;

/// A directory every platform running these tests has.
fn cwd() -> &'static Utf8Path {
    Utf8Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// `try_read` must return promptly when the child has produced no output.
///
/// The host loop calls `pump` (and so `try_read`) once per iteration, on the
/// same thread that polls for keys. A `try_read` that blocks until the child
/// writes something freezes the whole TUI: no keys are polled, so the user
/// can neither navigate nor quit.
#[test]
fn try_read_returns_when_the_child_is_silent() {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut pty = PtsPty::new();
        // `sleep` writes nothing at all: the only bytes a correct `try_read`
        // can return here are none.
        pty.spawn(&Command::new("sleep").arg("30"), cwd(), 80, 24)
            .expect("spawn sleep");
        let _ = tx.send(pty.try_read().map(|bytes| bytes.unwrap_or_default()));
    });

    match rx.recv_timeout(Duration::from_secs(2)) {
        Ok(result) => assert_eq!(result.unwrap(), Vec::<u8>::new()),
        Err(_) => panic!("try_read blocked on a silent child — the TUI would freeze"),
    }
}

/// `write` must accept more than one keystroke.
///
/// Focus mode sends one `write` per key. A `write` that only works once
/// silently drops every character after the first, so no command can be
/// typed into the shell.
#[test]
fn write_accepts_more_than_one_keystroke() {
    let mut pty = PtsPty::new();
    pty.spawn(&Command::new("cat"), cwd(), 80, 24)
        .expect("spawn cat");

    pty.write(b"a").expect("first write");
    pty.write(b"b").expect("second write");
    pty.write(b"c").expect("third write");
}

/// The whole round trip Focus mode depends on: keys go in, the shell's
/// output comes back out, and no step of it blocks the caller.
#[test]
fn a_typed_line_comes_back_from_the_shell() {
    let mut pty = PtsPty::new();
    // `cat` echoes what it is given without needing a prompt or a profile.
    pty.spawn(&Command::new("cat"), cwd(), 80, 24)
        .expect("spawn cat");
    assert!(pty.is_running());

    pty.write(b"hello\n").expect("write a line");

    // Poll the way the host loop does, rather than sleeping a fixed guess.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut seen = Vec::new();
    while std::time::Instant::now() < deadline {
        if let Some(bytes) = pty.try_read().expect("try_read") {
            seen.extend_from_slice(&bytes);
            if String::from_utf8_lossy(&seen).contains("hello") {
                return;
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!(
        "never read `hello` back; saw {:?}",
        String::from_utf8_lossy(&seen)
    );
}

/// The reported freeze, at the level it happened: the host loop calls
/// `pump` on the same thread that polls for keys, so a `pump` that blocks
/// is a TUI that cannot be navigated or quit. Driving the real driver over
/// a real PTY is the closest this can get to `ivar feature view` without a
/// terminal.
#[test]
fn the_drivers_steps_never_block_on_a_real_shell() {
    use crate::tui::driver::{Driver, ShellSpec};
    use crate::tui::key_router::Key;

    let spec = ShellSpec {
        label: "repo".to_owned(),
        cwd: cwd().to_path_buf(),
        command: Command::new("cat"),
    };

    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut driver = Driver::new(vec![spec], PtsPty::new, 80, 24);
        // Idle iterations first: a silent shell is exactly the state the
        // loop froze in.
        for _ in 0..5 {
            let _ = driver.pump();
        }
        driver.apply_event(Key::Char('h'));
        driver.apply_event(Key::Enter);

        let mut panel = String::new();
        for _ in 0..200 {
            let _ = driver.pump();
            panel = driver.snapshot("t", &[]).panel.lines.join("");
            if panel.contains('h') {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let _ = tx.send(panel);
    });

    let panel = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the driver's steps blocked — the TUI would freeze");
    assert!(panel.contains('h'), "the typed key never reached the panel");
}

/// Backspace, end to end: the byte the router sends must be the one the
/// terminal's line discipline treats as erase. `cat` echoes the line it
/// receives, so what it echoes is proof of what the erase did.
#[test]
fn backspace_erases_a_character_in_a_real_shell() {
    let mut pty = PtsPty::new();
    pty.spawn(&Command::new("cat"), cwd(), 80, 24)
        .expect("spawn cat");

    pty.write(b"ab").expect("type ab");
    // DEL, which is what `Key::Backspace` reduces to.
    pty.write(&[0x7f]).expect("backspace");
    pty.write(b"c\n").expect("type c and enter");

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut seen = Vec::new();
    while std::time::Instant::now() < deadline {
        if let Some(bytes) = pty.try_read().expect("try_read") {
            seen.extend_from_slice(&bytes);
            let text = String::from_utf8_lossy(&seen);
            // `cat` echoes the line it actually received.
            if text.contains("ac") {
                return;
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!(
        "the `b` was never erased; saw {:?}",
        String::from_utf8_lossy(&seen)
    );
}
