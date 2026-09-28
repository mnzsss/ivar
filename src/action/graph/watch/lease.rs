//! Leader lease on `.ivar/graph-watch.lock`.

use crate::error::Failure;
use crate::store::layout::Layout;

pub const LEASE_FILE: &str = "graph-watch.lock";

#[derive(Debug)]
pub struct Lease {
    #[allow(dead_code)]
    file: std::fs::File,
}

impl Lease {
    /// Attempts to acquire the watcher leader lease on `.ivar/graph-watch.lock`.
    ///
    /// Returns `Ok(Some(lease))` on success, `Ok(None)` if another process holds the lease,
    /// or `Err(Failure)` on I/O failure.
    ///
    /// # Errors
    ///
    /// Returns [`Failure`] if opening or writing to the lease file fails.
    pub fn try_acquire(layout: &Layout) -> Result<Option<Lease>, Failure> {
        let path = layout.ivar_dir().join(LEASE_FILE);
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path.as_std_path())
            .map_err(|e| Failure::failed("graph.watch_lease", format!("open {path}: {e}")))?;
        match file.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => return Ok(None),
            Err(std::fs::TryLockError::Error(e)) => {
                return Err(Failure::failed(
                    "graph.watch_lease",
                    format!("lock {path}: {e}"),
                ));
            }
        }
        file.set_len(0)
            .and_then(|()| {
                std::io::Write::write_all(&mut file, std::process::id().to_string().as_bytes())
            })
            .map_err(|e| {
                Failure::failed("graph.watch_lease", format!("write pid to {path}: {e}"))
            })?;
        Ok(Some(Lease { file }))
    }
}

/// Returns the PID of the current leader holding the lease, or `None` if no leader holds it.
/// Never creates the lease file.
#[must_use]
pub fn leader_pid(layout: &Layout) -> Option<u32> {
    let path = layout.ivar_dir().join(LEASE_FILE);
    let file = std::fs::OpenOptions::new()
        .read(true)
        .open(path.as_std_path())
        .ok()?;
    match file.try_lock() {
        Ok(()) => {
            let _ = file.unlock();
            None
        }
        Err(_) => std::fs::read_to_string(path.as_std_path())
            .ok()?
            .trim()
            .parse()
            .ok(),
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/graph/watch/lease.rs"]
mod tests;
