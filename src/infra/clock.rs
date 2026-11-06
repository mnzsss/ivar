//! The wall clock and the machine's time zone.

use crate::infra::proc::{Command, capture};

/// Epoch seconds now.
#[must_use]
pub fn now_epoch_secs() -> i64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    i64::try_from(now.as_secs()).unwrap_or(i64::MAX)
}

/// The epoch second of `wall` (`YYYY-MM-DD HH:MM`) read in the machine's time
/// zone, daylight saving included; `None` when `date` cannot place it (a time
/// skipped by a clock change, or an unparseable value). GNU `date -d` is tried
/// first, then the BSD/macOS `date -j -f` form.
#[must_use]
pub fn local_epoch_secs(wall: &str) -> Option<i64> {
    gnu_date(wall).or_else(|| bsd_date(wall))
}

fn gnu_date(wall: &str) -> Option<i64> {
    date_seconds(&Command::new("date").args(["-d", wall, "+%s"]))
}

fn bsd_date(wall: &str) -> Option<i64> {
    // Without `%S`, BSD `date` fills the seconds from the current time.
    let full = format!("{wall}:00");
    date_seconds(&Command::new("date").args(["-j", "-f", "%Y-%m-%d %H:%M:%S", &full, "+%s"]))
}

fn date_seconds(command: &Command) -> Option<i64> {
    let output = capture(command).ok()?;
    if !output.success() {
        return None;
    }
    output.stdout.trim().parse().ok()
}

#[cfg(test)]
#[path = "../../tests/unit/infra/clock.rs"]
mod tests;
