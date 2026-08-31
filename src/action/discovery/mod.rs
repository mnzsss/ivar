//! `ivar discovery …` — a unit of work's committed memory.
//!
//! Mirrors [`crate::action::plan`]'s shape: one file per verb, shared
//! loading here. Memory lives at `docs/<name>/`, execution at
//! `plans/<name>/`.
//!
//! Unlike `plan`, these verbs never require the feature to exist: a name
//! may earn memory long before it earns execution, and often never earns
//! execution at all.

use crate::domain::discovery::DiscoveryDoc;
use crate::domain::name::FeatureName;
use crate::error::{Failure, FixAction};
use crate::infra::fs;
use crate::store::discovery;
use crate::store::layout::Layout;

pub mod amend;
pub mod close;
pub mod create;
pub mod list;
pub mod show;

/// Read a name's discovery doc, or fail with the standard "no discovery"
/// message.
///
/// # Errors
///
/// When no `discovery.md` exists for `name`, or it cannot be read.
#[allow(dead_code)]
pub(crate) fn load(layout: &Layout, name: &FeatureName) -> Result<DiscoveryDoc, Failure> {
    let path = layout.discovery_doc(name);
    if !fs::is_file(&path)? {
        return Err(Failure::blocked(
            "discovery.not_found",
            format!("no discovery doc for `{name}`"),
        )
        .expected("a name with a discovery doc")
        .actual(format!("`{path}` does not exist"))
        .fix(FixAction::safe(
            "discovery.create_first",
            format!("Create it first with `ivar discovery create {name}`."),
        )));
    }
    // `is_file` above already ruled out the `None` case; treat a race as
    // an empty doc rather than panicking, and `parse` reports it unknown.
    let source = fs::read_text(&path)?.unwrap_or_default();
    Ok(discovery::parse(&source))
}

/// Refuse to rewrite a doc ivar could not parse.
///
/// # Errors
///
/// When the doc's front matter is unreadable (D5): rewriting it would drop
/// every key ivar failed to see.
#[allow(dead_code)]
pub(crate) fn ensure_writable(doc: &DiscoveryDoc, name: &FeatureName) -> Result<(), Failure> {
    if doc.is_writable() {
        return Ok(());
    }
    Err(Failure::blocked(
        "discovery.unreadable_frontmatter",
        format!("`{name}`'s discovery doc has front matter ivar cannot read"),
    )
    .expected("a doc with readable front matter")
    .actual("the front matter is missing, unterminated, or not a mapping")
    .fix(FixAction::safe(
        "discovery.fix_frontmatter_by_hand",
        "Repair the `---` block by hand; ivar will not rewrite a header it could not read.",
    )))
}
