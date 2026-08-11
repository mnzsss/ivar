#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::test_support::utf8_temp_dir;

fn hall() -> HallName {
    HallName::new("acme").unwrap()
}

fn repo(name: &str) -> RepoName {
    RepoName::new(name).unwrap()
}

fn alias(provider: Provider, path: Utf8PathBuf, enabled: bool) -> Alias {
    Alias {
        provider,
        path,
        enabled,
    }
}

// -- build_block ----------------------------------------------------------

#[test]
fn the_block_is_delimited_by_the_markers_and_names_the_hall() {
    let block = build_block(&hall(), &[repo("api")]);

    assert!(block.starts_with(MANAGED_START));
    assert!(block.ends_with(MANAGED_END));
    assert!(block.contains("# acme"));
}

#[test]
fn repos_are_listed_in_the_order_given() {
    let block = build_block(&hall(), &[repo("web"), repo("api")]);

    let web = block.find("`web`").unwrap();
    let api = block.find("`api`").unwrap();
    assert!(web < api, "manifest order must survive into the block");
}

#[test]
fn a_hall_with_no_repos_says_how_to_add_one() {
    let block = build_block(&hall(), &[]);

    assert!(block.contains("ivar.json"));
    assert!(block.contains("ivar sync"));
}

/// [`materialise`] decides "unchanged" by comparing bytes, so the builder
/// has to be a function of its arguments and nothing else.
#[test]
fn building_the_same_block_twice_produces_identical_bytes() {
    let first = build_block(&hall(), &[repo("api")]);
    let second = build_block(&hall(), &[repo("api")]);

    assert_eq!(first, second);
}

// -- materialise: the three placement cases -------------------------------

#[test]
fn an_absent_file_is_created_holding_only_the_block() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("CLAUDE.md");
    let block = build_block(&hall(), &[repo("api")]);

    assert_eq!(materialise(&path, &block).unwrap(), Change::Created);

    assert_eq!(fs::read_text(&path).unwrap().unwrap(), format!("{block}\n"));
}

#[test]
fn an_existing_block_is_replaced_in_place_leaving_the_users_text_alone() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("CLAUDE.md");
    let first = build_block(&hall(), &[repo("api")]);
    fs::write_text(
        &path,
        &format!("# House rules\n\n{first}\n\nNever force-push.\n"),
    )
    .unwrap();

    let second = build_block(&hall(), &[repo("api"), repo("web")]);
    assert_eq!(materialise(&path, &second).unwrap(), Change::Updated);

    let content = fs::read_text(&path).unwrap().unwrap();
    assert!(content.starts_with("# House rules\n"));
    assert!(content.ends_with("Never force-push.\n"));
    assert!(content.contains("`web`"));
    assert_eq!(content.matches(MANAGED_START).count(), 1);
}

#[test]
fn a_file_with_no_markers_keeps_every_byte_and_gains_the_block_on_top() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("CLAUDE.md");
    fs::write_text(&path, "# House rules\n\nNever force-push.\n").unwrap();
    let block = build_block(&hall(), &[repo("api")]);

    assert_eq!(materialise(&path, &block).unwrap(), Change::Updated);

    let content = fs::read_text(&path).unwrap().unwrap();
    assert!(content.starts_with(MANAGED_START));
    assert!(content.contains("# House rules"));
    assert!(content.contains("Never force-push."));
}

/// `ivar sync` runs after every `git pull`. A version that rewrote the file
/// each time would put a spurious modification in `git status` on every
/// run.
#[test]
fn materialising_the_same_block_twice_reports_unchanged_and_does_not_rewrite() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("CLAUDE.md");
    let block = build_block(&hall(), &[repo("api")]);

    assert_eq!(materialise(&path, &block).unwrap(), Change::Created);
    let after_first = fs::read_bytes(&path).unwrap().unwrap();

    assert_eq!(materialise(&path, &block).unwrap(), Change::Unchanged);
    assert_eq!(fs::read_bytes(&path).unwrap().unwrap(), after_first);
}

/// An end marker before a start marker is not a block to splice — treating
/// it as one would replace the region *between* them, which is the user's
/// text, with the block.
#[test]
fn reversed_markers_are_treated_as_no_block_rather_than_spliced() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("CLAUDE.md");
    fs::write_text(
        &path,
        &format!("{MANAGED_END}\nprecious user text\n{MANAGED_START}\n"),
    )
    .unwrap();
    let block = build_block(&hall(), &[repo("api")]);

    assert_eq!(materialise(&path, &block).unwrap(), Change::Updated);

    let content = fs::read_text(&path).unwrap().unwrap();
    assert!(
        content.contains("precious user text"),
        "the user's text must survive: {content}"
    );
}

// -- remove ---------------------------------------------------------------

#[test]
fn removing_from_a_file_that_held_only_the_block_deletes_the_file() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("AGENTS.md");
    let block = build_block(&hall(), &[repo("api")]);
    materialise(&path, &block).unwrap();

    assert_eq!(remove(&path).unwrap(), Change::Removed);
    assert!(!fs::exists(&path).unwrap());
}

#[test]
fn removing_from_a_file_the_user_wrote_in_keeps_the_file() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("AGENTS.md");
    let block = build_block(&hall(), &[repo("api")]);
    fs::write_text(&path, &format!("{block}\n\n# House rules\n")).unwrap();

    assert_eq!(remove(&path).unwrap(), Change::Removed);

    let content = fs::read_text(&path).unwrap().unwrap();
    assert_eq!(content, "# House rules\n");
}

#[test]
fn removing_when_there_is_nothing_to_remove_is_unchanged() {
    let (_guard, dir) = utf8_temp_dir();
    let absent = dir.join("AGENTS.md");
    assert_eq!(remove(&absent).unwrap(), Change::Unchanged);

    let untouched = dir.join("CLAUDE.md");
    fs::write_text(&untouched, "# House rules\n").unwrap();
    assert_eq!(remove(&untouched).unwrap(), Change::Unchanged);
    assert_eq!(
        fs::read_text(&untouched).unwrap().unwrap(),
        "# House rules\n"
    );
}

// -- reconcile: the canonical file ----------------------------------------

/// A fresh temp dir with a canonical `HALL.md` holding the block for a hall
/// with one repo, plus the reconciler's canonical entry.
fn canonical_root() -> (tempfile::TempDir, Utf8PathBuf) {
    let (_guard, root) = utf8_temp_dir();
    let canonical = root.join("HALL.md");
    let block = build_block(&hall(), &[repo("api")]);
    reconcile(&canonical, &block, &[]).unwrap();
    (_guard, root)
}

#[test]
fn an_absent_hall_file_is_created_holding_only_the_managed_block() {
    let (_guard, root) = utf8_temp_dir();
    let canonical = root.join("HALL.md");
    let block = build_block(&hall(), &[repo("api")]);

    let entries = reconcile(&canonical, &block, &[]).unwrap();

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, canonical);
    assert_eq!(entries[0].change, Change::Created);
    assert_eq!(
        fs::read_text(&canonical).unwrap().unwrap(),
        format!("{block}\n")
    );
    assert_eq!(
        fs::read_symlink(&canonical).unwrap(),
        fs::SymlinkTarget::NotASymlink,
        "the canonical file must be a regular file, never a symlink"
    );
}

#[test]
fn user_bytes_without_markers_survive_byte_for_byte_after_the_prepended_block() {
    let (_guard, root) = utf8_temp_dir();
    let canonical = root.join("HALL.md");
    let user = "# House rules\n\nNever force-push.\n";
    fs::write_text(&canonical, user).unwrap();
    let block = build_block(&hall(), &[repo("api")]);

    let entries = reconcile(&canonical, &block, &[]).unwrap();

    assert_eq!(entries[0].change, Change::Updated);
    let content = fs::read_text(&canonical).unwrap().unwrap();
    assert!(content.starts_with(MANAGED_START));
    assert!(
        content.ends_with(user),
        "the user's bytes must survive untouched: {content:?}"
    );
}

#[test]
fn an_existing_block_is_replaced_without_touching_bytes_before_or_after() {
    let (_guard, root) = utf8_temp_dir();
    let canonical = root.join("HALL.md");
    let first = build_block(&hall(), &[repo("api")]);
    fs::write_text(
        &canonical,
        &format!("# House rules\n\n{first}\n\nNever force-push.\n"),
    )
    .unwrap();

    let second = build_block(&hall(), &[repo("api"), repo("web")]);
    let entries = reconcile(&canonical, &second, &[]).unwrap();

    assert_eq!(entries[0].change, Change::Updated);
    let content = fs::read_text(&canonical).unwrap().unwrap();
    assert!(content.starts_with("# House rules\n"));
    assert!(content.ends_with("Never force-push.\n"));
    assert!(content.contains("`web`"));
    assert_eq!(content.matches(MANAGED_START).count(), 1);
}

#[test]
fn a_second_reconcile_of_the_canonical_file_is_unchanged_and_does_not_rewrite() {
    let (_guard, root) = utf8_temp_dir();
    let canonical = root.join("HALL.md");
    let block = build_block(&hall(), &[repo("api")]);
    reconcile(&canonical, &block, &[]).unwrap();
    let after_first = fs::read_bytes(&canonical).unwrap().unwrap();

    let entries = reconcile(&canonical, &block, &[]).unwrap();

    assert_eq!(entries[0].change, Change::Unchanged);
    assert_eq!(fs::read_bytes(&canonical).unwrap().unwrap(), after_first);
}

#[test]
fn a_directory_at_the_hall_file_path_is_a_conflict_and_is_not_touched() {
    let (_guard, root) = utf8_temp_dir();
    let canonical = root.join("HALL.md");
    fs::ensure_dir(&canonical).unwrap();
    let block = build_block(&hall(), &[]);

    let entries = reconcile(&canonical, &block, &[]).unwrap();

    assert_eq!(entries[0].change, Change::Conflict);
    assert!(entries[0].detail.is_some());
    assert!(fs::is_dir(&canonical).unwrap(), "the directory must be left alone");
    assert!(
        fs::read_dir(&canonical).unwrap().is_empty(),
        "nothing may be written into the directory"
    );
}

#[test]
fn a_symlink_at_the_hall_file_path_is_a_conflict_and_its_target_is_not_touched() {
    let (_guard, root) = utf8_temp_dir();
    let target = root.join("elsewhere.md");
    fs::write_text(&target, "someone else's file\n").unwrap();
    let canonical = root.join("HALL.md");
    fs::create_symlink(&target, &canonical).unwrap();
    let block = build_block(&hall(), &[]);

    let entries = reconcile(&canonical, &block, &[]).unwrap();

    assert_eq!(entries[0].change, Change::Conflict);
    assert_eq!(
        fs::read_text(&target).unwrap().unwrap(),
        "someone else's file\n",
        "the symlink's target must be untouched"
    );
}

// -- reconcile: alias topology --------------------------------------------

/// An alias for `provider` at `path`, enabled per `enabled`.
fn claude_alias(path: Utf8PathBuf, enabled: bool) -> Alias {
    alias(Provider::ClaudeCode, path, enabled)
}

#[test]
fn enabled_absent_alias_becomes_a_relative_symlink_to_hall() {
    for provider in Provider::ALL {
        let (_guard, root) = utf8_temp_dir();
        let canonical = root.join("HALL.md");
        let block = build_block(&hall(), &[]);
        reconcile(&canonical, &block, &[]).unwrap();
        let alias_path = root.join(provider.instruction_file());

        let entries = reconcile(&canonical, &block, &[alias(provider, alias_path.clone(), true)])
            .unwrap();

        let entry = entries.iter().find(|entry| entry.path == alias_path).unwrap();
        assert_eq!(entry.change, Change::Created, "{provider}");
        assert_eq!(
            fs::read_symlink(&alias_path).unwrap(),
            fs::SymlinkTarget::Target(Utf8PathBuf::from(ALIAS_TARGET)),
            "{provider} alias must be a relative symlink to HALL.md"
        );
    }
}

#[test]
fn enabled_correct_alias_is_unchanged() {
    for provider in Provider::ALL {
        let (_guard, root) = utf8_temp_dir();
        let canonical = root.join("HALL.md");
        let block = build_block(&hall(), &[]);
        reconcile(&canonical, &block, &[]).unwrap();
        let alias_path = root.join(provider.instruction_file());
        fs::create_symlink(canonical.file_name().unwrap().as_ref(), &alias_path).unwrap();
        let before = fs::read_symlink(&alias_path).unwrap();

        let entries = reconcile(&canonical, &block, &[alias(provider, alias_path.clone(), true)])
            .unwrap();

        let entry = entries.iter().find(|entry| entry.path == alias_path).unwrap();
        assert_eq!(entry.change, Change::Unchanged, "{provider}");
        assert_eq!(fs::read_symlink(&alias_path).unwrap(), before, "{provider}");
    }
}

#[test]
fn enabled_broken_or_wrong_target_alias_is_replaced() {
    for provider in Provider::ALL {
        // Broken: the target does not exist.
        let (_guard, root) = utf8_temp_dir();
        let canonical = root.join("HALL.md");
        let block = build_block(&hall(), &[]);
        reconcile(&canonical, &block, &[]).unwrap();
        let alias_path = root.join(provider.instruction_file());
        fs::create_symlink(Utf8Path::new("vanished.md"), &alias_path).unwrap();

        let entries = reconcile(&canonical, &block, &[alias(provider, alias_path.clone(), true)])
            .unwrap();

        let entry = entries.iter().find(|entry| entry.path == alias_path).unwrap();
        assert_eq!(entry.change, Change::Updated, "{provider} broken");
        assert_eq!(
            fs::read_symlink(&alias_path).unwrap(),
            fs::SymlinkTarget::Target(Utf8PathBuf::from(ALIAS_TARGET)),
            "{provider} broken alias must be repaired"
        );

        // Wrong target: a symlink to something that exists, but not HALL.md.
        let (_guard, root) = utf8_temp_dir();
        let canonical = root.join("HALL.md");
        reconcile(&canonical, &block, &[]).unwrap();
        let alias_path = root.join(provider.instruction_file());
        let other = root.join("other.md");
        fs::write_text(&other, "x").unwrap();
        fs::create_symlink(Utf8Path::new("other.md"), &alias_path).unwrap();

        let entries = reconcile(&canonical, &block, &[alias(provider, alias_path.clone(), true)])
            .unwrap();

        let entry = entries.iter().find(|entry| entry.path == alias_path).unwrap();
        assert_eq!(entry.change, Change::Updated, "{provider} wrong target");
        assert_eq!(
            fs::read_symlink(&alias_path).unwrap(),
            fs::SymlinkTarget::Target(Utf8PathBuf::from(ALIAS_TARGET)),
            "{provider} wrong-target alias must be repaired"
        );
    }
}

#[test]
fn enabled_regular_alias_is_a_conflict_and_preserved_byte_for_byte() {
    for provider in Provider::ALL {
        let (_guard, root) = utf8_temp_dir();
        let canonical = root.join("HALL.md");
        let block = build_block(&hall(), &[]);
        reconcile(&canonical, &block, &[]).unwrap();
        let alias_path = root.join(provider.instruction_file());
        fs::write_text(&alias_path, "legacy instructions, precious\n").unwrap();

        let entries = reconcile(&canonical, &block, &[alias(provider, alias_path.clone(), true)])
            .unwrap();

        let entry = entries.iter().find(|entry| entry.path == alias_path).unwrap();
        assert_eq!(entry.change, Change::Conflict, "{provider}");
        assert!(
            entry.detail.as_deref().unwrap().contains("HALL.md"),
            "the conflict must name the way forward: {:?}",
            entry.detail
        );
        assert_eq!(
            fs::read_text(&alias_path).unwrap().unwrap(),
            "legacy instructions, precious\n",
            "{provider}: an enabled regular alias must never be overwritten"
        );
    }
}

#[test]
fn disabled_absent_alias_is_unchanged() {
    for provider in Provider::ALL {
        let (_guard, root) = utf8_temp_dir();
        let canonical = root.join("HALL.md");
        let block = build_block(&hall(), &[]);
        reconcile(&canonical, &block, &[]).unwrap();
        let alias_path = root.join(provider.instruction_file());

        let entries = reconcile(&canonical, &block, &[alias(provider, alias_path.clone(), false)])
            .unwrap();

        let entry = entries.iter().find(|entry| entry.path == alias_path).unwrap();
        assert_eq!(entry.change, Change::Unchanged, "{provider}");
        assert!(!fs::exists(&alias_path).unwrap(), "{provider}");
    }
}

#[test]
fn disabled_alias_entries_are_removed_symlink_or_regular() {
    for provider in Provider::ALL {
        // A correct symlink left behind.
        let (_guard, root) = utf8_temp_dir();
        let canonical = root.join("HALL.md");
        let block = build_block(&hall(), &[]);
        reconcile(&canonical, &block, &[]).unwrap();
        let alias_path = root.join(provider.instruction_file());
        fs::create_symlink(Utf8Path::new(ALIAS_TARGET), &alias_path).unwrap();

        let entries = reconcile(&canonical, &block, &[alias(provider, alias_path.clone(), false)])
            .unwrap();

        let entry = entries.iter().find(|entry| entry.path == alias_path).unwrap();
        assert_eq!(entry.change, Change::Removed, "{provider} symlink");
        assert!(!fs::exists(&alias_path).unwrap(), "{provider} symlink");

        // A regular file left behind.
        let (_guard, root) = utf8_temp_dir();
        let canonical = root.join("HALL.md");
        reconcile(&canonical, &block, &[]).unwrap();
        let alias_path = root.join(provider.instruction_file());
        fs::write_text(&alias_path, "a dropped provider's alias\n").unwrap();

        let entries = reconcile(&canonical, &block, &[alias(provider, alias_path.clone(), false)])
            .unwrap();

        let entry = entries.iter().find(|entry| entry.path == alias_path).unwrap();
        assert_eq!(entry.change, Change::Removed, "{provider} regular");
        assert!(!fs::exists(&alias_path).unwrap(), "{provider} regular");
    }
}

/// The one invariant every reconcile run shares: `HALL.md` is never removed
/// and never rewritten by an alias decision.
#[test]
fn no_reconcile_result_removes_or_rewrites_hall() {
    for provider in Provider::ALL {
        let (_guard, root) = utf8_temp_dir();
        let canonical = root.join("HALL.md");
        let block = build_block(&hall(), &[repo("api")]);
        reconcile(&canonical, &block, &[]).unwrap();
        let before = fs::read_bytes(&canonical).unwrap().unwrap();
        let alias_path = root.join(provider.instruction_file());
        fs::write_text(&alias_path, "regular file\n").unwrap();

        // Both an enabled conflict and a disabled removal in one run.
        reconcile(&canonical, &block, &[alias(provider, alias_path, false)]).unwrap();

        assert!(fs::is_file(&canonical).unwrap());
        assert_eq!(fs::read_bytes(&canonical).unwrap().unwrap(), before);
    }
}

// -- inspect --------------------------------------------------------------

#[test]
fn inspect_sees_a_reconciled_hall_as_current() {
    let (_guard, root) = utf8_temp_dir();
    let canonical = root.join("HALL.md");
    let block = build_block(&hall(), &[repo("api")]);
    let alias_path = root.join(Provider::ClaudeCode.instruction_file());
    let aliases = [claude_alias(alias_path, true)];
    reconcile(&canonical, &block, &aliases).unwrap();

    let inspections = inspect(&canonical, &block, &aliases).unwrap();

    assert!(inspections.len() == 2);
    assert!(inspections.iter().all(|i| i.integrity == Integrity::Current));
}

#[test]
fn inspect_reports_every_canonical_and_alias_drift_in_one_run() {
    let (_guard, root) = utf8_temp_dir();
    // Canonical absent; alias a regular file (enabled).
    let canonical = root.join("HALL.md");
    let block = build_block(&hall(), &[repo("api")]);
    let alias_path = root.join(Provider::ClaudeCode.instruction_file());
    fs::write_text(&alias_path, "regular\n").unwrap();
    let aliases = [claude_alias(alias_path.clone(), true)];

    let inspections = inspect(&canonical, &block, &aliases).unwrap();

    assert!(inspections.contains(&Inspection {
        path: canonical,
        integrity: Integrity::Missing,
    }));
    assert!(inspections.contains(&Inspection {
        path: alias_path,
        integrity: Integrity::AliasIsRegular,
    }));
}

#[test]
fn inspect_distinguishes_broken_from_wrong_target_and_disabled_presence() {
    let (_guard, root) = utf8_temp_dir();
    let canonical = root.join("HALL.md");
    let block = build_block(&hall(), &[]);
    reconcile(&canonical, &block, &[]).unwrap();

    // Broken: target does not exist.
    let broken = root.join(Provider::ClaudeCode.instruction_file());
    fs::create_symlink(Utf8Path::new("vanished.md"), &broken).unwrap();
    // Wrong target: target exists but is not HALL.md.
    let wrong = root.join(Provider::OpenCode.instruction_file());
    let other = root.join("other.md");
    fs::write_text(&other, "x").unwrap();
    fs::create_symlink(Utf8Path::new("other.md"), &wrong).unwrap();

    let inspections = inspect(
        &canonical,
        &block,
        &[
            claude_alias(broken.clone(), true),
            alias(Provider::OpenCode, wrong.clone(), true),
        ],
    )
    .unwrap();

    assert!(inspections.contains(&Inspection {
        path: broken,
        integrity: Integrity::AliasBroken,
    }));
    assert!(inspections.contains(&Inspection {
        path: wrong,
        integrity: Integrity::AliasWrongTarget,
    }));
}

#[test]
fn inspect_marks_stale_block_and_disabled_presence() {
    let (_guard, root) = utf8_temp_dir();
    let canonical = root.join("HALL.md");
    let stale = build_block(&hall(), &[repo("api")]);
    reconcile(&canonical, &stale, &[]).unwrap();
    let current = build_block(&hall(), &[repo("api"), repo("web")]);
    let alias_path = root.join(Provider::ClaudeCode.instruction_file());
    fs::write_text(&alias_path, "regular\n").unwrap();
    let aliases = [claude_alias(alias_path.clone(), false)];

    let inspections = inspect(&canonical, &current, &aliases).unwrap();

    assert!(inspections.contains(&Inspection {
        path: canonical.clone(),
        integrity: Integrity::ManagedBlockStale,
    }));
    assert!(inspections.contains(&Inspection {
        path: alias_path,
        integrity: Integrity::DisabledAliasPresent,
    }));
}

// -- Error -> Failure ------------------------------------------------------

#[test]
fn an_io_error_keeps_the_fs_layers_code_and_names_the_file() {
    let (_guard, dir) = utf8_temp_dir();
    // A directory where a file is expected: writing fails at the fs layer,
    // which is the mechanical cause this module wraps.
    let path = dir.join("CLAUDE.md");
    fs::ensure_dir(&path).unwrap();

    let error = materialise(&path, "block").expect_err("cannot write a directory");
    let failure: Failure = error.into();

    assert!(
        failure
            .fix_actions
            .iter()
            .any(|fix| fix.code == "harness.check_instruction_file"),
        "expected the file-naming fix action, got {:?}",
        failure.fix_actions
    );
}
