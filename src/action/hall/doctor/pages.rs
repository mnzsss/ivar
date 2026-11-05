//! Read-only checks on the memory pages under `docs/memory/` and `.ivar/memory/`.

use crate::action::memory::reindex::{read_state, walk_pages};
use crate::domain::memory_page::PageIssueCode;
use crate::git::exec::changed_paths;
use crate::store::layout::Layout;
use crate::store::memory::page_source::validate_page;

use super::Diagnosis;

const LEGACY_FOLDERS: [(&str, &str); 2] = [
    ("docs/product", "docs/memory/product"),
    ("docs/updates", "docs/memory/update"),
];
const UNCOMMITTED_EXAMPLES: usize = 5;

pub(super) fn page_diagnoses(layout: &Layout) -> Vec<Diagnosis> {
    let mut findings = Vec::new();
    findings.extend(legacy_docs(layout));
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

fn legacy_docs(layout: &Layout) -> Option<Diagnosis> {
    let moves: Vec<String> = LEGACY_FOLDERS
        .into_iter()
        .filter(|(old, _)| layout.root().join(old).is_dir())
        .map(|(old, new)| format!("git mv {old} {new}"))
        .collect();
    (!moves.is_empty()).then(|| Diagnosis {
        code: "memory.legacy_docs",
        what: "pages still live in docs/product or docs/updates".to_owned(),
        fix: format!("mkdir -p docs/memory && {}", moves.join(" && ")),
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
