// Boot splash — the app's knowledge-graph mark, highlighted and pulsing, centered on the app surface
// for ~1.5s before the workbench / onboarding appears. Pure ornament: NO text, just the pulsing icon.
// Reuses the exact graph shape from the run-status pill so the launch mark matches the app's identity.
import { useEffect, useState } from "react";
import { NODES, EDGES, GRAPH_VIEWBOX } from "./RunStatusPill";

export function Splash({ onDone }: { onDone: () => void }) {
  const [leaving, setLeaving] = useState(false);
  useEffect(() => {
    const t1 = setTimeout(() => setLeaving(true), 1500); // ~1.5s of pulse, then fade out
    const t2 = setTimeout(onDone, 1820);
    return () => {
      clearTimeout(t1);
      clearTimeout(t2);
    };
  }, [onDone]);

  return (
    <div className={`ax-splash ${leaving ? "is-leaving" : ""}`} aria-hidden>
      <svg className="ax-splash__mark" viewBox={GRAPH_VIEWBOX} width="132" height="132">
        <path className="ax-splash__edges" d={EDGES} />
        {NODES.map(([cx, cy, r], i) => (
          <circle
            key={i}
            className="ax-splash__node"
            cx={cx}
            cy={cy}
            r={r}
            style={{ animationDelay: `${i * 0.11}s` }}
          />
        ))}
      </svg>
    </div>
  );
}
