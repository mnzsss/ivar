//! A unit of work's committed memory: `docs/<name>/discovery.md`.
//!
//! ivar owns the front matter; the agent owns the prose (ADR-0002 D1).
//!
//! # Parsing and rendering live in `store::discovery`
//!
//! Splitting and serializing frontmatter touches [`crate::infra::frontmatter`],
//! and `domain` may not import `infra`. This module keeps the pure
//! [`DiscoveryDoc`] struct, [`Frontmatter`] struct, [`DiscoveryStatus`] enum,
//! [`DiscoveryDoc::new`], and [`DiscoveryDoc::is_writable`]. Parsing and
//! rendering are in [`crate::store::discovery`].
//!
//! # Parsing is deliberately lenient
//!
//! Unlike [`crate::domain::session::SessionState`], this type does **not**
//! `deny_unknown_fields`. A key ivar does not recognise is preserved, not
//! rejected: a doc is written by agents and humans across versions, and a
//! stricter reader would turn a forward-compatible field into a hard error.
//!
//! A doc whose front matter cannot be read at all becomes
//! [`DiscoveryStatus::Unknown`] and reports [`DiscoveryDoc::is_writable`]
//! as `false`. ivar lists it and never rewrites it — rewriting a header it
//! failed to parse would drop every key it could not see.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domain::name::FeatureName;

/// Where a discovery stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiscoveryStatus {
    /// Open. The normal state of a discovery in progress.
    #[default]
    Exploring,
    /// Promoted to a feature by `session convert`.
    Converted,
    /// Closed without becoming a feature — the cheapest information a team
    /// owns, and the reason `feature delete` never touches `docs/<name>/`.
    Abandoned,
    /// The front matter could not be read. Never serialised into a doc:
    /// this is what ivar reports, not what ivar writes.
    #[serde(other)]
    Unknown,
}

impl DiscoveryStatus {
    /// The wire form, as written in front matter.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Exploring => "exploring",
            Self::Converted => "converted",
            Self::Abandoned => "abandoned",
            Self::Unknown => "unknown",
        }
    }
}

/// The ivar-owned header of a discovery doc.
///
/// `#[serde(default)]` with no `deny_unknown_fields`: see the module doc.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Frontmatter {
    /// The unit of work's name — the same string as its directory.
    pub name: String,
    /// A human-readable title. Defaults to `name`.
    pub title: String,
    /// Where the discovery stands.
    pub status: DiscoveryStatus,
    /// RFC 3339, as written at creation.
    pub created_at: String,
    /// RFC 3339, bumped on every ivar-owned write.
    pub updated_at: String,
    /// Every session that contributed, oldest first.
    pub sessions: Vec<String>,
    /// Header fields this ivar version does not understand, retained so an
    /// ivar-owned status or timestamp update cannot erase newer data.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// A parsed `discovery.md`: the ivar-owned header, and the agent's prose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryDoc {
    /// The ivar-owned header.
    pub frontmatter: Frontmatter,
    /// Everything after the front matter fence. Untouched byte-for-byte.
    pub body: String,
}

impl DiscoveryDoc {
    /// A fresh doc: `exploring`, no sessions, `title` defaulting to `name`.
    #[must_use]
    pub fn new(name: &FeatureName, title: Option<&str>, now: &str) -> Self {
        Self {
            frontmatter: Frontmatter {
                name: name.as_str().to_owned(),
                title: title.unwrap_or(name.as_str()).to_owned(),
                status: DiscoveryStatus::Exploring,
                created_at: now.to_owned(),
                updated_at: now.to_owned(),
                sessions: Vec::new(),
                extra: BTreeMap::new(),
            },
            body: String::new(),
        }
    }

    /// Whether ivar can safely update this doc's front matter.
    ///
    /// Returns `false` for [`DiscoveryStatus::Unknown`]: rewriting a header
    /// it failed to read would drop every key it could not see.
    #[must_use]
    pub fn is_writable(&self) -> bool {
        self.frontmatter.status != DiscoveryStatus::Unknown
    }
}

#[cfg(test)]
#[path = "../../tests/unit/domain/discovery.rs"]
mod tests;
