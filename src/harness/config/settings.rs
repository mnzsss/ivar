//! `.claude/settings.json` materialisation: ivar owns the `env` and `hooks`
//! keys; the user owns everything else.
//!
//! The pattern is identical to [`super::mcp`]: read the existing document,
//! merge ivar's keys, compare canonical bytes, write only on change. A file
//! that exists but cannot be parsed as a JSON object is never clobbered.

use camino::Utf8Path;

use crate::domain::name::HallName;
use crate::infra::{fs, json};

use super::{Change, Error};

/// The keys ivar owns inside `.claude/settings.json`.
const IVAR_ENV: &str = "env";
const IVAR_HOOKS: &str = "hooks";

/// Materialise the ivar-owned keys at `path` for the given `hall`.
///
/// The file is created when absent, merged when present (replacing exactly
/// the `env` and `hooks` keys), and left alone when the canonical bytes
/// already match. A file that exists but is not a JSON object is refused.
pub fn materialise_settings(
    path: &Utf8Path,
    hall: &HallName,
) -> Result<Change, Error> {
    let ivar_doc = ivar_doc(hall);
    let (existing, raw) = read_doc(path)?;

    let Some(mut doc) = existing else {
        return write_doc(path, &ivar_doc).map(|_| Change::Created);
    };

    let object = doc.as_object_mut().ok_or_else(|| Error::McpNotObject {
        path: path.to_path_buf(),
    })?;

    // Replace ivar-owned keys. Extract from ivar_doc first to avoid indexing.
    let env_value = ivar_doc.get(IVAR_ENV).cloned().unwrap_or_default();
    let hooks_value = ivar_doc.get(IVAR_HOOKS).cloned().unwrap_or_default();
    object.insert(IVAR_ENV.to_owned(), env_value);
    object.insert(IVAR_HOOKS.to_owned(), hooks_value);

    let rendered = json::to_canonical_string(&doc).map_err(|source| Error::Mcp {
        path: path.to_path_buf(),
        source,
    })?;
    if raw.as_deref() == Some(rendered.as_str()) {
        return Ok(Change::Unchanged);
    }

    write_doc(path, &doc)?;
    Ok(Change::Updated)
}

/// Remove ivar's keys from the settings file at `path`.
///
/// The file is deleted only when ivar's keys were its entire content. A file
/// carrying other keys keeps them, minus ivar's keys. Absent file is
/// [`Change::Unchanged`]. A file that cannot be parsed as a JSON object is
/// left alone.
pub fn remove_settings(path: &Utf8Path) -> Result<Change, Error> {
    let (existing, _) = read_doc(path)?;
    let Some(mut doc) = existing else {
        return Ok(Change::Unchanged);
    };

    let Some(object) = doc.as_object_mut() else {
        return Ok(Change::Unchanged);
    };

    let had_env = object.remove(IVAR_ENV).is_some();
    let had_hooks = object.remove(IVAR_HOOKS).is_some();

    if !had_env && !had_hooks {
        return Ok(Change::Unchanged);
    }

    if object.is_empty() {
        fs::remove_file(path).map_err(|source| Error::Mcp {
            path: path.to_path_buf(),
            source: json::Error::Fs(source),
        })?;
        return Ok(Change::Removed);
    }

    write_doc(path, &doc)?;
    Ok(Change::Removed)
}

/// The full document ivar wants: `env` holding the hall name, and `hooks`
/// holding the session lifecycle hooks. Used when the file is absent or when
/// merging into an existing document.
fn ivar_doc(hall: &HallName) -> serde_json::Value {
    let mut root = serde_json::Map::new();

    // env: the session variable that identifies the hall.
    let mut env = serde_json::Map::new();
    env.insert(
        "IVAR_HALL".to_owned(),
        serde_json::Value::String(hall.to_string()),
    );
    root.insert(IVAR_ENV.to_owned(), serde_json::Value::Object(env));

    // hooks: the lifecycle hooks that wire session env and guard into the
    // harness.
    let mut hooks = serde_json::Map::new();
    hooks.insert(
        "SessionStart".to_owned(),
        serde_json::json!([
            {
                "type": "command",
                "command": "ivar session env"
            }
        ]),
    );
    hooks.insert(
        "PreToolUse".to_owned(),
        serde_json::json!([
            {
                "type": "command",
                "command": "ivar guard --provider claude-code"
            }
        ]),
    );
    root.insert(IVAR_HOOKS.to_owned(), serde_json::Value::Object(hooks));

    serde_json::Value::Object(root)
}

/// Read `path` as JSON, returning the parsed document and its raw bytes.
///
/// `Ok((None, None))` when the file is absent. A file that exists but is not
/// valid JSON is an error — never a silent clobber of user config.
fn read_doc(path: &Utf8Path) -> Result<(Option<serde_json::Value>, Option<String>), Error> {
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
fn write_doc(path: &Utf8Path, doc: &serde_json::Value) -> Result<(), Error> {
    json::write_canonical(path, doc).map_err(|source| Error::Mcp {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
#[path = "../../../tests/unit/harness/config/settings.rs"]
mod tests;
