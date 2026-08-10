//! The real PTY adapter: `portable-pty` behind the [`Pty`] seam.
//!
//! `portable-pty` gives a `PtyPair`; reads go through the slave's reader
//! handle. Reads are blocking on the handle, so `try_read` is implemented
//! by checking the master's bytes available — `portable-pty` exposes a
//! non-blocking read on the master via `try_clone_reader` + polling; the
//! seam keeps that detail here, where it can be swapped.

/// The real PTY: `portable-pty` behind the [`Pty`] seam.
///
/// `portable-pty` gives a `PtyPair`; reads go through the slave's reader
/// handle. Reads are blocking on the handle, so `try_read` is implemented
/// by checking the master's bytes available — `portable-pty` exposes a
/// non-blocking read on the master via `try_clone_reader` + polling; the
/// seam keeps that detail here, where it can be swapped.
use std::io;

use camino::Utf8Path;

use crate::error::{Failure, FixAction};
use crate::infra::proc::Command;

use super::driver::Pty;

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
