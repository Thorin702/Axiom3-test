// The right-side live activity panel — mirrors Claude Code's Background tasks rail,
// themed to our design system. Self-contained: it subscribes to the orchestration
// `run_event` stream, folds it into per-run cards (status, duration, agents, tokens,
// phase dots), and offers per-run cancel. No props beyond lang + close.
import { useEffect, useState, useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import { X, Boxes, Square, ChevronRight, ChevronDown, Pause, Play } from "lucide-react";
import type { Lang } from "../i18n";
import type { RunView } from "../lib/types";
import { runStore } from "../lib/runStore";

const S = {
  zh: {
    title: "后台任务", none: "暂无运行", noneSub: "用 /工作流 启动一个多智能体任务,这里会实时显示。",
    agents: "智能体", tokens: "令牌", phases: "阶段",
    planning: "规划", executing: "执行", searching: "自适应搜索", verifying: "审查", synthesizing: "汇总", computing: "计算图", done: "完成",
    running: "运行中", ok: "完成", failed: "失败", cancelled: "已取消", cancel: "取消", tok: "令牌",
    colAgent: "智能体", colTokens: "令牌", colTools: "步", colTime: "耗时",
    finished: "已结束", clear: "清空",
  },
  en: {
    title: "Background tasks", none: "No runs yet", noneSub: "Start a multi-agent task with /workflow — it streams here live.",
    agents: "agents", tokens: "tokens", phases: "Phases",
    planning: "Planning", executing: "Executing", searching: "Adaptive search", verifying: "Reviewing", synthesizing: "Synthesizing", computing: "Computing figures", done: "Done",
    running: "running", ok: "done", failed: "failed", cancelled: "cancelled", cancel: "Cancel", tok: "tok",
    colAgent: "Agent", colTokens: "Tokens", colTools: "Tools", colTime: "Time",
    finished: "Finished", clear: "Clear",
  },
};

const PHASE_LABEL: Record<string, keyof typeof S.zh> = {
  planning: "planning", executing: "executing", searching: "searching", verifying: "verifying", synthesizing: "synthesizing", computing: "computing", done: "done",
};
// Phases where the specialist agents actually do work (AB-MCTS emits "searching",
// the flat/deep fan-out emits "executing"). Both render the live agent queue.
const WORK_PHASES = runStore.WORK_PHASES;

function fmtDur(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  if (s < 60) return `${s}s`;
  return `${Math.floor(s / 60)}m ${s % 60}s`;
}
function fmtTokens(n: number): string {
  if (n >= 1000) return `${(n / 1000).toFixed(1)}k`;
  return String(n);
}
function lampOf(status: RunView["status"]): string {
  return status === "done" ? "is-green" : status === "running" ? "is-amber" : status === "failed" ? "is-red" : "is-neutral";
}

export function ActivityRail({ lang, onClose }: { lang: Lang; onClose: () => void }) {
  const s = S[lang];
  // Runs come from a module-level store (survives this component unmounting/remounting,
  // so an in-flight run is never lost when the rail is toggled).
  const runs = useSyncExternalStore(runStore.subscribe, runStore.snapshot);
  const [open, setOpen] = useState<Record<string, boolean>>({});
  const [paused, setPaused] = useState<Record<string, boolean>>({});
  const [, setTick] = useState(0);

  // tick once a second for live durations while any run is active
  const live = runs.some((r) => r.status === "running");
  useEffect(() => {
    if (!live) return;
    const t = setInterval(() => setTick((n) => n + 1), 1000);
    return () => clearInterval(t);
  }, [live]);

  const finishedCount = runs.filter((r) => r.status !== "running").length;
  const clearFinished = () => runStore.clearFinished();

  return (
    <aside className="ax-rail">
      <div className="ax-rail__head">
        <span className="ax-rail__title">{s.title}</span>
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
            const tokens = r.tokensIn + r.tokensOut;
            const execAgents = r.agents.filter((a) => WORK_PHASES.includes(a.phase));
            const planned = Math.max(execAgents.length, r.agentsTotal || 0);
            const nAgents = planned || r.agents.length;
            const idx = Math.max(0, r.phases.indexOf(r.phase));
            const finished = r.status !== "running";
            const kindLabel =
              r.taskKind === "workflow" ? "Workflow"
              : r.taskKind === "research" ? (lang === "zh" ? "研究" : "Research")
              : r.taskKind === "analysis" || r.taskKind === "coding" ? (lang === "zh" ? "分析" : "Analysis")
              : r.taskKind;
            return (
              <div className="ax-runcard" key={r.id}>
                {/* ── task header ── */}
                <div className="ax-runcard__top">
                  <span className={`ax-lamp ${lampOf(r.status)}`} />
                  <span className="ax-runcard__name">{r.title}</span>
                  {r.status === "running" && (
                    <span className="ax-runcard__ctl">
                      {(r.taskKind === "coding" || r.taskKind === "analysis" || r.taskKind === "research") && (
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
                    </span>
                  )}
                </div>
                <div className="ax-runcard__line">
                  <span className="ax-runcard__kind">{kindLabel}</span>
                  <span className="ax-runcard__dur">{dur}</span>
                </div>
                <div className="ax-runcard__stat">
                  {nAgents} {s.agents}<span className="ax-runcard__sep">·</span>{fmtTokens(tokens)} {s.tokens}
                </div>

                {/* ── phases ── */}
                {r.phases.length > 0 && (
                  <>
                    <div className="ax-runcard__phlabel">{s.phases}</div>
                    <div className="ax-phaselist">
                      {r.phases.map((p, i) => {
                        const pAgents = r.agents.filter((a) => a.phase === p);
                        const isPast = finished || i < idx;
                        const isNow = !finished && i === idx;
                        const pk = `${r.id}:${p}`;
                        const canExpand = pAgents.length > 0;
                        const queuePad = WORK_PHASES.includes(p) ? Math.max(0, planned - pAgents.length) : 0;
                        return (
                          <div className={`ax-ph ${isNow ? "is-now" : ""}`} key={p}>
                            <button className="ax-ph__head" disabled={!canExpand} onClick={() => canExpand && setOpen((o) => ({ ...o, [pk]: !o[pk] }))}>
                              {canExpand ? (open[pk] ? <ChevronDown size={12} aria-hidden /> : <ChevronRight size={12} aria-hidden />) : <span className="ax-ph__gap" />}
                              <span className="ax-ph__name">{s[PHASE_LABEL[p] ?? "planning"]}</span>
                            </button>
                            <div className="ax-ph__sq">
                              {pAgents.length > 0 || queuePad > 0 ? (
                                <>
                                  {pAgents.map((a) => (
                                    <span key={a.id} className={`ax-adot ${a.status === "running" ? "is-run" : a.status === "ok" ? "is-fin" : a.status === "err" || a.status === "cancelled" ? "is-err" : "is-fin"}`} title={`${a.role} · ${a.model}`} />
                                  ))}
                                  {Array.from({ length: queuePad }).map((_, k) => (
                                    <span key={`q${k}`} className="ax-adot is-queue" title={lang === "zh" ? "排队中" : "queued"} />
                                  ))}
                                </>
                              ) : (
                                <span className={`ax-sq ${isPast ? "is-done" : isNow ? "is-now" : "is-idle"}`} />
                              )}
                            </div>
                            {open[pk] && pAgents.length > 0 && (
                              <div className="ax-agtable">
                                <div className="ax-agtable__hdr">
                                  <span>{s.colAgent}</span>
                                  <span>{s.colTokens}</span>
                                  <span>{s.colTools}</span>
                                  <span>{s.colTime}</span>
                                </div>
                                {pAgents.map((a) => {
                                  const tok = a.tokensIn + a.tokensOut;
                                  const atime = a.startedMs ? fmtDur((a.finishedMs ?? Date.now()) - a.startedMs) : "—";
                                  return (
                                    <div className="ax-agtable__row" key={a.id} title={`${a.role} · ${a.model}${a.summary ? "\n" + a.summary : ""}${a.error ? "\n" + a.error : ""}`}>
                                      <span className="ax-agtable__name">
                                        <span className={`ax-adot ${a.status === "running" ? "is-run" : a.status === "ok" ? "is-fin" : "is-err"}`} />
                                        <span className="ax-agtable__rolename">{a.role || a.id}</span>
                                      </span>
                                      <span className="ax-agtable__num">{tok > 0 ? fmtTokens(tok) : "—"}</span>
                                      <span className="ax-agtable__num">{a.steps.length || "—"}</span>
                                      <span className="ax-agtable__num">{atime}</span>
                                    </div>
                                  );
                                })}
                              </div>
                            )}
                          </div>
                        );
                      })}
                    </div>
                  </>
                )}

                {r.error && <div className="ax-runcard__err">{r.error}</div>}
              </div>
            );
          })
        )}
      </div>

      {runs.length > 0 && (
        <div className="ax-rail__foot">
          <span className="ax-rail__finished">{s.finished} {finishedCount}</span>
          {finishedCount > 0 && (
            <button className="ax-linkbtn" onClick={clearFinished}>{s.clear}</button>
          )}
        </div>
      )}
    </aside>
  );
}
