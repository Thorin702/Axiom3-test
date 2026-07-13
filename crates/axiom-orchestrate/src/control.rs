//! Cooperative pause flag for a running task, alongside `axiom_runtime::supervisor::Cancel`.
//! The coding loop checks it each turn and waits while set; cheap to clone (an `Arc`). Also carries
//! an opt-in plan-approval gate: when enabled, the loop pauses ONCE right after the first plan so
//! the user can approve / edit / reject before anything executes (resume = approve; cancel = reject).
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Inner {
    paused: AtomicBool,
    require_approval: AtomicBool,
    // When set, the `ask_user` tool does NOT park — it returns a proceed-on-defaults observation
    // immediately. Used by headless/secondary steps (e.g. figure recomputation) that have no UI to
    // surface a card and whose pause is not the one the `answer_ask` command can reach.
    asks_disabled: AtomicBool,
    // When set, the coding loop FORCES the first action of an analysis run to be an `ask_user` intake
    // card — GLM (and weaker models) won't ask on their own discretion even when the prompt demands
    // it, so without this the clarifying intake never fires. See the intake gate in coding::run.
    require_intake: AtomicBool,
    // When set, the `delegate` tool is available (deep-research mode, depth 0): the agent can spawn a
    // bounded multi-agent sub-workflow for a complex subtask. Off by default so analysis stays single-
    // agent and sub-agents can't recurse (a sub-run never sets it).
    delegate_allowed: AtomicBool,
    // When set, the coding loop NUDGES the model to open a `delegate` sub-team a few steps in — GLM
    // won't call this discretionary meta-tool on its own judgment (same as `ask_user`), so without a
    // bounded push the multi-agent capability never actually fires on a complex research topic.
    encourage_delegate: AtomicBool,
    // Answer channel for the `ask_user` tool: the parked tool polls `take_answer`; the UI fills it
    // via the `answer_ask` command. One slot is enough — asks are strictly one-at-a-time per run.
    answer: Mutex<Option<String>>,
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
    /// Deliver the human's answer to a parked `ask_user` tool call.
    pub fn set_answer(&self, answer: String) {
        *self.0.answer.lock().unwrap_or_else(|p| p.into_inner()) = Some(answer);
    }
    /// The parked tool polls this; returns Some once the human has answered (and clears the slot).
    pub fn take_answer(&self) -> Option<String> {
        self.0.answer.lock().unwrap_or_else(|p| p.into_inner()).take()
    }
    /// Disable interactive `ask_user` parking for this run (headless/secondary steps with no UI).
    pub fn disable_asks(&self) {
        self.0.asks_disabled.store(true, Ordering::Relaxed);
    }
    pub fn asks_disabled(&self) -> bool {
        self.0.asks_disabled.load(Ordering::Relaxed)
    }
    /// Require the coding loop's FIRST action to be an `ask_user` intake card (analysis first round).
    pub fn require_intake(&self) {
        self.0.require_intake.store(true, Ordering::Relaxed);
    }
    pub fn intake_required(&self) -> bool {
        self.0.require_intake.load(Ordering::Relaxed)
    }
    /// Allow the `delegate` tool (deep-research mode). Sub-runs never call this → no recursion.
    pub fn allow_delegate(&self) {
        self.0.delegate_allowed.store(true, Ordering::Relaxed);
    }
    pub fn delegate_allowed(&self) -> bool {
        self.0.delegate_allowed.load(Ordering::Relaxed)
    }
    /// Nudge the model to open a `delegate` sub-team on a complex research topic (bounded; it relents).
    pub fn encourage_delegate(&self) {
        self.0.encourage_delegate.store(true, Ordering::Relaxed);
    }
    pub fn delegate_encouraged(&self) -> bool {
        self.0.encourage_delegate.load(Ordering::Relaxed)
    }
}
