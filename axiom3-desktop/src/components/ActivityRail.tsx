// The right-side live activity panel — mirrors Claude Code's Background tasks rail,
// themed to our design system. Self-contained: it subscribes to the orchestration
// `run_event` stream, folds it into per-run cards (status, duration, agents, tokens,
// phase dots), and offers per-run cancel. No props beyond lang + close.
import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { X, Boxes, Square } from "lucide-react";
import type { Lang } from "../i18n";
import type { RunEvent, RunView } from "../lib/types";

const S = {
  zh: {
    title: "活动", none: "暂无运行", noneSub: "用 /工作流 启动一个多智能体任务,这里会实时显示。",
    agents: "智能体", planning: "规划", executing: "执行", synthesizing: "汇总", done: "完成",
    running: "运行中", ok: "完成", failed: "失败", cancelled: "已取消", cancel: "取消", tok: "令牌",
  },
  en: {
    title: "Activity", none: "No runs yet", noneSub: "Start a multi-agent task with /workflow — it streams here live.",
    agents: "agents", planning: "Planning", executing: "Executing", synthesizing: "Synthesizing", done: "Done",
    running: "running", ok: "done", failed: "failed", cancelled: "cancelled", cancel: "Cancel", tok: "tok",
  },
};

const PHASE_LABEL: Record<string, keyof typeof S.zh> = {
  planning: "planning", executing: "executing", synthesizing: "synthesizing", done: "done",
};

function applyEvent(runs: RunView[], ev: RunEvent): RunView[] {
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
        r.agents.push({ id: ev.agent_id, role: ev.role, model: ev.model, vendor: ev.vendor, status: "running", tokensIn: 0, tokensOut: 0 });
      break;
    case "agent_progress":
      r.agents = r.agents.map((a) => (a.id === ev.agent_id ? { ...a, tokensIn: ev.tokens_in, tokensOut: ev.tokens_out } : a));
      break;
    case "agent_finished":
      r.agents = r.agents.map((a) =>
        a.id === ev.agent_id
          ? { ...a, status: ev.outcome === "ok" ? "ok" : ev.outcome === "cancelled" ? "cancelled" : "err", tokensIn: ev.tokens_in, tokensOut: ev.tokens_out, summary: ev.summary ?? undefined, error: ev.error ?? undefined }
          : a
      );
      break;
    case "run_progress": r.agentsDone = ev.agents_done; r.agentsTotal = ev.agents_total; break;
    case "run_finished":
      r.status = ev.status === "done" ? "done" : ev.status === "cancelled" ? "cancelled" : "failed";
      r.finishedMs = ev.ts; r.error = ev.error ?? undefined; break;
  }
  r.tokensIn = r.agents.reduce((s, a) => s + a.tokensIn, 0);
  r.tokensOut = r.agents.reduce((s, a) => s + a.tokensOut, 0);
  const next = [...runs];
  next[i] = r;
  return next;
}

function fmtDur(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  if (s < 60) return `${s}s`;
  return `${Math.floor(s / 60)}m ${s % 60}s`;
}
function lampOf(status: RunView["status"]): string {
  return status === "done" ? "is-green" : status === "running" ? "is-amber" : status === "failed" ? "is-red" : "is-neutral";
}

export function ActivityRail({ lang, onClose }: { lang: Lang; onClose: () => void }) {
  const s = S[lang];
  const [runs, setRuns] = useState<RunView[]>([]);
  const [, setTick] = useState(0);

  useEffect(() => {
    const un = listen<RunEvent>("run_event", (e) => setRuns((rs) => applyEvent(rs, e.payload)));
    return () => { un.then((f) => f()); };
  }, []);

  // tick once a second for live durations while any run is active
  const live = runs.some((r) => r.status === "running");
  useEffect(() => {
    if (!live) return;
    const t = setInterval(() => setTick((n) => n + 1), 1000);
    return () => clearInterval(t);
  }, [live]);

  return (
    <aside className="ax-rail">
      <div className="ax-rail__head">
        <span className="ax-rail__title">{s.title}{runs.length > 0 ? ` · ${runs.length}` : ""}</span>
        <button className="ax-iconbtn" onClick={onClose} title="×" aria-label="close">
          <X size={16} aria-hidden />
        </button>
      </div>

      <div className="ax-rail__body">
        {runs.length === 0 ? (
          <div className="ax-rail__empty">
            <Boxes size={20} aria-hidden />
            <p className="ax-rail__empty-t">{s.none}</p>
            <p className="ax-rail__empty-s">{s.noneSub}</p>
          </div>
        ) : (
          runs.map((r) => {
            const dur = fmtDur((r.finishedMs ?? Date.now()) - r.startedMs);
            const statusLabel = r.status === "running" ? s.running : r.status === "done" ? s.ok : r.status === "failed" ? s.failed : s.cancelled;
            const tokens = r.tokensIn + r.tokensOut;
            const activePhase = Math.max(0, r.phases.indexOf(r.phase));
            return (
              <div className="ax-runcard" key={r.id}>
                <div className="ax-runcard__head">
                  <span className={`ax-lamp ${lampOf(r.status)}`} />
                  <span className="ax-runcard__title">{r.title}</span>
                  {r.status === "running" && (
                    <button className="ax-iconbtn" onClick={() => invoke("cancel_run", { runid: r.id }).catch(() => {})} title={s.cancel} aria-label={s.cancel}>
                      <Square size={12} aria-hidden />
                    </button>
                  )}
                </div>
                <div className="ax-runcard__meta">
                  <span className="ax-badge">/{r.taskKind === "workflow" ? (lang === "zh" ? "工作流" : "workflow") : r.taskKind}</span>
                  <span>{statusLabel}</span>
                  <span>· {dur}</span>
                  <span>· {r.agents.length}/{r.agentsTotal || r.agents.length} {s.agents}</span>
                  {tokens > 0 && <span>· {tokens.toLocaleString()} {s.tok}</span>}
                </div>

                {r.phases.length > 0 && (
                  <div className="ax-phases">
                    {r.phases.map((p, i) => (
                      <span key={p} className="ax-phase">
                        <span className={`ax-phase-dot ${i < activePhase || r.status === "done" ? "is-past" : i === activePhase ? "is-now" : ""}`} />
                        <span className="ax-phase-l">{s[PHASE_LABEL[p] ?? "planning"]}</span>
                      </span>
                    ))}
                  </div>
                )}

                {r.agents.length > 0 && (
                  <div className="ax-agents">
                    {r.agents.slice(0, 12).map((a) => (
                      <div className="ax-agent" key={a.id} title={a.summary || a.error || a.role}>
                        <span className={`ax-lamp ${a.status === "ok" ? "is-green" : a.status === "running" ? "is-amber" : a.status === "err" ? "is-red" : "is-neutral"}`} />
                        <span className="ax-agent__role">{a.role}</span>
                        <span className="ax-agent__model">{a.vendor}</span>
                      </div>
                    ))}
                    {r.agents.length > 12 && <div className="ax-agent ax-hint">+{r.agents.length - 12}</div>}
                  </div>
                )}
              </div>
            );
          })
        )}
      </div>
    </aside>
  );
}
