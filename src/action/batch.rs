//! Standard-library bounded parallel executor with order preservation and panic safety.

use std::panic::{catch_unwind, AssertUnwindSafe, RefUnwindSafe};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;

use crate::error::Failure;

/// Maps a function `f` over `items` in parallel with a bounded concurrency `limit`.
///
/// Returns results matching input slice order exactly.
/// Worker panics are caught with `std::panic::catch_unwind` and translated into
/// `Err(Failure::failed("batch.worker_panic", ...))` without leaking threads.
pub fn bounded_map<T, R, F>(
    items: &[T],
    limit: usize,
    f: F,
) -> Vec<Result<R, Failure>>
where
    T: Sync,
    R: Send,
    F: Fn(&T) -> Result<R, Failure> + Sync + RefUnwindSafe,
{
    if items.is_empty() {
        return Vec::new();
    }

    // Default concurrency or 0 is handled: 0 fallback to 1 (sequential)
    let concurrency = limit.clamp(1, items.len());

    let (tx, rx) = mpsc::channel::<(usize, Result<R, Failure>)>();
    let next_idx = Arc::new(AtomicUsize::new(0));

    thread::scope(|s| {
        for _ in 0..concurrency {
            let tx = tx.clone();
            let next_idx = Arc::clone(&next_idx);
            let f = &f;

            s.spawn(move || {
                loop {
                    let idx = next_idx.fetch_add(1, Ordering::SeqCst);
                    if idx >= items.len() {
                        break;
                    }

                    let item = &items[idx];
                    let outcome = match catch_unwind(AssertUnwindSafe(|| f(item))) {
                        Ok(res) => res,
                        Err(payload) => {
                            let panic_msg = if let Some(msg) = payload.downcast_ref::<&str>() {
                                msg.to_string()
                            } else if let Some(msg) = payload.downcast_ref::<String>() {
                                msg.clone()
                            } else {
                                "unknown panic in batch worker".to_string()
                            };
                            Err(Failure::failed(
                                "batch.worker_panic",
                                format!("worker panicked on item {idx}: {panic_msg}"),
                            ))
                        }
                    };

                    let _ = tx.send((idx, outcome));
                }
            });
        }

        // Drop the primary transmitter so rx closes when all workers finish
        drop(tx);

        let mut indexed_results = Vec::with_capacity(items.len());
        while let Ok((idx, result)) = rx.recv() {
            indexed_results.push((idx, result));
        }

        indexed_results.sort_by_key(|(idx, _)| *idx);
        indexed_results.into_iter().map(|(_, res)| res).collect()
    })
}

#[cfg(test)]
#[path = "../../tests/unit/action/batch.rs"]
mod tests;
