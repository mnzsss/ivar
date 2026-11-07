//! `ivar script` — the hall's scripts, under `.ivar/scripts/` (committed, for
//! the team) and `.ivar/scripts-local/` (personal).
//!
//! `ivar` only reads scripts: it lists and validates them, checks and approves
//! their content, and announces them in the session brief. No command here
//! writes, chmods, moves or deletes one. The format is
//! [`crate::domain::script`]; the folder walk is [`crate::store::script`];
//! the approvals are [`crate::domain::script_approval`] and
//! [`crate::store::memory::approvals`].

pub mod approve;
pub mod check;
pub mod list;

use camino::{Utf8Component, Utf8Path, Utf8PathBuf};

use crate::action::Ctx;
use crate::error::Failure;
use crate::store::layout::Layout;
use crate::store::script::ScriptFile;

/// A script the user named: where it is, and how approvals know it.
pub(crate) struct Target {
    /// The path relative to the hall root, `/`-separated.
    pub rel: String,
    /// The path to read.
    pub abs: Utf8PathBuf,
}

/// Resolve `path` (absolute, or relative to the working directory) to a file
/// inside the hall, normalised to the hall-relative form approvals are keyed
/// by, so `./.ivar/scripts/a.sh` and the absolute path are the same script.
///
/// # Errors
///
/// `script.outside_hall` when it lies outside the hall; `script.not_found`
/// when it is not a regular file.
pub(crate) fn resolve(ctx: &Ctx, layout: &Layout, path: &Utf8Path) -> Result<Target, Failure> {
    let abs = normalize(&ctx.resolve(path));
    let Ok(rel) = abs.strip_prefix(layout.root()) else {
        return Err(Failure::blocked(
            "script.outside_hall",
            format!("`{path}` is not inside the hall `{}`", layout.root()),
        ));
    };
    if rel.as_str().is_empty() {
        return Err(Failure::blocked(
            "script.not_found",
            format!("`{path}` is the hall root, not a script"),
        ));
    }
    if !crate::infra::fs::is_file(&abs)? {
        return Err(Failure::blocked(
            "script.not_found",
            format!("`{path}` is not a file"),
        ));
    }
    Ok(Target {
        rel: rel.to_string(),
        abs,
    })
}

/// Drop `.` components and fold `..` into the component before it.
fn normalize(path: &Utf8Path) -> Utf8PathBuf {
    let mut out = Utf8PathBuf::new();
    for component in path.components() {
        match component {
            Utf8Component::CurDir => {}
            Utf8Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_str()),
        }
    }
    out
}

/// The failure for an approvals database that cannot be read or written.
pub(crate) fn store_failure(error: impl std::fmt::Display) -> Failure {
    Failure::failed(
        "script.approvals_unavailable",
        format!("could not use the script approvals in memory.db: {error}"),
    )
}

/// The script's path relative to the hall root, the way a session runs it
/// from there (`.ivar/scripts/deploy.sh`).
pub(crate) fn hall_relative(layout: &Layout, file: &ScriptFile) -> String {
    file.path
        .strip_prefix(layout.root())
        .unwrap_or(&file.path)
        .to_string()
}
