//! `ivar discovery list` — every unit of work with committed memory.
//!
//! Scans both `<hall>/.ivar/features/*/discovery.md` and
//! `<hall>/.ivar/sessions/*/discovery.md`.

use std::fmt;
use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::action::Ctx;
use crate::domain::discovery::DiscoveryStatus;
use crate::domain::name::{FeatureName, SessionId};
use crate::error::{Outcome, Report, WriteHuman};
use crate::infra::fs;
use crate::store::discovery;

use super::super::discover_hall;

/// The identity of a listed discovery: either a converted feature or an
/// unconverted discovery session.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(untagged)]
pub enum DiscoveryName {
    /// A converted discovery doc living in `.ivar/features/<name>/discovery.md`.
    Feature(FeatureName),
    /// An unconverted discovery session doc living in `.ivar/sessions/<id>/discovery.md`.
    Session(SessionId),
}

impl DiscoveryName {
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Feature(name) => name.as_str(),
            Self::Session(id) => id.as_str(),
        }
    }
}

impl fmt::Display for DiscoveryName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Feature(name) => write!(f, "{name}"),
            Self::Session(id) => write!(f, "{id}"),
        }
    }
}

impl PartialEq<FeatureName> for DiscoveryName {
    fn eq(&self, other: &FeatureName) -> bool {
        match self {
            Self::Feature(name) => name == other,
            Self::Session(_) => false,
        }
    }
}

impl PartialEq<DiscoveryName> for FeatureName {
    fn eq(&self, other: &DiscoveryName) -> bool {
        other == self
    }
}

/// What `ivar discovery list` needs.
#[derive(Debug, Clone)]
pub struct ListInput {
    /// Report only discoveries in this status. `None` reports all.
    pub status: Option<DiscoveryStatus>,
}

/// One discovery, as listed.
#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    /// The unit of work or session identity.
    pub name: DiscoveryName,
    /// Its title, or the name when the header could not be read.
    pub title: String,
    /// Where it stands.
    pub status: DiscoveryStatus,
}

/// What `ivar discovery list` found.
#[derive(Debug, Clone, Serialize)]
pub struct ListOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// One entry per discovery, sorted by name.
    pub discoveries: Vec<Summary>,
}

impl WriteHuman for ListOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.discoveries.is_empty() {
            writeln!(w, "No discoveries in {}.", self.root)?;
            return Ok(());
        }
        writeln!(w, "Discoveries in {}:", self.root)?;
        for entry in &self.discoveries {
            writeln!(
                w,
                "  {}  [{}]  {}",
                entry.name,
                entry.status.as_str(),
                entry.title
            )?;
        }
        Ok(())
    }
}

/// List every unit of work with discovery memory across feature dirs and active discovery sessions.
///
/// # Errors
///
/// When no hall is found, or directories cannot be read.
pub fn list(ctx: &Ctx, input: ListInput) -> Outcome<ListOutcome> {
    let layout = discover_hall(ctx)?;
    let mut discoveries = Vec::new();

    // 1. Scan converted features: .ivar/features/*/discovery.md
    let features_dir = layout.features_dir();
    if fs::is_dir(&features_dir)? {
        for child in fs::read_dir(&features_dir)? {
            if !fs::is_dir(&child)? {
                continue;
            }
            let Some(basename) = child.file_name() else {
                continue;
            };
            let Ok(feature_name) = FeatureName::new(basename) else {
                continue;
            };
            let doc_path = layout.discovery_doc(&feature_name);
            if !fs::is_file(&doc_path)? {
                continue;
            }

            let doc = discovery::parse(&fs::read_text(&doc_path)?.unwrap_or_default());
            let title = if doc.frontmatter.title.is_empty() {
                feature_name.as_str().to_owned()
            } else {
                doc.frontmatter.title.clone()
            };
            discoveries.push(Summary {
                name: DiscoveryName::Feature(feature_name),
                title,
                status: doc.frontmatter.status,
            });
        }
    }

    // 2. Scan unconverted discovery sessions: .ivar/sessions/*/discovery.md
    let sessions_dir = layout.discovery_sessions_dir();
    if fs::is_dir(&sessions_dir)? {
        for child in fs::read_dir(&sessions_dir)? {
            if !fs::is_dir(&child)? {
                continue;
            }
            let Some(basename) = child.file_name() else {
                continue;
            };
            let Ok(session_id) = SessionId::new(basename) else {
                continue;
            };
            let doc_path = child.join("discovery.md");
            if !fs::is_file(&doc_path)? {
                continue;
            }

            let doc = discovery::parse(&fs::read_text(&doc_path)?.unwrap_or_default());
            let title = if doc.frontmatter.title.is_empty() {
                if doc.frontmatter.name.is_empty() {
                    session_id.as_str().to_owned()
                } else {
                    doc.frontmatter.name.clone()
                }
            } else {
                doc.frontmatter.title.clone()
            };
            discoveries.push(Summary {
                name: DiscoveryName::Session(session_id),
                title,
                status: doc.frontmatter.status,
            });
        }
    }

    if let Some(wanted) = input.status {
        discoveries.retain(|entry| entry.status == wanted);
    }
    discoveries.sort_by(|a, b| a.name.as_str().cmp(b.name.as_str()));

    Ok(Report::new(ListOutcome {
        root: layout.root().to_path_buf(),
        discoveries,
    }))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/discovery/list.rs"]
mod tests;
