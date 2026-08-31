//! Serialization, parsing, and rendering for discovery docs.
//!
//! Parsing and rendering live here in `store::discovery` rather than in
//! `domain::discovery` because splitting and serializing frontmatter touches
//! [`crate::infra::frontmatter`], and `domain` may not import `infra`.
//! The pure domain struct and status checks stay in [`crate::domain::discovery`].

use crate::domain::discovery::{DiscoveryDoc, DiscoveryStatus, Frontmatter};
use crate::error::Failure;
use crate::infra::frontmatter;

/// Read a doc. Never fails: an unreadable header becomes
/// [`DiscoveryStatus::Unknown`] with the whole input kept as body, so
/// the prose is never lost to a parse error.
#[must_use]
pub fn parse(source: &str) -> DiscoveryDoc {
    let unknown = || DiscoveryDoc {
        frontmatter: Frontmatter {
            status: DiscoveryStatus::Unknown,
            ..Frontmatter::default()
        },
        body: source.to_owned(),
    };

    let Ok(split) = frontmatter::split(source) else {
        return unknown();
    };
    if split.frontmatter.is_none() {
        return unknown();
    }
    let Ok(parsed) = frontmatter::parse::<Frontmatter>(source) else {
        return unknown();
    };
    if parsed.status == DiscoveryStatus::Unknown {
        return unknown();
    }

    DiscoveryDoc {
        frontmatter: parsed,
        body: split.body.to_owned(),
    }
}

/// Render a doc back to its text form: YAML front matter followed by the body.
///
/// Refuses to render if `doc.is_writable()` is false (i.e. status is `Unknown`).
/// Preserves unknown keys present in `doc.frontmatter.extra`.
pub fn render(doc: &DiscoveryDoc) -> Result<String, Failure> {
    if !doc.is_writable() {
        return Err(Failure::blocked(
            "discovery.unwritable",
            "cannot re-render a discovery doc whose front matter failed to parse",
        )
        .expected("a writable discovery doc")
        .actual("a doc with status: unknown"));
    }

    frontmatter::replace(&doc.body, &doc.frontmatter).map_err(Into::into)
}

#[cfg(test)]
#[path = "../../tests/unit/store/discovery.rs"]
mod tests;
