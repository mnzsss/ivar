//! The move from the pre-memory doc folders (`docs/product`, `docs/updates`) into
//! `docs/memory/`, shared by `ivar doctor` and `ivar feature cleanup`.

use camino::Utf8Path;

use super::reindex::collect;
use crate::store::layout::Layout;

/// Old folder and the `docs/memory` folder that replaced it.
pub(crate) const LEGACY_FOLDERS: [(&str, &str); 2] = [
    ("docs/product", "docs/memory/product"),
    ("docs/updates", "docs/memory/update"),
];

/// A legacy folder only counts when it still holds a Markdown page at any depth.
pub(crate) fn holds_pages(dir: &Utf8Path) -> bool {
    let mut found = Vec::new();
    collect(dir, &mut found);
    !found.is_empty()
}

/// The shell command that moves every legacy folder still holding pages, or `None`
/// when nothing needs moving.
pub(crate) fn migration_command(layout: &Layout, hall_is_git: bool) -> Option<String> {
    // `git mv` refuses to overwrite; plain `mv` needs `-n` to do the same.
    let mv = if hall_is_git { "git mv" } else { "mv -n" };
    let moves: Vec<String> = LEGACY_FOLDERS
        .into_iter()
        .filter(|(old, _)| holds_pages(&layout.root().join(old)))
        .map(|(old, new)| {
            if layout.root().join(new).is_dir() {
                format!("{mv} {old}/* {new}/")
            } else if hall_is_git {
                format!("git mv {old} {new}")
            } else {
                format!("mv {old} {new}")
            }
        })
        .collect();
    (!moves.is_empty()).then(|| format!("mkdir -p docs/memory && {}", moves.join(" && ")))
}

/// Whether `docs/updates` still holds pages, the case `feature cleanup` points at a move.
pub(crate) fn updates_need_migration(layout: &Layout) -> bool {
    holds_pages(&layout.root().join("docs/updates"))
}
