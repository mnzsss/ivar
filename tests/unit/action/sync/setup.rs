use super::*;

#[test]
fn a_repos_setup_script_runs_in_its_worktree_with_the_ivar_environment() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    write_setup_script(
        &root,
        "api",
        "#!/usr/bin/env bash\nset -euo pipefail\n\
             printf '%s %s %s\\n' \"$IVAR_REPO\" \"$IVAR_BRANCH\" \"$IVAR_WORKTREE_KIND\" > .ivar-setup-ran\n",
    );
    let ctx = Ctx::new(root.clone());

    let report = sync(&ctx, &SyncInput::default()).unwrap();

    assert!(report.is_clean());
    assert_eq!(
        entry(&report.value, "repo api", "setup script").change,
        Change::Created
    );
    let evidence = root.join(".ivar/repos/api/main/.ivar-setup-ran");
    assert_eq!(
        std::fs::read_to_string(&evidence).unwrap(),
        "api main default\n"
    );
}

/// `sync` runs against the default worktree, where there is no feature —
/// so `IVAR_SECRETS_DIR` is set and `IVAR_FEATURE` deliberately is not.
#[test]
fn a_setup_script_gets_the_secrets_dir_and_no_feature_on_the_default_worktree() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    write_setup_script(
        &root,
        "api",
        "#!/usr/bin/env bash\nset -euo pipefail\n\
             printf '%s\\n%s\\n' \"$IVAR_SECRETS_DIR\" \"${IVAR_FEATURE:-<unset>}\" > .ivar-env\n",
    );
    let ctx = Ctx::new(root.clone());

    sync(&ctx, &SyncInput::default()).unwrap();

    let evidence = std::fs::read_to_string(root.join(".ivar/repos/api/main/.ivar-env")).unwrap();
    let mut lines = evidence.lines();
    assert!(
        lines.next().unwrap().ends_with("/.ivar/secrets"),
        "was: {evidence}"
    );
    assert_eq!(lines.next().unwrap(), "<unset>");
}

/// The directory has to exist for a user to find it and drop a file in.
/// It is local and gitignored, so creating it promises a teammate nothing.
#[test]
fn sync_creates_the_secrets_dir() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    let ctx = Ctx::new(root.clone());

    sync(&ctx, &SyncInput::default()).unwrap();

    assert!(fs::is_dir(&root.join(".ivar/secrets")).unwrap());
}

#[test]
fn a_setup_script_does_not_run_twice_for_the_same_content() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    write_setup_script(
        &root,
        "api",
        "#!/usr/bin/env bash\nprintf x >> .ivar-setup-runs\n",
    );
    let ctx = Ctx::new(root.clone());
    sync(&ctx, &SyncInput::default()).unwrap();

    let report = sync(&ctx, &SyncInput::default()).unwrap();

    assert_eq!(
        entry(&report.value, "repo api", "setup script").change,
        Change::Unchanged
    );
    let runs = root.join(".ivar/repos/api/main/.ivar-setup-runs");
    assert_eq!(std::fs::read_to_string(&runs).unwrap(), "x");
}

#[test]
fn changing_the_script_makes_it_run_again() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    write_setup_script(
        &root,
        "api",
        "#!/usr/bin/env bash\nprintf x >> .ivar-setup-runs\n",
    );
    let ctx = Ctx::new(root.clone());
    sync(&ctx, &SyncInput::default()).unwrap();

    write_setup_script(
        &root,
        "api",
        "#!/usr/bin/env bash\nprintf y >> .ivar-setup-runs\n",
    );
    let report = sync(&ctx, &SyncInput::default()).unwrap();

    assert_eq!(
        entry(&report.value, "repo api", "setup script").change,
        Change::Updated
    );
    let runs = root.join(".ivar/repos/api/main/.ivar-setup-runs");
    assert_eq!(std::fs::read_to_string(&runs).unwrap(), "xy");
}

#[test]
fn force_setup_runs_an_unchanged_script_again() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    write_setup_script(
        &root,
        "api",
        "#!/usr/bin/env bash\nprintf x >> .ivar-setup-runs\n",
    );
    let ctx = Ctx::new(root.clone());
    sync(&ctx, &SyncInput::default()).unwrap();

    sync(&ctx, &SyncInput { force_setup: true }).unwrap();

    let runs = root.join(".ivar/repos/api/main/.ivar-setup-runs");
    assert_eq!(std::fs::read_to_string(&runs).unwrap(), "xx");
}

/// A failed setup that recorded "done" would leave every later sync
/// silently skipping the repair the user is waiting for.
#[test]
fn a_failing_setup_script_warns_and_is_retried_on_the_next_sync() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    write_setup_script(
        &root,
        "api",
        "#!/usr/bin/env bash\nprintf x >> .ivar-setup-runs\nexit 1\n",
    );
    let ctx = Ctx::new(root.clone());

    let first = sync(&ctx, &SyncInput::default()).unwrap();
    assert!(!first.is_clean());
    assert_eq!(
        entry(&first.value, "repo api", "setup script").change,
        Change::Failed
    );

    let second = sync(&ctx, &SyncInput::default()).unwrap();
    assert_eq!(
        entry(&second.value, "repo api", "setup script").change,
        Change::Failed
    );
    let runs = root.join(".ivar/repos/api/main/.ivar-setup-runs");
    assert_eq!(
        std::fs::read_to_string(&runs).unwrap(),
        "xx",
        "a failed setup must be retried, not remembered as done"
    );
}

#[test]
fn a_repo_with_no_setup_script_produces_no_setup_entry() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    let ctx = Ctx::new(root);

    let report = sync(&ctx, &SyncInput::default()).unwrap();

    assert!(
        !report
            .value
            .entries
            .iter()
            .any(|e| e.label == "setup script"),
        "expected no setup entry, got {:?}",
        report.value.entries
    );
}
