//! The `at:` of an Agent Task: one absolute minute, run once.

use std::fmt;

use crate::domain::session::{epoch_secs_from_civil, utc_calendar_from_epoch_secs};

/// Read as local time when it has no offset, else at its own offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtTime {
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    /// Minutes east of UTC; `None` is local time.
    offset: Option<i64>,
}

const MAX_OFFSET_MINUTES: i64 = 14 * 60;

fn digits(text: &str, from: usize, to: usize) -> Option<i64> {
    let part = text.get(from..to)?;
    if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    part.parse().ok()
}

fn separator(text: &str, at: usize, expected: char) -> Option<()> {
    (text.get(at..=at)? == expected.to_string()).then_some(())
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn offset_of(tail: &str) -> Option<Option<i64>> {
    if tail.is_empty() {
        return Some(None);
    }
    if tail == "Z" {
        return Some(Some(0));
    }
    let sign = match tail.get(..1)? {
        "+" => 1,
        "-" => -1,
        _ => return None,
    };
    separator(tail, 3, ':')?;
    if tail.len() != 6 {
        return None;
    }
    let (hours, minutes) = (digits(tail, 1, 3)?, digits(tail, 4, 6)?);
    let total = hours * 60 + minutes;
    (minutes < 60 && total <= MAX_OFFSET_MINUTES).then_some(Some(sign * total))
}

fn is_relative(text: &str) -> bool {
    ['s', 'm', 'h', 'd', 'w'].iter().any(|unit| {
        text.strip_suffix(*unit)
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    })
}

impl AtTime {
    /// `YYYY-MM-DDTHH:MM`, optionally followed by `Z` or `+HH:MM` / `-HH:MM`.
    ///
    /// # Errors
    ///
    /// A reason when `text` is relative (`2d`), has seconds, or is not a real
    /// calendar minute.
    pub fn parse(text: &str) -> Result<Self, String> {
        if is_relative(text) {
            return Err(format!(
                "`{text}` is relative; write an absolute time like 2026-11-08T09:00 or 2026-11-08T09:00-03:00"
            ));
        }
        let malformed = || {
            format!(
                "`{text}` is not an absolute time like 2026-11-08T09:00 or 2026-11-08T09:00-03:00 (minute precision)"
            )
        };
        let parsed = (|| {
            separator(text, 4, '-')?;
            separator(text, 7, '-')?;
            separator(text, 10, 'T')?;
            separator(text, 13, ':')?;
            Some(Self {
                year: digits(text, 0, 4)?,
                month: digits(text, 5, 7)?,
                day: digits(text, 8, 10)?,
                hour: digits(text, 11, 13)?,
                minute: digits(text, 14, 16)?,
                offset: offset_of(text.get(16..)?)?,
            })
        })()
        .ok_or_else(malformed)?;
        let real = (1970..=9999).contains(&parsed.year)
            && (1..=12).contains(&parsed.month)
            && (1..=days_in_month(parsed.year, parsed.month)).contains(&parsed.day)
            && parsed.hour < 24
            && parsed.minute < 60;
        if real {
            Ok(parsed)
        } else {
            Err(format!("`{text}` is not a real calendar time"))
        }
    }

    /// Whether the time carries its own offset.
    #[must_use]
    pub const fn has_offset(&self) -> bool {
        self.offset.is_some()
    }

    fn civil_secs(&self) -> i64 {
        epoch_secs_from_civil(self.year, self.month, self.day, self.hour, self.minute)
    }

    /// `YYYY-MM-DD HH:MM`, the wall clock as written.
    #[must_use]
    pub fn wall(&self) -> String {
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute
        )
    }

    /// `YYYY-MM-DDTHHMM`, the stamp in a concluded task's file name.
    #[must_use]
    pub fn file_stamp(&self) -> String {
        format!(
            "{:04}-{:02}-{:02}T{:02}{:02}",
            self.year, self.month, self.day, self.hour, self.minute
        )
    }

    /// The systemd `OnCalendar=` of this time: local as written, or the same
    /// instant in UTC when the time has an offset.
    #[must_use]
    pub fn on_calendar(&self) -> String {
        match self.offset {
            None => format!("{}:00", self.wall()),
            Some(offset) => format!(
                "{} UTC",
                utc_calendar_from_epoch_secs(self.civil_secs() - offset * 60)
            ),
        }
    }

    /// Epoch seconds; `local` places a time without an offset in this machine's zone.
    #[must_use]
    pub fn epoch_secs(&self, local: &dyn Fn(&str) -> Option<i64>) -> Option<i64> {
        match self.offset {
            Some(offset) => Some(self.civil_secs() - offset * 60),
            None => local(&self.wall()),
        }
    }
}

impl fmt::Display for AtTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:04}-{:02}-{:02}T{:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute
        )?;
        match self.offset {
            None => Ok(()),
            Some(0) => f.write_str("Z"),
            Some(offset) => write!(
                f,
                "{}{:02}:{:02}",
                if offset < 0 { '-' } else { '+' },
                offset.abs() / 60,
                offset.abs() % 60
            ),
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/domain/agent/at.rs"]
mod tests;
