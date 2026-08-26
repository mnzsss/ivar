//! The only place that writes JSON to disk.
//!
//! # Why this module exists
//!
//! The Rust port's first differential run against the TypeScript original failed
//! 13 of 13 cases with **semantically identical** output. TypeScript emitted
//! object keys in spread order; `serde` emitted struct field order. Same data,
//! different bytes.
//!
//! Two consequences, both real:
//!
//! - Golden vectors shared with the surviving TypeScript package can only be
//!   compared byte-for-byte if both sides canonicalise.
//! - A user's hall is a git repo. Two writers disagreeing on key order means a
//!   diff on every write, with no semantic change in it.
//!
//! # The format, fixed
//!
//! - object keys sorted lexicographically, at every depth
//! - two-space indent
//! - `\n` line endings, never `\r\n`
//! - exactly one trailing newline
//! - no non-ASCII escaping beyond what JSON requires
//!
//! # Contract
//!
//! - `write_canonical(path, &value)` — serialize, canonicalise, write atomically
//!   (creating missing parent directories, then writing to a sibling temp file
//!   and renaming, so a crash never leaves a half-written state file).
//! - `to_canonical_string(&value)` — the same bytes, without touching disk. This
//!   is what tests and the differential harness compare.
//! - `read(path)` — deserialize, distinguishing "genuinely absent" from "present
//!   but unparseable". Absent is `Ok(None)`; unparseable is an error naming the
//!   path and the position.
//!
//! Reading does **not** canonicalise — it accepts any valid JSON. Only writing is
//! constrained.
//!
//! # How the sort actually happens
//!
//! `serde_json` does not sort keys when serializing a struct directly — it emits
//! field declaration order. The trick is to round-trip through
//! [`serde_json::Value`] first: with the `preserve_order` cargo feature *off*
//! (verified below — it is off for this crate, including under
//! `--all-features`, because nothing in the dependency graph turns it on),
//! `serde_json::Map` is backed by a `BTreeMap`, so converting any `Serialize`
//! value to a `Value` and back out sorts every object, at every depth, for free.
//! [`Error::Serialize`] handles the (rare) case where a value refuses to convert
//! at all. [`tests::struct_fields_declared_out_of_order_still_serialize_sorted`]
//! and [`tests::nested_objects_are_sorted_at_every_depth`] are the guard on this
//! contract — if a future dependency bump ever enables `preserve_order`, those
//! tests fail loudly instead of the on-disk format silently drifting.

use camino::Utf8Path;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::Failure;
use crate::infra::fs;

/// Everything that can go wrong producing or reading canonical JSON.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A value could not be converted to JSON at all (e.g. a map with
    /// non-string keys, or a `NaN` float). Not the common case.
    #[error("could not serialize value to JSON: {0}")]
    Serialize(#[source] serde_json::Error),
    /// The bytes at `path` are not valid JSON.
    #[error("{path}: invalid JSON")]
    Parse {
        path: camino::Utf8PathBuf,
        #[source]
        source: serde_json::Error,
    },
    /// The underlying filesystem operation failed.
    #[error(transparent)]
    Fs(#[from] fs::Error),
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        match error {
            Error::Serialize(source) => Failure::failed(
                "json.serialize_failed",
                format!("could not serialize value to JSON: {source}"),
            ),
            Error::Parse { path, source } => Failure::failed(
                "json.parse_failed",
                format!("{path}: invalid JSON: {source}"),
            )
            .expected("valid JSON")
            .actual(source.to_string()),
            Error::Fs(source) => source.into(),
        }
    }
}

/// Serialize `value` to the canonical byte format, in memory. This is what
/// [`write_canonical`] writes and what tests / the differential harness compare
/// against — never `serde_json::to_string` directly.
pub fn to_canonical_string<T: Serialize>(value: &T) -> Result<String, Error> {
    let value = serde_json::to_value(value).map_err(Error::Serialize)?;
    let mut rendered = serde_json::to_string_pretty(&value).map_err(Error::Serialize)?;
    rendered.push('\n');
    Ok(rendered)
}

/// Serialize `value` to the canonical format and write it to `path` atomically,
/// creating missing parent directories first. The only function in the crate
/// that should ever write JSON to disk.
pub fn write_canonical<T: Serialize>(path: &Utf8Path, value: &T) -> Result<(), Error> {
    let rendered = to_canonical_string(value)?;
    if let Some(parent) = path.parent().filter(|parent| !parent.as_str().is_empty()) {
        fs::ensure_dir(parent)?;
    }
    fs::write_atomic(path, rendered.as_bytes())?;
    Ok(())
}

/// Read and deserialize JSON from `path`. `Ok(None)` if `path` does not exist;
/// an error naming the path and the parse position if it exists but is not
/// valid JSON. Accepts any valid JSON — reading does not require canonical
/// formatting.
pub fn read<T: DeserializeOwned>(path: &Utf8Path) -> Result<Option<T>, Error> {
    let Some(text) = fs::read_text(path)? else {
        return Ok(None);
    };
    let value = serde_json::from_str(&text).map_err(|source| Error::Parse {
        path: path.to_owned(),
        source,
    })?;
    Ok(Some(value))
}

#[cfg(test)]
#[path = "../../tests/unit/infra/json.rs"]
mod tests;
