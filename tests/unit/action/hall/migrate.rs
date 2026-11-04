use super::*;

#[test]
fn migrate_on_a_current_hall_reports_nothing_to_do_and_writes_nothing() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root.clone());
    init(&ctx, &fresh_input()).unwrap();
    let before = fs::read_text(&root.join("ivar.json")).unwrap().unwrap();

    let report = migrate(&ctx).unwrap();

    assert_eq!(report.value.plan, MigrationPlan::Current { version: 5 });
    assert!(!report.value.migrated);
    assert!(report.is_clean());
    assert!(human(&report.value).contains("Nothing to do"));
    assert_eq!(
        fs::read_text(&root.join("ivar.json")).unwrap().unwrap(),
        before,
        "a no-op migrate must not rewrite the file"
    );
}

#[test]
fn migrate_outside_a_hall_is_blocked() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root);

    let failure = migrate(&ctx).unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "hall.not_found");
}

#[test]
fn migrate_refuses_a_file_newer_than_this_build_without_touching_it() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root.clone());
    init(&ctx, &fresh_input()).unwrap();
    let on_disk = hall_at_version(&root, 99);

    let report = migrate(&ctx).unwrap();

    assert_eq!(
        report.value.plan,
        MigrationPlan::TooNew {
            found: 99,
            highest: 5
        }
    );
    assert!(!report.value.migrated);
    // The whole point of `plan` over `read`: a too-new hall gets described,
    // not refused into silence.
    assert!(human(&report.value).contains("understands up to 5"));
    // ...but describing it must not report success. A warning is what
    // makes `bin/ivar.rs` exit 1 instead of 0.
    assert!(!report.is_clean(), "a too-new hall must not exit clean");
    assert_eq!(report.warnings[0].code, "hall.manifest_too_new");
    assert!(
        report.warnings[0].what.contains("`ivar upgrade`"),
        "{}",
        report.warnings[0].what
    );
    assert_eq!(
        fs::read_text(&root.join("ivar.json")).unwrap().unwrap(),
        on_disk,
        "a too-new file must never be modified"
    );
}

#[test]
fn migrate_reports_an_unversioned_file_as_unreachable_rather_than_adopting_it() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root.clone());
    init(&ctx, &fresh_input()).unwrap();
    let on_disk = hall_at_version(&root, 0);

    let report = migrate(&ctx).unwrap();

    // `ivar.json`'s first public version is 1, so there is no v0 to migrate
    // from. Relabelling it as current would adopt a foreign file as ours —
    // the format contract forbids exactly that.
    assert_eq!(
        report.value.plan,
        MigrationPlan::Unreachable { from: 0, to: 5 }
    );
    assert!(!report.value.migrated);
    assert!(human(&report.value).contains("no migration to reach version 5"));
    assert!(
        !report.is_clean(),
        "an unreachable hall must not exit clean"
    );
    assert_eq!(report.warnings[0].code, "hall.manifest_unreachable");
    assert_eq!(
        fs::read_text(&root.join("ivar.json")).unwrap().unwrap(),
        on_disk
    );
}
