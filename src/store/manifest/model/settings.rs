use serde::{Deserialize, Serialize};

use super::Error;

/// Which hall-root directories `ivar graph index` keeps in the graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Graph {
    /// Hall-relative directories, normalised and sorted.
    pub(super) paths: Vec<String>,
}

impl Graph {
    #[must_use]
    pub fn paths(&self) -> &[String] {
        &self.paths
    }
}

/// The one directory of `.ivar` a graph path may name: the hall's committed skills.
const HALL_SKILLS_DIR: &str = "skills";

pub(crate) fn normalise_graph_path(raw: &str) -> Result<String, Error> {
    let invalid = |reason: &str| Error::InvalidGraphPath {
        path: raw.to_owned(),
        reason: reason.to_owned(),
    };
    let trimmed = raw.trim();
    if trimmed.starts_with('/') {
        return Err(invalid("must be relative to the hall root"));
    }
    if trimmed.contains('\\') {
        return Err(invalid("use `/` as the separator"));
    }
    let components: Vec<&str> = trimmed
        .split('/')
        .filter(|c| !c.is_empty() && *c != ".")
        .collect();
    if components.is_empty() {
        return Err(invalid("the hall root itself cannot be a graph path"));
    }
    if components.contains(&"..") {
        return Err(invalid("must not leave the hall root"));
    }
    if components.first() == Some(&".ivar") && components.get(1) != Some(&HALL_SKILLS_DIR) {
        return Err(invalid(
            "`.ivar` is ivar's own state; only `.ivar/skills`, the hall's committed skills, can be indexed",
        ));
    }
    Ok(components.join("/"))
}
