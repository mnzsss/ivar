//! The workstream inbox: the channel a human's reply travels to a blocked
//! workstream, and back into the prompt when `tick` relaunches it.
//!
//! # Why a file rather than the board
//!
//! One append-only JSONL file per workstream, at
//! `features/<feature>/execution/inbox/<workstream>.jsonl`, deliberately
//! outside `board.json` — the board stays a single small document the plan
//! owns, and a reply is a conversation, unbounded in length and count.
//!
//! # Both ends in one place
//!
//! [`append`] is `reply`'s end of the channel and [`read`] is `tick`'s. They
//! live together because they are one format: a reply written in a shape the
//! relaunch cannot read is a human answer silently thrown away, and the two
//! ends drifting apart is exactly how that happens.

use std::fs::OpenOptions;
use std::io::Write;

use camino::Utf8Path;

use crate::domain::name::FeatureName;
use crate::error::Failure;
use crate::infra::fs;
use crate::store::layout::Layout;

/// Append one reply to `workstream`'s inbox, creating the file (and its
/// directory) on first use. Append-only — a reply is never rewritten.
pub fn append(
    layout: &Layout,
    feature: &FeatureName,
    workstream: &str,
    message: &str,
) -> Result<(), Failure> {
    let inbox_path = layout.execution_inbox(feature, workstream);

    if let Some(parent) = inbox_path.parent() {
        fs::ensure_dir(parent).map_err(Failure::from)?;
    }

    let line = serde_json::json!({
        "kind": "inbox",
        "timestamp": epoch_seconds(),
        "message": message,
    });

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(inbox_path.as_std_path())
        .map_err(|source| write_failed(&inbox_path, &source))?;
    file.write_all(format!("{line}\n").as_bytes())
        .map_err(|source| write_failed(&inbox_path, &source))?;

    Ok(())
}

/// Every reply in `workstream`'s inbox, oldest first. An inbox that does not
/// exist yet is simply empty — the common case, since most workstreams are
/// never blocked.
///
/// A line that is not a JSON object with a `message` string is skipped rather
/// than refused: the file is append-only and one damaged line must not make
/// every later reply unreadable.
pub fn read(
    layout: &Layout,
    feature: &FeatureName,
    workstream: &str,
) -> Result<Vec<String>, Failure> {
    let inbox_path = layout.execution_inbox(feature, workstream);
    let Some(text) = fs::read_text(&inbox_path)? else {
        return Ok(Vec::new());
    };
    Ok(text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|entry| {
            entry
                .get("message")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .collect())
}

/// The failure both ends report when the file itself will not cooperate.
fn write_failed(path: &Utf8Path, source: &std::io::Error) -> Failure {
    Failure::failed(
        "execute.inbox_write_failed",
        format!("could not append to `{path}`"),
    )
    .actual(source.to_string())
}

/// Epoch seconds, the timestamp format the journal uses too.
fn epoch_seconds() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "../../../tests/unit/action/execute/inbox.rs"]
mod tests;
