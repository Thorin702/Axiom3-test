// A Claude-Code-style context-window meter for the composer: a compact button showing how much of
// the model's context the active run is using, expanding to a breakdown (messages / system prompt /
// tools / skills / memory / free space). The coding agent doesn't emit real token counts yet, so
// this ESTIMATES usage from the run's structure (step count, enabled skill/memory) against GLM-5.2's
// window — good enough to show the shape; swap the estimate for backend-emitted usage when available.
import { useState } from "react";
import type { RunView } from "../lib/types";
import type { Lang } from "../i18n";

const LIMIT = 128_000; // glm-5.2 context window, tokens

type Slice = { key: string; zh: string; en: string; tok: number; color: string };

function estimate(run: RunView | undefined, specialist: boolean, memory: boolean): { slices: Slice[]; used: number } {
  const steps = run ? run.agents.reduce((n, a) => n + (a.rich?.length ?? a.steps.length), 0) : 0;
  const sys = 4200; // agent instructions
  const tools = 2400; // tool definitions (python/litsearch/…)
  const msgs = Math.min(steps * 460, 9000); // transcript, compacted cap
  const skills = specialist ? 1200 : 0;
  const mem = memory ? 700 : 0;
  const used = sys + tools + msgs + skills + mem;
  const slices: Slice[] = [
    { key: "msgs", zh: "消息", en: "Messages", tok: msgs, color: "#C96442" },
    { key: "sys", zh: "系统提示", en: "System prompt", tok: sys, color: "#8A6FC4" },
    { key: "tools", zh: "工具", en: "Tools", tok: tools, color: "#4B9E6F" },
    ...(skills ? [{ key: "skills", zh: "技能", en: "Skills", tok: skills, color: "#C98A3C" }] : []),
    ...(mem ? [{ key: "mem", zh: "记忆", en: "Memory", tok: mem, color: "#5E86B8" }] : []),
  ];
  return { slices, used };
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
  const { slices, used } = estimate(run, specialist, memory);
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
        <span className="ax-ctx__bar"><span className="ax-ctx__fill" style={{ width: `${pct}%` }} /></span>
        <span className="ax-ctx__pct">{pct}%</span>
      </button>
      {open && (
        <>
          <div className="ax-ctx__scrim" onClick={() => setOpen(false)} />
          <div className="ax-ctx__panel" role="dialog" aria-label={zh ? "上下文窗口" : "Context window"}>
            <div className="ax-ctx__head">
              <span>{zh ? "上下文窗口" : "Context window"}</span>
              <span className="ax-ctx__total">{fmt(used)} / {fmt(LIMIT)} ({pct}%)</span>
            </div>
            <div className="ax-ctx__track">
              {slices.map((s) => (
                <span key={s.key} className="ax-ctx__seg" style={{ width: `${(s.tok / LIMIT) * 100}%`, background: s.color }} />
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
            <p className="ax-ctx__note">{zh ? "估算值（按当前任务结构推算，非精确 token 计数）" : "estimated from run structure — not exact token counts"}</p>
          </div>
        </>
      )}
    </div>
  );
}
