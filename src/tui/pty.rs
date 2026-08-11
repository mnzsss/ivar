//! The real PTY adapter: `portable-pty` behind the [`Pty`] seam.
//!
//! Two `portable-pty` facts shape this whole module, and both are load-bearing
//! for a TUI whose host loop is one thread:
//!
//! - **The master's reader blocks.** `try_clone_reader` hands back a plain
//!   `dup` of the master fd with no `O_NONBLOCK` on it, so `read` parks until
//!   the child writes. The host loop polls keys and pumps output on the same
//!   thread, so a blocking read there freezes the entire TUI — no keys, no
//!   quit. A detached reader thread does the blocking read and drops bytes
//!   into a channel; [`Pty::try_read`] only ever takes what already arrived.
//! - **The master hands out exactly one writer.** A second `take_writer` is an
//!   error, not a second handle. Focus mode writes once per keystroke, so the
//!   writer is taken at spawn and kept for the PTY's lifetime.
//!
//! Keeping both behind this seam is the point of the [`Pty`] trait: the driver
//! stays a pure sequence of steps and never learns that a thread exists.

use std::cell::RefCell;
use std::io::{self, Read, Write};
use std::sync::mpsc::{self, Receiver, Sender};

use camino::Utf8Path;

use crate::error::{Failure, FixAction};
use crate::infra::proc::Command;

use super::driver::Pty;

/// How much of the master is read in one go by the reader thread.
const READ_CHUNK: usize = 4096;

pub struct PtsPty {
    /// The master side, kept alive for the PTY's lifetime — dropping it pulls
    /// the terminal out from under the child.
    master: Option<Box<dyn portable_pty::MasterPty + Send>>,
    /// The one writer this master will ever hand out, taken at spawn.
    writer: Option<Box<dyn Write + Send>>,
    /// Bytes the reader thread has drained off the master and not yet handed
    /// to the driver.
    output: Option<Receiver<Vec<u8>>>,
    /// The child, so liveness is a real answer and an exited shell is reaped
    /// rather than left a zombie. `RefCell` because `try_wait` needs `&mut`
    /// and [`Pty::is_running`] is a `&self` question.
    child: RefCell<Option<Box<dyn portable_pty::Child + Send + Sync>>>,
}

impl std::fmt::Debug for PtsPty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PtsPty")
            .field("spawned", &self.master.is_some())
            .finish()
    }
}

impl PtsPty {
    /// A fresh, unspawned PTY.
    #[must_use]
    pub fn new() -> Self {
        Self {
            master: None,
            writer: None,
            output: None,
            child: RefCell::new(None),
        }
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
        // The child holds its own copy of the slave fd. While *this* process
        // also holds one, the master never reaches EOF, so the reader thread
        // below would outlive the shell it drains. Dropping it here is what
        // makes the shell's exit observable.
        drop(pair.slave);

        let reader = pair.master.try_clone_reader().map_err(|source| {
            Failure::failed(
                "session.pty_read_failed",
                format!("could not read from the PTY: {source}"),
            )
        })?;
        let writer = pair.master.take_writer().map_err(|source| {
            Failure::failed(
                "session.pty_write_failed",
                format!("could not write to the PTY: {source}"),
            )
        })?;

        let (sender, receiver) = mpsc::channel();
        // Detached on purpose: it ends on EOF (the shell exited) or on a send
        // failure (this `PtsPty` was dropped), and it owns nothing the driver
        // can observe except the channel.
        std::thread::spawn(move || drain(reader, &sender));

        self.master = Some(pair.master);
        self.writer = Some(writer);
        self.output = Some(receiver);
        *self.child.borrow_mut() = Some(child);
        Ok(())
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), io::Error> {
        let Some(writer) = &mut self.writer else {
            return Ok(());
        };
        writer.write_all(bytes)?;
        // One keystroke per call: an unflushed byte is a keystroke the user
        // typed and the shell never saw.
        writer.flush()
    }

    fn try_read(&mut self) -> Result<Option<Vec<u8>>, io::Error> {
        let Some(output) = &self.output else {
            return Ok(None);
        };
        // Everything that arrived since the last pump, coalesced: the emulator
        // is fed once per frame, not once per read the thread happened to make.
        // `try_recv` ends the loop on both `Empty` ("nothing yet") and
        // `Disconnected` ("and there never will be"): either way, return what
        // is in hand rather than block.
        let mut bytes = Vec::new();
        while let Ok(chunk) = output.try_recv() {
            bytes.extend_from_slice(&chunk);
        }
        if bytes.is_empty() {
            Ok(None)
        } else {
            Ok(Some(bytes))
        }
    }

    fn is_running(&self) -> bool {
        let mut child = self.child.borrow_mut();
        let Some(handle) = child.as_mut() else {
            return false;
        };
        // `try_wait` does not block and reaps the child once it has exited.
        // An error is not evidence the shell is alive, so it reads as gone.
        matches!(handle.try_wait(), Ok(None))
    }
}

/// Drain the PTY master into `sender` until the shell exits or the receiving
/// [`PtsPty`] is dropped. This is the blocking read the host loop must never
/// make itself.
fn drain(mut reader: Box<dyn Read + Send>, sender: &Sender<Vec<u8>>) {
    let mut buf = [0u8; READ_CHUNK];
    loop {
        match reader.read(&mut buf) {
            // EOF: the slave closed, so the shell is gone.
            Ok(0) => break,
            // `n` is at most the buffer's length, so the slice is always in
            // bounds — `get` is the lint-clean spelling of that guarantee.
            Ok(n) => {
                let Some(chunk) = buf.get(..n).map(<[u8]>::to_vec) else {
                    break;
                };
                // Nobody listening any more means the `PtsPty` was dropped.
                if sender.send(chunk).is_err() {
                    break;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/tui/pty.rs"]
mod tests;
