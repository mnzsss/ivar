#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rstest::rstest;

use super::*;

#[rstest]
#[case::weekdays_at_nine("0 9 * * 1-5", "Mon,Tue,Wed,Thu,Fri *-*-* 09:00:00")]
#[case::every_fifteen_minutes("*/15 * * * *", "*-*-* *:00,15,30,45:00")]
#[case::quarterly("30 2 1 */3 *", "*-01,04,07,10-01 02:30:00")]
#[case::sunday_as_seven("0 0 * * 7", "Sun *-*-* 00:00:00")]
#[case::names("0 6 * jan,jul sat", "Sat *-01,07-* 06:00:00")]
#[case::stepped_range("0 8-18/5 * * *", "*-*-* 08,13,18:00:00")]
#[case::daily_macro("@daily", "*-*-* 00:00:00")]
#[case::weekly_macro("@weekly", "Sun *-*-* 00:00:00")]
#[case::yearly_macro("@yearly", "*-01-01 00:00:00")]
fn translates_exactly(#[case] cron: &str, #[case] on_calendar: &str) {
    let expr = CronExpr::parse(cron).expect("translatable");
    assert_eq!(expr.on_calendar(), on_calendar);
    assert_eq!(expr.as_str(), cron);
}

#[rstest]
#[case::dom_and_dow("0 0 1 * 1", "day-of-month and day-of-week")]
#[case::reboot("@reboot", "@reboot")]
#[case::hour_out_of_range("0 24 * * *", "hour")]
#[case::four_fields("0 0 * *", "5 fields")]
#[case::zero_step("*/0 * * * *", "step")]
#[case::reversed_range("0 5-2 * * *", "range")]
fn refuses_what_it_cannot_translate(#[case] cron: &str, #[case] mentions: &str) {
    let error = CronExpr::parse(cron).expect_err("refused");
    assert!(error.reason.contains(mentions), "{}", error.reason);
}

fn wall(text: &str) -> WallTime {
    WallTime::parse(text).unwrap()
}

#[rstest]
#[case::daily_before_the_hour("0 7 * * *", "2026-11-06 06:59", "2026-11-05 07:00")]
#[case::daily_after_the_hour("0 7 * * *", "2026-11-06 09:30", "2026-11-06 07:00")]
#[case::exactly_on_an_occurrence("0 7 * * *", "2026-11-06 07:00", "2026-11-06 07:00")]
#[case::the_minute_after("0 7 * * *", "2026-11-06 07:01", "2026-11-06 07:00")]
#[case::daily_macro("@daily", "2026-11-06 12:00", "2026-11-06 00:00")]
#[case::hourly("0 * * * *", "2026-11-06 12:34", "2026-11-06 12:00")]
#[case::hourly_across_midnight("0 * * * *", "2026-11-07 00:00", "2026-11-07 00:00")]
#[case::every_fifteen("*/15 * * * *", "2026-11-06 12:44", "2026-11-06 12:30")]
#[case::fifteen_before_the_first("*/15 9 * * *", "2026-11-06 10:00", "2026-11-06 09:45")]
#[case::minute_within_an_earlier_hour("30 8,17 * * *", "2026-11-06 17:10", "2026-11-06 08:30")]
#[case::weekdays_on_a_saturday("0 9 * * 1-5", "2026-11-07 12:00", "2026-11-06 09:00")]
#[case::weekdays_on_a_monday_morning("0 9 * * 1-5", "2026-11-09 08:00", "2026-11-06 09:00")]
#[case::weekly_sunday("@weekly", "2026-11-06 12:00", "2026-11-01 00:00")]
#[case::sunday_as_seven("0 0 * * 7", "2026-11-06 12:00", "2026-11-01 00:00")]
#[case::monthly_across_the_month("@monthly", "2026-12-01 00:00", "2026-12-01 00:00")]
#[case::end_of_month_across_the_turn("0 8 31 * *", "2026-12-15 12:00", "2026-10-31 08:00")]
#[case::new_year("30 23 * * *", "2027-01-01 00:10", "2026-12-31 23:30")]
#[case::yearly("@yearly", "2026-11-06 12:00", "2026-01-01 00:00")]
#[case::leap_day("0 0 29 2 *", "2026-11-06 12:00", "2024-02-29 00:00")]
fn finds_the_last_occurrence_at_or_before(
    #[case] cron: &str,
    #[case] now: &str,
    #[case] expected: &str,
) {
    let found = CronExpr::parse(cron)
        .unwrap()
        .last_at_or_before(wall(now))
        .unwrap();
    assert_eq!(found.to_string(), expected);
}

#[test]
fn a_schedule_that_never_matches_a_real_date_has_no_last_occurrence() {
    let never = CronExpr::parse("0 0 31 2 *").unwrap();
    assert_eq!(never.last_at_or_before(wall("2026-11-06 12:00")), None);
}

#[test]
fn a_wall_time_round_trips_and_refuses_what_is_not_a_date() {
    assert_eq!(wall("2026-02-28 07:05").to_string(), "2026-02-28 07:05");
    for bad in [
        "2026-02-29 07:05",
        "2026-13-01 00:00",
        "2026-01-01 24:00",
        "2026-01-01",
        "2026-01-01 00:00:00",
        "x y",
    ] {
        assert_eq!(WallTime::parse(bad), None, "{bad}");
    }
}
