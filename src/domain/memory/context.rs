//! Memory prompt context and session view projection.
//!
//! Renders deterministic, bounded memory blocks for prompt injection (inside
//! managed markers `<!-- ivar:memory:start -->` and `<!-- ivar:memory:end -->`)
//! and manages session-view projection of the canonical `memory/` directory.
use camino::Utf8Path;
use serde::{Deserialize, Serialize};
use crate::domain::name::FeatureName;
use crate::error::Failure;
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;
use crate::store::memory::document::read_topic;

/// Marker opening the ivar-managed memory region in instructions.
pub const MEMORY_MANAGED_START: &str = "<!-- ivar:memory:start -->";

/// Marker closing the ivar-managed memory region in instructions.
pub const MEMORY_MANAGED_END: &str = "<!-- ivar:memory:end -->";

/// A single rendered memory block with measured character count and budget.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryBlock {
    pub heading: String,
    pub body: String,
    pub chars: usize,
    pub budget: usize,
}

/// The aggregated memory prompt context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryContext {
    pub hall_block: String,
    pub feature_block: String,
    pub hot_block: Option<String>,
    pub total_chars: usize,
}

impl MemoryContext {
    /// Render the combined prompt context as an injected block with delimiters.
    ///
    /// The stable sections (hall and feature blocks) come first, followed by
    /// any dynamic hot handoff context.
    #[must_use]
    pub fn render_block(&self) -> String {
        let mut sections = Vec::new();
        if !self.hall_block.is_empty() {
            sections.push(self.hall_block.clone());
        }
        if !self.feature_block.is_empty() {
            sections.push(self.feature_block.clone());
        }
        if let Some(hot) = &self.hot_block {
            if !hot.is_empty() {
                sections.push(hot.clone());
            }
        }

        if sections.is_empty() {
            String::new()
        } else {
            sections.join("\n\n")
        }
    }
}

/// Render the deterministic memory prompt context for the given layout, manifest,
/// optional feature, and optional hot handoff string.
pub fn render_memory_context(
    layout: &Layout,
    manifest: &Manifest,
    feature: Option<&FeatureName>,
    hot_handoff: Option<&str>,
) -> Result<MemoryContext, Failure> {
    let mut total_chars = 0;

    // 1. Hall Memory Block
    let mut hall_block = String::new();
    if let Some(memory_cfg) = manifest.memory() {
        let mut rendered_scopes = Vec::new();

        for scope in &memory_cfg.scopes {
            let mut scope_text = format!("### Scope `{}`: {}\n", scope.id, scope.purpose);
            let mut scope_chars = 0;

            for slug in &scope.stable_topics {
                if let Ok(topic) = read_topic(layout, &scope.id, slug) {
                    let topic_content = format!(
                        "\n#### {}\n{}\n",
                        topic.metadata.title,
                        topic.content.trim()
                    );
                    scope_chars += topic_content.chars().count();
                    scope_text.push_str(&topic_content);
                }
            }

            if scope_chars > scope.budget {
                scope_text.push_str(&format!(
                    "\n> **Warning**: Scope `{}` character count ({}) exceeds declared budget ({}). Propose condensation.\n",
                    scope.id, scope_chars, scope.budget
                ));
            }

            rendered_scopes.push(scope_text.trim_end().to_string());
        }

        if !rendered_scopes.is_empty() {
            let content = rendered_scopes.join("\n\n");
            total_chars += content.chars().count();
            hall_block = format!(
                "{}\n## Shared Memory (Hall Context)\n\n{}\n{}",
                MEMORY_MANAGED_START,
                content,
                MEMORY_MANAGED_END
            );
        }
    }

    // 2. Feature Memory Block
    let mut feature_block = String::new();
    if let Some(feat) = feature {
        let inbox_dir = layout.feature_memory_inbox(feat);
        let feature_topics: Vec<String> = Vec::new();
        if fs::is_dir(&inbox_dir)? {
            // Read active topics in feature inbox if any (future-proofed)
        }

        if !feature_topics.is_empty() {
            let content = feature_topics.join("\n\n");
            total_chars += content.chars().count();
            feature_block = format!(
                "<!-- ivar:feature-memory:start -->\n## Feature Memory (`{}`)\n\n{}\n<!-- ivar:feature-memory:end -->",
                feat, content
            );
        }
    }

    // 3. Hot Dynamic Context
    let hot_block = hot_handoff.and_then(|hot| {
        let trimmed = hot.trim();
        if trimmed.is_empty() {
            None
        } else {
            total_chars += trimmed.chars().count();
            Some(format!(
                "<!-- ivar:hot-memory:start -->\n## Recent Handoff Context\n\n{}\n<!-- ivar:hot-memory:end -->",
                trimmed
            ))
        }
    });

    Ok(MemoryContext {
        hall_block,
        feature_block,
        hot_block,
        total_chars,
    })
}

/// Project the canonical `memory/` directory into `<view_dir>/memory` as a symlink.
pub fn project_memory_symlink(layout: &Layout, view_dir: &Utf8Path) -> Result<(), Failure> {
    let memory_root = layout.memory_root();
    if !fs::is_dir(&memory_root)? {
        return Ok(());
    }

    let link = view_dir.join("memory");
    fs::replace_symlink_if_changed(&memory_root, &link)?;
    Ok(())
}

#[cfg(test)]
#[path = "../../../tests/unit/domain/memory/context.rs"]
mod tests;
