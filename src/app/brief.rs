use std::io::{self, Read as _, Write as _};

use camino::Utf8PathBuf;

use crate::action::memory::brief::brief_for;
use crate::action::memory::drain::epoch_secs_now;

/// The hook payload only carries a `cwd`; a larger one is cut and ignored.
const MAX_BRIEF_PAYLOAD_BYTES: u64 = 64 * 1024;

pub(super) fn brief_event(provider: Option<&str>) {
    let mut bytes = Vec::new();
    let _ = io::stdin()
        .take(MAX_BRIEF_PAYLOAD_BYTES)
        .read_to_end(&mut bytes);
    let payload_cwd = serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|v| v.get("cwd").and_then(|c| c.as_str()).map(Utf8PathBuf::from));
    let cwd = payload_cwd.unwrap_or_else(super::respond::current_dir);
    let non_empty = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    let hall = non_empty("IVAR_HALL");
    let ambient = non_empty("IVAR_SESSION_ID");
    let now = i64::try_from(epoch_secs_now()).unwrap_or(i64::MAX);
    let text = brief_for(&cwd, hall.as_deref(), ambient.as_deref(), provider, now);
    let _ = io::stdout().write_all(text.as_bytes());
}

pub(super) fn is_brief_invocation(args: &[std::ffi::OsString]) -> bool {
    matches!(args, [_, memory, brief, ..] if memory == "memory" && brief == "brief")
}
