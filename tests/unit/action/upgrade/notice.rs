#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use super::*;
use crate::action::upgrade::cache::{self, CacheEntry};
use crate::domain::upgrade::{CHECK_INTERVAL_SECS, NoticeContext, Version};
use crate::error::Failure;
use crate::infra::release::LatestRelease;
use crate::test_support::utf8_temp_dir;

#[derive(Clone)]
struct Fake {
    calls: Arc<AtomicUsize>,
    answer: Result<String, &'static str>,
}

impl LatestRelease for Fake {
    fn latest_location(&self, _timeout: Duration) -> Result<String, Failure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.answer
            .clone()
            .map_err(|code| Failure::failed(code, "fake"))
    }
}

fn fake(answer: Result<&str, &'static str>) -> Fake {
    Fake {
        calls: Arc::new(AtomicUsize::new(0)),
        answer: answer.map(str::to_owned),
    }
}

fn on() -> NoticeContext {
    NoticeContext {
        stderr_tty: true,
        release_build: true,
        ..NoticeContext::default()
    }
}

fn current() -> Version {
    Version::parse("0.12.0").unwrap()
}

const NOW: u64 = 100 * CHECK_INTERVAL_SECS;
const TAG_13: &str = "https://github.com/mnzsss/ivar/releases/tag/v0.13.0";

fn finish(notice: Notice) -> String {
    let mut out = Vec::new();
    notice.finish(&mut out);
    String::from_utf8(out).unwrap()
}

#[test]
fn a_fresh_cache_with_a_newer_version_prints_one_line_and_never_calls_out() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join("update-check.json");
    cache::write(
        &path,
        &CacheEntry {
            last_checked_at: NOW - 1,
            latest_version: Some("0.13.0".to_owned()),
        },
    );
    let source = fake(Ok(TAG_13));
    let calls = Arc::clone(&source.calls);

    let out = finish(Notice::start_with(
        &on(),
        source,
        Some(path),
        NOW,
        current(),
        Duration::from_secs(5),
    ));

    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(out.lines().count(), 1, "{out}");
    assert!(
        out.contains("0.13.0") && out.contains("ivar upgrade"),
        "{out}"
    );
}

#[test]
fn a_missing_cache_prints_nothing_now_and_records_the_latest_for_next_time() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join("update-check.json");
    let source = fake(Ok(TAG_13));

    let out = finish(Notice::start_with(
        &on(),
        source,
        Some(path.clone()),
        NOW,
        current(),
        Duration::from_secs(5),
    ));

    assert_eq!(
        out, "",
        "the notice is decided from the cache read before the check"
    );
    assert_eq!(
        cache::read(&path),
        Some(CacheEntry {
            last_checked_at: NOW,
            latest_version: Some("0.13.0".to_owned())
        })
    );
}

#[test]
fn a_failed_check_still_claims_the_interval_and_keeps_the_old_answer() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join("update-check.json");
    cache::write(
        &path,
        &CacheEntry {
            last_checked_at: 0,
            latest_version: Some("0.12.5".to_owned()),
        },
    );

    let out = finish(Notice::start_with(
        &on(),
        fake(Err("release.request_failed")),
        Some(path.clone()),
        NOW,
        current(),
        Duration::from_secs(5),
    ));

    assert!(out.contains("0.12.5"), "stale answer still shown: {out}");
    assert_eq!(
        cache::read(&path),
        Some(CacheEntry {
            last_checked_at: NOW,
            latest_version: Some("0.12.5".to_owned())
        })
    );
}

#[test]
fn a_disabled_context_neither_prints_nor_calls_nor_writes() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join("update-check.json");
    let source = fake(Ok(TAG_13));
    let calls = Arc::clone(&source.calls);
    let off = NoticeContext {
        opted_out: true,
        ..on()
    };

    let out = finish(Notice::start_with(
        &off,
        source,
        Some(path.clone()),
        NOW,
        current(),
        Duration::from_secs(5),
    ));

    assert_eq!(out, "");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(cache::read(&path), None);
}

#[test]
fn no_cache_location_means_no_check() {
    let source = fake(Ok(TAG_13));
    let calls = Arc::clone(&source.calls);

    let out = finish(Notice::start_with(
        &on(),
        source,
        None,
        NOW,
        current(),
        Duration::from_secs(5),
    ));

    assert_eq!(out, "");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn finish_gives_up_at_the_deadline() {
    struct Slow;
    impl LatestRelease for Slow {
        fn latest_location(&self, _timeout: Duration) -> Result<String, Failure> {
            std::thread::sleep(Duration::from_secs(5));
            Ok(TAG_13.to_owned())
        }
    }
    let (_dir, root) = utf8_temp_dir();
    let started = std::time::Instant::now();

    let _ = finish(Notice::start_with(
        &on(),
        Slow,
        Some(root.join("c.json")),
        NOW,
        current(),
        Duration::from_millis(50),
    ));

    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
}
