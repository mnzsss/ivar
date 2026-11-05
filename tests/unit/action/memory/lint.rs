#![allow(clippy::unwrap_used)]

use super::*;
use crate::store::layout::Layout;
use crate::test_support::seeded_hall;

fn page(layout: &Layout, rel: &str, text: &str) {
    let path = layout.root().join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn codes(layout: &Layout) -> Vec<&'static str> {
    lint_in(layout)
        .into_iter()
        .map(|issue| issue.code)
        .collect()
}

#[test]
fn a_clean_hall_has_no_issues() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    page(
        &layout,
        "docs/memory/decision/a.md",
        "# A\n\nsee [b](../gotcha/b.md)\n",
    );
    page(&layout, "docs/memory/gotcha/b.md", "# B\n");
    assert!(lint_in(&layout).is_empty());
}

#[test]
fn a_markdown_link_to_a_missing_page_is_an_error_but_links_outside_memory_are_ignored() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    page(
        &layout,
        "docs/memory/decision/a.md",
        "# A\n[gone](../gotcha/gone.md) [code](../../../src/lib.rs) [ticket](../../wayfinder/x/tickets/01.md)\n",
    );
    assert_eq!(codes(&layout), ["lint.link_broken"]);
}

#[test]
fn a_same_directory_link_to_a_missing_page_is_an_error_and_a_fenced_example_is_not() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    page(
        &layout,
        "docs/memory/decision/a.md",
        "# A\n[gone](gone.md) [ok](a.md)\n```\n[example](nowhere.md)\n```\n",
    );
    assert_eq!(codes(&layout), ["lint.link_broken"]);
}

#[test]
fn an_unresolved_wikilink_and_relation_are_not_reported() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    page(
        &layout,
        "docs/memory/decision/a.md",
        "---\nrelations:\n  - type: supersedes\n    target: decision/not-yet\n---\nsee [[stub]]\n",
    );
    assert!(lint_in(&layout).is_empty());
}

#[test]
fn an_unknown_frontmatter_field_is_an_error_here() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    page(
        &layout,
        "docs/memory/decision/a.md",
        "---\ntitle: A\nconfidence: high\n---\nbody\n",
    );
    assert_eq!(codes(&layout), ["lint.field_unknown"]);
}

#[test]
fn kind_frontmatter_and_placement_problems_map_to_their_codes() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    page(&layout, "docs/memory/mystery/a.md", "# A\n");
    page(
        &layout,
        "docs/memory/decision/b.md",
        "---\nkind: gotcha\n---\nx\n",
    );
    page(&layout, "docs/memory/decision/Bad Name.md", "# C\n");
    page(&layout, "docs/memory/update/no-number.md", "# D\n");
    let found = codes(&layout);
    for code in [
        "lint.kind_unknown",
        "lint.frontmatter_invalid",
        "lint.misplaced",
    ] {
        assert!(found.contains(&code), "{code} in {found:?}");
    }
}

#[test]
fn the_outcome_reports_errors_so_the_verb_can_exit_non_zero() {
    let clean = LintOutcome { issues: Vec::new() };
    let broken = LintOutcome {
        issues: vec![LintIssue {
            path: "p".into(),
            code: "lint.link_broken",
            severity: "error",
            message: "m".into(),
        }],
    };
    assert!(!clean.has_errors());
    assert!(broken.has_errors());
}

#[test]
fn lint_reads_the_disk_not_the_index() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    page(
        &layout,
        "docs/memory/decision/a.md",
        "---\ntitle: A\nconfidence: high\n---\n",
    );
    assert_eq!(codes(&layout).len(), 1);
    std::fs::remove_file(layout.root().join("docs/memory/decision/a.md")).unwrap();
    assert!(lint_in(&layout).is_empty());
}
