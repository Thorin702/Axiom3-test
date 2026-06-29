//! The verification supervisor — v1.
//!
//! Runs a batch of async tasks with a concurrency cap, LIVE PROGRESS, and
//! CANCELLATION. This is the small, honest foundation the "dynamic workflow"
//! (many-agent) orchestration grows from: today it supervises the per-claim
//! verification fleet; later it grows task graphs, budgets, and retries.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use futures::stream::StreamExt;

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
/// `on_progress(done, total)` as each completes. Stops scheduling once `cancel`
/// fires (in-flight tasks still finish). Returns the outputs that completed
/// (≤ `items.len()` if cancelled).
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
        .map(task)
        .buffer_unordered(concurrency.max(1));
    let mut done = 0;
    while let Some(result) = stream.next().await {
        out.push(result);
        done += 1;
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
    async fn zero_concurrency_is_treated_as_one() {
        let cancel = Cancel::new();
        let out = run_batch(vec![1, 2], 0, &cancel, |x| async move { x }, |_, _| {}).await;
        assert_eq!(out.len(), 2);
    }
}
