//! Read-only checks on the memory pages under `docs/memory/` and `.ivar/memory/`.

use crate::action::memory::reindex::{collect, read_state, walk_pages};
use crate::domain::memory_page::PageIssueCode;
use crate::git::exec::changed_paths;
use crate::store::layout::Layout;
use crate::store::memory::page_source::validate_page;

use camino::Utf8Path;

use super::Diagnosis;

const LEGACY_FOLDERS: [(&str, &str); 2] = [
    ("docs/product", "docs/memory/product"),
    ("docs/updates", "docs/memory/update"),
];
/// Regions `ivar feature cleanup` used to maintain in `HALL.md` before memory pages.
const LEGACY_HALL_REGIONS: [&str; 2] = ["ivar:product-docs", "ivar:updates"];
const UNCOMMITTED_EXAMPLES: usize = 5;

pub(super) fn page_diagnoses(layout: &Layout, hall_is_git: bool) -> Vec<Diagnosis> {
    let mut findings = Vec::new();
    findings.extend(legacy_docs(layout, hall_is_git));
    findings.extend(legacy_hall_index(layout));
    findings.extend(uncommitted(layout));
    findings.extend(index_failed(layout));
    findings.extend(page_issues(layout));
    findings
}

const fn doctor_code(issue: PageIssueCode) -> &'static str {
    match issue {
        PageIssueCode::KindUnknown => "memory.page_kind_unknown",
        PageIssueCode::KindMismatch
        | PageIssueCode::FrontmatterInvalid
        | PageIssueCode::ExpiresAtInvalid => "memory.page_invalid",
        PageIssueCode::Misplaced => "memory.page_misplaced",
        PageIssueCode::FieldUnknown => "memory.page_field_unknown",
    }
}

const fn issue_fix(issue: PageIssueCode) -> &'static str {
    match issue {
        PageIssueCode::KindUnknown => "Move the page into a known kind directory.",
        PageIssueCode::KindMismatch => "Make the `kind` field match the directory or remove it.",
        PageIssueCode::FrontmatterInvalid => "Fix the frontmatter so it parses.",
        PageIssueCode::ExpiresAtInvalid => "Set `expires_at` to an RFC 3339 timestamp.",
        PageIssueCode::Misplaced => {
            "Rename the file to a kebab-case slug directly under its kind directory."
        }
        PageIssueCode::FieldUnknown => "Remove the field or rename it to a supported one.",
    }
}

fn page_issues(layout: &Layout) -> Vec<Diagnosis> {
    walk_pages(layout)
        .into_iter()
        .filter_map(|file| {
            let source = std::fs::read_to_string(&file.path).ok()?;
            Some((file.rel.clone(), validate_page(&file.rel, &source)))
        })
        .flat_map(|(rel, issues)| {
            issues.into_iter().map(move |issue| Diagnosis {
                code: doctor_code(issue.code),
                what: format!("`{rel}`: {}", issue.message),
                fix: issue_fix(issue.code).to_owned(),
            })
        })
        .collect()
}

/// A legacy folder only counts when it still holds a Markdown page at any depth.
fn holds_pages(dir: &Utf8Path) -> bool {
    let mut found = Vec::new();
    collect(dir, &mut found);
    !found.is_empty()
}

fn legacy_docs(layout: &Layout, hall_is_git: bool) -> Option<Diagnosis> {
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
    (!moves.is_empty()).then(|| Diagnosis {
        code: "memory.legacy_docs",
        what: "pages still live in docs/product or docs/updates".to_owned(),
        fix: format!("mkdir -p docs/memory && {}", moves.join(" && ")),
    })
}

/// `HALL.md` still carrying a region ivar no longer writes or reads. It reaches
/// every session through the generated root instructions file.
fn legacy_hall_index(layout: &Layout) -> Option<Diagnosis> {
    let text = std::fs::read_to_string(layout.hall_instructions()).ok()?;
    let found: Vec<&str> = LEGACY_HALL_REGIONS
        .into_iter()
        .filter(|region| text.contains(&format!("<!-- {region}:start -->")))
        .collect();
    (!found.is_empty()).then(|| Diagnosis {
        code: "memory.legacy_hall_index",
        what: format!(
            "HALL.md still has the dead {} region(s), which ivar no longer reads or writes",
            found
                .iter()
                .map(|region| format!("`{region}`"))
                .collect::<Vec<_>>()
                .join(" and ")
        ),
        fix: "Delete the `ivar:product-docs` and `ivar:updates` regions from HALL.md, markers \
              included; find the pages with `memory_query`."
            .to_owned(),
    })
}

fn uncommitted(layout: &Layout) -> Option<Diagnosis> {
    let changed: Vec<_> = changed_paths(layout.root())
        .ok()?
        .into_iter()
        .filter(|path| path.starts_with("docs/memory"))
        .collect();
    (!changed.is_empty()).then(|| Diagnosis {
        code: "memory.page_uncommitted",
        what: format!(
            "{} page file(s) not committed, e.g. {}",
            changed.len(),
            changed
                .iter()
                .take(UNCOMMITTED_EXAMPLES)
                .map(|path| path.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        fix: "Commit them with the commit skill.".to_owned(),
    })
}

fn index_failed(layout: &Layout) -> Option<Diagnosis> {
    read_state(layout).last_error.map(|error| Diagnosis {
        code: "memory.index_failed",
        what: format!("the last page reindex failed: {error}"),
        fix: "Run `ivar memory reindex`.".to_owned(),
    })
}
