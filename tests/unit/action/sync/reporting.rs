use super::*;

// -- not in a hall ---------------------------------------------------------

#[test]
fn syncing_outside_a_hall_is_blocked_and_points_at_init() {
    let (_guard, root) = hall_root();
    let ctx = Ctx::new(root);

    let failure = sync(&ctx, &SyncInput::default()).unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "hall.not_found");
    assert_eq!(failure.fix_actions[0].command.as_deref(), Some("ivar init"));
}

#[test]
fn sync_works_from_a_subdirectory_of_the_hall() {
    let (_guard, root) = hall_with(&[]);
    let nested = root.join("deep/inside");
    fs::ensure_dir(&nested).unwrap();
    let ctx = Ctx::new(nested);

    let report = sync(&ctx, &SyncInput::default()).unwrap();

    assert_eq!(report.value.root, root);
}

// -- rendering -------------------------------------------------------------

#[test]
fn the_human_surface_groups_by_surface_and_ends_with_the_counts() {
    let outcome = SyncOutcome {
        root: Utf8PathBuf::from("/hall"),
        entries: vec![
            Entry::new("hall", ".ivar/", Change::Unchanged),
            Entry::new("repo api", "bare clone", Change::Created),
            Entry::new("repo api", "worktree main", Change::Failed).detail("branch not found"),
        ],
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Synced /hall\n\
             \n\
             hall:\n\
             \x20 = .ivar/\n\
             \n\
             repo api:\n\
             \x20 + bare clone\n\
             \x20 x worktree main — branch not found\n\
             \n\
             created: 1  updated: 0  removed: 0  unchanged: 1  failed: 1\n"
    );
}

#[test]
fn the_json_surface_carries_every_entry_and_its_change() {
    let outcome = SyncOutcome {
        root: Utf8PathBuf::from("/hall"),
        entries: vec![Entry::new("hall", ".ivar/", Change::Created)],
    };

    let json = serde_json::to_string(&Report::new(outcome)).unwrap();

    assert_eq!(
        json,
        r#"{"ok":true,"root":"/hall","entries":[{"surface":"hall","label":".ivar/","change":"created"}]}"#
    );
}

#[test]
fn listing_t3_adds_no_sync_entry_and_removing_it_strips_nothing() {
    let (_guard, root) = hall_with(&[]);
    let ctx = Ctx::new(root.clone());
    sync(&ctx, &SyncInput::default()).unwrap();
    let claude_commands = root.join(".claude/commands/ivar-review.md");
    let claude_skill = root.join(".claude/skills/ivar-execute/SKILL.md");
    assert!(fs::is_file(&claude_commands).unwrap());
    assert!(fs::is_file(&claude_skill).unwrap());

    crate::action::provider::add::add(
        &ctx,
        &crate::action::provider::add::AddInput {
            name: "t3".to_owned(),
        },
    )
    .unwrap();
    let listed = sync(&ctx, &SyncInput::default()).unwrap();
    assert!(
        listed
            .value
            .entries
            .iter()
            .all(|entry| entry.change != Change::Failed),
        "listing t3 must not make any sync step fail"
    );

    crate::action::provider::remove::remove(
        &ctx,
        &crate::action::provider::remove::RemoveInput {
            name: "t3".to_owned(),
            default: None,
        },
    )
    .unwrap();
    sync(&ctx, &SyncInput::default()).unwrap();
    assert!(
        fs::is_file(&claude_commands).unwrap(),
        "claude-code commands survive"
    );
    assert!(
        fs::is_file(&claude_skill).unwrap(),
        "claude-code skills survive"
    );
}
