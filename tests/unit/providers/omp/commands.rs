//! Tests for OMP profile resolution and command bridge projection.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use camino::Utf8PathBuf;

use crate::infra::fs::{self, SymlinkTarget};
use crate::providers::omp::commands::{
    bridge_remove_under, bridge_sync_under, normalize_profile_name, resolve_commands_dir_from,
    resolve_profile_from_env, user_home_from,
};
use crate::test_support::utf8_temp_dir;

const TEST_COMMANDS: &[&str] = &[
    "ivar-deliver.md",
    "ivar-feedback.md",
    "ivar-help.md",
    "ivar-plan.md",
    "ivar-status.md",
    "ivar-sync.md",
];

#[test]
fn normalize_profile_name_rules() {
    // Empty, None, or "default" -> None
    assert_eq!(normalize_profile_name(None).unwrap(), None);
    assert_eq!(normalize_profile_name(Some("")).unwrap(), None);
    assert_eq!(normalize_profile_name(Some("   ")).unwrap(), None);
    assert_eq!(normalize_profile_name(Some("default")).unwrap(), None);
    assert_eq!(normalize_profile_name(Some(" default ")).unwrap(), None);

    // Valid names
    assert_eq!(
        normalize_profile_name(Some("work")).unwrap(),
        Some("work".to_owned())
    );
    assert_eq!(
        normalize_profile_name(Some("project-1_dev.test")).unwrap(),
        Some("project-1_dev.test".to_owned())
    );

    // Invalid names
    assert!(normalize_profile_name(Some(".")).is_err());
    assert!(normalize_profile_name(Some("..")).is_err());
    assert!(normalize_profile_name(Some("test.")).is_err());
    assert!(normalize_profile_name(Some("-badstart")).is_err());
    assert!(normalize_profile_name(Some("_badstart")).is_err());
    assert!(normalize_profile_name(Some("CON")).is_err());
    assert!(normalize_profile_name(Some("con.txt")).is_err());
    assert!(normalize_profile_name(Some("NUL")).is_err());
    assert!(normalize_profile_name(Some("aux")).is_err());
    assert!(normalize_profile_name(Some("com1")).is_err());
}

#[test]
fn resolve_profile_from_env_precedence() {
    // OMP_PROFILE takes precedence over PI_PROFILE
    assert_eq!(
        resolve_profile_from_env(Some("work"), Some("legacy")).unwrap(),
        Some("work".to_owned())
    );

    // Explicit empty OMP_PROFILE selects default (ignores PI_PROFILE)
    assert_eq!(
        resolve_profile_from_env(Some(""), Some("legacy")).unwrap(),
        None
    );

    // PI_PROFILE fallback when OMP_PROFILE is None
    assert_eq!(
        resolve_profile_from_env(None, Some("legacy")).unwrap(),
        Some("legacy".to_owned())
    );

    // Neither set -> default
    assert_eq!(resolve_profile_from_env(None, None).unwrap(), None);
}

#[test]
fn user_home_from_resolution() {
    assert_eq!(
        user_home_from(Some("/home/user".to_owned()), None, "linux").unwrap(),
        Utf8PathBuf::from("/home/user")
    );
    assert_eq!(
        user_home_from(None, Some("C:\\Users\\User".to_owned()), "windows").unwrap(),
        Utf8PathBuf::from("C:\\Users\\User")
    );
    assert!(user_home_from(None, None, "linux").is_err());
    assert!(user_home_from(Some("relative/path".to_owned()), None, "linux").is_err());
}

#[test]
fn resolve_commands_dir_default_and_named_profiles() {
    let home = Utf8PathBuf::from("/home/user");

    // Default profile, standard config dir
    let dir = resolve_commands_dir_from(&home, None, None, None).unwrap();
    assert_eq!(dir, Utf8PathBuf::from("/home/user/.omp/agent/commands"));

    // Named profile "work"
    let dir = resolve_commands_dir_from(&home, None, Some("work"), None).unwrap();
    assert_eq!(
        dir,
        Utf8PathBuf::from("/home/user/.omp/profiles/work/agent/commands")
    );

    // Custom PI_CONFIG_DIR ".config/omp" with named profile
    let dir = resolve_commands_dir_from(&home, Some(".config/omp"), Some("work"), None).unwrap();
    assert_eq!(
        dir,
        Utf8PathBuf::from("/home/user/.config/omp/profiles/work/agent/commands")
    );

    // Custom PI_CONFIG_DIR with default profile
    let dir = resolve_commands_dir_from(&home, Some(".config/omp"), None, None).unwrap();
    assert_eq!(
        dir,
        Utf8PathBuf::from("/home/user/.config/omp/agent/commands")
    );
}

#[test]
fn bridge_sync_creates_symlinks_and_preserves_user_files() {
    let (_guard, root) = utf8_temp_dir();
    let hall_omp_commands = root.join("hall/.omp/commands");
    fs::ensure_dir(&hall_omp_commands).unwrap();
    for cmd in TEST_COMMANDS {
        fs::write_text(&hall_omp_commands.join(cmd), "# Command").unwrap();
    }

    let profile_commands = root.join("profile_commands");
    fs::ensure_dir(&profile_commands).unwrap();

    // Place a user-owned custom command and a colliding non-symlink file
    fs::write_text(&profile_commands.join("my-custom.md"), "# Custom").unwrap();
    fs::write_text(&profile_commands.join("ivar-plan.md"), "# User collision").unwrap();

    let mut warnings = Vec::new();
    bridge_sync_under(
        &profile_commands,
        &hall_omp_commands,
        TEST_COMMANDS,
        &mut warnings,
    );

    // Collision on ivar-plan.md should produce a warning and preserve the user file
    assert!(
        warnings
            .iter()
            .any(|w| w.code == "omp.profile_bridge_conflict"),
        "expected conflict warning: {:?}",
        warnings
    );
    assert_eq!(
        fs::read_text(&profile_commands.join("ivar-plan.md"))
            .unwrap()
            .unwrap(),
        "# User collision"
    );

    // User's custom command must still exist
    assert!(fs::is_file(&profile_commands.join("my-custom.md")).unwrap());

    // Other catalog commands should be properly projected as symlinks pointing to hall
    for cmd in TEST_COMMANDS {
        if *cmd == "ivar-plan.md" {
            continue;
        }
        let link = profile_commands.join(cmd);
        assert_eq!(
            fs::read_symlink(&link).unwrap(),
            SymlinkTarget::Target(hall_omp_commands.join(cmd)),
            "command {} must link to hall",
            cmd
        );
    }
}

#[test]
fn bridge_sync_removes_stale_ivar_links_for_this_hall() {
    let (_guard, root) = utf8_temp_dir();
    let hall_omp_commands = root.join("hall/.omp/commands");
    fs::ensure_dir(&hall_omp_commands).unwrap();
    for cmd in TEST_COMMANDS {
        fs::write_text(&hall_omp_commands.join(cmd), "# Command").unwrap();
    }

    let other_hall = root.join("other_hall");

    let profile_commands = root.join("profile_commands");
    fs::ensure_dir(&profile_commands).unwrap();

    // Create a stale ivar link pointing to this hall
    let stale_link = profile_commands.join("ivar-old.md");
    fs::create_symlink(&hall_omp_commands.join("ivar-old.md"), &stale_link).unwrap();

    // Create an ivar link pointing to another hall (should NOT be removed)
    let other_hall_commands = other_hall.join(".omp/commands");
    fs::ensure_dir(&other_hall_commands).unwrap();
    fs::write_text(&other_hall_commands.join("ivar-other.md"), "# Other").unwrap();
    let other_link = profile_commands.join("ivar-other.md");
    fs::create_symlink(&other_hall_commands.join("ivar-other.md"), &other_link).unwrap();

    let mut warnings = Vec::new();
    bridge_sync_under(
        &profile_commands,
        &hall_omp_commands,
        TEST_COMMANDS,
        &mut warnings,
    );

    // stale link to this hall should be gone
    assert!(
        !fs::exists(&stale_link).unwrap(),
        "stale link pointing to this hall must be removed"
    );

    // link to other hall should survive
    assert!(
        fs::exists(&other_link).unwrap(),
        "link pointing to other hall must survive"
    );
}

#[test]
fn bridge_remove_cleans_up_all_links_for_hall() {
    let (_guard, root) = utf8_temp_dir();
    let hall_omp_commands = root.join("hall/.omp/commands");
    fs::ensure_dir(&hall_omp_commands).unwrap();
    for cmd in TEST_COMMANDS {
        fs::write_text(&hall_omp_commands.join(cmd), "# Command").unwrap();
    }

    let profile_commands = root.join("profile_commands");
    fs::ensure_dir(&profile_commands).unwrap();

    let mut warnings = Vec::new();
    bridge_sync_under(
        &profile_commands,
        &hall_omp_commands,
        TEST_COMMANDS,
        &mut warnings,
    );

    // Verify commands exist
    assert!(fs::is_file(&profile_commands.join("ivar-plan.md")).unwrap());

    // Call bridge_remove_under
    bridge_remove_under(&profile_commands, &hall_omp_commands, &mut warnings);

    // All ivar links for this hall should be removed
    for cmd in TEST_COMMANDS {
        let link = profile_commands.join(cmd);
        assert!(
            !fs::exists(&link).unwrap(),
            "{} should be removed by bridge_remove",
            cmd
        );
    }
}
