//! Shared JSON document read/write helpers for `mcp` and `settings`.

use camino::Utf8Path;

use crate::infra::fs;
use crate::infra::json;

use super::{Change, Error};

/// Read `path` as JSON, returning the parsed document and its raw bytes.
///
/// `Ok((None, None))` when the file is absent. A file that exists but is not
/// valid JSON is an error — never a silent clobber of user config.
pub(super) fn read_doc(
    path: &Utf8Path,
) -> Result<(Option<serde_json::Value>, Option<String>), Error> {
    let Some(text) = fs::read_text(path).map_err(|source| Error::Mcp {
        path: path.to_path_buf(),
        source: json::Error::Fs(source),
    })?
    else {
        return Ok((None, None));
    };
    let value = serde_json::from_str(&text).map_err(|source| Error::Mcp {
        path: path.to_path_buf(),
        source: json::Error::Parse {
            path: path.to_path_buf(),
            source,
        },
    })?;
    Ok((Some(value), Some(text)))
}

/// Write `doc` to `path` in the canonical byte format.
pub(super) fn write_doc(path: &Utf8Path, doc: &serde_json::Value) -> Result<(), Error> {
    json::write_canonical(path, doc).map_err(|source| Error::Mcp {
        path: path.to_path_buf(),
        source,
    })
}

/// Finish a key removal: delete `path` when nothing is left in the document,
/// else write it back with the key gone.
pub(super) fn finish_removal(
    path: &Utf8Path,
    is_empty: bool,
    doc: &serde_json::Value,
) -> Result<Change, Error> {
    if is_empty {
        fs::remove_file(path).map_err(|source| Error::Mcp {
            path: path.to_path_buf(),
            source: json::Error::Fs(source),
        })?;
        return Ok(Change::Removed);
    }

    write_doc(path, doc)?;
    Ok(Change::Removed)
}
