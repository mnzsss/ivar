use super::{PageKind, Relation};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    Wikilink,
    Markdown,
    Relation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    Slug(String),
    KindSlug(PageKind, String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub kind: LinkKind,
    pub target: LinkTarget,
    pub relation_type: String,
}

#[must_use]
pub fn relation_target(target: &str) -> Option<LinkTarget> {
    let (kind_str, slug) = target.split_once('/')?;
    if slug.is_empty() || slug.contains('/') {
        return None;
    }
    let kind = PageKind::parse(kind_str)?;
    Some(LinkTarget::KindSlug(kind, slug.to_owned()))
}

#[must_use]
pub fn relation_links(relations: &[Relation]) -> Vec<Link> {
    relations
        .iter()
        .filter_map(|r| {
            let target = relation_target(&r.target)?;
            Some(Link {
                kind: LinkKind::Relation,
                target,
                relation_type: r.kind.clone(),
            })
        })
        .collect()
}

fn strip_inline_code(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_code = false;
    for ch in line.chars() {
        if ch == '`' {
            in_code = !in_code;
            out.push(' ');
        } else if in_code {
            out.push(' ');
        } else {
            out.push(ch);
        }
    }
    out
}

fn parse_wikilink_target(inner: &str) -> Option<LinkTarget> {
    let inner = inner.trim();
    if inner.is_empty() {
        return None;
    }
    if let Some((kind_str, slug)) = inner.split_once('/')
        && let Some(kind) = PageKind::parse(kind_str)
    {
        return Some(LinkTarget::KindSlug(kind, slug.to_owned()));
    }
    Some(LinkTarget::Slug(inner.to_owned()))
}

fn parse_markdown_target(raw_target: &str) -> Option<LinkTarget> {
    let target = raw_target.split('#').next().unwrap_or("").trim();
    if target.contains("://") {
        return None;
    }
    let without_ext = target.strip_suffix(".md")?;
    let mut segments = without_ext.rsplit('/');
    let slug = segments.next()?;
    let kind_str = segments.next()?;
    if slug.is_empty() {
        return None;
    }
    let kind = PageKind::parse(kind_str)?;
    Some(LinkTarget::KindSlug(kind, slug.to_owned()))
}

fn scan_line(line: &str, links: &mut Vec<Link>) {
    let mut rem = line;
    while let Some(start) = rem.find("[[") {
        rem = &rem[start + 2..];
        if let Some(end) = rem.find("]]") {
            let target_str = &rem[..end];
            if let Some(target) = parse_wikilink_target(target_str) {
                links.push(Link {
                    kind: LinkKind::Wikilink,
                    target,
                    relation_type: String::new(),
                });
            }
            rem = &rem[end + 2..];
        } else {
            break;
        }
    }

    let mut rem = line;
    while let Some(start) = rem.find("](") {
        rem = &rem[start + 2..];
        if let Some(end) = rem.find(')') {
            let target_str = &rem[..end];
            if let Some(target) = parse_markdown_target(target_str) {
                links.push(Link {
                    kind: LinkKind::Markdown,
                    target,
                    relation_type: String::new(),
                });
            }
            rem = &rem[end + 1..];
        } else {
            break;
        }
    }
}

#[must_use]
pub fn extract_links(body: &str) -> Vec<Link> {
    let mut links = Vec::new();
    let mut fenced = false;
    for line in body.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if !fenced {
            scan_line(&strip_inline_code(line), &mut links);
        }
    }
    links
}

#[cfg(test)]
#[path = "../../../tests/unit/domain/memory_page_links.rs"]
mod tests;
