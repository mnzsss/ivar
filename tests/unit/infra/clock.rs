#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

#[test]
fn now_is_after_2026() {
    assert!(now_epoch_secs() > 1_767_225_600);
}

#[test]
fn a_local_time_resolves_within_a_day_of_its_utc_reading() {
    // 2030-01-01 00:00 UTC; every real zone is within +-14h of it.
    let utc = 1_893_456_000;
    let local = local_epoch_secs("2030-01-01 00:00").unwrap();
    assert!((local - utc).abs() <= 14 * 3600, "{local}");
}

#[test]
fn nonsense_is_not_placed() {
    assert_eq!(local_epoch_secs("not a time"), None);
}
