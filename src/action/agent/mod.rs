//! `agent`: scheduled agents run in Docker from systemd user timers.

pub mod requirements;
pub mod sync;

use crate::infra::hash;
use crate::store::layout::Layout;

/// The 8-hex-char tag that ties a hall to its systemd units.
#[must_use]
pub fn hall_hash(layout: &Layout) -> String {
    hash::text(layout.root().as_str()).chars().take(8).collect()
}
