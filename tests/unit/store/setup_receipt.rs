#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::error::Status;
use crate::infra::{fs, json};
use crate::test_support::utf8_temp_dir;

const DIGEST: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
const OTHER: &str = "sha256:2222222222222222222222222222222222222222222222222222222222222222";

// -- round-trip ------------------------------------------------------------

#[test]
fn a_written_receipt_reads_back_identical_and_in_canonical_form() {
    let (_guard, git_dir) = utf8_temp_dir();
    let receipt = Receipt::of_run(DIGEST, Some(0));

    Receipt::write(&git_dir, &receipt).unwrap();

    assert_eq!(Receipt::read(&git_dir), Some(receipt));

    let expected = json::to_canonical_string(&serde_json::json!({
        "version": 1,
        "fingerprint": DIGEST,
        "outcome": "success",
        "exit_code": 0,
    }))
    .unwrap();
    assert_eq!(
        fs::read_text(&Receipt::path_in(&git_dir)).unwrap().unwrap(),
        expected
    );
}

#[test]
fn the_receipt_is_namespaced_under_ivar_inside_the_git_dir() {
    let path = Receipt::path_in(Utf8Path::new("/bare/worktrees/main"));

    assert_eq!(
        path,
        Utf8PathBuf::from("/bare/worktrees/main/ivar/setup-receipt.json")
    );
}

// -- the outcome is derived, not supplied ---------------------------------

#[test]
fn the_outcome_follows_the_exit_code() {
    assert_eq!(
        Receipt::of_run(DIGEST, Some(0)).outcome(),
        RunOutcome::Success
    );
    assert_eq!(
        Receipt::of_run(DIGEST, Some(1)).outcome(),
        RunOutcome::Failure
    );
    assert_eq!(Receipt::of_run(DIGEST, None).outcome(), RunOutcome::Failure);
}

/// `fingerprint` and `exit_code` have no accessor, so the round-trip test
/// above is what proves they reach disk — which is the whole reason they
/// are on the struct.
#[test]
fn the_exit_code_is_recorded_even_for_a_signal_death() {
    let (_guard, git_dir) = utf8_temp_dir();
    let receipt = Receipt::of_run(DIGEST, None);

    Receipt::write(&git_dir, &receipt).unwrap();

    assert_eq!(Receipt::read(&git_dir), Some(receipt));
    let on_disk = fs::read_text(&Receipt::path_in(&git_dir)).unwrap().unwrap();
    assert!(on_disk.contains(r#""exit_code": null"#), "was: {on_disk}");
}

// -- read is total ---------------------------------------------------------

#[test]
fn an_absent_receipt_reads_as_none() {
    let (_guard, git_dir) = utf8_temp_dir();

    assert_eq!(Receipt::read(&git_dir), None);
}

/// Every unreadable shape has the same correct answer: run the script.
/// Refusing to sync because a cache file is corrupt would trade a slow run
/// for a broken one.
#[test]
fn a_corrupt_or_unversioned_or_too_new_receipt_all_read_as_none() {
    for content in [
        "{ not json",
        r#"{"fingerprint":"x","outcome":"success","exit_code":0}"#,
        r#"{"version":99,"fingerprint":"x","outcome":"success","exit_code":0}"#,
        r#"{"version":1,"fingerprint":"x","outcome":"maybe","exit_code":0}"#,
    ] {
        let (_guard, git_dir) = utf8_temp_dir();
        let path = Receipt::path_in(&git_dir);
        fs::ensure_dir(path.parent().unwrap()).unwrap();
        fs::write_text(&path, content).unwrap();

        assert_eq!(Receipt::read(&git_dir), None, "for content: {content}");
    }
}

// -- should_run ------------------------------------------------------------

#[test]
fn with_no_receipt_the_script_runs() {
    assert!(Receipt::should_run(None, DIGEST, false));
}

#[test]
fn a_matching_successful_receipt_skips_the_run() {
    let receipt = Receipt::of_run(DIGEST, Some(0));

    assert!(!Receipt::should_run(Some(&receipt), DIGEST, false));
}

#[test]
fn a_changed_script_runs_again() {
    let receipt = Receipt::of_run(DIGEST, Some(0));

    assert!(Receipt::should_run(Some(&receipt), OTHER, false));
}

/// A failed setup that recorded "done" would leave every later sync
/// silently skipping the repair the user is waiting for.
#[test]
fn a_failed_run_is_retried_even_though_the_script_is_unchanged() {
    let receipt = Receipt::of_run(DIGEST, Some(1));

    assert!(Receipt::should_run(Some(&receipt), DIGEST, false));
}

#[test]
fn force_overrides_a_matching_successful_receipt() {
    let receipt = Receipt::of_run(DIGEST, Some(0));

    assert!(Receipt::should_run(Some(&receipt), DIGEST, true));
}

// -- write failure ---------------------------------------------------------

#[test]
fn a_write_that_cannot_land_is_a_failed_failure_pointing_back_at_sync() {
    let (_guard, dir) = utf8_temp_dir();
    // A file where the `ivar/` namespace directory needs to be.
    let git_dir = dir.join("git");
    fs::ensure_dir(&git_dir).unwrap();
    fs::write_text(&git_dir.join("ivar"), "in the way").unwrap();

    let error = Receipt::write(&git_dir, &Receipt::of_run(DIGEST, Some(0)))
        .expect_err("cannot create the namespace directory");

    let failure: Failure = error.into();
    assert_eq!(failure.status, Status::Failed);
    assert_eq!(failure.code, "setup.receipt_write_failed");
    assert_eq!(failure.fix_actions[0].command.as_deref(), Some("ivar sync"));
}
