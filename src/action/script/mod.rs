//! `ivar script` — the hall's scripts, under `.ivar/scripts/` (committed, for
//! the team) and `.ivar/scripts-local/` (personal).
//!
//! `ivar` only reads scripts: it lists and validates them and announces them in
//! the session brief. No command here writes, chmods, moves or deletes one. The
//! format is [`crate::domain::script`]; the folder walk is
//! [`crate::store::script`].

pub mod list;

use crate::store::layout::Layout;
use crate::store::script::ScriptFile;

/// The script's path relative to the hall root, the way a session runs it
/// from there (`.ivar/scripts/deploy.sh`).
pub(crate) fn hall_relative(layout: &Layout, file: &ScriptFile) -> String {
    file.path
        .strip_prefix(layout.root())
        .unwrap_or(&file.path)
        .to_string()
}
