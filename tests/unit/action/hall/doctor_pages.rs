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

#[test]
fn legacy_doc_folders_are_named_with_the_git_mv_commands() {
    let (_guard, root) = seeded_hall();
    put(&root, "docs/updates/001-x.md", "# X\n");
    let report = doctor(&Ctx::new(root.clone())).unwrap().value;
    let found = finding(&report, "memory.legacy_docs");
    assert!(
        found.fix.contains("git mv docs/updates docs/memory/update"),
        "{}",
        found.fix
    );
}

#[test]
fn an_existing_destination_folder_gets_a_merging_move_not_a_nesting_one() {
    let (_guard, root) = seeded_hall();
    put(&root, "docs/updates/001-x.md", "# X\n");
    put(&root, "docs/memory/update/002-y.md", "# Y\n");
    let report = doctor(&Ctx::new(root.clone())).unwrap().value;
    let found = finding(&report, "memory.legacy_docs");
    assert!(
        found
            .fix
            .contains("git mv docs/updates/* docs/memory/update/"),
        "{}",
        found.fix
    );
    assert!(
        !found.fix.contains("git mv docs/updates docs/memory/update"),
        "{}",
        found.fix
    );
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
