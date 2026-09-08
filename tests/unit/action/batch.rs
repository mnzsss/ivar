//! Unit tests for `crate::action::batch` — bounded parallel executor.
//!
//! Physically located here but compiled inside the library crate via `#[path]`
//! so `use super::*` reaches private parent items.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;

#[test]
fn bounded_map_preserves_input_order() {
    let inputs = vec![10, 20, 30, 40, 50];
    let results = bounded_map(&inputs, 2, |&x| Ok(x * 2));
    let values: Vec<i32> = results.into_iter().map(|r| r.unwrap()).collect();
    assert_eq!(values, vec![20, 40, 60, 80, 100]);
}

#[test]
fn bounded_map_respects_concurrency_limit() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    let active = Arc::new(AtomicUsize::new(0));
    let max_active = Arc::new(AtomicUsize::new(0));
    let inputs: Vec<usize> = (0..8).collect();

    let active_clone = Arc::clone(&active);
    let max_clone = Arc::clone(&max_active);

    let _ = bounded_map(&inputs, 3, move |_| {
        let current = active_clone.fetch_add(1, Ordering::SeqCst) + 1;
        max_clone.fetch_max(current, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(20));
        active_clone.fetch_sub(1, Ordering::SeqCst);
        Ok(())
    });

    assert!(max_active.load(Ordering::SeqCst) <= 3);
}

#[test]
fn bounded_map_captures_worker_panic_as_failure() {
    let inputs = vec![1, 2, 3];
    let results = bounded_map(&inputs, 2, |&x| {
        if x == 2 {
            panic!("simulated panic");
        }
        Ok(x * 10)
    });
    assert_eq!(results[0].as_ref().unwrap(), &10);
    assert!(results[1].is_err());
    assert_eq!(results[1].as_ref().unwrap_err().code, "batch.worker_panic");
    assert_eq!(results[2].as_ref().unwrap(), &30);
}

#[test]
fn bounded_map_handles_empty_slice() {
    let inputs: Vec<i32> = vec![];
    let results = bounded_map(&inputs, 4, |&x| Ok(x * 2));
    assert!(results.is_empty());
}

#[test]
fn bounded_map_handles_zero_limit_as_sequential() {
    let inputs = vec![1, 2, 3];
    let results = bounded_map(&inputs, 0, |&x| Ok(x + 1));
    let values: Vec<i32> = results.into_iter().map(|r| r.unwrap()).collect();
    assert_eq!(values, vec![2, 3, 4]);
}

#[test]
fn test_run_feature_batch_runs_all_targets_even_if_one_fails() {
    use crate::error::Report;

    let features = vec!["feat-1".to_string(), "feat-2".to_string(), "feat-3".to_string()];
    let results = run_feature_batch(&features, 2, |feat| {
        if feat == "feat-2" {
            Err(Failure::failed("mock.fail", "failed for feat-2"))
        } else {
            Ok(Report::new(format!("success for {feat}")))
        }
    });

    assert_eq!(results.len(), 3);
    assert_eq!(results[0].feature, "feat-1");
    assert!(results[0].outcome.is_ok());
    assert_eq!(results[0].outcome.as_ref().unwrap().value, "success for feat-1");

    assert_eq!(results[1].feature, "feat-2");
    assert!(results[1].outcome.is_err());
    assert_eq!(results[1].outcome.as_ref().unwrap_err().code, "mock.fail");

    assert_eq!(results[2].feature, "feat-3");
    assert!(results[2].outcome.is_ok());
    assert_eq!(results[2].outcome.as_ref().unwrap().value, "success for feat-3");
}

#[test]
fn test_run_feature_batch_preserves_order() {
    use crate::error::Report;

    let features = vec!["z".to_string(), "a".to_string(), "m".to_string()];
    let results = run_feature_batch(&features, 4, |feat| {
        Ok(Report::new(feat.to_uppercase()))
    });
    let keys: Vec<String> = results.into_iter().map(|r| r.feature).collect();
    assert_eq!(keys, vec!["z", "a", "m"]);
}
