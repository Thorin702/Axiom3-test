// A Claude-Code-style context-window meter for the composer: a compact button showing how much of
// the model's context the active run is using, expanding to a breakdown. Once a run is live it uses
// REAL, content-derived token counts emitted by the engine (input = system+task+tool results fed to
// the model, output = what the model produced) — the meter tracks actual routed work, not a guess.
// Before a run starts it falls back to the standing session overhead (system + tools + skills/memory)
// so the composer still shows the baseline it will open with.
import { useState } from "react";
import type { RunView } from "../lib/types";
import type { Lang } from "../i18n";

const LIMIT = 1_000_000; // GLM-5.2 (1M) context window, tokens

type Slice = { key: string; zh: string; en: string; tok: number; color: string };

// Real usage when a run is live (tokens are content-derived from the actual calls); otherwise the
// standing overhead the next run will open with. `real` drives the honesty label at the bottom.
function usage(run: RunView | undefined, specialist: boolean, memory: boolean): { slices: Slice[]; used: number; real: boolean; model?: string } {
  const live = run && run.tokensIn + run.tokensOut > 0;
  if (live) {
    const input = run!.tokensIn; // system prompt + task + tool results already baked in by the engine
    const output = run!.tokensOut; // everything the model(s) produced
    const model = run!.agents.find((a) => a.status === "running")?.model ?? run!.agents[run!.agents.length - 1]?.model;
    const slices: Slice[] = [
      { key: "in", zh: "输入 · 系统+任务+上下文", en: "Input · sys+task+context", tok: input, color: "#8A6FC4" },
      { key: "out", zh: "模型产出", en: "Model output", tok: output, color: "#C96442" },
    ];
    return { slices, used: input + output, real: true, model };
  }
  // Idle: the overhead the next run opens with (no real tokens yet).
  const sys = 4200; // agent instructions
  const tools = 2400; // tool definitions (python/litsearch/…)
  const skills = specialist ? 1200 : 0;
  const mem = memory ? 700 : 0;
  const slices: Slice[] = [
    { key: "sys", zh: "系统提示", en: "System prompt", tok: sys, color: "#8A6FC4" },
    { key: "tools", zh: "工具", en: "Tools", tok: tools, color: "#4B9E6F" },
    ...(skills ? [{ key: "skills", zh: "技能", en: "Skills", tok: skills, color: "#C98A3C" }] : []),
    ...(mem ? [{ key: "mem", zh: "记忆", en: "Memory", tok: mem, color: "#5E86B8" }] : []),
  ];
  return { slices, used: sys + tools + skills + mem, real: false };
}

function fmt(n: number): string {
  return n >= 1000 ? `${(n / 1000).toFixed(n >= 10_000 ? 0 : 1)}k` : String(n);
}

export function ContextWindow({ run, specialist, memory, lang }: {
  run?: RunView;
  specialist: boolean;
  memory: boolean;
  lang: Lang;
}) {
  const [open, setOpen] = useState(false);
  const zh = lang === "zh";
  const { slices, used, real, model } = usage(run, specialist, memory);
  const pct = Math.min(100, Math.round((used / LIMIT) * 100));
  const free = Math.max(0, LIMIT - used);

  return (
    <div className="ax-ctx">
      <button
        className={`ax-ctx__btn ${open ? "on" : ""}`}
        onClick={() => setOpen((o) => !o)}
        aria-haspopup="dialog"
        aria-expanded={open}
        title={`${zh ? "上下文窗口" : "Context window"} · ${fmt(used)}/${fmt(LIMIT)} (${pct}%)`}
        aria-label={`${zh ? "上下文窗口" : "Context window"} ${pct}%`}
      >
        <svg className="ax-ctx__ring" viewBox="0 0 24 24" width="15" height="15" aria-hidden>
          <circle className="ax-ctx__ring-bg" cx="12" cy="12" r="9" fill="none" strokeWidth="3" />
          <circle
            className="ax-ctx__ring-fg"
            cx="12"
            cy="12"
            r="9"
            fill="none"
            strokeWidth="3"
            strokeLinecap="round"
            transform="rotate(-90 12 12)"
            strokeDasharray={2 * Math.PI * 9}
            strokeDashoffset={2 * Math.PI * 9 * (1 - Math.max(pct, used > 0 ? 1.5 : 0) / 100)}
          />
        </svg>
        <span className="ax-ctx__pct">{pct}%</span>
      </button>
      {open && (
        <>
          <div className="ax-ctx__scrim" onClick={() => setOpen(false)} />
          <div className="ax-ctx__panel" role="dialog" aria-label={zh ? "上下文窗口" : "Context window"}>
            <div className="ax-ctx__head">
              <span>{zh ? "上下文窗口" : "Context window"}{model ? ` · ${model}` : ""}</span>
              <span className="ax-ctx__total">{fmt(used)} / {fmt(LIMIT)} ({pct}%)</span>
            </div>
            <div className="ax-ctx__track">
              {slices.map((s) => (
                <span key={s.key} className="ax-ctx__seg" style={{ width: `${Math.min(100, (s.tok / LIMIT) * 100)}%`, background: s.color }} />
              ))}
              <span className="ax-ctx__seg ax-ctx__seg--free" style={{ width: `${(free / LIMIT) * 100}%` }} />
            </div>
            <ul className="ax-ctx__list">
              {slices.map((s) => (
                <li key={s.key}>
                  <span className="ax-ctx__dot" style={{ background: s.color }} />
                  <span className="ax-ctx__name">{zh ? s.zh : s.en}</span>
                  <span className="ax-ctx__val">{fmt(s.tok)}</span>
                  <span className="ax-ctx__p">{((s.tok / LIMIT) * 100).toFixed(1)}%</span>
                </li>
              ))}
              <li className="ax-ctx__free">
                <span className="ax-ctx__dot" style={{ background: "var(--ax-line)" }} />
                <span className="ax-ctx__name">{zh ? "空闲" : "Free space"}</span>
                <span className="ax-ctx__val">{fmt(free)}</span>
                <span className="ax-ctx__p">{((free / LIMIT) * 100).toFixed(1)}%</span>
              </li>
            </ul>
            <p className="ax-ctx__note">
              {real
                ? (zh ? "实时用量 · 按各智能体真实输入/产出内容换算(暂无精确 API 计数)" : "live usage · derived from real routed content (exact API counts pending)")
                : (zh ? "会话待命开销(估算,任务启动后转为实时用量)" : "idle session overhead (estimate; becomes live once a task starts)")}
            </p>
          </div>
        </>
      )}
    </div>
  );
}
