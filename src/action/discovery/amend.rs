//! `ivar discovery amend <name>` — add to a unit of work's memory.
//!
//! # Two modes
//!
//! **Append** (the default) adds a dated `## Amendment` block carrying the
//! session id. It cannot destroy anything, so it takes no guard — a
//! discovery that spans four sessions ends up with four blocks, in order.
//!
//! **Merge** (`--merge`) replaces the whole body, so it *can* destroy work.
//! It requires `--expected-hash <sha256>`: the caller states the version it
//! read, and a mismatch means someone else wrote in between. The write is
//! refused rather than merged — ivar owns the container, not the prose, and
//! resolving a prose conflict is not its judgement to make.
//!
//! Both modes bump `updated_at` and record the session in `sessions`. That
//! is the ivar-owned half of the doc (ADR-0002 D1).

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::action::Ctx;
use crate::domain::name::FeatureName;
use crate::domain::session::rfc3339_now;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::{fs, hash};

use super::super::discover_hall;

/// What `ivar discovery amend` needs.
#[derive(Debug, Clone)]
pub struct AmendInput {
    /// The unit of work's name.
    pub name: String,
    /// The prose to add, or the whole new body under `--merge`.
    pub content: String,
    /// Replace the body instead of appending to it.
    pub merge: bool,
    /// The doc's expected SHA-256. Required with `merge`.
    pub expected_hash: Option<String>,
    /// The session doing the writing, recorded in the doc.
    pub session_id: Option<String>,
}

/// How the doc was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// A dated block was added.
    Append,
    /// The body was replaced.
    Merge,
}

/// What `ivar discovery amend` did.
#[derive(Debug, Clone, Serialize)]
pub struct AmendOutcome {
    /// The doc's path.
    pub path: Utf8PathBuf,
    /// Which mode wrote it.
    pub mode: Mode,
    /// The doc's SHA-256 after the write — pass this to the next `--merge`.
    pub hash: String,
}

impl WriteHuman for AmendOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        let verb = match self.mode {
            Mode::Append => "Appended to",
            Mode::Merge => "Rewrote",
        };
        writeln!(w, "{verb} {}. Hash: {}", self.path, self.hash)
    }
}

/// Add to a discovery doc.
///
/// # Errors
///
/// When no hall is found, the name has no discovery doc, its front matter
/// is unreadable, `--merge` was given without `--expected-hash`, or the
/// expected hash does not match the file on disk.
pub fn amend(ctx: &Ctx, input: AmendInput) -> Outcome<AmendOutcome> {
    let layout = discover_hall(ctx)?;
    let name = FeatureName::new(input.name)?;

    let mut doc = super::load(&layout, &name)?;
    super::ensure_writable(&doc, &name)?;

    let path = layout.discovery_doc(&name);
    let now = rfc3339_now();

    let mode = if input.merge {
        let Some(expected) = input.expected_hash else {
            return Err(Failure::blocked(
                "discovery.merge_needs_hash",
                "`--merge` replaces the whole document and needs `--expected-hash`",
            )
            .expected("`--expected-hash <sha256>` alongside `--merge`")
            .actual("`--merge` was given with no expected hash")
            .fix(FixAction::safe(
                "discovery.read_then_merge",
                format!(
                    "Read the doc first — `ivar discovery show {name}` — then pass its sha256 as `--expected-hash`."
                ),
            )));
        };
        let actual = hash::file(&path)?;
        if actual != expected {
            return Err(Failure::blocked(
                "discovery.drift",
                format!("`{name}`'s discovery doc changed since it was read"),
            )
            .expected(format!("a doc hashing to {expected}"))
            .actual(format!("it hashes to {actual}"))
            .fix(FixAction::safe(
                "discovery.reread_then_merge",
                "Re-read the doc, fold in the change, and merge again with the new hash.",
            )));
        }
        doc.body = ensure_trailing_newline(&input.content);
        Mode::Merge
    } else {
        doc.body = append_block(&doc.body, &input.content, &now, input.session_id.as_deref());
        Mode::Append
    };

    doc.frontmatter.updated_at = now;
    if let Some(session) = input
        .session_id
        .filter(|s| !doc.frontmatter.sessions.contains(s))
    {
        doc.frontmatter.sessions.push(session);
    }

    fs::write_text(&path, &crate::store::discovery::render(&doc)?)?;
    let hash = hash::file(&path)?;

    Ok(Report::new(AmendOutcome { path, mode, hash }))
}

/// `body` plus a dated amendment block.
///
/// The date is the `YYYY-MM-DD` prefix of an RFC 3339 timestamp — the
/// format `rfc3339_now` guarantees, so slicing is safe.
fn append_block(body: &str, content: &str, now: &str, session: Option<&str>) -> String {
    let day = now.get(..10).unwrap_or(now);
    let mut out = ensure_trailing_newline(body);
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(&format!("## Amendment ({day})\n\n"));
    if let Some(session) = session {
        out.push_str(&format!("Session: {session}\n\n"));
    }
    out.push_str(&ensure_trailing_newline(content));
    out
}

/// `text` with exactly one trailing newline, or empty if it is empty.
fn ensure_trailing_newline(text: &str) -> String {
    if text.is_empty() || text.ends_with('\n') {
        text.to_owned()
    } else {
        format!("{text}\n")
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/action/discovery/amend.rs"]
mod tests;
