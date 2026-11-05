//! Exclusive advisory file locks.

use std::time::{Duration, Instant};

use camino::Utf8Path;

use super::Error;

/// Block until this process holds an exclusive advisory lock on `path`,
/// creating the file if needed. The lock is released when the returned file
/// is dropped.
///
/// # Errors
///
/// Returns [`Error::Write`] if the file cannot be created or locked.
pub fn lock_exclusive(path: &Utf8Path) -> Result<std::fs::File, Error> {
    let write_error = |source| Error::Write {
        path: path.to_path_buf(),
        source,
    };
    let file = std::fs::File::create(path.as_std_path()).map_err(write_error)?;
    file.lock().map_err(write_error)?;
    Ok(file)
}

/// Take an exclusive advisory lock on `path` without waiting, creating the
/// file and its parent directories if needed. `None` when another handle
/// still holds it after [`TRY_LOCK_GRACE`].
///
/// # Errors
///
/// Returns [`Error::Write`] if the file cannot be created or locked.
pub fn try_lock_exclusive(path: &Utf8Path) -> Result<Option<std::fs::File>, Error> {
    let write_error = |source| Error::Write {
        path: path.to_path_buf(),
        source,
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent.as_std_path()).map_err(write_error)?;
    }
    let file = std::fs::File::create(path.as_std_path()).map_err(write_error)?;
    let deadline = Instant::now() + TRY_LOCK_GRACE;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(Some(file)),
            Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(TRY_LOCK_RETRY);
            }
            Err(std::fs::TryLockError::WouldBlock) => return Ok(None),
            Err(std::fs::TryLockError::Error(source)) => return Err(write_error(source)),
        }
    }
}

/// A child forked by another thread holds a copy of a just-released lock fd
/// until it execs, so a lock can look busy for an instant after its holder
/// dropped it.
const TRY_LOCK_GRACE: Duration = Duration::from_millis(50);
const TRY_LOCK_RETRY: Duration = Duration::from_millis(2);

#[cfg(test)]
#[path = "../../../tests/unit/infra/fs/lock.rs"]
mod tests;
