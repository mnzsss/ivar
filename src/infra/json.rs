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
//!   (write to a sibling temp file, then rename, so a crash never leaves a
//!   half-written state file).
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

/// Serialize `value` to the canonical format and write it to `path` atomically.
/// The only function in the crate that should ever write JSON to disk.
pub fn write_canonical<T: Serialize>(path: &Utf8Path, value: &T) -> Result<(), Error> {
    let rendered = to_canonical_string(value)?;
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
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use serde::Serialize;

    use super::*;
    use crate::test_support::utf8_temp_dir;

    #[test]
    fn struct_fields_declared_out_of_order_still_serialize_sorted() {
        // Field declaration order is deliberately NOT alphabetical. If
        // `to_canonical_string` ever regressed to `serde_json::to_string`, this
        // would emit `{"zebra":...,"apple":...,"mango":...}` instead.
        #[derive(Serialize)]
        struct OutOfOrder {
            zebra: u8,
            apple: u8,
            mango: u8,
        }

        let rendered = to_canonical_string(&OutOfOrder {
            zebra: 1,
            apple: 2,
            mango: 3,
        })
        .unwrap();

        assert_eq!(
            rendered,
            "{\n  \"apple\": 2,\n  \"mango\": 3,\n  \"zebra\": 1\n}\n"
        );
    }

    #[test]
    fn nested_objects_are_sorted_at_every_depth() {
        #[derive(Serialize)]
        struct Inner {
            zeta: u8,
            beta: u8,
        }

        #[derive(Serialize)]
        struct Outer {
            wombat: Inner,
            alpha: u8,
        }

        let rendered = to_canonical_string(&Outer {
            wombat: Inner { zeta: 1, beta: 2 },
            alpha: 3,
        })
        .unwrap();

        assert_eq!(
            rendered,
            "{\n  \"alpha\": 3,\n  \"wombat\": {\n    \"beta\": 2,\n    \"zeta\": 1\n  }\n}\n"
        );
    }

    #[test]
    fn canonical_string_uses_two_space_indent_lf_and_one_trailing_newline() {
        #[derive(Serialize)]
        struct Point {
            x: u8,
            y: u8,
        }

        let rendered = to_canonical_string(&Point { x: 1, y: 2 }).unwrap();

        assert!(!rendered.contains('\r'), "must never emit CRLF");
        assert_eq!(rendered.matches('\n').count(), rendered.lines().count());
        assert!(rendered.ends_with('\n') && !rendered.ends_with("\n\n"));
        assert!(rendered.contains("\n  \"x\": 1,\n  \"y\": 2\n"));
    }

    #[test]
    fn write_canonical_is_atomic_and_readable_back() {
        #[derive(Serialize, serde::Deserialize, PartialEq, Debug)]
        struct State {
            count: u32,
            name: String,
        }

        let (_dir, root) = utf8_temp_dir();
        let path = root.join("state.json");
        let value = State {
            count: 3,
            name: "hall".to_owned(),
        };

        write_canonical(&path, &value).unwrap();

        let roundtripped: Option<State> = read(&path).unwrap();
        assert_eq!(roundtripped, Some(value));

        // No leftover temp file from the write-then-rename.
        let entries = fs::read_dir(&root).unwrap();
        assert_eq!(entries, vec![path]);
    }

    #[test]
    fn write_canonical_overwrite_leaves_no_partial_state() {
        #[derive(Serialize, serde::Deserialize, PartialEq, Debug)]
        struct State {
            value: String,
        }

        let (_dir, root) = utf8_temp_dir();
        let path = root.join("state.json");

        write_canonical(
            &path,
            &State {
                value: "first".to_owned(),
            },
        )
        .unwrap();
        write_canonical(
            &path,
            &State {
                value: "second".to_owned(),
            },
        )
        .unwrap();

        let roundtripped: Option<State> = read(&path).unwrap();
        assert_eq!(
            roundtripped,
            Some(State {
                value: "second".to_owned()
            })
        );
    }

    #[test]
    fn absent_file_reads_as_ok_none() {
        let (_dir, root) = utf8_temp_dir();
        let missing = root.join("missing.json");

        let value: Option<serde_json::Value> = read(&missing).unwrap();
        assert_eq!(value, None);
    }

    #[test]
    fn unparseable_file_is_a_hard_error_naming_the_path() {
        let (_dir, root) = utf8_temp_dir();
        let path = root.join("broken.json");
        fs::write_text(&path, "{ not json").unwrap();

        let result: Result<Option<serde_json::Value>, Error> = read(&path);

        match result {
            Err(Error::Parse { path: err_path, .. }) => assert_eq!(err_path, path),
            other => panic!("expected Error::Parse, got {other:?}"),
        }
    }

    #[test]
    fn reading_accepts_non_canonical_but_valid_json() {
        let (_dir, root) = utf8_temp_dir();
        let path = root.join("loose.json");
        // Unsorted keys, no trailing newline, four-space indent — none of that
        // matters for reading.
        fs::write_text(&path, "{\"b\":1,\"a\":2}").unwrap();

        let value: Option<serde_json::Value> = read(&path).unwrap();
        assert_eq!(value, Some(serde_json::json!({"a": 2, "b": 1})));
    }
}
