//! Memory configuration for shared memory scopes.

use std::collections::HashSet;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::{Failure, FixAction};

/// A validated memory scope identifier.
/// Single-segment kebab-case string, non-empty, no path separators or traversal.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(try_from = "String", into = "String")]
pub struct ScopeName(String);

impl ScopeName {
    /// Create a new `ScopeName` after validating kebab-case and traversal rules.
    pub fn new(raw: impl Into<String>) -> Result<Self, Failure> {
        let s = raw.into();
        if s.is_empty() {
            return Err(
                Failure::blocked("scope_name.empty", "validating scope name")
                    .expected("a non-empty scope identifier")
                    .actual("an empty string")
                    .fix(FixAction::safe(
                        "specify_scope_name",
                        "Provide a non-empty kebab-case scope identifier.",
                    )),
            );
        }

        if s == "." || s == ".." || s.contains('/') || s.contains('\\') {
            return Err(
                Failure::blocked("scope_name.invalid_segment", "validating scope name")
                    .expected("a single path segment without traversal")
                    .actual(format!("'{s}' contains traversal or path separators"))
                    .fix(FixAction::safe(
                        "use_single_segment",
                        "Use a single kebab-case segment like 'product' or 'team-conventions'.",
                    )),
            );
        }

        // Must be valid kebab-case (lowercase ASCII letters, digits, hyphens; no consecutive hyphens or leading/trailing hyphens)
        let is_kebab = !s.starts_with('-')
            && !s.ends_with('-')
            && !s.contains("--")
            && s.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');

        if !is_kebab {
            return Err(
                Failure::blocked("scope_name.invalid_kebab", "validating scope name")
                    .expected("a kebab-case identifier (e.g. 'product', 'team-conventions')")
                    .actual(format!("'{s}' is not valid kebab-case"))
                    .fix(FixAction::safe(
                        "use_kebab_case",
                        "Ensure the name uses only lowercase letters, numbers, and single hyphens.",
                    )),
            );
        }

        Ok(Self(s))
    }

    /// The scope name as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for ScopeName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ScopeName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TryFrom<String> for ScopeName {
    type Error = Failure;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<ScopeName> for String {
    fn from(name: ScopeName) -> Self {
        name.0
    }
}

/// A declared memory scope within the hall.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryScope {
    /// Safe identifier for the scope (e.g. "product", "architecture").
    pub id: ScopeName,
    /// Human-readable purpose of this memory scope.
    pub purpose: String,
    /// Character budget for context injection. Must be > 0.
    pub budget: usize,
    /// Stable topic markdown slugs under this scope.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stable_topics: Vec<String>,
}

/// Top-level configuration for shared memory in `ivar.json`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryConfig {
    /// Ordered list of memory scopes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<MemoryScope>,
}

impl MemoryConfig {
    /// Validate value invariants: unique scope IDs, non-empty purpose, positive budget > 0.
    pub fn validate(&self) -> Result<(), Failure> {
        let mut seen = HashSet::new();
        for scope in &self.scopes {
            if !seen.insert(&scope.id) {
                return Err(Failure::blocked(
                    "memory_config.duplicate_scope",
                    "validating memory configuration",
                )
                .expected("unique scope identifiers")
                .actual(format!("duplicate scope '{}'", scope.id))
                .fix(FixAction::safe(
                    "deduplicate_scopes",
                    "Remove or rename the duplicate scope.",
                )));
            }

            if scope.budget == 0 {
                return Err(Failure::blocked(
                    "memory_config.zero_budget",
                    "validating memory configuration",
                )
                .expected("a positive character budget > 0")
                .actual(format!("scope '{}' has budget 0", scope.id))
                .fix(FixAction::safe(
                    "set_positive_budget",
                    "Set a positive character budget for the scope.",
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/domain/memory/config.rs"]
mod tests;
