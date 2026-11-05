use serde::Deserialize;

pub const DURABLE_ROOT: &str = "docs/memory/";
pub const EPHEMERAL_ROOT: &str = ".ivar/memory/";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageKind {
    Product,
    Update,
    Decision,
    Gotcha,
    Procedure,
    Concept,
    Session,
}

impl PageKind {
    pub const DURABLE: [PageKind; 6] = [
        PageKind::Product,
        PageKind::Update,
        PageKind::Decision,
        PageKind::Gotcha,
        PageKind::Procedure,
        PageKind::Concept,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Product => "product",
            Self::Update => "update",
            Self::Decision => "decision",
            Self::Gotcha => "gotcha",
            Self::Procedure => "procedure",
            Self::Concept => "concept",
            Self::Session => "session",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "product" => Some(Self::Product),
            "update" => Some(Self::Update),
            "decision" => Some(Self::Decision),
            "gotcha" => Some(Self::Gotcha),
            "procedure" => Some(Self::Procedure),
            "concept" => Some(Self::Concept),
            "session" => Some(Self::Session),
            _ => None,
        }
    }

    #[must_use]
    pub const fn is_durable(self) -> bool {
        !matches!(self, Self::Session)
    }

    #[must_use]
    pub const fn numbered(self) -> bool {
        matches!(self, Self::Product | Self::Update)
    }

    #[must_use]
    pub const fn tier(self) -> Tier {
        match self {
            Self::Product | Self::Decision | Self::Concept => Tier::Semantic,
            Self::Gotcha | Self::Procedure => Tier::Procedural,
            Self::Update | Self::Session => Tier::Episodic,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Semantic,
    Procedural,
    Episodic,
    Working,
}

impl Tier {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Semantic => "semantic",
            Self::Procedural => "procedural",
            Self::Episodic => "episodic",
            Self::Working => "working",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "semantic" => Some(Self::Semantic),
            "procedural" => Some(Self::Procedural),
            "episodic" => Some(Self::Episodic),
            "working" => Some(Self::Working),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Relation {
    #[serde(rename = "type")]
    pub kind: String,
    pub target: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct PageMeta {
    pub title: Option<String>,
    pub kind: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub relations: Vec<Relation>,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub path: String,
    pub kind: PageKind,
    pub slug: String,
    pub title: String,
    pub meta: PageMeta,
    pub body: String,
    pub unknown_fields: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageIssueCode {
    KindUnknown,
    KindMismatch,
    FrontmatterInvalid,
    ExpiresAtInvalid,
    Misplaced,
    FieldUnknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageIssue {
    pub code: PageIssueCode,
    pub message: String,
}

impl PageIssue {
    #[must_use]
    pub const fn is_error(&self) -> bool {
        !matches!(self.code, PageIssueCode::FieldUnknown)
    }
}

#[derive(Debug, Clone)]
pub struct ParsedFrontmatter {
    pub meta: PageMeta,
    pub unknown_fields: Vec<String>,
}

impl Page {
    #[must_use]
    pub fn warnings(&self) -> Vec<PageIssue> {
        self.unknown_fields
            .iter()
            .map(|key| PageIssue {
                code: PageIssueCode::FieldUnknown,
                message: format!("unknown frontmatter field `{key}`"),
            })
            .collect()
    }
}

fn issue(code: PageIssueCode, message: impl Into<String>) -> PageIssue {
    PageIssue {
        code,
        message: message.into(),
    }
}

fn is_kebab(slug: &str) -> bool {
    !slug.is_empty()
        && slug.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

fn has_number_prefix(slug: &str) -> bool {
    slug.split_once('-').is_some_and(|(number, rest)| {
        number.len() == 3 && number.bytes().all(|b| b.is_ascii_digit()) && !rest.is_empty()
    })
}

fn locate(rel_path: &str) -> Result<(PageKind, String), PageIssue> {
    let (root, rest, durable_root) = if let Some(rest) = rel_path.strip_prefix(DURABLE_ROOT) {
        (DURABLE_ROOT, rest, true)
    } else if let Some(rest) = rel_path.strip_prefix(EPHEMERAL_ROOT) {
        (EPHEMERAL_ROOT, rest, false)
    } else {
        return Err(issue(
            PageIssueCode::Misplaced,
            format!("`{rel_path}` is outside `docs/memory/` and `.ivar/memory/`"),
        ));
    };
    let (dir, file) = rest.split_once('/').ok_or_else(|| {
        issue(
            PageIssueCode::Misplaced,
            format!("`{rel_path}` has no kind directory"),
        )
    })?;
    let kind = PageKind::parse(dir)
        .filter(|kind| kind.is_durable() == durable_root)
        .ok_or_else(|| {
            issue(
                PageIssueCode::KindUnknown,
                format!("`{root}{dir}` is not a page kind of this root"),
            )
        })?;
    if file.contains('/') {
        return Err(issue(
            PageIssueCode::Misplaced,
            format!("`{rel_path}` is nested; pages are flat"),
        ));
    }
    let Some(slug) = file.strip_suffix(".md") else {
        return Err(issue(
            PageIssueCode::Misplaced,
            format!("`{rel_path}` must end in `.md`"),
        ));
    };
    if !is_kebab(slug) || (kind.numbered() && !has_number_prefix(slug)) {
        return Err(issue(
            PageIssueCode::Misplaced,
            format!(
                "`{rel_path}` needs a kebab-case slug{}",
                if kind.numbered() {
                    " with an `NNN-` prefix"
                } else {
                    ""
                }
            ),
        ));
    }
    Ok((kind, slug.to_owned()))
}

fn kind_mismatch(meta: &PageMeta, kind: PageKind) -> Option<PageIssue> {
    let declared = meta.kind.as_deref()?;
    (declared != kind.as_str()).then(|| {
        issue(
            PageIssueCode::KindMismatch,
            format!(
                "`kind: {declared}` differs from the directory `{}`",
                kind.as_str()
            ),
        )
    })
}

fn is_valid_rfc3339_utc(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 20 {
        return false;
    }
    if b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
        || b[19] != b'Z'
    {
        return false;
    }
    let is_num = |idx: usize| b[idx].is_ascii_digit();
    if !is_num(0) || !is_num(1) || !is_num(2) || !is_num(3) {
        return false;
    }
    if !is_num(5) || !is_num(6) || !is_num(8) || !is_num(9) {
        return false;
    }
    if !is_num(11) || !is_num(12) || !is_num(14) || !is_num(15) || !is_num(17) || !is_num(18) {
        return false;
    }
    let parse_u8 = |start: usize| -> u8 { (b[start] - b'0') * 10 + (b[start + 1] - b'0') };
    let month = parse_u8(5);
    let day = parse_u8(8);
    let hour = parse_u8(11);
    let min = parse_u8(14);
    let sec = parse_u8(17);
    (1..=12).contains(&month) && (1..=31).contains(&day) && hour <= 23 && min <= 59 && sec <= 60
}

fn validate_expires_at(meta: &PageMeta) -> Option<PageIssue> {
    let expires = meta.expires_at.as_deref()?;
    (!is_valid_rfc3339_utc(expires)).then(|| {
        issue(
            PageIssueCode::ExpiresAtInvalid,
            format!("`expires_at: {expires}` is not a valid UTC RFC 3339 timestamp (YYYY-MM-DDTHH:MM:SSZ)"),
        )
    })
}

fn extract_h1(body: &str) -> Option<String> {
    for line in body.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix('#') {
            if !rest.starts_with('#') {
                let heading = rest.trim();
                if !heading.is_empty() {
                    return Some(heading.to_owned());
                }
            }
        }
    }
    None
}

pub fn build_page(
    rel_path: &str,
    front: Result<ParsedFrontmatter, String>,
    body: &str,
) -> Result<Page, Vec<PageIssue>> {
    let mut issues = Vec::new();

    let located = match locate(rel_path) {
        Ok(loc) => Some(loc),
        Err(err) => {
            issues.push(err);
            None
        }
    };

    let parsed_front = match front {
        Ok(pf) => Some(pf),
        Err(err) => {
            issues.push(issue(PageIssueCode::FrontmatterInvalid, err));
            None
        }
    };

    if let (Some((kind, _)), Some(pf)) = (&located, &parsed_front) {
        if let Some(mismatch) = kind_mismatch(&pf.meta, *kind) {
            issues.push(mismatch);
        }
    }

    if let Some(pf) = &parsed_front {
        if let Some(err) = validate_expires_at(&pf.meta) {
            issues.push(err);
        }
    }

    if !issues.is_empty() {
        return Err(issues);
    }

    let (kind, slug) = located.unwrap();
    let ParsedFrontmatter {
        meta,
        unknown_fields,
    } = parsed_front.unwrap();

    let title = meta
        .title
        .as_ref()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .or_else(|| extract_h1(body))
        .unwrap_or_else(|| slug.clone());

    Ok(Page {
        path: rel_path.to_owned(),
        kind,
        slug,
        title,
        meta,
        body: body.to_owned(),
        unknown_fields,
    })
}

#[cfg(test)]
#[path = "../../tests/unit/domain/memory_page.rs"]
mod tests;
