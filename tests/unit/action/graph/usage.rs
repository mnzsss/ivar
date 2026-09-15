#![allow(clippy::unwrap_used)]

use super::*;
use crate::action::graph::outcome::{DeadCodeOutcome, HierarchyOutcome, PathOutcome};
use crate::domain::graph::UsageSource;

#[test]
fn an_empty_list_outcome_counts_zero_results() {
    assert_eq!(DeadCodeOutcome(Vec::new()).result_count(), Some(0));
}

#[test]
fn a_missing_optional_outcome_counts_zero_results() {
    assert_eq!(PathOutcome(None).result_count(), Some(0));
    assert_eq!(HierarchyOutcome(None).result_count(), Some(0));
}

#[test]
fn recording_outside_a_hall_does_not_panic() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = Ctx::new(camino::Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap());
    record_usage(
        &ctx,
        &UsageEvent {
            command: "find".to_owned(),
            source: UsageSource::Cli,
            duration_ms: 1,
            result_count: Some(0),
            error: false,
        },
    );
}
