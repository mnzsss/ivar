use super::*;

#[test]
fn a_non_prompting_run_never_answers_yes() {
    // The safety property both `cleanup` and `migrate` rest on: a Ctx built
    // without an explicit confirmer (the test-suite case, and the `--json`,
    // `$CI`, and non-tty cases in the binary) never consents. A pipe is not
    // consent.
    let (_tmp, root) = utf8_temp_dir();
    let ctx = Ctx::new(root);
    assert!(!ctx.confirm("Delete everything?", None).unwrap());
    assert!(!ctx.confirm("Rewrite it?", Some("careful")).unwrap());
}

#[test]
fn cleanup_removes_when_the_confirmation_seam_says_yes() {
    // The seam, not the terminal, decides: a test that installs a fixed yes
    // gets the deletion half of the prompt deterministically.
    let (_guard, root) = utf8_temp_dir();
    let mut ctx = Ctx::new(root.clone());
    ctx = ctx.with_confirm(crate::action::confirm::fixed(true));
    init(&ctx, &fresh_input()).unwrap();

    // A stale repo directory — not in the manifest.
    fs::ensure_dir(&root.join(".ivar/repos/stale")).unwrap();

    let report = cleanup(&ctx).unwrap();

    assert_eq!(report.value.removed.len(), 1);
    assert!(report.value.removed[0].contains("stale"));
    assert!(!fs::exists(&root.join(".ivar/repos/stale")).unwrap());
}

#[test]
fn cleanup_forgets_a_removed_repo_in_the_graph() {
    let (_guard, root) = utf8_temp_dir();
    let mut ctx = Ctx::new(root.clone());
    ctx = ctx.with_confirm(crate::action::confirm::fixed(true));
    init(&ctx, &fresh_input()).unwrap();
    fs::ensure_dir(&root.join(".ivar/repos/stale")).unwrap();
    {
        let db = crate::store::graph::db::GraphDb::open(root.join(".ivar/memory.db").as_std_path())
            .unwrap();
        db.insert_repo("stale", "/x/stale", "main", None).unwrap();
        db.insert_repo("api", "/x/api", "main", None).unwrap();
    }

    let report = cleanup(&ctx).unwrap();

    assert_eq!(report.value.graph_forgotten, vec!["stale"]);
    let db = crate::store::graph::db::GraphDb::open_read_only(
        root.join(".ivar/memory.db").as_std_path(),
    )
    .unwrap();
    assert!(db.get_repo("stale").unwrap().is_none());
    assert!(db.get_repo("api").unwrap().is_some());
}

#[test]
fn cleanup_warns_when_the_graph_cannot_forget_a_removed_repo() {
    let (_guard, root) = utf8_temp_dir();
    let mut ctx = Ctx::new(root.clone());
    ctx = ctx.with_confirm(crate::action::confirm::fixed(true));
    init(&ctx, &fresh_input()).unwrap();
    fs::ensure_dir(&root.join(".ivar/repos/stale")).unwrap();
    std::fs::write(root.join(".ivar/memory.db"), b"not a sqlite database").unwrap();

    let report = cleanup(&ctx).unwrap();

    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.code == "graph.repo_not_forgotten" && w.subject == "stale"),
        "{:?}",
        report.warnings
    );
    assert!(report.value.graph_forgotten.is_empty());
    assert!(!fs::exists(&root.join(".ivar/repos/stale")).unwrap());
}

#[test]
fn cleanup_without_a_graph_database_still_removes_the_directory() {
    let (_guard, root) = utf8_temp_dir();
    let mut ctx = Ctx::new(root.clone());
    ctx = ctx.with_confirm(crate::action::confirm::fixed(true));
    init(&ctx, &fresh_input()).unwrap();
    fs::ensure_dir(&root.join(".ivar/repos/stale")).unwrap();

    let report = cleanup(&ctx).unwrap();

    assert_eq!(report.value.removed.len(), 1);
    assert!(report.value.graph_forgotten.is_empty());
}

// -- cleanup --------------------------------------------------------------

#[test]
fn cleanup_in_a_non_tty_run_keeps_everything() {
    let (_guard, root) = hall_with_repo();
    let ctx = Ctx::new(root.clone());
    // A repo dir for a repo that is no longer in the manifest.
    let stale = root.join(".ivar/repos/old");
    fs::ensure_dir(&stale).unwrap();

    let report = cleanup(&ctx).unwrap();

    // Non-tty: nothing is deleted without a human.
    assert!(report.value.removed.is_empty());
    assert_eq!(report.value.kept.len(), 1);
    assert!(fs::is_dir(&stale).unwrap());
}

#[test]
fn cleanup_leaves_repos_still_in_the_manifest_alone() {
    let (_guard, root) = hall_with_repo();
    let ctx = Ctx::new(root.clone());
    crate::action::sync::sync(&ctx, &Default::default()).unwrap();

    let report = cleanup(&ctx).unwrap();

    assert!(report.value.removed.is_empty());
    assert!(report.value.kept.is_empty());
    assert!(root.join(".ivar/repos/api/.bare/HEAD").is_file());
}
