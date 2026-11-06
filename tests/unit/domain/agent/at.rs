#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

#[test]
fn a_local_minute_is_read_as_written() {
    let at = AtTime::parse("2026-11-08T09:00").unwrap();
    assert!(!at.has_offset());
    assert_eq!(at.to_string(), "2026-11-08T09:00");
    assert_eq!(at.on_calendar(), "2026-11-08 09:00:00");
    assert_eq!(at.file_stamp(), "2026-11-08T0900");
    assert_eq!(at.wall(), "2026-11-08 09:00");
}

#[test]
fn an_offset_minute_resolves_to_utc() {
    let at = AtTime::parse("2026-11-08T09:00-03:00").unwrap();
    assert!(at.has_offset());
    assert_eq!(at.to_string(), "2026-11-08T09:00-03:00");
    assert_eq!(at.on_calendar(), "2026-11-08 12:00:00 UTC");
    assert_eq!(at.file_stamp(), "2026-11-08T0900");
    assert_eq!(at.epoch_secs(&|_| None), Some(1_794_139_200));
}

#[test]
fn an_offset_can_cross_midnight_and_z_is_utc() {
    let east = AtTime::parse("2026-01-01T01:30+05:30").unwrap();
    assert_eq!(east.on_calendar(), "2025-12-31 20:00:00 UTC");
    let z = AtTime::parse("2026-11-08T09:00Z").unwrap();
    assert_eq!(z.to_string(), "2026-11-08T09:00Z");
    assert_eq!(z.on_calendar(), "2026-11-08 09:00:00 UTC");
}

#[test]
fn a_local_minute_is_placed_by_the_machine_zone() {
    let at = AtTime::parse("2026-11-08T09:00").unwrap();
    assert_eq!(
        at.epoch_secs(&|wall| (wall == "2026-11-08 09:00").then_some(42)),
        Some(42)
    );
    assert_eq!(at.epoch_secs(&|_| None), None);
}

#[test]
fn relative_values_are_refused_with_a_hint() {
    for text in ["2d", "90m", "1w", "3h", "10s"] {
        let reason = AtTime::parse(text).unwrap_err();
        assert!(reason.contains("relative"), "{text}: {reason}");
    }
}

#[test]
fn anything_but_a_minute_is_refused() {
    for text in [
        "",
        "tomorrow",
        "2026-11-08",
        "2026-11-08T09",
        "2026-11-08T09:00:30",
        "2026-11-08 09:00",
        "2026-11-08T09:00+3",
        "2026-11-08T09:00+0300",
        "2026-11-08T09:00+15:00",
        "2026-11-08T09:00+03:60",
        "2026-11-08T24:00",
        "2026-11-08T09:60",
        "2026-13-08T09:00",
        "2026-02-30T09:00",
        "2026-00-10T09:00",
        "+026-11-08T09:00",
        "2026-11-08T09:00é",
    ] {
        assert!(AtTime::parse(text).is_err(), "{text:?} was accepted");
    }
}

#[test]
fn leap_days_exist_only_in_leap_years() {
    assert!(AtTime::parse("2028-02-29T09:00").is_ok());
    assert!(AtTime::parse("2027-02-29T09:00").is_err());
    assert!(AtTime::parse("2100-02-29T09:00").is_err());
}
