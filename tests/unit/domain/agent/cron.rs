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
