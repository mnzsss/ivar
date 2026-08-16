//! The incremental runner: [`stream`] and the [`Stream`] it returns.
//!
//! Everything else in `proc` is stateless — one call in, one [`Output`] or
//! exit code out. This is the module's only thread, its only stateful type,
//! and its only resumable line protocol: a provider process speaking
//! line-oriented JSON, read line by line while it runs rather than captured
//! whole once it's dead. See the parent module doc comment for why that
//! split exists at all.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdout, Stdio};
use std::thread::JoinHandle;

use super::{Command, Error, decode, spawn_error};

/// Spawn `command` with stdout piped for incremental reading while it runs,
/// stdin `/dev/null`, and stderr piped so it can be drained and explained if
/// the child fails.
///
/// [`capture`](super::capture) buffers both streams and only hands them back
/// once the process is dead; that is exactly wrong for a caller parsing a
/// provider's line protocol, which needs each line as it arrives, not the
/// whole transcript after the fact. A
/// [`portable_pty`](https://docs.rs/portable-pty)-backed PTY was rejected for
/// the same reason `session::start` uses one for a human and this doesn't: a
/// PTY interleaves and reflows what it displays, which destroys line
/// boundaries in a protocol that depends on them.
///
/// Stdin is `/dev/null` unless the caller supplied text with
/// [`Command::stdin`], for the same reason [`capture`](super::capture) sets
/// it: a child nobody is watching interactively must never sit blocked on a
/// prompt only a human could answer. Supplied text is written on its own
/// thread and the handle closed, so the child sees EOF; writing it inline
/// would deadlock the moment a prompt outgrew the pipe buffer while the
/// child was filling its own stdout pipe that this thread has not started
/// reading yet.
pub fn stream(command: &Command) -> Result<Stream, Error> {
    let stdin = match command.stdin_text() {
        Some(_) => Stdio::piped(),
        None => Stdio::null(),
    };
    let mut child = command
        .to_std()
        .stdin(stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| spawn_error(command, source))?;

    if let (Some(text), Some(mut handle)) = (command.stdin_text(), child.stdin.take()) {
        let text = text.to_owned();
        // Detached, and its errors dropped on purpose: a child that exits
        // before reading its input gives this thread a broken pipe, which is
        // that child's exit code to explain, not a second failure to report.
        std::thread::spawn(move || {
            let _ = handle.write_all(text.as_bytes());
        });
    }

    let (stdout, stderr) = match (child.stdout.take(), child.stderr.take()) {
        (Some(stdout), Some(stderr)) => (stdout, stderr),
        // Unreachable in practice: both were just requested as `Stdio::piped()`
        // above. Handled as a spawn error rather than `expect`/`unwrap` (the
        // crate warns on both) because there is nowhere safer to report it.
        _ => {
            return Err(spawn_error(
                command,
                io::Error::other("child stdout/stderr missing after a piped spawn"),
            ));
        }
    };

    // Drained on its own thread from the moment of spawn, for the same reason
    // the stdin text above is written on one: a pipe nobody is reading fills,
    // and a child blocked on a full stderr pipe never reaches the stdout write
    // that `read_line` is waiting for. Draining in `wait` instead would only
    // be reached once stdout had already ended, which is precisely the thing
    // the flooded child cannot do.
    let stderr_drain = std::thread::spawn(move || {
        let mut raw = Vec::new();
        let mut stderr = stderr;
        let _ = stderr.read_to_end(&mut raw);
        raw
    });

    Ok(Stream {
        command: command.clone(),
        child,
        stdout: BufReader::new(stdout),
        stderr: Some(stderr_drain),
        captured_stderr: String::new(),
    })
}

/// A child spawned by [`stream`]: readable line by line while it runs, with a
/// wait-for-exit for once the caller is done draining it.
///
/// `stdout` piped through a [`BufReader`] rather than [`capture`](super::capture)'s
/// "read it all" is the entire capability this type exists to add — see the
/// module doc comment.
#[derive(Debug)]
pub struct Stream {
    /// Kept only so a failure inside [`Self::wait`] can render through
    /// [`spawn_error`], the same `Error::Spawn` shape [`capture`](super::capture)
    /// and [`inherit`](super::inherit) already produce, rather than a second
    /// error type.
    command: Command,
    /// `pub(super)` rather than private: `tests/unit/infra/proc.rs` is
    /// `#[path]`-linked as a child of `proc`, not of `proc::streaming`, and
    /// reaches into this field directly (`child_stream.child.try_wait()`) to
    /// prove a line was read *while the child was still running* — the one
    /// assertion `stream`'s whole reason for existing depends on. `pub(super)`
    /// keeps that test whole instead of splitting it into a second file.
    pub(super) child: Child,
    stdout: BufReader<ChildStdout>,
    /// The drain thread started by [`stream`], not the pipe itself. Taken by
    /// the first [`Self::wait`], which joins it for the bytes it collected;
    /// later calls find `None` and keep the text already decoded.
    stderr: Option<JoinHandle<Vec<u8>>>,
    captured_stderr: String,
}

impl Stream {
    /// The next line of stdout, blocking until one arrives or the stream
    /// ends. `Ok(None)` is end of stream — the child closed stdout, which
    /// happens at or before exit, so [`Self::wait`] is what to call next, not
    /// this again.
    ///
    /// The trailing line ending is stripped (`\n`, or `\r\n` with both
    /// removed), matching [`decode`]'s trimming for [`capture`](super::capture)'s
    /// output.
    pub fn read_line(&mut self) -> io::Result<Option<String>> {
        let mut line = String::new();
        let bytes_read = self.stdout.read_line(&mut line)?;
        if bytes_read == 0 {
            return Ok(None);
        }
        if line.ends_with('\n') {
            line.pop();
            if line.ends_with('\r') {
                line.pop();
            }
        }
        Ok(Some(line))
    }

    /// Block for the child to exit. Returns its exit code, `None` for a
    /// signal death — the same shape [`inherit`](super::inherit) returns,
    /// because both answer exactly one question: how did the process end.
    ///
    /// Also collects whatever the child wrote to stderr and decodes it
    /// lossily, like [`capture`](super::capture), so [`Self::stderr`] has
    /// something to explain a failure with. Safe to call more than once, like
    /// the underlying [`Child::wait`].
    ///
    /// The bytes were being read all along by the drain thread [`stream`]
    /// starts; this only joins it. A child still holding stderr open blocks
    /// that join, which is the known limitation the plan's safeguards accept:
    /// a hung child hangs the caller, visibly rather than silently.
    pub fn wait(&mut self) -> Result<Option<i32>, Error> {
        if let Some(drain) = self.stderr.take() {
            // A drain thread that panicked is not a second failure to report:
            // the child's own exit code is the answer, and stderr exists only
            // to explain it.
            self.captured_stderr = decode(&drain.join().unwrap_or_default());
        }

        let status = self
            .child
            .wait()
            .map_err(|source| spawn_error(&self.command, source))?;
        Ok(status.code())
    }

    /// The child's stderr, decoded and trimmed like [`capture`](super::capture)'s.
    /// Empty until [`Self::wait`] has drained it.
    #[must_use]
    pub fn stderr(&self) -> &str {
        &self.captured_stderr
    }
}
