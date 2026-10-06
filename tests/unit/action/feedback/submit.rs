//! Unit tests for `crate::action::feedback::submit`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::str_to_string
)]

use super::*;
use crate::action::Ctx;
use crate::action::confirm::fixed_interactive;
use crate::domain::feedback::{FeedbackEntry, FeedbackKind, FeedbackStatus, Frontmatter};
use crate::error::Failure;
use crate::store::layout::Layout;
use crate::test_support::seeded_hall;
use std::sync::Mutex;

struct MockGh {
    login_result: Result<String, Failure>,
    create_result: Result<String, Failure>,
    invocations: Mutex<Vec<(Vec<String>, String)>>,
}

impl MockGh {
    fn success(url: &str) -> Self {
        Self {
            login_result: Ok("testuser".to_owned()),
            create_result: Ok(url.to_owned()),
            invocations: Mutex::new(Vec::new()),
        }
    }

    fn unauthenticated() -> Self {
        Self {
            login_result: Err(Failure::blocked(
                "github.gh_unauthenticated",
                "not logged in",
            )),
            create_result: Err(Failure::blocked(
                "github.gh_unauthenticated",
                "not logged in",
            )),
            invocations: Mutex::new(Vec::new()),
        }
    }
}

impl GhClient for MockGh {
    fn gh_login(&self) -> Result<String, Failure> {
        self.login_result.clone()
    }

    fn gh_issue_create(&self, repo: &str, title: &str, body: &str) -> Result<String, Failure> {
        self.invocations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((
                vec![
                    "issue".into(),
                    "create".into(),
                    "--repo".into(),
                    repo.into(),
                    "--title".into(),
                    title.into(),
                ],
                body.into(),
            ));
        self.create_result.clone()
    }
}

fn seed_entry(layout: &Layout, id: &str, status: FeedbackStatus, body: &str) -> FeedbackEntry {
    let entry = FeedbackEntry {
        id: id.to_owned(),
        frontmatter: Frontmatter {
            title: "Test feedback title".to_owned(),
            kind: FeedbackKind::Bug,
            status,
            created_at: "2026-10-05T12:00:00Z".to_owned(),
            ivar_version: "0.14.0".to_owned(),
            os: "linux".to_owned(),
            arch: "x86_64".to_owned(),
            provider: Some("omp".to_owned()),
            session: Some("ses-123".to_owned()),
            feature: Some("my-feature".to_owned()),
            published_url: if status == FeedbackStatus::Published {
                Some("https://github.com/mnzsss/ivar/issues/42".to_owned())
            } else {
                None
            },
            extra: std::collections::BTreeMap::new(),
        },
        body: body.to_owned(),
    };
    crate::store::feedback::write(layout, &entry).unwrap();
    entry
}

#[test]
fn refuses_submission_when_confirm_is_non_interactive() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    seed_entry(&layout, "001-test", FeedbackStatus::Open, "body");
    let ctx = Ctx::new(root).with_confirm(crate::action::confirm::reporter(false));
    let gh = MockGh::success("https://github.com/mnzsss/ivar/issues/1");

    let err = submit_with(
        &ctx,
        SubmitInput {
            id: "001-test".into(),
            repo: None,
        },
        &gh,
    )
    .unwrap_err();
    assert_eq!(err.code, "feedback.submit_needs_terminal");
    assert!(
        gh.invocations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
    );
}

#[test]
fn declined_confirm_leaves_file_identical_and_reports_declined() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let _before = seed_entry(
        &layout,
        "001-test",
        FeedbackStatus::Open,
        "some private /home/user path",
    );
    let ctx = Ctx::new(root).with_confirm(fixed_interactive(false));
    let gh = MockGh::success("https://github.com/mnzsss/ivar/issues/1");

    let report = submit_with(
        &ctx,
        SubmitInput {
            id: "001-test".into(),
            repo: None,
        },
        &gh,
    )
    .unwrap();
    assert_eq!(
        report.value,
        SubmitReport::Declined {
            id: "001-test".into()
        }
    );

    let after = crate::store::feedback::read(&layout, "001-test")
        .unwrap()
        .unwrap();
    assert_eq!(after.frontmatter.status, FeedbackStatus::Open);
    assert_eq!(after.frontmatter.published_url, None);
    assert!(
        gh.invocations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
    );
}

#[test]
fn publishes_via_gh_and_updates_status_and_published_url() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    seed_entry(&layout, "001-test", FeedbackStatus::Open, "bug details");
    let ctx = Ctx::new(root).with_confirm(fixed_interactive(true));
    let gh = MockGh::success("https://github.com/mnzsss/ivar/issues/99");

    let report = submit_with(
        &ctx,
        SubmitInput {
            id: "001-test".into(),
            repo: None,
        },
        &gh,
    )
    .unwrap();
    assert_eq!(
        report.value,
        SubmitReport::Published {
            id: "001-test".into(),
            url: "https://github.com/mnzsss/ivar/issues/99".into()
        }
    );

    let updated = crate::store::feedback::read(&layout, "001-test")
        .unwrap()
        .unwrap();
    assert_eq!(updated.frontmatter.status, FeedbackStatus::Published);
    assert_eq!(
        updated.frontmatter.published_url,
        Some("https://github.com/mnzsss/ivar/issues/99".into())
    );
}

#[test]
fn falls_back_to_prefilled_url_when_gh_unauthenticated() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    seed_entry(&layout, "001-test", FeedbackStatus::Open, "short body");
    let ctx = Ctx::new(root).with_confirm(fixed_interactive(true));
    let gh = MockGh::unauthenticated();

    let report = submit_with(
        &ctx,
        SubmitInput {
            id: "001-test".into(),
            repo: None,
        },
        &gh,
    )
    .unwrap();
    match report.value {
        SubmitReport::Prefilled { id, url, body } => {
            assert_eq!(id, "001-test");
            assert!(url.starts_with("https://github.com/mnzsss/ivar/issues/new?title="));
            assert!(url.contains("&body="));
            assert_eq!(body, None);
        }
        other => panic!("expected Prefilled, got {other:?}"),
    }

    let kept = crate::store::feedback::read(&layout, "001-test")
        .unwrap()
        .unwrap();
    assert_eq!(kept.frontmatter.status, FeedbackStatus::Open);
}

#[test]
fn falls_back_to_title_only_url_when_body_exceeds_url_limit() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let long_body = "x".repeat(9000);
    seed_entry(&layout, "001-test", FeedbackStatus::Open, &long_body);
    let ctx = Ctx::new(root).with_confirm(fixed_interactive(true));
    let gh = MockGh::unauthenticated();

    let report = submit_with(
        &ctx,
        SubmitInput {
            id: "001-test".into(),
            repo: None,
        },
        &gh,
    )
    .unwrap();
    match report.value {
        SubmitReport::Prefilled { id, url, body } => {
            assert_eq!(id, "001-test");
            assert!(url.starts_with("https://github.com/mnzsss/ivar/issues/new?title="));
            assert!(!url.contains("&body="));
            assert!(body.is_some());
        }
        other => panic!("expected Prefilled with split body, got {other:?}"),
    }
}

#[test]
fn refuses_submitting_already_published_entry() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    seed_entry(
        &layout,
        "001-test",
        FeedbackStatus::Published,
        "already published",
    );
    let ctx = Ctx::new(root).with_confirm(fixed_interactive(true));
    let gh = MockGh::success("https://github.com/mnzsss/ivar/issues/1");

    let err = submit_with(
        &ctx,
        SubmitInput {
            id: "001-test".into(),
            repo: None,
        },
        &gh,
    )
    .unwrap_err();
    assert_eq!(err.code, "feedback.already_published");
    assert!(
        err.what
            .contains("https://github.com/mnzsss/ivar/issues/42")
    );
}
