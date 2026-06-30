//! The verification supervisor.
//!
//! Runs batches of async tasks with a concurrency cap, LIVE PROGRESS, CANCELLATION,
//! panic isolation, and per-activity RETRY (exponential backoff with retryable-error
//! classification). This is the honest foundation the "dynamic workflow" (many-agent)
//! orchestration grows from: today it supervises the per-claim verification fleet;
//! later it grows task graphs, budgets, idempotency, and event-replay.

use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

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

/// Exponential-backoff retry policy for a single retryable activity (one provider
/// call). Conservative by default — a verification fleet should recover a transient
/// blip, not hammer a vendor: 2 retries, 0.5s base doubling to a 8s cap.
#[derive(Clone, Copy, Debug)]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self { max_retries: 2, base_delay_ms: 500, max_delay_ms: 8_000 }
    }
}

impl RetryPolicy {
    /// Backoff before attempt `attempt` (0-based): `base * 2^attempt`, capped.
    fn delay(&self, attempt: u32) -> Duration {
        let factor = 1u64.checked_shl(attempt).unwrap_or(u64::MAX);
        let ms = self.base_delay_ms.saturating_mul(factor).min(self.max_delay_ms);
        Duration::from_millis(ms)
    }
}

/// Best-effort classification of a transient, retryable provider error. Retry only
/// what a moment's wait can fix — rate limits (429), upstream gateway 5xx, transient
/// network faults. A TIMEOUT is NOT retried (the call is already bounded; retrying just
/// doubles the wait), and a 4xx / content-policy / unknown-provider error fails fast.
///
/// Matching is on the error STRING (rig surfaces provider errors as text), so the
/// markers are deliberately SPECIFIC multi-word phrases rather than bare 3-digit codes,
/// to avoid retrying a hard 4xx whose response body merely happens to mention "500" or
/// "connection". Over-retrying is bounded anyway (it still ends fail-closed), but
/// specificity keeps wasted calls down.
pub fn is_retryable_error(e: &anyhow::Error) -> bool {
    let s = e.to_string().to_lowercase();
    // OUR own bounded client timeout fast-fails (retrying just doubles the 45s wait).
    // Matched specifically ("timed out after") so a provider-side "504 gateway timeout"
    // — a genuinely transient upstream fault — is NOT swept up by it.
    if s.contains("timed out after") {
        return false;
    }
    // SPECIFIC transient markers. Bare "500" is omitted on purpose (it shows up in 4xx
    // bodies — token counts, ids); HTTP 500 is matched via "internal server error".
    const RETRYABLE: &[&str] = &[
        "429",
        "too many requests",
        "rate limit",
        "rate-limit",
        "overloaded",
        "service unavailable",
        "internal server error",
        "bad gateway",
        "gateway timeout",
        "temporarily unavailable",
        "502",
        "503",
        "504",
        "connection reset",
        "connection refused",
        "connection closed",
        "reset by peer",
        "broken pipe",
    ];
    RETRYABLE.iter().any(|m| s.contains(m))
}

/// Run a fallible async activity, retrying while the error is [`is_retryable_error`]
/// (or a caller-supplied classifier) up to `policy.max_retries`, with exponential
/// backoff between attempts. Returns the value on success, or the LAST error once
/// attempts are exhausted or the error is non-retryable.
pub async fn retry_async<T, F, Fut>(
    policy: &RetryPolicy,
    is_retryable: impl Fn(&anyhow::Error) -> bool,
    mut op: F,
) -> anyhow::Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = anyhow::Result<T>>,
{
    let mut attempt = 0u32;
    loop {
        match op().await {
            Ok(v) => return Ok(v),
            Err(e) => {
                if attempt >= policy.max_retries || !is_retryable(&e) {
                    return Err(e);
                }
                tokio::time::sleep(policy.delay(attempt)).await;
                attempt += 1;
            }
        }
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
    async fn retry_recovers_a_transient_failure() {
        // Fails twice with a retryable error, then succeeds → returns Ok.
        let calls = AtomicUsize::new(0);
        let policy = RetryPolicy { max_retries: 2, base_delay_ms: 0, max_delay_ms: 0 };
        let r: anyhow::Result<&str> = retry_async(&policy, is_retryable_error, || {
            let n = calls.fetch_add(1, Ordering::Relaxed);
            async move {
                if n < 2 {
                    Err(anyhow::anyhow!("HTTP 429 rate limit exceeded"))
                } else {
                    Ok("ok")
                }
            }
        })
        .await;
        assert_eq!(r.unwrap(), "ok");
        assert_eq!(calls.load(Ordering::Relaxed), 3, "two retries then success");
    }

    #[tokio::test]
    async fn retry_gives_up_after_max_attempts() {
        let calls = AtomicUsize::new(0);
        let policy = RetryPolicy { max_retries: 2, base_delay_ms: 0, max_delay_ms: 0 };
        let r: anyhow::Result<()> = retry_async(&policy, is_retryable_error, || {
            calls.fetch_add(1, Ordering::Relaxed);
            async move { Err(anyhow::anyhow!("503 service unavailable")) }
        })
        .await;
        assert!(r.is_err());
        assert_eq!(calls.load(Ordering::Relaxed), 3, "initial + 2 retries, then give up");
    }

    #[tokio::test]
    async fn retry_does_not_retry_a_fast_fail_error() {
        // A non-retryable (4xx / unknown) error returns immediately — no retries.
        let calls = AtomicUsize::new(0);
        let policy = RetryPolicy::default();
        let r: anyhow::Result<()> = retry_async(&policy, is_retryable_error, || {
            calls.fetch_add(1, Ordering::Relaxed);
            async move { Err(anyhow::anyhow!("HTTP 400 invalid request")) }
        })
        .await;
        assert!(r.is_err());
        assert_eq!(calls.load(Ordering::Relaxed), 1, "fast-fail: no retries");
    }

    #[test]
    fn error_classification_is_conservative() {
        assert!(is_retryable_error(&anyhow::anyhow!("HTTP 429 Too Many Requests")));
        assert!(is_retryable_error(&anyhow::anyhow!("upstream 503")));
        assert!(is_retryable_error(&anyhow::anyhow!("connection reset by peer")));
        // OUR bounded client timeout is NOT retried…
        assert!(!is_retryable_error(&anyhow::anyhow!("verifier timed out after 45s")));
        // …but a provider-side 504 gateway timeout IS (transient upstream).
        assert!(is_retryable_error(&anyhow::anyhow!("504 gateway timeout")));
        // Client/content errors fail fast.
        assert!(!is_retryable_error(&anyhow::anyhow!("HTTP 401 unauthorized")));
        assert!(!is_retryable_error(&anyhow::anyhow!("unknown provider: foo")));
        // A hard 4xx whose BODY merely mentions a code/word must NOT be retried
        // (specificity: bare "500"/"connection" are not markers).
        assert!(!is_retryable_error(&anyhow::anyhow!("HTTP 400: prompt exceeded 500 tokens")));
        assert!(!is_retryable_error(&anyhow::anyhow!("HTTP 400: invalid connection parameter")));
    }

    #[tokio::test]
    async fn zero_concurrency_is_treated_as_one() {
        let cancel = Cancel::new();
        let out = run_batch(vec![1, 2], 0, &cancel, |x| async move { x }, |_, _| {}).await;
        assert_eq!(out.len(), 2);
    }
}
