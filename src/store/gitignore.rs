//! The hall's `.gitignore`, kept correct without clobbering the user's.
//!
//! [`Layout::gitignore_lines`] says *which* lines a hall needs and why the
//! `.ivar/*` form is the only one that works. This module is the other half:
//! getting them into the file, on a file the user also writes in.
//!
//! # Append, never overwrite
//!
//! A hall is very often initialised inside a directory that already has a
//! `.gitignore` — `node_modules/`, build output, editor droppings. Writing the
//! hall's lines over that would be the same silent-overwrite bug `init` refuses
//! to commit against `ivar.json`, on a file people notice later. So: read what
//! is there, add only the lines that are missing, keep everything else byte for
//! byte.
//!
//! # Why both `init` and `sync` call this
//!
//! `init` writes it because a hall needs it from the first commit. `sync`
//! writes it because the hall a teammate clones has the `.gitignore` committed
//! *and* because a hall whose `.gitignore` was edited by hand is exactly the
//! case where `git pull && ivar sync` should be self-healing. Two callers, one
//! implementation — the alternative was two copies of the append rules, which
//! is how they drift.

use crate::infra::fs;
use crate::store::layout::Layout;

/// Ensure `<hall>/.gitignore` contains every line in
/// [`Layout::gitignore_lines`], adding only the ones that are missing.
///
/// Reports whether the file changed, so a caller building a sync report can say
/// "unchanged" honestly rather than claiming work it did not do.
pub fn ensure(layout: &Layout) -> Result<Changed, fs::Error> {
    let path = layout.gitignore_path();
    let original = fs::read_text(&path)?;
    let mut content = original.clone().unwrap_or_default();

    for line in Layout::gitignore_lines() {
        if content.lines().any(|existing| existing == line) {
            continue;
        }
        // A file that does not end in a newline would otherwise get the new
        // line glued onto its last one, silently turning two patterns into one
        // nonsense pattern.
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(line);
        content.push('\n');
    }

    if original.as_deref() == Some(content.as_str()) {
        return Ok(Changed::No);
    }

    fs::write_atomic(&path, content.as_bytes())?;
    Ok(if original.is_none() {
        Changed::Created
    } else {
        Changed::Yes
    })
}

/// What [`ensure`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Changed {
    /// There was no `.gitignore`; there is now.
    Created,
    /// It existed and was missing at least one line.
    Yes,
    /// It already said everything it needed to.
    No,
}

#[cfg(test)]
#[path = "../../tests/unit/store/gitignore.rs"]
mod tests;
