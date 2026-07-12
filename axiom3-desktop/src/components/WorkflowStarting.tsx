// The "Starting workflow" launch card — shown in the live thinking chain during the brief window
// after a run is accepted but BEFORE its first agent spawns. Mirrors Claude Code's workflow-start
// card: a dark rounded card with the app's knowledge-graph mark pulsing, a breathing label, a
// chevron, and a shimmering skeleton line. Reuses the RunStatusPill mark classes so the pulse
// animation stays identical across the app.
import type { Lang } from "../i18n";

// Same knowledge-graph shape as the app icon / RunStatusPill — inherits currentColor; each node
// pulses on a staggered delay via the shared .ax-runpill__node / .is-pulsing CSS.
const NODES: [number, number, number][] = [
  [13, 15, 3.3], [30, 11, 3.3], [41, 21, 3.3], [24, 30, 4.3], [38, 35, 3.3], [16, 42, 3.3], [31, 43, 3.3],
];
const EDGES =
  "M13 15 L30 11 M30 11 L41 21 M13 15 L24 30 M30 11 L24 30 M41 21 L38 35 M24 30 L38 35 M24 30 L16 42 M38 35 L31 43 M16 42 L31 43";

export function WorkflowStarting({ lang, label }: { lang: Lang; label?: string }) {
  const zh = lang === "zh";
  const text = label ?? (zh ? "正在启动工作流" : "Starting workflow");
  return (
    <div className="ax-wfstart" role="status" aria-live="polite">
      <div className="ax-wfstart__head">
        <svg className="ax-runpill__mark is-pulsing ax-wfstart__mark" viewBox="7 5 40 44" width="17" height="17" aria-hidden>
          <path className="ax-runpill__edges" d={EDGES} />
          {NODES.map(([cx, cy, r], i) => (
            <circle key={i} className="ax-runpill__node" cx={cx} cy={cy} r={r} style={{ animationDelay: `${i * 0.15}s` }} />
          ))}
        </svg>
        <span className="ax-wfstart__label">{text}</span>
        <span className="ax-wfstart__dots" aria-hidden><i /><i /><i /></span>
        <span className="ax-wfstart__spacer" />
        <span className="ax-wfstart__chev" aria-hidden>›</span>
      </div>
      <div className="ax-wfstart__skeleton" aria-hidden>
        <span className="ax-wfstart__bar" />
      </div>
    </div>
  );
}
