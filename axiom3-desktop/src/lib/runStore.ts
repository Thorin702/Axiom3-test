// A module-level store for live orchestration runs. It lives OUTSIDE React so the run
// history + in-flight runs survive the ActivityRail unmounting/remounting (toggling the
// rail, or a finished task swapping in the ArtifactPane). Previously the rail held this in
// local state and re-subscribed on every mount, so an in-flight run's earlier events were
// gone and it vanished forever. One listener is registered here for the app's lifetime.
import { listen } from "@tauri-apps/api/event";
import type { RunEvent, RunView } from "./types";

const WORK_PHASES = ["executing", "searching"];

function reduce(runs: RunView[], ev: RunEvent): RunView[] {
  if (ev.kind === "run_started") {
    if (runs.some((r) => r.id === ev.run_id)) return runs;
    return [
      { id: ev.run_id, taskKind: ev.task_kind, title: ev.title, status: "running", phase: "planning", phases: [], agents: [], agentsTotal: 0, agentsDone: 0, tokensIn: 0, tokensOut: 0, startedMs: ev.ts },
      ...runs,
    ];
  }
  const i = runs.findIndex((r) => r.id === ev.run_id);
  if (i < 0) return runs;
  const r: RunView = { ...runs[i], agents: [...runs[i].agents] };
  switch (ev.kind) {
    case "plan_ready": r.phases = ev.phases; r.agentsTotal = ev.planned_agents; break;
    case "phase_changed": r.phase = ev.phase; break;
    case "agent_spawned":
      if (!r.agents.some((a) => a.id === ev.agent_id))
        r.agents.push({ id: ev.agent_id, role: ev.role, model: ev.model, vendor: ev.vendor, phase: ev.phase, status: "running", tokensIn: 0, tokensOut: 0, steps: [], startedMs: ev.ts });
      break;
    case "agent_progress":
      r.agents = r.agents.map((a) =>
        a.id === ev.agent_id
          ? { ...a, tokensIn: ev.tokens_in, tokensOut: ev.tokens_out, steps: ev.note ? [...a.steps, ev.note] : a.steps }
          : a
      );
      break;
    case "agent_finished":
      r.agents = r.agents.map((a) =>
        a.id === ev.agent_id
          ? { ...a, status: ev.outcome === "ok" ? "ok" : ev.outcome === "cancelled" ? "cancelled" : "err", tokensIn: ev.tokens_in, tokensOut: ev.tokens_out, summary: ev.summary ?? undefined, error: ev.error ?? undefined, finishedMs: ev.ts }
          : a
      );
      break;
    case "run_progress": r.agentsDone = ev.agents_done; r.agentsTotal = Math.max(r.agentsTotal, ev.agents_total); break;
    case "run_finished":
      r.status = ev.status === "done" ? "done" : ev.status === "cancelled" ? "cancelled" : "failed";
      r.finishedMs = ev.ts; r.error = ev.error ?? undefined;
      // Reconcile any agent still "running" (its finish event never arrived / it panicked) to
      // a terminal state so its dot and Time column resolve instead of freezing forever.
      r.agents = r.agents.map((a) =>
        a.status === "running" ? { ...a, status: r.status === "cancelled" ? "cancelled" : "err", finishedMs: a.finishedMs ?? ev.ts } : a
      );
      break;
  }
  r.tokensIn = r.agents.reduce((s, a) => s + a.tokensIn, 0);
  r.tokensOut = r.agents.reduce((s, a) => s + a.tokensOut, 0);
  const next = [...runs];
  next[i] = r;
  return next;
}

let runs: RunView[] = [];
const subs = new Set<() => void>();
function set(next: RunView[]) { runs = next; subs.forEach((f) => f()); }

// One lifetime listener — folds every run_event into the store.
listen<RunEvent>("run_event", (e) => set(reduce(runs, e.payload)));

export const runStore = {
  subscribe(cb: () => void) { subs.add(cb); return () => { subs.delete(cb); }; },
  snapshot(): RunView[] { return runs; },
  clearFinished() { set(runs.filter((r) => r.status === "running")); },
  WORK_PHASES,
};
