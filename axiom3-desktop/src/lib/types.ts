// Shared types mirroring the Rust command outputs (serde snake_case).

export type ProjectStatus = {
  project_id: string;
  root: string;
  event_count: number;
  chain_head: string | null;
  chain_ok: boolean;
};

export type EventInfo = {
  seq: number;
  ts: number;
  actor: string;
  op: string;
  entry_hash: string;
};

export type FindingView = { checker: string; message: string; excerpt: string | null };
export type VendorView = { family: string; model: string; status: string; reasoning: string };
export type FleetView = {
  status: string;
  independence: string;
  reason: string;
  vendors: VendorView[];
};
export type ClaimEntry = {
  claim_id: string;
  text: string;
  claim_type: string;
  is_bearing: boolean;
  status: string;
  reason: string | null;
  findings: FindingView[];
  fleet: FleetView | null;
  evidence: string[];
};
export type DraftCheck = {
  event_seq: number;
  total: number;
  flagged: number;
  fleet_verified: number;
  entries: ClaimEntry[];
};

// An imported document kept in the local source library (阅读/摄入).
export type Source = { id: string; name: string; chars: number; ts: number; text: string };

export type ExportSummary = {
  path: string;
  bytes: number;
  claims: number;
  event_seq: number;
  chain_intact: boolean;
};

// ── Orchestration: the run_event stream from axiom-orchestrate ──────────────
// Mirrors the Rust RunEventBody (serde tag = "kind"), flattened onto the envelope.
export type RunEvent = { run_id: string; seq: number; ts: number } & (
  | { kind: "run_started"; task_kind: string; title: string; cancellable: boolean }
  | { kind: "plan_ready"; phases: string[]; planned_agents: number; notes: string[] }
  | { kind: "phase_changed"; phase: string; index: number; total: number }
  | { kind: "agent_spawned"; agent_id: string; role: string; model: string; vendor: string; phase: string }
  | { kind: "agent_progress"; agent_id: string; step: number; tokens_in: number; tokens_out: number; note: string | null }
  | { kind: "agent_finished"; agent_id: string; outcome: string; tokens_in: number; tokens_out: number; error: string | null; summary: string | null }
  | { kind: "run_progress"; agents_done: number; agents_total: number; agents_running: number; agents_queued: number }
  | { kind: "run_finished"; status: string; agents_total: number; agents_ok: number; agents_err: number; error: string | null }
);

export type AgentView = {
  id: string;
  role: string;
  model: string;
  vendor: string;
  phase: string;
  status: "running" | "ok" | "err" | "cancelled";
  tokensIn: number;
  tokensOut: number;
  summary?: string;
  error?: string;
  steps: string[]; // live per-step notes (reasoning + tool action) — the streamed transcript
};
export type RunView = {
  id: string;
  taskKind: string;
  title: string;
  status: "running" | "done" | "failed" | "cancelled";
  phase: string;
  phases: string[];
  agents: AgentView[];
  agentsTotal: number;
  agentsDone: number;
  tokensIn: number;
  tokensOut: number;
  startedMs: number;
  finishedMs?: number;
  error?: string;
};

// A unit of work the platform runs. Today check_draft + workflow execute; the others
// are declared so the workbench is task-oriented from the start.
export type TaskType = "check_draft" | "workflow" | "ppt" | "code" | "research";
export type TaskStatus = "running" | "done" | "failed" | "cancelled";
export type Task = {
  id: string;
  type: TaskType;
  title: string;
  status: TaskStatus;
  ts: number;
  result?: DraftCheck;
  synthesis?: string; // workflow runs: the folded multi-agent output
  error?: string;
};
