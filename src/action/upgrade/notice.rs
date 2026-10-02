//! The non-blocking update notice check.

use std::io;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use camino::Utf8PathBuf;

use super::cache::{self, CacheEntry};
use crate::domain::upgrade::{
    NoticeContext, Version, is_stale, notice_enabled, notice_line, tag_from_location,
};
use crate::infra::release::{GithubRelease, LatestRelease};

/// Maximum time to wait on background release check during shutdown.
pub const NOTICE_BUDGET: Duration = Duration::from_millis(800);

/// An in-flight or completed update notice check.
#[derive(Debug)]
pub struct Notice {
    line: Option<String>,
    pending: Option<Receiver<()>>,
    deadline: Instant,
}

const BUILD_VERSION: &str = env!("IVAR_BUILD_VERSION");

/// The running build's version parsed into domain `Version`, or `None` if unparseable.
pub fn current_version() -> Option<Version> {
    Version::parse(BUILD_VERSION)
}

/// Whether this binary was built from a local checkout rather than released.
pub fn is_dev_build() -> bool {
    BUILD_VERSION.contains("-dev+")
}

impl Notice {
    fn off() -> Self {
        Self {
            line: None,
            pending: None,
            deadline: Instant::now(),
        }
    }

    /// Start the update notice check for the current process.
    pub fn start(ctx: &NoticeContext) -> Self {
        match current_version() {
            Some(current) => Self::start_with(
                ctx,
                GithubRelease,
                cache::cache_path(),
                cache::now_secs(),
                current,
                NOTICE_BUDGET,
            ),
            None => Self::off(),
        }
    }

    /// Start the update notice check with injected dependencies.
    pub fn start_with<S: LatestRelease + Send + 'static>(
        ctx: &NoticeContext,
        source: S,
        cache: Option<Utf8PathBuf>,
        now: u64,
        current: Version,
        budget: Duration,
    ) -> Self {
        if !notice_enabled(ctx) {
            return Self::off();
        }
        let Some(path) = cache else {
            return Self::off();
        };
        let entry = cache::read(&path);
        let previous = entry.as_ref().and_then(|e| e.latest_version.clone());
        let line = previous
            .as_deref()
            .and_then(Version::parse)
            .and_then(|latest| notice_line(current, latest));

        let deadline = Instant::now() + budget;
        if !is_stale(entry.as_ref().map(|e| e.last_checked_at), now) {
            return Self {
                line,
                pending: None,
                deadline,
            };
        }
        // Claim the interval before touching the network: a failing or
        // sandboxed check must not retry on every command.
        let claim = CacheEntry {
            last_checked_at: now,
            latest_version: previous,
        };
        if !cache::write(&path, &claim) {
            return Self {
                line,
                pending: None,
                deadline,
            };
        }
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            if let Some(latest) = source
                .latest_location(budget)
                .ok()
                .and_then(|location| tag_from_location(&location))
            {
                let _ = cache::write(
                    &path,
                    &CacheEntry {
                        last_checked_at: now,
                        latest_version: Some(latest.to_string()),
                    },
                );
            }
            let _ = tx.send(());
        });
        Self {
            line,
            pending: Some(rx),
            deadline,
        }
    }

    /// Finish the check, waiting up to the deadline if a background check is pending,
    /// and write the notice line to `stderr` if applicable.
    pub fn finish(self, stderr: &mut impl io::Write) {
        if let Some(rx) = self.pending {
            let _ = rx.recv_timeout(self.deadline.saturating_duration_since(Instant::now()));
        }
        if let Some(line) = self.line {
            let _ = writeln!(stderr, "{line}");
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/action/upgrade/notice.rs"]
mod tests;
