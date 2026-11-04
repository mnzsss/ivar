use super::*;

#[test]
fn status_reports_a_fresh_hall_as_operational() {
    let (_guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());
    init(&ctx, &fresh_input()).unwrap();

    let report = status(&ctx).unwrap();

    assert_eq!(report.value.health, "operational");
    assert!(report.value.repos.is_empty());
}

#[test]
fn status_reports_a_synced_hall_with_repos_as_operational() {
    let (_guard, root) = hall_with_repo();
    let ctx = Ctx::new(root.clone());
    crate::action::sync::sync(&ctx, &Default::default()).unwrap();

    let report = status(&ctx).unwrap();

    assert_eq!(report.value.health, "operational");
    assert_eq!(report.value.repos.len(), 1);
    assert!(report.value.repos[0].bare_cloned);
    assert!(report.value.repos[0].worktree);
}

#[test]
fn status_reports_a_never_synced_repo_as_degraded() {
    let (_guard, root) = hall_with_repo();
    let ctx = Ctx::new(root);

    let report = status(&ctx).unwrap();

    assert_eq!(report.value.health, "degraded");
    assert!(!report.value.repos[0].bare_cloned);
}

#[test]
fn the_human_surface_of_status_names_the_health() {
    let outcome = StatusOutcome {
        root: Utf8PathBuf::from("/hall"),
        health: "operational",
        repos: vec![RepoStatusEntry {
            name: crate::domain::name::RepoName::new("api").unwrap(),
            bare_cloned: true,
            worktree: true,
        }],
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    let stripped = anstream::adapter::strip_str(&String::from_utf8(out).unwrap()).to_string();

    assert_eq!(
        stripped,
        "Hall at /hall — operational\nREPO  CLONE   WORKTREE\napi   cloned  worktree ok\n"
    );
}
