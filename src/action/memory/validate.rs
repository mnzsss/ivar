//! `ivar memory validate` — inspect manifest memory config, validate topic metadata, and check budgets.

use std::collections::HashSet;
use std::io;

use serde::Serialize;

use crate::action::{Ctx, discover_hall, read_manifest};
use crate::error::{Outcome, Report, WriteHuman};
use crate::store::memory::document::read_topic;

/// Input for the `ivar memory validate` command.
#[derive(Debug, Clone, Default)]
pub struct MemoryValidateInput {}

/// Outcome of the `ivar memory validate` command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemoryValidateOutcome {
    /// True if memory configuration and topics are fully valid.
    pub valid: bool,
    /// Number of declared memory scopes checked.
    pub scopes_checked: usize,
    /// Number of topic documents checked across scopes.
    pub topics_checked: usize,
    /// Budget warning messages if any scope exceeds budget.
    pub budget_warnings: Vec<String>,
}

impl WriteHuman for MemoryValidateOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.valid {
            writeln!(
                w,
                "Memory valid: {} scope(s) checked, {} topic(s) checked.",
                self.scopes_checked, self.topics_checked
            )?;
        } else {
            writeln!(
                w,
                "Memory invalid: {} scope(s) checked, {} topic(s) checked.",
                self.scopes_checked, self.topics_checked
            )?;
        }
        for warning in &self.budget_warnings {
            writeln!(w, "  Warning: {}", warning)?;
        }
        Ok(())
    }
}

/// Validate the memory store against manifest configuration.
pub fn validate(ctx: &Ctx, _input: MemoryValidateInput) -> Outcome<MemoryValidateOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;

    let mut scopes_checked = 0;
    let mut topics_checked = 0;
    let mut budget_warnings = Vec::new();

    if let Some(mem_config) = manifest.memory() {
        // Validate memory config invariants
        mem_config.validate()?;

        let mut declared_scopes = HashSet::new();
        for scope in &mem_config.scopes {
            scopes_checked += 1;
            declared_scopes.insert(&scope.id);

            let scope_dir = layout.memory_scope_dir(&scope.id);
            if !scope_dir.exists() {
                continue;
            }

            let mut total_chars = 0;
            if let Ok(entries) = std::fs::read_dir(scope_dir.as_std_path()) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().is_some_and(|ext| ext == "md") {
                        let file_stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
                        // Validate and read topic
                        let topic = read_topic(&layout, &scope.id, file_stem)?;
                        topics_checked += 1;

                        // Check scope matching
                        if topic.metadata.scope != scope.id {
                            return Err(crate::error::Failure::blocked(
                                "memory_validate.scope_mismatch",
                                format!(
                                    "topic `{}` in scope directory `{}` declared scope `{}` in frontmatter",
                                    file_stem, scope.id, topic.metadata.scope
                                ),
                            ));
                        }

                        total_chars += topic.content.chars().count();
                    }
                }
            }

            if total_chars > scope.budget {
                budget_warnings.push(format!(
                    "Scope `{}` character count ({}) exceeds declared budget ({})",
                    scope.id, total_chars, scope.budget
                ));
            }
        }
    }

    let is_valid = budget_warnings.is_empty();

    Ok(Report::new(MemoryValidateOutcome {
        valid: is_valid,
        scopes_checked,
        topics_checked,
        budget_warnings,
    }))
}
