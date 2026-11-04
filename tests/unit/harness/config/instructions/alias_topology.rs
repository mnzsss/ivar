#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::test_support::utf8_temp_dir;

// -- reconcile: alias topology --------------------------------------------

#[test]
fn enabled_absent_alias_becomes_a_relative_symlink_to_hall() {
    for provider in Provider::ALL {
        let (_guard, root) = utf8_temp_dir();
        let canonical = root.join("HALL.md");
        let block = build_block(&hall(), &[]);
        reconcile(&canonical, &block, &[]).unwrap();
        let alias_path = root.join(provider.instruction_file());

        let entries = reconcile(
            &canonical,
            &block,
            &[alias(provider, alias_path.clone(), true)],
        )
        .unwrap();

        let entry = entries
            .iter()
            .find(|entry| entry.path == alias_path)
            .unwrap();
        assert_eq!(entry.change, Change::Created, "{provider}");
        assert_eq!(
            fs::read_symlink(&alias_path).unwrap(),
            fs::SymlinkTarget::Target(Utf8PathBuf::from(CANONICAL_FILE)),
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

        let entries = reconcile(
            &canonical,
            &block,
            &[alias(provider, alias_path.clone(), true)],
        )
        .unwrap();

        let entry = entries
            .iter()
            .find(|entry| entry.path == alias_path)
            .unwrap();
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

        let entries = reconcile(
            &canonical,
            &block,
            &[alias(provider, alias_path.clone(), true)],
        )
        .unwrap();

        let entry = entries
            .iter()
            .find(|entry| entry.path == alias_path)
            .unwrap();
        assert_eq!(entry.change, Change::Updated, "{provider} broken");
        assert_eq!(
            fs::read_symlink(&alias_path).unwrap(),
            fs::SymlinkTarget::Target(Utf8PathBuf::from(CANONICAL_FILE)),
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

        let entries = reconcile(
            &canonical,
            &block,
            &[alias(provider, alias_path.clone(), true)],
        )
        .unwrap();

        let entry = entries
            .iter()
            .find(|entry| entry.path == alias_path)
            .unwrap();
        assert_eq!(entry.change, Change::Updated, "{provider} wrong target");
        assert_eq!(
            fs::read_symlink(&alias_path).unwrap(),
            fs::SymlinkTarget::Target(Utf8PathBuf::from(CANONICAL_FILE)),
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

        let entries = reconcile(
            &canonical,
            &block,
            &[alias(provider, alias_path.clone(), true)],
        )
        .unwrap();

        let entry = entries
            .iter()
            .find(|entry| entry.path == alias_path)
            .unwrap();
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

        let entries = reconcile(
            &canonical,
            &block,
            &[alias(provider, alias_path.clone(), false)],
        )
        .unwrap();

        let entry = entries
            .iter()
            .find(|entry| entry.path == alias_path)
            .unwrap();
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
        fs::create_symlink(Utf8Path::new(CANONICAL_FILE), &alias_path).unwrap();

        let entries = reconcile(
            &canonical,
            &block,
            &[alias(provider, alias_path.clone(), false)],
        )
        .unwrap();

        let entry = entries
            .iter()
            .find(|entry| entry.path == alias_path)
            .unwrap();
        assert_eq!(entry.change, Change::Removed, "{provider} symlink");
        assert!(!fs::exists(&alias_path).unwrap(), "{provider} symlink");

        // A regular file left behind.
        let (_guard, root) = utf8_temp_dir();
        let canonical = root.join("HALL.md");
        reconcile(&canonical, &block, &[]).unwrap();
        let alias_path = root.join(provider.instruction_file());
        fs::write_text(&alias_path, "a dropped provider's alias\n").unwrap();

        let entries = reconcile(
            &canonical,
            &block,
            &[alias(provider, alias_path.clone(), false)],
        )
        .unwrap();

        let entry = entries
            .iter()
            .find(|entry| entry.path == alias_path)
            .unwrap();
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

#[test]
fn shared_instruction_surface_matrix_opencode_and_omp() {
    let (_guard, root) = utf8_temp_dir();
    let canonical = root.join("HALL.md");
    let block = build_block(&hall(), &[]);
    reconcile(&canonical, &block, &[]).unwrap();
    let agents_path = root.join("AGENTS.md");

    // State 1: Both OpenCode and OMP enabled -> AGENTS.md created once as symlink to HALL.md
    let entries = reconcile(
        &canonical,
        &block,
        &[Alias {
            path: agents_path.clone(),
            owners: vec![Provider::OpenCode, Provider::Omp],
            enabled: true,
        }],
    )
    .unwrap();

    assert_eq!(entries.len(), 2); // HALL.md (Unchanged) + AGENTS.md (Created)
    let agents_entry = entries.iter().find(|e| e.path == agents_path).unwrap();
    assert_eq!(agents_entry.change, Change::Created);
    assert_eq!(
        fs::read_symlink(&agents_path).unwrap(),
        fs::SymlinkTarget::Target(Utf8PathBuf::from(CANONICAL_FILE))
    );

    // State 2: OpenCode disabled, OMP enabled -> AGENTS.md preserved as Unchanged
    let entries = reconcile(
        &canonical,
        &block,
        &[Alias {
            path: agents_path.clone(),
            owners: vec![Provider::OpenCode, Provider::Omp],
            enabled: true, // any owner enabled means alias is enabled
        }],
    )
    .unwrap();
    let agents_entry = entries.iter().find(|e| e.path == agents_path).unwrap();
    assert_eq!(agents_entry.change, Change::Unchanged);
    assert_eq!(
        fs::read_symlink(&agents_path).unwrap(),
        fs::SymlinkTarget::Target(Utf8PathBuf::from(CANONICAL_FILE))
    );

    // State 3: OpenCode enabled, OMP disabled -> AGENTS.md preserved as Unchanged
    let entries = reconcile(
        &canonical,
        &block,
        &[Alias {
            path: agents_path.clone(),
            owners: vec![Provider::OpenCode, Provider::Omp],
            enabled: true,
        }],
    )
    .unwrap();
    let agents_entry = entries.iter().find(|e| e.path == agents_path).unwrap();
    assert_eq!(agents_entry.change, Change::Unchanged);
    assert_eq!(
        fs::read_symlink(&agents_path).unwrap(),
        fs::SymlinkTarget::Target(Utf8PathBuf::from(CANONICAL_FILE))
    );
    // State 4: Neither enabled -> AGENTS.md removed
    let entries = reconcile(
        &canonical,
        &block,
        &[Alias {
            path: agents_path.clone(),
            owners: vec![Provider::OpenCode, Provider::Omp],
            enabled: false,
        }],
    )
    .unwrap();
    let agents_entry = entries.iter().find(|e| e.path == agents_path).unwrap();
    assert_eq!(agents_entry.change, Change::Removed);
    assert!(!fs::exists(&agents_path).unwrap());
}

#[test]
fn shared_instruction_surface_user_content_outside_managed_block_is_preserved_on_conflict() {
    let (_guard, root) = utf8_temp_dir();
    let canonical = root.join("HALL.md");
    let block = build_block(&hall(), &[]);
    reconcile(&canonical, &block, &[]).unwrap();
    let agents_path = root.join("AGENTS.md");

    // User created regular file at AGENTS.md
    let user_content = "# Custom Team Rules\nDo not break production.\n";
    fs::write_text(&agents_path, user_content).unwrap();

    let entries = reconcile(
        &canonical,
        &block,
        &[Alias {
            path: agents_path.clone(),
            owners: vec![Provider::OpenCode, Provider::Omp],
            enabled: true,
        }],
    )
    .unwrap();

    let agents_entry = entries.iter().find(|e| e.path == agents_path).unwrap();
    assert_eq!(agents_entry.change, Change::Conflict);
    assert_eq!(
        fs::read_text(&agents_path).unwrap().unwrap(),
        user_content,
        "User content in regular file must be preserved byte-for-byte on conflict"
    );
}
