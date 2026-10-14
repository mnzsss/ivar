#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::domain::skill::RenderMode;
use crate::domain::skill_sync::MaterialStatus;
use crate::infra::fs;
use crate::test_support::canonical_temp_dir;

#[test]
fn verify_status_copy_mode_cases() {
    let (_guard, root) = canonical_temp_dir();
    let expected = root.join("expected_source");
    fs::ensure_dir(&expected).unwrap();

    // 1. Absent path -> Missing
    let missing_path = root.join("missing");
    assert_eq!(
        verify_status(&missing_path, &expected, RenderMode::Copy),
        MaterialStatus::Missing
    );

    // 2. Real directory (not symlink) -> Ok
    let real_dir = root.join("real_dir");
    fs::ensure_dir(&real_dir).unwrap();
    assert_eq!(
        verify_status(&real_dir, &expected, RenderMode::Copy),
        MaterialStatus::Ok
    );

    // 3. Symlink -> NotLink
    let symlink_path = root.join("symlink_target");
    fs::create_symlink(&expected, &symlink_path).unwrap();
    assert_eq!(
        verify_status(&symlink_path, &expected, RenderMode::Copy),
        MaterialStatus::NotLink
    );
}

#[test]
fn verify_status_symlink_mode_cases() {
    let (_guard, root) = canonical_temp_dir();
    let expected = root.join("expected_source");
    let other = root.join("other_source");
    fs::ensure_dir(&expected).unwrap();
    fs::ensure_dir(&other).unwrap();

    // Absent -> Missing
    let missing = root.join("missing");
    assert_eq!(
        verify_status(&missing, &expected, RenderMode::Symlink),
        MaterialStatus::Missing
    );

    // Correct symlink -> Ok
    let valid_link = root.join("valid_link");
    fs::create_symlink(&expected, &valid_link).unwrap();
    assert_eq!(
        verify_status(&valid_link, &expected, RenderMode::Symlink),
        MaterialStatus::Ok
    );

    // Symlink to wrong target -> WrongLink
    let wrong_link = root.join("wrong_link");
    fs::create_symlink(&other, &wrong_link).unwrap();
    assert_eq!(
        verify_status(&wrong_link, &expected, RenderMode::Symlink),
        MaterialStatus::WrongLink
    );

    // Real directory -> NotLink
    let real_dir = root.join("real_dir");
    fs::ensure_dir(&real_dir).unwrap();
    assert_eq!(
        verify_status(&real_dir, &expected, RenderMode::Symlink),
        MaterialStatus::NotLink
    );
}
