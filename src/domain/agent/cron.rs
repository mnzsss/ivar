//! A cron expression and its exact systemd `OnCalendar` translation.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Serialize, Serializer};

use crate::domain::session::{civil_from_days, days_from_civil};

struct Field {
    name: &'static str,
    min: u32,
    max: u32,
    names: &'static [&'static str],
}

const MINUTE: Field = Field {
    name: "minute",
    min: 0,
    max: 59,
    names: &[],
};
const HOUR: Field = Field {
    name: "hour",
    min: 0,
    max: 23,
    names: &[],
};
const DAY_OF_MONTH: Field = Field {
    name: "day-of-month",
    min: 1,
    max: 31,
    names: &[],
};
const MONTH: Field = Field {
    name: "month",
    min: 1,
    max: 12,
    names: &[
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ],
};
const DAY_OF_WEEK: Field = Field {
    name: "day-of-week",
    min: 0,
    max: 7,
    names: &["sun", "mon", "tue", "wed", "thu", "fri", "sat"],
};
const SYSTEMD_DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronExpr {
    source: String,
    on_calendar: String,
    minutes: BTreeSet<u32>,
    hours: BTreeSet<u32>,
    days_of_month: BTreeSet<u32>,
    months: BTreeSet<u32>,
    /// `0` is Sunday; `7` is already folded into it.
    days_of_week: BTreeSet<u32>,
}

/// A civil date and time to the minute, with no zone: what `OnCalendar` is read
/// against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct WallTime {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
}

impl WallTime {
    /// Reads `YYYY-MM-DD HH:MM`, the form [`fmt::Display`] writes.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let (date, time) = text.trim().split_once(' ')?;
        let mut date = date.split('-');
        let mut time = time.trim().split(':');
        let wall = Self {
            year: date.next()?.parse().ok()?,
            month: date.next()?.parse().ok()?,
            day: date.next()?.parse().ok()?,
            hour: time.next()?.parse().ok()?,
            minute: time.next()?.parse().ok()?,
        };
        let real = date.next().is_none()
            && time.next().is_none()
            && (1..=12).contains(&wall.month)
            && (1..=days_in_month(wall.year, wall.month)).contains(&wall.day)
            && wall.hour < 24
            && wall.minute < 60;
        real.then_some(wall)
    }
}

impl fmt::Display for WallTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:04}-{:02}-{:02} {:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute
        )
    }
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// The longest gap between two occurrences of a schedule that has any: 29 Feb
/// across a skipped leap year (1896 to 1904, 2096 to 2104) is just under eight
/// years.
const LOOKBACK_DAYS: i64 = 366 * 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronError {
    pub reason: String,
}

impl fmt::Display for CronError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.reason)
    }
}

impl std::error::Error for CronError {}

impl Serialize for CronExpr {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.source)
    }
}

impl CronExpr {
    /// # Errors
    ///
    /// Returns a [`CronError`] naming the field when the expression is malformed
    /// or has no exact `OnCalendar` equivalent.
    pub fn parse(text: &str) -> Result<Self, CronError> {
        let trimmed = text.trim();
        let expanded = if trimmed.starts_with('@') {
            expand_macro(trimmed)?
        } else {
            trimmed
        };
        let parts: Vec<&str> = expanded.split_whitespace().collect();
        let [minute, hour, dom, month, dow]: [&str; 5] =
            parts.try_into().map_err(|parts: Vec<&str>| {
                error(format!("expected 5 fields, found {}", parts.len()))
            })?;
        let dow_restricted = dow != "*";
        // cron ORs a restricted day-of-month with a restricted day-of-week; OnCalendar ANDs them.
        if dom != "*" && dow_restricted {
            return Err(error(
                "day-of-month and day-of-week are both restricted; cron matches either, systemd would require both"
                    .to_owned(),
            ));
        }
        let minute = parse_field(minute, &MINUTE)?;
        let hour = parse_field(hour, &HOUR)?;
        let dom = parse_field(dom, &DAY_OF_MONTH)?;
        let month = parse_field(month, &MONTH)?;
        let dow: BTreeSet<u32> = parse_field(dow, &DAY_OF_WEEK)?
            .iter()
            .map(|day| day % 7)
            .collect();
        let prefix = if dow_restricted {
            let names: Vec<&str> = SYSTEMD_DAYS
                .iter()
                .zip(0u32..)
                .filter(|(_, day)| dow.contains(day))
                .map(|(name, _)| *name)
                .collect();
            format!("{} ", names.join(","))
        } else {
            String::new()
        };
        let on_calendar = format!(
            "{prefix}*-{}-{} {}:{}:00",
            render(&month, &MONTH),
            render(&dom, &DAY_OF_MONTH),
            render(&hour, &HOUR),
            render(&minute, &MINUTE),
        );
        Ok(Self {
            source: text.to_owned(),
            on_calendar,
            minutes: minute,
            hours: hour,
            days_of_month: dom,
            months: month,
            days_of_week: dow,
        })
    }

    /// The latest occurrence at or before `wall`, read as a civil time; `None`
    /// when the schedule never matches a real date (`0 0 31 2 *`) within
    /// eight years back.
    ///
    /// Inclusive on purpose: `wall` is a minute, and an occurrence in that same
    /// minute has already come, so a timer stamped with it must not fire for it
    /// again.
    #[must_use]
    pub fn last_at_or_before(&self, wall: WallTime) -> Option<WallTime> {
        let today = days_from_civil(wall.year, i64::from(wall.month), i64::from(wall.day));
        for back in 0..=LOOKBACK_DAYS {
            let (year, month, day) = civil_from_days(today - back);
            if !self.matches_date(today - back, month, day) {
                continue;
            }
            let (hour_limit, minute_limit) = if back == 0 {
                (wall.hour, wall.minute)
            } else {
                (23, 59)
            };
            let found = self.hours.range(..=hour_limit).rev().find_map(|&hour| {
                let limit = if hour == hour_limit { minute_limit } else { 59 };
                self.minutes
                    .range(..=limit)
                    .next_back()
                    .map(|&minute| (hour, minute))
            });
            if let Some((hour, minute)) = found {
                return Some(WallTime {
                    year,
                    month,
                    day,
                    hour,
                    minute,
                });
            }
        }
        None
    }

    fn matches_date(&self, days: i64, month: u32, day: u32) -> bool {
        // 1970-01-01 was a Thursday.
        let weekday = u32::try_from((days + 4).rem_euclid(7)).unwrap_or(0);
        self.months.contains(&month)
            && self.days_of_month.contains(&day)
            && self.days_of_week.contains(&weekday)
    }

    pub fn as_str(&self) -> &str {
        &self.source
    }

    pub fn on_calendar(&self) -> &str {
        &self.on_calendar
    }
}

fn error(reason: String) -> CronError {
    CronError { reason }
}

fn expand_macro(text: &str) -> Result<&'static str, CronError> {
    match text {
        "@hourly" => Ok("0 * * * *"),
        "@daily" => Ok("0 0 * * *"),
        "@weekly" => Ok("0 0 * * 0"),
        "@monthly" => Ok("0 0 1 * *"),
        "@yearly" | "@annually" => Ok("0 0 1 1 *"),
        other => Err(error(format!(
            "`{other}` has no systemd timer equivalent; use a 5-field expression"
        ))),
    }
}

fn parse_field(text: &str, field: &Field) -> Result<BTreeSet<u32>, CronError> {
    let name = field.name;
    let mut values = BTreeSet::new();
    for term in text.split(',') {
        let (base, step) = match term.split_once('/') {
            Some((base, step)) => (base, Some(step)),
            None => (term, None),
        };
        let (start, end) = if base == "*" {
            (field.min, field.max)
        } else if let Some((a, b)) = base.split_once('-') {
            let (a, b) = (value(a, field)?, value(b, field)?);
            if a > b {
                return Err(error(format!("{name} range {a}-{b} runs backwards")));
            }
            (a, b)
        } else {
            let a = value(base, field)?;
            (a, if step.is_some() { field.max } else { a })
        };
        let step = match step {
            Some(step) => step
                .parse::<usize>()
                .map_err(|_| error(format!("{name} step `{step}` is not a number")))?,
            None => 1,
        };
        if step == 0 {
            return Err(error(format!("{name} step must be at least 1")));
        }
        values.extend((start..=end).step_by(step));
    }
    Ok(values)
}

fn value(text: &str, field: &Field) -> Result<u32, CronError> {
    let Field {
        name,
        min,
        max,
        names,
    } = *field;
    let lower = text.to_ascii_lowercase();
    let named = names
        .iter()
        .zip(min..)
        .find(|(candidate, _)| **candidate == lower)
        .map(|(_, number)| number);
    let number = match named {
        Some(number) => number,
        None => text
            .parse::<u32>()
            .map_err(|_| error(format!("{name} `{text}` is not a number")))?,
    };
    if number < min || number > max {
        return Err(error(format!("{name} {number} is outside {min}-{max}")));
    }
    Ok(number)
}

fn render(values: &BTreeSet<u32>, field: &Field) -> String {
    if (field.min..=field.max).all(|v| values.contains(&v)) {
        return "*".to_owned();
    }
    values
        .iter()
        .map(|v| format!("{v:02}"))
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
#[path = "../../../tests/unit/domain/agent/cron.rs"]
mod tests;
