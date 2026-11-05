use camino::{Utf8Path, Utf8PathBuf};

use crate::domain::memory_page::{DURABLE_ROOT, EPHEMERAL_ROOT, PageKind};
use crate::store::layout::Layout;

fn is_page_path(rel: &str) -> bool {
    let inside_roots = rel.starts_with(DURABLE_ROOT) || rel.starts_with(EPHEMERAL_ROOT);
    let clean = !rel.contains('\\')
        && rel
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..");
    let known_kind = rel.starts_with(EPHEMERAL_ROOT)
        || rel.split('/').nth(2).and_then(PageKind::parse).is_some();
    inside_roots && clean && known_kind && rel.ends_with(".md")
}

pub(super) fn resolve_readable(layout: &Layout, rel: &str) -> Result<Utf8PathBuf, String> {
    if !is_page_path(rel) {
        return Err(format!(
            "`{rel}` is not a page path under docs/memory/<kind>/ or .ivar/memory/"
        ));
    }
    let path = layout.root().join(rel);
    if let (Ok(real), Ok(real_root)) = (path.canonicalize_utf8(), layout.root().canonicalize_utf8())
        && !real.starts_with(&real_root)
    {
        return Err(format!("`{rel}` resolves outside the hall"));
    }
    Ok(path)
}

pub(super) struct Target {
    pub kind: PageKind,
    pub path: Utf8PathBuf,
}

pub(super) fn resolve_durable(layout: &Layout, rel: &str) -> Result<Target, String> {
    let parts: Vec<&str> = rel.split('/').collect();
    let shaped = match parts.as_slice() {
        ["docs", "memory", kind, file] => PageKind::parse(kind)
            .filter(|kind| kind.is_durable())
            .filter(|_| {
                !rel.contains('\\')
                    && file.len() > ".md".len()
                    && file.ends_with(".md")
                    && !file.starts_with('.')
            }),
        _ => None,
    };
    let Some(kind) = shaped else {
        return Err(format!(
            "`{rel}` is not docs/memory/<durable kind>/<slug>.md"
        ));
    };
    let path = layout.root().join(rel);
    refuse_symlinks(layout.root(), &path)?;
    Ok(Target { kind, path })
}

pub(super) fn refuse_symlinks(root: &Utf8Path, path: &Utf8Path) -> Result<(), String> {
    let mut current = root.to_path_buf();
    for component in path
        .strip_prefix(root)
        .map_err(|error| error.to_string())?
        .components()
    {
        current.push(component);
        if std::fs::symlink_metadata(&current).is_ok_and(|meta| meta.file_type().is_symlink()) {
            return Err(format!("`{current}` is a symlink"));
        }
    }
    Ok(())
}
