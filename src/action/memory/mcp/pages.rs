use std::collections::BTreeSet;
use std::fmt::Write as _;

use camino::{Utf8Path, Utf8PathBuf};
use serde_json::{Map, Value, json};

use super::{SIMILAR_TITLE_THRESHOLD, confine, to_text};
use crate::action::memory::reindex;
use crate::domain::memory_page::{
    PageIssue, PageKind, PageMeta, Relation, has_number_prefix, is_kebab,
};
use crate::infra::{frontmatter, fs, hash};
use crate::store::layout::Layout;
use crate::store::memory::page_source::parse_page;

pub(super) fn write(layout: &Layout, args: &Value) -> Result<String, String> {
    let kind_name = str_arg(args, "kind")?;
    let kind =
        PageKind::parse(kind_name).ok_or_else(|| format!("unknown page kind `{kind_name}`"))?;
    if !kind.is_durable() {
        return Err(
            "`session` pages are written by the consolidation, not by this tool".to_owned(),
        );
    }
    let title = str_arg(args, "title")?;
    let slug = args
        .get("slug")
        .and_then(Value::as_str)
        .map_or_else(|| kebab(title), str::to_owned);
    validate_slug(&slug, kind)?;
    let replace = args
        .get("replace")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if replace && kind == PageKind::Update {
        return Err(refuse_update("replaced"));
    }
    let dir = layout.page_dir(kind);
    confine::refuse_symlinks(layout.root(), &dir)?;

    let lock = fs::lock_exclusive(&layout.memory_lock()).map_err(|error| error.to_string())?;
    let file = if kind.numbered() {
        format!("{:03}-{slug}.md", next_page_number(&dir))
    } else {
        format!("{slug}.md")
    };
    let rel = format!("docs/memory/{}/{file}", kind.as_str());
    let target = confine::resolve_durable(layout, &rel)?;
    let similar = similar_titles(&dir, target.kind, title, replace.then_some(rel.as_str()));
    let content = render(title, args, &body_arg(args));
    parse_page(&rel, &content).map_err(|issues| {
        let messages: Vec<&str> = issues.iter().map(|issue| issue.message.as_str()).collect();
        format!("the page would be invalid: {}", messages.join("; "))
    })?;
    publish(&target.path, &rel, &content, replace)?;
    drop(lock);

    let indexed = reindex::refresh(layout).is_ok();
    to_text(&json!({
        "path": rel,
        "hash": hash::text(&content),
        "indexed": indexed,
        "similar": similar,
    }))
}

pub(super) fn patch(layout: &Layout, args: &Value) -> Result<String, String> {
    let rel = str_arg(args, "path")?;
    let target = confine::resolve_durable(layout, rel)?;
    if target.kind == PageKind::Update {
        return Err(refuse_update("patched"));
    }
    let old = args.get("old").and_then(Value::as_str);
    if old.is_none() && args.get("new").is_some() {
        return Err("`new` needs `old`: pass both to replace a piece of the body".to_owned());
    }
    let lock = fs::lock_exclusive(&layout.memory_lock()).map_err(|error| error.to_string())?;
    let source = std::fs::read_to_string(&target.path)
        .map_err(|error| format!("could not read `{rel}`: {error}"))?;
    let page = parse_page(rel, &source).map_err(|issues| invalid(rel, &issues))?;

    let mut body = page.body.clone();
    if let Some(old) = old {
        let found = body.matches(old).count();
        if found != 1 {
            return Err(format!(
                "`old` must appear exactly once in the body (found {found})"
            ));
        }
        let new = args.get("new").and_then(Value::as_str).unwrap_or("");
        body = body.replacen(old, new, 1);
    }
    let metadata_changed = ["tags", "pinned", "relations", "expires_at"]
        .iter()
        .any(|key| args.get(key).is_some());
    let head = if metadata_changed {
        let mut meta = page.meta.clone();
        apply_metadata(&mut meta, args);
        let rewritten = frontmatter::replace(&source, &patched_fields(&source, &meta)?)
            .map_err(|error| format!("could not rewrite the frontmatter of `{rel}`: {error}"))?;
        rewritten
            .strip_suffix(&page.body)
            .map(str::to_owned)
            .ok_or_else(|| format!("`{rel}` could not be split into frontmatter and body"))?
    } else {
        source
            .len()
            .checked_sub(page.body.len())
            .and_then(|end| source.get(..end))
            .map(str::to_owned)
            .ok_or_else(|| format!("`{rel}` could not be split into frontmatter and body"))?
    };
    let content = format!("{head}{body}");
    parse_page(rel, &content).map_err(|issues| invalid(rel, &issues))?;
    publish(&target.path, rel, &content, true)?;
    drop(lock);

    let indexed = reindex::refresh(layout).is_ok();
    to_text(&json!({ "path": rel, "hash": hash::text(&content), "indexed": indexed }))
}

pub(super) fn delete(layout: &Layout, args: &Value) -> Result<String, String> {
    let rel = str_arg(args, "path")?;
    let target = confine::resolve_durable(layout, rel)?;
    if target.kind == PageKind::Update {
        return Err(refuse_update("deleted"));
    }
    let lock = fs::lock_exclusive(&layout.memory_lock()).map_err(|error| error.to_string())?;
    std::fs::remove_file(&target.path)
        .map_err(|error| format!("could not delete `{rel}`: {error}"))?;
    drop(lock);
    let indexed = reindex::refresh(layout).is_ok();
    to_text(&json!({ "path": rel, "indexed": indexed }))
}

fn refuse_update(action: &str) -> String {
    format!("`update` pages belong to the delivery record and cannot be {action}")
}

fn patched_fields(source: &str, meta: &PageMeta) -> Result<Map<String, Value>, String> {
    let mut fields: Map<String, Value> =
        frontmatter::parse(source).map_err(|error| error.to_string())?;
    let mut set = |key: &str, value: Option<Value>| match value {
        Some(value) => fields.insert(key.to_owned(), value),
        None => fields.remove(key),
    };
    set("tags", (!meta.tags.is_empty()).then(|| json!(meta.tags)));
    set("pinned", meta.pinned.then_some(Value::Bool(true)));
    set(
        "relations",
        (!meta.relations.is_empty()).then(|| {
            meta.relations
                .iter()
                .map(|relation| json!({"type": relation.kind, "target": relation.target}))
                .collect()
        }),
    );
    set("expires_at", meta.expires_at.as_ref().map(|at| json!(at)));
    Ok(fields)
}

fn invalid(rel: &str, issues: &[PageIssue]) -> String {
    let messages: Vec<&str> = issues.iter().map(|issue| issue.message.as_str()).collect();
    format!("`{rel}` is not a valid page: {}", messages.join("; "))
}

pub(super) fn temp_path(dir: &Utf8Path) -> Utf8PathBuf {
    dir.join(format!(
        ".{}-{:?}.tmp",
        std::process::id(),
        std::thread::current().id()
    ))
}

pub(super) fn publish(
    path: &Utf8Path,
    rel: &str,
    content: &str,
    replace: bool,
) -> Result<(), String> {
    let dir = path.parent().ok_or("page path has no directory")?;
    std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    let temp = temp_path(dir);
    if let Err(error) = std::fs::write(&temp, content) {
        let _ = std::fs::remove_file(&temp);
        return Err(error.to_string());
    }
    // hard_link fails with AlreadyExists instead of replacing, so a page another writer created first is never clobbered; rename would overwrite it.
    let placed = if replace {
        std::fs::rename(&temp, path)
    } else {
        std::fs::hard_link(&temp, path)
    };
    let _ = std::fs::remove_file(&temp);
    placed.map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            format!("`{rel}` already exists; pass `replace: true` to overwrite it")
        } else if replace {
            error.to_string()
        } else {
            format!("could not write `{rel}` without overwriting; the filesystem may not support hard links: {error}")
        }
    })
}

fn next_page_number(dir: &Utf8Path) -> u32 {
    let highest = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let (digits, _) = name.split_once('-')?;
            if digits.len() == 3 {
                digits.parse::<u32>().ok()
            } else {
                None
            }
        })
        .max()
        .unwrap_or(0);
    highest + 1
}

pub(super) fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("`{key}` is required"))
}

fn body_arg(args: &Value) -> String {
    let body = args.get("body").and_then(Value::as_str).unwrap_or("");
    if body.is_empty() || body.ends_with('\n') {
        body.to_owned()
    } else {
        format!("{body}\n")
    }
}

fn kebab(title: &str) -> String {
    title
        .to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn validate_slug(slug: &str, kind: PageKind) -> Result<(), String> {
    if !is_kebab(slug) {
        return Err(format!("`{slug}` is not a kebab-case ASCII slug"));
    }
    if kind.numbered() && has_number_prefix(slug) {
        return Err("`slug` must not carry the NNN- prefix; the tool assigns it".to_owned());
    }
    Ok(())
}

fn json_scalar(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_default()
}

pub(super) fn apply_metadata(meta: &mut PageMeta, args: &Value) {
    if let Some(tags) = args.get("tags").and_then(Value::as_array) {
        meta.tags = tags
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
    }
    if let Some(pinned) = args.get("pinned").and_then(Value::as_bool) {
        meta.pinned = pinned;
    }
    if let Some(relations) = args.get("relations").and_then(Value::as_array) {
        meta.relations = relations
            .iter()
            .filter_map(|relation| {
                Some(Relation {
                    kind: relation.get("type")?.as_str()?.to_owned(),
                    target: relation.get("target")?.as_str()?.to_owned(),
                })
            })
            .collect();
    }
    match args.get("expires_at") {
        Some(Value::String(expires_at)) => meta.expires_at = Some(expires_at.clone()),
        Some(Value::Null) => meta.expires_at = None,
        _ => {}
    }
}

fn render(title: &str, args: &Value, body: &str) -> String {
    let mut meta = PageMeta::default();
    apply_metadata(&mut meta, args);
    render_meta(Some(title), &meta, body)
}

pub(super) fn render_meta(title: Option<&str>, meta: &PageMeta, body: &str) -> String {
    let mut out = String::from("---\n");
    if let Some(title) = title {
        let _ = writeln!(out, "title: {}", json_scalar(title));
    }
    if !meta.tags.is_empty() {
        let tags: Vec<String> = meta.tags.iter().map(|tag| json_scalar(tag)).collect();
        let _ = writeln!(out, "tags: [{}]", tags.join(", "));
    }
    if meta.pinned {
        out.push_str("pinned: true\n");
    }
    if !meta.relations.is_empty() {
        out.push_str("relations:\n");
        for relation in &meta.relations {
            let _ = writeln!(
                out,
                "  - type: {}\n    target: {}",
                json_scalar(&relation.kind),
                json_scalar(&relation.target)
            );
        }
    }
    if let Some(expires_at) = &meta.expires_at {
        let _ = writeln!(out, "expires_at: {}", json_scalar(expires_at));
    }
    if out == "---\n" {
        return body.to_owned();
    }
    out.push_str("---\n");
    out.push_str(body);
    out
}

fn words(title: &str) -> BTreeSet<String> {
    title
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}

fn overlap(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f64 {
    let either = a.union(b).count();
    if either == 0 {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)]
    let ratio = a.intersection(b).count() as f64 / either as f64;
    ratio
}

fn similar_titles(dir: &Utf8Path, kind: PageKind, title: &str, skip: Option<&str>) -> Vec<Value> {
    let wanted = words(title);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".md") {
                return None;
            }
            let rel = format!("docs/memory/{}/{name}", kind.as_str());
            if skip == Some(rel.as_str()) {
                return None;
            }
            let page = parse_page(&rel, &std::fs::read_to_string(entry.path()).ok()?).ok()?;
            (overlap(&wanted, &words(&page.title)) >= SIMILAR_TITLE_THRESHOLD)
                .then(|| json!({ "path": rel, "title": page.title }))
        })
        .collect()
}
