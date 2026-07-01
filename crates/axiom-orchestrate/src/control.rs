//! Cooperative pause flag for a running task, alongside `axiom_runtime::supervisor::Cancel`.
//! The coding loop checks it each turn and waits while set; cheap to clone (an `Arc`).
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Clone, Default)]
pub struct Pause(Arc<AtomicBool>);

impl Pause {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn pause(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    pub fn resume(&self) {
        self.0.store(false, Ordering::Relaxed);
    }
    pub fn is_paused(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}
