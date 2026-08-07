//! Scratch directories for tests. Compiled only under `cfg(test)`.
//!
//! This exists because the same four-line `TempDir` + `Utf8PathBuf` dance was
//! written out in five modules, byte for byte, and two more had already
//! diverged into a canonicalising variant. Five copies means five places to fix
//! when the construction needs a tweak, and the odds of all five getting it are
//! not good.
//!
//! Integration tests under `tests/` cannot see this — `cfg(test)` items are not
//! part of the compiled library — so `tests/init.rs` still carries its own.
//! That is the cost of the boundary, not an oversight.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use camino::Utf8PathBuf;
use tempfile::TempDir;

/// A scratch directory and its UTF-8 path.
///
/// The `TempDir` is returned so the caller can bind it — dropping it deletes the
/// directory, so `let (_dir, root) = ...` is the shape that works and
/// `let (_, root) = ...` is the shape that mysteriously does not.
pub(crate) fn utf8_temp_dir() -> (TempDir, Utf8PathBuf) {
    let dir = TempDir::new().unwrap();
    let path = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
    (dir, path)
}

/// A scratch directory, canonicalised, with an empty `hall` subdirectory in it.
///
/// Canonicalising matters for anything that compares paths against the result of
/// [`crate::store::layout::Layout::discover`], which canonicalises too: on macOS
/// `TempDir` hands back a `/var/...` path whose real name is `/private/var/...`,
/// and the two are not equal as strings.
pub(crate) fn canonical_temp_dir() -> (TempDir, Utf8PathBuf) {
    let (dir, path) = utf8_temp_dir();
    let canonical = path.canonicalize_utf8().unwrap();
    (dir, canonical)
}

/// A canonicalised scratch directory with an empty `hall` subdirectory inside it.
///
/// The subdirectory is not incidental: `TempDir` names itself `.tmpXXXX`, and a
/// leading dot is refused by `HallName` ([`crate::domain::name::InvalidName::Hidden`]).
/// Using the tempdir itself as a hall root would make every test that exercises
/// name *derivation* collide with an unrelated rule.
pub(crate) fn hall_root() -> (TempDir, Utf8PathBuf) {
    let (dir, canonical) = canonical_temp_dir();
    let root = canonical.join("hall");
    std::fs::create_dir_all(&root).unwrap();
    (dir, root)
}
