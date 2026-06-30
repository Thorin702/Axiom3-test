//! The verification supervisor — v1.
//!
//! Runs a batch of async tasks with a concurrency cap, LIVE PROGRESS, and
//! CANCELLATION. This is the small, honest foundation the "dynamic workflow"
//! (many-agent) orchestration grows from: today it supervises the per-claim
//! verification fleet; later it grows task graphs, budgets, and retries.

use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use futures::stream::StreamExt;
use futures::FutureExt;

/// A shared cancel flag handed to a running job. Flipping it stops NEW work from
/// being scheduled (in-flight tasks still finish). Cheap to clone (an `Arc`).
#[derive(Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Run `task` over each item with up to `concurrency` futures in flight, calling
/// `on_progress(done, total)` as each settles. Returns the outputs that completed
/// (≤ `items.len()`).
///
/// Robustness (the orchestration seam everything else runs on):
/// - CANCEL is checked at each task's ENTRY (before its work begins) AND after each
///   completion, so flipping `cancel` skips not-yet-started items immediately —
///   in-flight tasks still finish (or hit their own timeout).
/// - PANIC ISOLATION: a task that panics is caught and DROPPED (fail-closed — it
///   contributes no output), instead of unwinding and destroying the whole batch.
///   This is the isolation the workspace's `panic = unwind` profile exists to enable.
pub async fn run_batch<I, T, F, Fut>(
    items: Vec<I>,
    concurrency: usize,
    cancel: &Cancel,
    task: F,
    mut on_progress: impl FnMut(usize, usize),
) -> Vec<T>
where
    F: Fn(I) -> Fut,
    Fut: std::future::Future<Output = T>,
{
    let total = items.len();
    let mut out = Vec::with_capacity(total);
    let mut stream = futures::stream::iter(items)
        .map(|item| async {
            // Cancel checked at entry: a not-yet-started item short-circuits the
            // moment cancel fires, without ever dispatching its (e.g. network) work.
            if cancel.is_cancelled() {
                return None;
            }
            // Isolate a panicking task so one bad item can't unwind the whole batch.
            AssertUnwindSafe(task(item)).catch_unwind().await.ok()
        })
        .buffer_unordered(concurrency.max(1));
    let mut done = 0;
    while let Some(result) = stream.next().await {
        done += 1;
        if let Some(v) = result {
            out.push(v);
        }
        on_progress(done, total);
        if cancel.is_cancelled() {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[tokio::test]
    async fn runs_all_and_reports_progress() {
        let cancel = Cancel::new();
        let peak = AtomicUsize::new(0);
        let out = run_batch(
            vec![1, 2, 3, 4, 5],
            2,
            &cancel,
            |x| async move { x * 10 },
            |done, total| {
                assert_eq!(total, 5);
                peak.fetch_max(done, Ordering::Relaxed);
            },
        )
        .await;
        assert_eq!(out.len(), 5);
        assert_eq!(peak.load(Ordering::Relaxed), 5, "progress reached the total");
        let mut sorted = out;
        sorted.sort_unstable();
        assert_eq!(sorted, vec![10, 20, 30, 40, 50]);
    }

    #[tokio::test]
    async fn stops_scheduling_after_cancel() {
        let cancel = Cancel::new();
        cancel.cancel(); // pre-cancelled
        let out = run_batch(vec![1, 2, 3, 4, 5, 6], 1, &cancel, |x| async move { x }, |_, _| {}).await;
        assert!(out.len() < 6, "cancellation stopped the batch early: {}", out.len());
    }

    #[tokio::test]
    async fn a_panicking_task_is_isolated_not_fatal() {
        // One item panics; the batch must survive and return the others (fail-closed),
        // not unwind and lose everything.
        let cancel = Cancel::new();
        let out = run_batch(
            vec![1, 2, 3],
            3,
            &cancel,
            |x| async move {
                assert!(x != 2, "boom on 2");
                x * 10
            },
            |_, _| {},
        )
        .await;
        let mut sorted = out;
        sorted.sort_unstable();
        assert_eq!(sorted, vec![10, 30], "the panicking item is dropped, the rest survive");
    }

    #[tokio::test]
    async fn zero_concurrency_is_treated_as_one() {
        let cancel = Cancel::new();
        let out = run_batch(vec![1, 2], 0, &cancel, |x| async move { x }, |_, _| {}).await;
        assert_eq!(out.len(), 2);
    }
}
