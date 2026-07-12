// A "running" status pill rendered right above the composer, aligned to its left edge — mirrors
// Claude Code's activity line. Shows the app's knowledge-graph mark (the icon) as a live
// "thinking" pulse while agents run, followed by elapsed time · tokens · task state. It PERSISTS
// after the run finishes (showing the last run's final stats, static graph), and renders nothing
// only when the run store is empty. Reads the module-level runStore. (Lives in the composer, so
// it shows on the home + thread views where work is started, not on the settings-like tabs.)
import { useEffect, useState, useSyncExternalStore } from "react";
import type { Lang } from "../i18n";
import { runStore } from "../lib/runStore";

function fmtDur(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m ${s % 60}s`;
}
function fmtTokens(n: number): string {
  return n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n);
}

// JUST the knowledge-graph shape from the app icon — nodes + edges, no background. It inherits
// currentColor (near-white in dark, ink in light); each node pulses on a staggered delay while
// running, and holds static once done.
const NODES: [number, number, number][] = [
  [13, 15, 3.3], [30, 11, 3.3], [41, 21, 3.3], [24, 30, 4.3], [38, 35, 3.3], [16, 42, 3.3], [31, 43, 3.3],
];
const EDGES =
  "M13 15 L30 11 M30 11 L41 21 M13 15 L24 30 M30 11 L24 30 M41 21 L38 35 M24 30 L38 35 M24 30 L16 42 M38 35 L31 43 M16 42 L31 43";

function GraphMark({ active }: { active: boolean }) {
  return (
    <svg className={`ax-runpill__mark ${active ? "is-pulsing" : ""}`} viewBox="7 5 40 44" width="18" height="18" aria-hidden>
      <path className="ax-runpill__edges" d={EDGES} />
      {NODES.map(([cx, cy, r], i) => (
        <circle key={i} className="ax-runpill__node" cx={cx} cy={cy} r={r} style={{ animationDelay: `${i * 0.15}s` }} />
      ))}
    </svg>
  );
}

export function RunStatusPill({ lang }: { lang: Lang }) {
  const zh = lang === "zh";
  const runs = useSyncExternalStore(runStore.subscribe, runStore.snapshot);
  const running = runs.filter((r) => r.status === "running");
  const active = running.length > 0;
  const [, setTick] = useState(0);
  // tick every second for a live "thinking time" while anything runs
  useEffect(() => {
    if (!active) return;
    const t = setInterval(() => setTick((x) => x + 1), 1000);
    return () => clearInterval(t);
  }, [active]);

  if (runs.length === 0) return null;

  // running → aggregate the in-flight runs; otherwise → hold the most recent finished run's stats
  let elapsedMs: number;
  let tokens: number;
  let label: string;
  if (active) {
    elapsedMs = Date.now() - Math.min(...running.map((r) => r.startedMs));
    tokens = running.reduce((s, r) => s + r.tokensIn + r.tokensOut, 0);
    label = `${running.length} ${zh ? "个任务运行中" : running.length > 1 ? "running tasks" : "running task"}`;
  } else {
    const latest = runs[0]; // store prepends newest
    elapsedMs = (latest.finishedMs ?? latest.startedMs) - latest.startedMs;
    tokens = latest.tokensIn + latest.tokensOut;
    label = zh
      ? latest.status === "done" ? "已完成" : latest.status === "cancelled" ? "已取消" : "已结束"
      : latest.status === "done" ? "done" : latest.status === "cancelled" ? "cancelled" : "ended";
  }

  return (
    <span
      className={`ax-runpill ${active ? "is-running" : "is-done"}`}
      title={active ? (zh ? "有智能体正在运行" : "agents running") : (zh ? "最近一次运行" : "last run")}
    >
      <GraphMark active={active} />
      <span className="ax-runpill__t">
        <span className="ax-runpill__time">{fmtDur(elapsedMs)}</span>
        {tokens > 0 && (
          <>
            <span className="ax-runpill__sep">·</span>
            {fmtTokens(tokens)} tokens
          </>
        )}
        <span className="ax-runpill__sep">·</span>
        {label}
      </span>
    </span>
  );
}
