//! Worktree write permission guard for safe temporary modification of default branch worktrees.

use camino::{Utf8Path, Utf8PathBuf};

use crate::error::{Failure, FixAction};

/// Scope guard ensuring read-only default worktree permissions are always restored on drop.
#[derive(Debug)]
pub struct WorktreeWriteGuard {
    lifted: Vec<(Utf8PathBuf, u32)>,
}

impl WorktreeWriteGuard {
    pub fn lift(worktrees: &[&Utf8Path]) -> Result<Self, Failure> {
        let mut guard = Self { lifted: Vec::new() };
        for &wt in worktrees {
            match crate::infra::fs::unix_mode(wt) {
                Ok(Some(mode)) if mode & 0o222 == 0 => {
                    if let Err(e) = crate::infra::fs::restore_write_bits(wt) {
                        return Err(Failure::failed(
                            "deliver.lift_write_bits_failed",
                            format!("could not lift write permissions on `{wt}`: {e}"),
                        )
                        .expected(format!("write permissions to be lifted on `{wt}`"))
                        .actual(format!("chmod failed: {e}"))
                        .fix(FixAction::safe(
                            "deliver.check_permissions",
                            format!("Ensure user has permission to modify permissions on `{wt}`."),
                        )));
                    }
                    guard.lifted.push((wt.to_path_buf(), mode));
                }
                Ok(_) => {}
                Err(e) => {
                    return Err(Failure::failed(
                        "deliver.read_mode_failed",
                        format!("could not inspect permissions on `{wt}`: {e}"),
                    )
                    .expected(format!("path `{wt}` to exist and be readable"))
                    .actual(format!("fs error: {e}"))
                    .fix(FixAction::safe(
                        "deliver.check_path",
                        format!("Ensure `{wt}` exists and is accessible."),
                    )));
                }
            }
        }
        Ok(guard)
    }
}

impl Drop for WorktreeWriteGuard {
    fn drop(&mut self) {
        for (wt, mode) in &self.lifted {
            let _ = crate::infra::fs::chmod(wt, *mode);
        }
    }
}
