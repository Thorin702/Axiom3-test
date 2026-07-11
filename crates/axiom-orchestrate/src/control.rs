//! Cooperative pause flag for a running task, alongside `axiom_runtime::supervisor::Cancel`.
//! The coding loop checks it each turn and waits while set; cheap to clone (an `Arc`). Also carries
//! an opt-in plan-approval gate: when enabled, the loop pauses ONCE right after the first plan so
//! the user can approve / edit / reject before anything executes (resume = approve; cancel = reject).
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Default)]
struct Inner {
    paused: AtomicBool,
    require_approval: AtomicBool,
}

#[derive(Clone, Default)]
pub struct Pause(Arc<Inner>);

impl Pause {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn pause(&self) {
        self.0.paused.store(true, Ordering::Relaxed);
    }
    pub fn resume(&self) {
        self.0.paused.store(false, Ordering::Relaxed);
    }
    pub fn is_paused(&self) -> bool {
        self.0.paused.load(Ordering::Relaxed)
    }
    /// Turn on plan-approval gating for this run.
    pub fn require_plan_approval(&self) {
        self.0.require_approval.store(true, Ordering::Relaxed);
    }
    pub fn approval_required(&self) -> bool {
        self.0.require_approval.load(Ordering::Relaxed)
    }
}
