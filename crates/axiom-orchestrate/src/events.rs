//! The structured event stream a run emits — the contract between the engine and the
//! frontend activity panel. One Tauri event name (`run_event`) carries an internally-
//! tagged enum; every event has `run_id` + a gap-free monotonic `seq` + `ts`.
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// The body of a run event. `#[serde(tag = "kind")]` flattens into the envelope, so the
/// JSON the frontend sees is `{ run_id, seq, ts, kind: "agent_spawned", agent_id, ... }`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunEventBody {
    RunStarted { task_kind: String, title: String, cancellable: bool },
    PlanReady { phases: Vec<String>, planned_agents: usize, notes: Vec<String> },
    PhaseChanged { phase: String, index: usize, total: usize },
    AgentSpawned { agent_id: String, role: String, model: String, vendor: String, phase: String },
    AgentProgress { agent_id: String, step: u32, tokens_in: u64, tokens_out: u64, note: Option<String> },
    /// A fully-structured step for a rich, live thinking chain: the step's title, the agent's
    /// reasoning narration, and the markdown `output` body (which for a code step contains the
    /// VERBATIM code it ran + the result). Streamed alongside `AgentProgress` so the frontend
    /// can render the actual code live, not just a short note. Mirrors `CodingStepOut`.
    AgentStep { agent_id: String, step: u32, role: String, status: String, title: String, reasoning: String, output: String, lines: usize },
    AgentFinished { agent_id: String, outcome: String, tokens_in: u64, tokens_out: u64, error: Option<String>, summary: Option<String> },
    RunProgress { agents_done: usize, agents_total: usize, agents_running: usize, agents_queued: usize },
    RunFinished { status: String, agents_total: usize, agents_ok: usize, agents_err: usize, error: Option<String> },
    /// The agent is PAUSED waiting for the human to answer a structured clarifying question
    /// (the `ask_user` tool). `card` is the JSON the model passed — a `questions` array, each with
    /// a header/question/options — which the UI renders as an AskUserQuestion-style card. The run
    /// parks until `answer_ask` delivers the human's selection back into the loop.
    AskUser { agent_id: String, card_id: String, card: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct RunEvent {
    pub run_id: String,
    pub seq: u64,
    pub ts: u64,
    #[serde(flatten)]
    pub body: RunEventBody,
}

/// Where events go. The Tauri shell implements this over `app.emit("run_event", ...)`;
/// tests use [`VecSink`].
pub trait EventSink: Send + Sync {
    fn emit(&self, ev: RunEvent);
}

/// Stamps a gap-free monotonic `seq` + `ts` onto each event and forwards it. The
/// `AtomicU64` fetch_add is the single serialization point, so `seq` stays gap-free
/// even when emitted from many concurrent agents — the frontend relies on that to
/// detect dropped events.
pub struct SeqEmitter {
    sink: Arc<dyn EventSink>,
    run_id: String,
    seq: AtomicU64,
}

impl SeqEmitter {
    pub fn new(sink: Arc<dyn EventSink>, run_id: impl Into<String>) -> Self {
        Self { sink, run_id: run_id.into(), seq: AtomicU64::new(0) }
    }
    pub fn emit(&self, body: RunEventBody) {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst);
        self.sink.emit(RunEvent { run_id: self.run_id.clone(), seq, ts: now_ms(), body });
    }
    pub fn run_id(&self) -> &str {
        &self.run_id
    }
}

/// In-memory sink for tests.
#[derive(Default)]
pub struct VecSink(Mutex<Vec<RunEvent>>);

impl EventSink for VecSink {
    fn emit(&self, ev: RunEvent) {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).push(ev);
    }
}

impl VecSink {
    pub fn events(&self) -> Vec<RunEvent> {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}
