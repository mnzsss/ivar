use camino::Utf8PathBuf;

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
