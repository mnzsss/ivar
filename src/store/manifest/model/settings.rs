use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};

use super::{Error, Manifest};
use crate::domain::coordination::DEFAULT_TTL_DAYS;

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

/// How `ivar` treats the hall's session memory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct MemorySettings {
    /// Whether session hooks record Observations. On unless set to `false`.
    #[serde(default = "capture_on", skip_serializing_if = "is_on")]
    capture: bool,
    /// Days an open handoff or a pending message stays visible. 7 when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    handoff_ttl_days: Option<NonZeroU32>,
}

const fn capture_on() -> bool {
    true
}

#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_on(capture: &bool) -> bool {
    *capture
}

impl MemorySettings {
    /// `None` when every field is at its default, so the `memory` key disappears.
    fn normalised(self) -> Option<Self> {
        (!self.capture || self.handoff_ttl_days.is_some()).then_some(self)
    }
}

impl Manifest {
    /// Whether session hooks record Observations; on unless the hall opted out.
    #[must_use]
    pub fn memory_capture(&self) -> bool {
        self.memory.as_ref().is_none_or(|settings| settings.capture)
    }

    /// Days an open handoff or a pending message stays visible.
    #[must_use]
    pub fn memory_handoff_ttl_days(&self) -> u32 {
        self.memory
            .as_ref()
            .and_then(|settings| settings.handoff_ttl_days)
            .map_or(DEFAULT_TTL_DAYS, NonZeroU32::get)
    }

    /// Return a new `Manifest` with memory capture switched on or off. A
    /// `memory` key left at its defaults is dropped, so a hall that never
    /// opted out stays byte-identical.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`] if the rebuilt manifest fails validation.
    pub fn with_memory_capture(&self, enabled: bool) -> Result<Self, Error> {
        let mut manifest = self.rebuild(self.providers.clone(), self.repos.clone())?;
        manifest.memory = MemorySettings {
            capture: enabled,
            handoff_ttl_days: self
                .memory
                .as_ref()
                .and_then(|settings| settings.handoff_ttl_days),
        }
        .normalised();
        manifest.validate()?;
        Ok(manifest)
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/store/manifest/memory.rs"]
mod tests;
