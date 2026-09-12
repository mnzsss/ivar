//! Store operations for session episodes.

use crate::domain::memory::episode::EpisodePayload;
use crate::error::Failure;
use crate::infra::fs;
use crate::store::layout::Layout;

/// Persist an episode markdown file into `<hall>/memory/sessions/<session_id>.md`.
pub fn persist_episode(layout: &Layout, episode: &EpisodePayload) -> Result<(), Failure> {
    let episodes_dir = layout.memory_episodes_dir();
    fs::ensure_dir(&episodes_dir)?;

    let target_file = episodes_dir.join(format!("{}.md", episode.session_id.as_str()));
    let content = episode.render_markdown();
    fs::write_text(&target_file, &content)?;
    Ok(())
}

#[cfg(test)]
#[path = "../../../tests/unit/store/memory/episode.rs"]
mod tests;
