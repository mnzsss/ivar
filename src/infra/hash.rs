//! Content fingerprints.
//!
//! Three things in the domain are identified by their content rather than by a
//! version number: a skill's materialised state, a config's drift, and a plan's
//! revision. All three use the same primitive, so it lives here once.
//!
//! # Contract
//!
//! - `file(path)` — SHA-256 of a file's bytes, lowercase hex.
//! - `bytes(&[u8])` / `text(&str)` — the same, in memory.
//! - `tree(root)` — a single digest over a directory tree.
//!
//! # `tree` is a compatibility surface, not a free choice
//!
//! Its output must match the TypeScript implementation byte-for-byte — the
//! differential harness asserts exactly that, and the shared golden vectors
//! depend on it. So the traversal order, the separator bytes, and whether symlinks
//! are followed are all **fixed by the existing implementation**, not chosen here.
//!
//! Walk in sorted order at every level, and hash the relative path alongside the
//! content so that renaming a file changes the digest. Document the exact framing
//! in this module, because the next person cannot re-derive it from the code
//! without the vectors.
//!
//! # The framing, exactly, and where it comes from
//!
//! Ported from `Hash.tree` in
//! `packages/ragnar/src/hash.ts` in the Valhalla monorepo. Read against
//! that source directly if this drifts.
//!
//! 1. Walk `root` recursively. At every directory level, entries whose *name*
//!    starts with `.` are excluded — both files and directories (a dot-directory
//!    is neither descended into nor hashed). The check is on the entry's own
//!    name, not the full path, so `root` itself may live under a dotted ancestor.
//! 2. Symlinks are excluded entirely: neither followed into, nor hashed as leaf
//!    entries. This matches the TypeScript side, which drives the walk off
//!    `Dirent.isDirectory()` / `Dirent.isFile()` — both `false` for a symlink
//!    entry, so a symlink is silently invisible to `Hash.tree` on both sides.
//! 3. Every remaining regular file contributes one line:
//!    `"<path-relative-to-root>:<sha256-hex-of-the-files-bytes>"`. The relative
//!    path uses `/` separators (this crate does not ship for Windows; see
//!    `ARCHITECTURE.md`), matching Node's `path.relative` on POSIX.
//! 4. The lines are sorted lexicographically by their relative path (the
//!    TypeScript side sorts the file list before mapping it to lines) and joined
//!    with `"\n"` — no trailing newline before hashing.
//! 5. The final digest is the SHA-256 of that joined string, rendered as
//!    lowercase hex and prefixed with `sha256:` — e.g. `sha256:e3b0c4...`. That
//!    prefix is part of the TypeScript contract (`Hash.tree` / `Hash.content`
//!    both emit it) and is **only** on `tree`'s output here: `file`/`bytes`/`text`
//!    are the lower-level primitives this module's own contract (above) defines
//!    as plain lowercase hex, with no prefix, since nothing outside this crate
//!    depends on their exact bytes the way the shared tree vectors do.

use camino::{Utf8Path, Utf8PathBuf};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::error::Failure;
use crate::infra::fs;

/// Everything that can go wrong producing a content fingerprint.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Asked to hash a file that does not exist. Unlike `infra::fs`'s read
    /// primitives, a missing file here is not "genuinely absent" — hashing a
    /// specific file is only ever called when the caller believes it exists, so
    /// a miss is a real problem, not an optional-config-file shrug.
    #[error("{path}: file not found")]
    NotFound { path: Utf8PathBuf },
    /// A directory entry's name, or its path relative to the tree root, was not
    /// valid UTF-8.
    #[error("path is not valid UTF-8: {display}")]
    NotUtf8 { display: String },
    /// Walking the directory tree failed (permissions, a vanished entry mid-walk,
    /// the root itself missing, ...).
    #[error("could not walk directory tree at {root}")]
    Walk {
        root: Utf8PathBuf,
        #[source]
        source: walkdir::Error,
    },
    /// The underlying filesystem operation failed.
    #[error(transparent)]
    Fs(#[from] fs::Error),
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        // The `#[error(...)]` attribute is the single source of the sentence.
        // Re-typing it per arm is how the two drift — they already had.
        let what = error.to_string();

        match error {
            Error::NotFound { .. } => {
                Failure::blocked("hash.not_found", what).expected("an existing file")
            }
            Error::NotUtf8 { .. } => Failure::blocked("hash.not_utf8", what),
            Error::Walk { source, .. } => {
                Failure::failed("hash.walk_failed", what).actual(source.to_string())
            }
            Error::Fs(source) => source.into(),
        }
    }
}

/// Lowercase hex encoding. No external crate needed for something this small.
fn to_hex(bytes: impl AsRef<[u8]>) -> String {
    use std::fmt::Write as _;

    let bytes = bytes.as_ref();
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        // `write!` to a `String` never fails.
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// SHA-256 of `data`, lowercase hex, no prefix.
#[must_use]
pub fn bytes(data: &[u8]) -> String {
    to_hex(Sha256::digest(data))
}

/// SHA-256 of `text`'s UTF-8 bytes, lowercase hex, no prefix.
#[must_use]
pub fn text(data: &str) -> String {
    bytes(data.as_bytes())
}

/// SHA-256 of a file's bytes, lowercase hex, no prefix. Errors — rather than
/// returning `Ok(None)` — if `path` does not exist: hashing a named file is only
/// meaningful when the caller expects it to be there.
pub fn file(path: &Utf8Path) -> Result<String, Error> {
    let contents = fs::read_bytes(path)?.ok_or_else(|| Error::NotFound {
        path: path.to_owned(),
    })?;
    Ok(bytes(&contents))
}

/// Whether a walked entry (or its ancestor, since `filter_entry` prunes whole
/// subtrees) should be excluded: any component named `.something`, checked
/// against the entry's own name, not the full path. The tree root itself
/// (`depth() == 0`) is never excluded this way, even if its own directory name
/// starts with a dot.
fn is_dot_named(entry: &walkdir::DirEntry) -> bool {
    entry.depth() > 0
        && entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with('.'))
}

fn not_utf8(path: std::path::PathBuf) -> Error {
    Error::NotUtf8 {
        display: path.to_string_lossy().into_owned(),
    }
}

/// A single digest over a directory tree. See the module doc comment for the
/// exact framing this must produce — it is a compatibility surface, not a free
/// choice.
pub fn tree(root: &Utf8Path) -> Result<String, Error> {
    let mut relative_paths = Vec::new();

    for entry in WalkDir::new(root.as_std_path())
        .into_iter()
        .filter_entry(|entry| !is_dot_named(entry))
    {
        let entry = entry.map_err(|source| Error::Walk {
            root: root.to_owned(),
            source,
        })?;

        let file_type = entry.file_type();
        if file_type.is_symlink() || !file_type.is_file() {
            continue;
        }

        let absolute = Utf8PathBuf::from_path_buf(entry.into_path()).map_err(not_utf8)?;
        let relative = absolute.strip_prefix(root).unwrap_or(&absolute).to_owned();
        relative_paths.push(relative);
    }

    relative_paths.sort();

    let mut lines = Vec::with_capacity(relative_paths.len());
    for relative in &relative_paths {
        let digest = file(&root.join(relative))?;
        lines.push(format!("{relative}:{digest}"));
    }

    let joined = lines.join("\n");
    Ok(format!("sha256:{}", text(&joined)))
}

#[cfg(test)]
#[path = "../../tests/unit/infra/hash.rs"]
mod tests;
