#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::test_support::hall_root;

fn conflict(path: &str, ours: Option<&str>, theirs: Option<&str>) -> Conflict {
    Conflict {
        path: Utf8PathBuf::from(path),
        ours: ours.map(str::to_owned),
        theirs: theirs.map(str::to_owned),
    }
}

#[test]
fn dated_run_files_are_the_ones_under_reports_and_brand_metricas() {
    for covered in [
        "reports/memory-health/2026-11-09.md",
        "reports/memory-health/2026-11-09.json",
        "reports/a/b/c/2026-11-09-extra.md",
        "reports/2026-11-09.md",
        "brand/metricas/2026-11-09.json",
        "brand/metricas/2026-11-09-weekly.md",
    ] {
        assert!(is_dated_run_file(Utf8Path::new(covered)), "{covered}");
    }
    for not_covered in [
        "reports/memory-health/summary.md",
        "reports/memory-health/26-11-09.md",
        "reports/memory-health/2026_11_09.md",
        "docs/2026-11-09.md",
        "brand/2026-11-09.md",
        "brand/metricas/nested/2026-11-09.md",
        "brand/metricas",
        "reportsx/2026-11-09.md",
    ] {
        assert!(
            !is_dated_run_file(Utf8Path::new(not_covered)),
            "{not_covered}"
        );
    }
}

#[test]
fn a_rule_needs_the_versions_it_keeps() {
    assert_eq!(
        rule_for(&conflict(SKILLS_STATE, Some("a"), Some("b"))),
        Some(Rule::KeepRemote)
    );
    assert_eq!(rule_for(&conflict(SKILLS_STATE, None, Some("b"))), None);
    assert_eq!(
        rule_for(&conflict("reports/x/2026-11-09.md", Some("a"), Some("b"))),
        Some(Rule::KeepBoth)
    );
    assert_eq!(
        rule_for(&conflict("reports/x/2026-11-09.md", Some("a"), None)),
        None
    );
    assert_eq!(
        rule_for(&conflict("docs/prose.md", Some("a"), Some("b"))),
        None
    );
}

#[test]
fn the_hostname_is_always_safe_in_a_file_name() {
    let host = hostname();

    assert!(!host.is_empty());
    assert!(
        host.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')),
        "{host}"
    );
}

#[test]
fn the_saved_copy_sits_beside_the_original_and_never_takes_a_name_in_use() {
    let (_guard, root) = hall_root();
    let original = Utf8Path::new("reports/m/2026-11-09.md");

    let first = free_name(&root, original).unwrap();
    assert_eq!(first.parent(), Some(Utf8Path::new("reports/m")));
    assert_eq!(first.extension(), Some("md"));
    assert!(first.file_stem().unwrap().starts_with("2026-11-09-"));

    std::fs::create_dir_all(root.join("reports/m")).unwrap();
    std::fs::write(root.join(&first), "taken").unwrap();
    let second = free_name(&root, original).unwrap();
    assert_ne!(first, second);
    assert!(is_dated_run_file(&second));
}

#[test]
fn a_path_outside_the_rules_leaves_everything_untouched() {
    let (_guard, root) = hall_root();
    let conflicts = [
        conflict(SKILLS_STATE, Some("a"), Some("b")),
        conflict("docs/prose.md", Some("a"), Some("b")),
    ];

    // `git` is never reached: the check comes first, so no repository is needed.
    let resolution = resolve_conflicts(&crate::git::System, &root, &conflicts).unwrap();

    assert_eq!(
        resolution,
        Resolution::Unresolved(vec![Utf8PathBuf::from("docs/prose.md")])
    );
}
