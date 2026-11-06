use super::*;

fn seeded_hall() -> (tempfile::TempDir, Utf8PathBuf) {
    hall_with_repo()
}

fn put(root: &Utf8Path, rel: &str, text: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn codes(root: &Utf8Path) -> Vec<&'static str> {
    doctor(&Ctx::new(root.to_path_buf()))
        .unwrap()
        .value
        .findings
        .into_iter()
        .map(|f| f.code)
        .filter(|code| {
            code.starts_with("memory.page")
                || *code == "memory.legacy_docs"
                || *code == "memory.index_failed"
        })
        .collect()
}

#[test]
fn a_hall_without_pages_has_no_page_findings() {
    let (_guard, root) = seeded_hall();
    assert!(codes(&root).is_empty());
}

fn git_init(root: &Utf8Path) {
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    );
}

#[rstest]
#[case::git(true, "mkdir -p docs/memory && git mv docs/updates docs/memory/update")]
#[case::plain(false, "mkdir -p docs/memory && mv docs/updates docs/memory/update")]
fn legacy_doc_folders_are_moved_with_the_hall_s_own_tooling(
    #[case] hall_is_git: bool,
    #[case] expected: &str,
) {
    let (_guard, root) = seeded_hall();
    if hall_is_git {
        git_init(&root);
    }
    put(&root, "docs/updates/001-x.md", "# X\n");
    let report = doctor(&Ctx::new(root.clone())).unwrap().value;
    let found = finding(&report, "memory.legacy_docs");
    assert_eq!(found.fix, expected);
    assert_eq!(found.fix.contains("git"), hall_is_git, "{}", found.fix);
}

#[rstest]
#[case::git(
    true,
    "mkdir -p docs/memory && git mv docs/updates/* docs/memory/update/"
)]
#[case::plain(
    false,
    "mkdir -p docs/memory && mv -n docs/updates/* docs/memory/update/"
)]
fn an_existing_destination_folder_gets_a_merging_move_not_a_nesting_one(
    #[case] hall_is_git: bool,
    #[case] expected: &str,
) {
    let (_guard, root) = seeded_hall();
    if hall_is_git {
        git_init(&root);
    }
    put(&root, "docs/updates/001-x.md", "# X\n");
    put(&root, "docs/memory/update/002-y.md", "# Y\n");
    let report = doctor(&Ctx::new(root.clone())).unwrap().value;
    let found = finding(&report, "memory.legacy_docs");
    assert_eq!(found.fix, expected);
    assert_eq!(found.fix.contains("git"), hall_is_git, "{}", found.fix);
}

fn has_legacy_finding(root: &Utf8Path) -> bool {
    codes(root).contains(&"memory.legacy_docs")
}

fn run_fix(root: &Utf8Path, fix: &str) {
    let status = std::process::Command::new("sh")
        .args(["-c", fix])
        .current_dir(root)
        .status()
        .unwrap();
    assert!(status.success(), "{fix}");
}

fn git_add_all(root: &Utf8Path) {
    assert!(
        std::process::Command::new("git")
            .args(["add", "-A"])
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    );
}

#[rstest]
#[case::empty_updates(&[], &["docs/updates"])]
#[case::empty_product(&[], &["docs/product"])]
#[case::only_non_markdown(&["docs/updates/notes.txt", "docs/product/img/a.png"], &[])]
fn a_legacy_folder_without_pages_is_not_reported(#[case] files: &[&str], #[case] dirs: &[&str]) {
    let (_guard, root) = seeded_hall();
    for dir in dirs {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    for file in files {
        put(&root, file, "x\n");
    }
    assert!(!has_legacy_finding(&root));
}

#[rstest]
#[case::nested("docs/updates/2026/001-x.md")]
#[case::upper_case("docs/updates/001-x.MD")]
fn a_nested_or_upper_case_page_is_still_reported(#[case] rel: &str) {
    let (_guard, root) = seeded_hall();
    put(&root, rel, "# X\n");
    assert!(has_legacy_finding(&root));
}

#[rstest]
#[case::git(true)]
#[case::plain(false)]
fn running_the_fix_moves_the_pages_and_clears_the_finding(
    #[case] hall_is_git: bool,
    #[values(false, true)] destination_exists: bool,
) {
    let (_guard, root) = seeded_hall();
    if hall_is_git {
        git_init(&root);
    }
    put(&root, "docs/updates/001-x.md", "# X\n");
    if destination_exists {
        put(&root, "docs/memory/update/002-y.md", "# Y\n");
    }
    if hall_is_git {
        git_add_all(&root);
    }
    let report = doctor(&Ctx::new(root.clone())).unwrap().value;
    run_fix(&root, &finding(&report, "memory.legacy_docs").fix);
    assert!(root.join("docs/memory/update/001-x.md").is_file());
    assert!(!has_legacy_finding(&root));
}

#[test]
fn the_plain_fix_never_overwrites_a_page_already_at_the_destination() {
    let (_guard, root) = seeded_hall();
    put(&root, "docs/updates/001-x.md", "# legacy\n");
    put(&root, "docs/updates/003-z.md", "# Z\n");
    put(&root, "docs/memory/update/001-x.md", "# kept\n");
    let report = doctor(&Ctx::new(root.clone())).unwrap().value;
    run_fix(&root, &finding(&report, "memory.legacy_docs").fix);
    let kept = std::fs::read_to_string(root.join("docs/memory/update/001-x.md")).unwrap();
    assert_eq!(kept, "# kept\n");
    assert!(root.join("docs/memory/update/003-z.md").is_file());
}

#[test]
fn each_page_problem_has_its_own_code() {
    let (_guard, root) = seeded_hall();
    put(&root, "docs/memory/rule/x.md", "# x\n");
    put(
        &root,
        "docs/memory/gotcha/a.md",
        "---\nkind: decision\n---\n# a\n",
    );
    put(&root, "docs/memory/gotcha/Bad_Slug.md", "# b\n");
    put(
        &root,
        "docs/memory/concept/c.md",
        "---\ntier: semantic\n---\n# c\n",
    );
    let found = codes(&root);
    for code in [
        "memory.page_kind_unknown",
        "memory.page_invalid",
        "memory.page_misplaced",
        "memory.page_field_unknown",
    ] {
        assert!(found.contains(&code), "{code} missing from {found:?}");
    }
}

#[test]
fn page_findings_leave_the_warnings_empty_so_the_exit_code_is_unchanged() {
    let (_guard, root) = seeded_hall();
    put(&root, "docs/memory/rule/x.md", "# x\n");
    let outcome = doctor(&Ctx::new(root)).unwrap();
    assert!(
        outcome
            .value
            .findings
            .iter()
            .any(|f| f.code == "memory.page_kind_unknown")
    );
    assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
}

#[test]
fn a_failed_reindex_is_reported_with_the_fix() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    crate::action::memory::reindex::write_state(
        &layout,
        &crate::action::memory::reindex::ReindexState {
            last_error: Some("database is locked".into()),
            ..Default::default()
        },
    );
    let report = doctor(&Ctx::new(root)).unwrap().value;
    let found = finding(&report, "memory.index_failed");
    assert!(found.fix.contains("ivar memory reindex"), "{}", found.fix);
}

#[test]
fn uncommitted_pages_are_one_finding_that_clears_after_a_commit() {
    let (_guard, root) = seeded_hall();
    let git = |args: &[&str]| {
        assert!(
            std::process::Command::new("git")
                .args(["-c", "user.name=t", "-c", "user.email=t@t"])
                .args(args)
                .current_dir(&root)
                .status()
                .unwrap()
                .success()
        );
    };
    git(&["init", "-q"]);
    for n in 0..7 {
        put(&root, &format!("docs/memory/gotcha/page-{n}.md"), "# p\n");
    }
    let report = doctor(&Ctx::new(root.clone())).unwrap().value;
    let uncommitted: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.code == "memory.page_uncommitted")
        .collect();
    assert_eq!(uncommitted.len(), 1);
    assert!(
        uncommitted[0]
            .what
            .matches("docs/memory/gotcha/page-")
            .count()
            <= 5
    );
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "pages"]);
    assert!(!codes(&root).contains(&"memory.page_uncommitted"));
}
