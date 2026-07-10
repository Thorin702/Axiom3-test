// The right-side live activity panel — mirrors Claude Code's Background tasks rail,
// themed to our design system. Self-contained: it subscribes to the orchestration
// `run_event` stream, folds it into per-run cards (status, duration, agents, tokens,
// phase dots), and offers per-run cancel. No props beyond lang + close.
import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { X, Boxes, Square, ChevronRight, ChevronDown, Pause, Play } from "lucide-react";
import type { Lang } from "../i18n";
import type { RunEvent, RunView } from "../lib/types";

const S = {
  zh: {
    title: "活动", none: "暂无运行", noneSub: "用 /工作流 启动一个多智能体任务,这里会实时显示。",
    agents: "智能体", planning: "规划", executing: "执行", searching: "自适应搜索", verifying: "审查", synthesizing: "汇总", done: "完成",
    running: "运行中", ok: "完成", failed: "失败", cancelled: "已取消", cancel: "取消", tok: "令牌",
    lRun: "运行中", lFin: "已结束", lErr: "出错/暂停",
  },
  en: {
    title: "Activity", none: "No runs yet", noneSub: "Start a multi-agent task with /workflow — it streams here live.",
    agents: "agents", planning: "Planning", executing: "Executing", searching: "Adaptive search", verifying: "Reviewing", synthesizing: "Synthesizing", done: "Done",
    running: "running", ok: "done", failed: "failed", cancelled: "cancelled", cancel: "Cancel", tok: "tok",
    lRun: "running", lFin: "finished", lErr: "error/paused",
  },
};

const PHASE_LABEL: Record<string, keyof typeof S.zh> = {
  planning: "planning", executing: "executing", searching: "searching", verifying: "verifying", synthesizing: "synthesizing", done: "done",
};
// Phases where the specialist agents actually do work (AB-MCTS emits "searching",
// the flat/deep fan-out emits "executing"). Both render the live agent queue.
const WORK_PHASES = ["executing", "searching"];

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
        r.agents.push({ id: ev.agent_id, role: ev.role, model: ev.model, vendor: ev.vendor, phase: ev.phase, status: "running", tokensIn: 0, tokensOut: 0, steps: [] });
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
          ? { ...a, status: ev.outcome === "ok" ? "ok" : ev.outcome === "cancelled" ? "cancelled" : "err", tokensIn: ev.tokens_in, tokensOut: ev.tokens_out, summary: ev.summary ?? undefined, error: ev.error ?? undefined }
          : a
      );
      break;
    // Never let a progress total SHRINK the planned estimate (the deep path announces
    // top×sub up front; a group-granularity total must not clobber it — belt-and-suspenders
    // alongside the backend fix).
    case "run_progress": r.agentsDone = ev.agents_done; r.agentsTotal = Math.max(r.agentsTotal, ev.agents_total); break;
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
  const [open, setOpen] = useState<Record<string, boolean>>({});
  const [paused, setPaused] = useState<Record<string, boolean>>({});
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
          <>
          <div className="ax-rail__legend">
            <span><span className="ax-adot is-run" /> {s.lRun}</span>
            <span><span className="ax-adot is-fin" /> {s.lFin}</span>
            <span><span className="ax-adot is-err" /> {s.lErr}</span>
          </div>
          {runs.map((r) => {
            const dur = fmtDur((r.finishedMs ?? Date.now()) - r.startedMs);
            const statusLabel = r.status === "running" ? s.running : r.status === "done" ? s.ok : r.status === "failed" ? s.failed : s.cancelled;
            const tokens = r.tokensIn + r.tokensOut;
            const execAgents = r.agents.filter((a) => WORK_PHASES.includes(a.phase));
            const planned = Math.max(execAgents.length, r.agentsTotal || 0);
            const nAgents = planned || r.agents.length;
            const qDone = execAgents.filter((a) => a.status === "ok" || a.status === "err").length;
            const qRunning = execAgents.filter((a) => a.status === "running").length;
            const qQueued = Math.max(0, planned - execAgents.length);
            const idx = Math.max(0, r.phases.indexOf(r.phase));
            const finished = r.status !== "running";
            return (
              <div className="ax-runcard" key={r.id}>
                <div className="ax-runcard__head">
                  <span className={`ax-lamp ${lampOf(r.status)}`} />
                  <span className="ax-runcard__title">{r.title}</span>
                  {r.status === "running" && (
                    <>
                      {r.taskKind === "coding" && (
                        <button
                          className="ax-iconbtn"
                          onClick={() => {
                            const p = !paused[r.id];
                            invoke(p ? "pause_run" : "resume_run", { runid: r.id }).catch(() => {});
                            setPaused((o) => ({ ...o, [r.id]: p }));
                          }}
                          title={paused[r.id] ? (lang === "zh" ? "继续" : "Resume") : lang === "zh" ? "暂停" : "Pause"}
                          aria-label={paused[r.id] ? "resume" : "pause"}
                        >
                          {paused[r.id] ? <Play size={12} aria-hidden /> : <Pause size={12} aria-hidden />}
                        </button>
                      )}
                      <button className="ax-iconbtn" onClick={() => invoke("cancel_run", { runid: r.id }).catch(() => {})} title={s.cancel} aria-label={s.cancel}>
                        <Square size={12} aria-hidden />
                      </button>
                    </>
                  )}
                </div>
                <div className="ax-runcard__meta">
                  <span className="ax-badge">/{r.taskKind === "workflow" ? (lang === "zh" ? "工作流" : "workflow") : r.taskKind}</span>
                  <span>{statusLabel}</span>
                  <span>· {nAgents} {s.agents}</span>
                  <span>· {dur}</span>
                  {!finished && planned > 0 && (qQueued > 0 || qRunning > 0) && (
                    <span className="ax-q">
                      {qDone}/{planned} · {qRunning}{lang === "zh" ? "运行" : " run"}
                      {qQueued > 0 ? ` · ${qQueued}${lang === "zh" ? "排队" : " queued"}` : ""}
                    </span>
                  )}
                  {tokens > 0 && <span>· {tokens.toLocaleString()} {s.tok}</span>}
                </div>

                {r.phases.length > 0 && (
                  <div className="ax-phaselist">
                    {r.phases.map((p, i) => {
                      const pAgents = r.agents.filter((a) => a.phase === p);
                      const isPast = finished || i < idx;
                      const isNow = !finished && i === idx;
                      const pk = `${r.id}:${p}`;
                      const canExpand = pAgents.length > 0;
                      return (
                        <div className="ax-ph" key={p}>
                          <button
                            className="ax-ph__head"
                            disabled={!canExpand}
                            onClick={() => canExpand && setOpen((o) => ({ ...o, [pk]: !o[pk] }))}
                          >
                            <span className="ax-ph__name">{s[PHASE_LABEL[p] ?? "planning"]}</span>
                            {canExpand &&
                              (open[pk] ? <ChevronDown size={12} aria-hidden /> : <ChevronRight size={12} aria-hidden />)}
                          </button>
                          <div className="ax-ph__sq">
                            {(() => {
                              // the executing phase shows the whole queue: started agents by
                              // status + queued placeholders up to the planned count.
                              const queuePad = WORK_PHASES.includes(p) ? Math.max(0, planned - pAgents.length) : 0;
                              if (pAgents.length > 0 || queuePad > 0) {
                                return (
                                  <>
                                    {pAgents.map((a) => (
                                      <span
                                        key={a.id}
                                        className={`ax-adot ${a.status === "running" ? "is-run" : a.status === "ok" ? "is-fin" : a.status === "err" || a.status === "cancelled" ? "is-err" : "is-fin"}`}
                                        title={`${a.role} · ${a.model}`}
                                      />
                                    ))}
                                    {Array.from({ length: queuePad }).map((_, k) => (
                                      <span key={`q${k}`} className="ax-adot is-queue" title={lang === "zh" ? "排队中" : "queued"} />
                                    ))}
                                  </>
                                );
                              }
                              return <span className={`ax-sq ${isPast ? "is-done" : isNow ? "is-now" : "is-idle"}`} />;
                            })()}
                          </div>
                          {open[pk] && (
                            <div className="ax-tr">
                              {pAgents.map((a) => {
                                const tok = a.tokensIn + a.tokensOut;
                                return (
                                  <div className="ax-tr__agent" key={a.id}>
                                    <div className="ax-tr__role">
                                      <span className={`ax-adot ${a.status === "running" ? "is-run" : a.status === "ok" ? "is-fin" : "is-err"}`} />
                                      <span className={`ax-tr__kind ${a.phase === "verifying" ? "is-reviewer" : ""}`}>
                                        {a.phase === "verifying" ? (lang === "zh" ? "审查员" : "reviewer") : lang === "zh" ? "专家" : "specialist"}
                                      </span>
                                      <span className="ax-tr__rolename">{a.role}</span>
                                      <span className="ax-tr__model">{a.model}</span>
                                      <span className="ax-tr__tok">{tok > 0 ? `${tok.toLocaleString()} ${s.tok}` : "—"}</span>
                                    </div>
                                    {a.steps.map((st, k) => (
                                      <div className="ax-tr__step" key={k}>{st}</div>
                                    ))}
                                    {a.summary && <div className="ax-tr__sum">{a.summary}</div>}
                                    {a.error && <div className="ax-tr__err">{a.error}</div>}
                                  </div>
                                );
                              })}
                            </div>
                          )}
                        </div>
                      );
                    })}
                  </div>
                )}

                {r.error && <div className="ax-runcard__err">{r.error}</div>}
              </div>
            );
          })}
          </>
        )}
      </div>
    </aside>
  );
}
