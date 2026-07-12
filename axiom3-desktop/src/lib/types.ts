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
  | { kind: "agent_step"; agent_id: string; step: number; role: string; status: string; title: string; reasoning: string; output: string; lines: number }
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
  rich?: AgentStepView[]; // fully-structured live steps (title + reasoning + code body) — coding/analysis
  startedMs?: number; // spawn time — for the per-agent Time column
  finishedMs?: number;
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
// One sub-agent's rolled-up result from a workflow run (persisted so the per-agent output
// is readable after reload, not just as an ephemeral 120-char rail summary). `model` is
// kept so an artifact can state which model produced each part (provenance).
// A thinking-chain step (Claude-Science-style): a short natural `title` + `reasoning` narration +
// the collapsible `output` body + an output line count. title/reasoning/lines are present for
// coding/analysis steps; workflow sub-agent steps carry only role/status/output/model.
export type AgentStepView = {
  role: string;
  status: string;
  output: string;
  model?: string;
  title?: string;
  reasoning?: string;
  lines?: number;
};
// One version of a session's artifact — a follow-up run iterates the artifact (v1 → v2 …)
// rather than spawning a new session (forking does that). The task's top-level
// synthesis/review/steps mirror the LATEST version.
export type ArtifactVersion = { synthesis?: string; review?: string; steps?: AgentStepView[]; ts: number; goal: string };
// A research SESSION: the unit of work. Carries its full goal so it can be FORKED (re-run
// a variant without losing the original thread), and enough lineage/provenance for the
// artifact to state how it was made.
export type Task = {
  id: string;
  type: TaskType;
  title: string;
  status: TaskStatus;
  ts: number;
  goal?: string; // the full input (title is a truncation) — needed to fork
  forkedFrom?: string; // session id this was forked from
  deep?: boolean; // ran as an adaptive multi-agent (AB-MCTS) search
  result?: DraftCheck;
  synthesis?: string; // workflow runs: the folded multi-agent output (latest version)
  review?: string; // the reviewer agent's critique/findings (never a verdict)
  steps?: AgentStepView[]; // workflow runs: each sub-agent's output
  versions?: ArtifactVersion[]; // artifact history (v1…vN); present once iterated
  // The run id of the CURRENT/last orchestration run for this session. Differs from `id` on a
  // follow-up iteration (append), where a fresh run id is minted — the live thinking chain
  // matches runStore by THIS id, not the session id.
  liveRunId?: string;
  error?: string;
};
