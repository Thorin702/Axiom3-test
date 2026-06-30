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

export type ExportSummary = {
  path: string;
  bytes: number;
  claims: number;
  event_seq: number;
  chain_intact: boolean;
};

// A unit of work the platform runs. Today only check_draft actually executes; the other
// types are declared so the workbench is task-oriented from the start (the dynamic-
// workflow / multimodal / coding engine fills them in).
export type TaskType = "check_draft" | "ppt" | "code" | "research";
export type TaskStatus = "running" | "done" | "failed" | "cancelled";
export type Task = {
  id: string;
  type: TaskType;
  title: string;
  status: TaskStatus;
  ts: number;
  result?: DraftCheck;
  error?: string;
};
