//! `.claude/settings.json` materialisation: ivar owns the `hooks` and
//! `attribution` keys; the user owns everything else, `env` included.
//! `attribution` is blanked so Claude Code adds no AI attribution to commits or
//! PRs, overriding any user value. Halls commit this file, so it carries no
//! clone-specific value such as the hall root; the process environment carries
//! `IVAR_HALL` instead, and a legacy `env.IVAR_HALL` entry is dropped.
//!
//! The pattern is identical to [`super::mcp`]: read the existing document,
//! merge ivar's keys, compare canonical bytes, write only on change. A file
//! that exists but cannot be parsed as a JSON object is never clobbered.

use camino::Utf8Path;

use crate::infra::json;

use super::doc;
use super::{Change, Error};

/// The keys ivar owns inside `.claude/settings.json`.
const IVAR_HOOKS: &str = "hooks";
const IVAR_ATTRIBUTION: &str = "attribution";
const IVAR_KEYS: [&str; 2] = [IVAR_HOOKS, IVAR_ATTRIBUTION];

const ENV: &str = "env";
const LEGACY_ENV_IVAR_HALL: &str = "IVAR_HALL";

/// Materialise the ivar-owned keys at `path`.
///
/// The file is created when absent, merged when present (replacing exactly
/// the `hooks` and `attribution` keys and dropping a legacy `env.IVAR_HALL`),
/// and left alone when the canonical bytes already match. A file that exists
/// but is not a JSON object is refused.
///
/// # Errors
///
/// Returns [`Error`] if the existing file cannot be parsed as a JSON object
/// or the merged document cannot be written.
pub fn materialise_settings(path: &Utf8Path) -> Result<Change, Error> {
    let ivar_doc = ivar_doc();
    let (existing, raw) = doc::read_doc(path)?;

    let Some(mut doc) = existing else {
        return doc::write_doc(path, &ivar_doc).map(|_| Change::Created);
    };

    let object = doc.as_object_mut().ok_or_else(|| Error::McpNotObject {
        path: path.to_path_buf(),
    })?;

    // Replace ivar-owned keys. Extract from ivar_doc first to avoid indexing.
    for key in IVAR_KEYS {
        let value = ivar_doc.get(key).cloned().unwrap_or_default();
        object.insert(key.to_owned(), value);
    }
    remove_legacy_ivar_hall(object);

    let rendered = json::to_canonical_string(&doc).map_err(|source| Error::Mcp {
        path: path.to_path_buf(),
        source,
    })?;
    if raw.as_deref() == Some(rendered.as_str()) {
        return Ok(Change::Unchanged);
    }

    doc::write_doc(path, &doc)?;
    Ok(Change::Updated)
}

/// Remove ivar's keys, and a legacy `env.IVAR_HALL`, from the settings file at
/// `path`.
///
/// The file is deleted only when ivar's keys were its entire content. A file
/// carrying other keys keeps them, minus ivar's keys. Absent file is
/// [`Change::Unchanged`]. A file that cannot be parsed as a JSON object is
/// left alone.
///
/// # Errors
///
/// Returns [`Error`] if the existing file cannot be parsed as a JSON object
/// or the merged document cannot be written.
pub fn remove_settings(path: &Utf8Path) -> Result<Change, Error> {
    let (existing, _) = doc::read_doc(path)?;
    let Some(mut doc) = existing else {
        return Ok(Change::Unchanged);
    };

    let Some(object) = doc.as_object_mut() else {
        return Ok(Change::Unchanged);
    };

    let mut removed_any = remove_legacy_ivar_hall(object);
    for key in IVAR_KEYS {
        if object.remove(key).is_some() {
            removed_any = true;
        }
    }

    if !removed_any {
        return Ok(Change::Unchanged);
    }
    let is_empty = object.is_empty();

    doc::finish_removal(path, is_empty, &doc)
}

/// Drop a legacy `env.IVAR_HALL` entry, and `env` itself when that empties it.
/// Returns whether the entry was present.
fn remove_legacy_ivar_hall(object: &mut serde_json::Map<String, serde_json::Value>) -> bool {
    let Some(env) = object
        .get_mut(ENV)
        .and_then(serde_json::Value::as_object_mut)
    else {
        return false;
    };
    let removed = env.remove(LEGACY_ENV_IVAR_HALL).is_some();
    if removed && env.is_empty() {
        object.remove(ENV);
    }
    removed
}

/// The full document ivar wants: `hooks` holding the session lifecycle hooks
/// and `attribution` blanked. Used when the file is absent or when merging
/// into an existing document.
fn ivar_doc() -> serde_json::Value {
    let mut root = serde_json::Map::new();

    // hooks: the lifecycle hooks that wire session env and guard into the
    // harness.
    let mut hooks = serde_json::Map::new();
    hooks.insert(
        "SessionStart".to_owned(),
        serde_json::json!([
            {
                "matcher": "",
                "hooks": [
                    {
                        "type": "command",
                        "command": "ivar session env"
                    }
                ]
            }
        ]),
    );
    hooks.insert(
        "PreToolUse".to_owned(),
        serde_json::json!([
            {
                "matcher": "",
                "hooks": [
                    {
                        "type": "command",
                        "command": "ivar guard --provider claude-code"
                    }
                ]
            }
        ]),
    );
    root.insert(IVAR_HOOKS.to_owned(), serde_json::Value::Object(hooks));
    root.insert(
        IVAR_ATTRIBUTION.to_owned(),
        serde_json::json!({ "commit": "", "pr": "" }),
    );
    serde_json::Value::Object(root)
}

#[cfg(test)]
#[path = "../../../tests/unit/harness/config/settings.rs"]
mod tests;
