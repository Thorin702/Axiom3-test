// Native binary-classifier evaluation curve: ROC (TPR vs FPR, with the chance diagonal) or
// precision-recall (precision vs recall, no diagonal). One or more models overlaid on a fixed
// [0,1]×[0,1] square; each curve is a pure-SVG polyline in a categorical hue, with its AUC shown
// in the legend exactly as emitted (never recomputed). Axes are fixed to [0,1].
//
// Spec (```roc JSON): {"title"?, "kind"?:"roc"|"pr",
//   "curves":[{"name":<str>,"auc"?:<0..1>,"ci"?:<half-width>|[lo,hi],"dash"?:<str>,"points":[[x,y],...]}]}
//   kind:"roc" -> points=[fpr,tpr] (draws chance diagonal) | "pr" -> points=[recall,precision].
// Optional publication-grade fields (all degrade to the current render when absent):
//   curves[].ci        — number (± half-width) or [lo,hi]; annotated next to the verbatim AUC.
//   curves[].dash      — per-curve SVG dash override; otherwise a color-blind-safe dash cycle is
//                        applied automatically when >1 curve overlap (redundant to hue, not color-only).
//   positive/negative  — class definitions surfaced in the caption.
//   n                  — sample size surfaced in the caption.
//   split              — data division ("train"|"held-out"|"cv"|"test"|free text) surfaced in caption.
//   illustrative       — when true, stamps a "示意·非实测" badge (no fabricated numbers implied).
import { catColor, isDark } from "./theme";

type CurveIn = { name?: string; auc?: number; ci?: number | number[]; dash?: string; points?: number[][] };
type Spec = {
  title?: string;
  kind?: string;
  curves: CurveIn[];
  positive?: string;
  negative?: string;
  n?: number;
  split?: string;
  illustrative?: boolean;
};

// color-blind-safe dash cycle: first curve solid, rest get distinguishable patterns so the
// figure stays readable in grayscale / for dichromats (redundant encoding, not hue-only).
const DASH_CYCLE = ["", "6 4", "2 3", "9 4 2 4", "1 4", "10 5"];

const clamp01 = (v: number) => Math.max(0, Math.min(1, v));

export function Roc({ spec: raw, caption }: { spec: string; caption?: string }) {
  let spec: Spec;
  try {
    spec = JSON.parse(raw.trim());
  } catch (e) {
    return <div className="ax-fig__err">ROC 规范不是有效 JSON · {String((e as Error)?.message || e)}</div>;
  }
  if (!spec || typeof spec !== "object" || !Array.isArray(spec.curves)) {
    return <div className="ax-fig__err">ROC 需要 curves 数组</div>;
  }

  const kind: "roc" | "pr" = spec.kind === "pr" ? "pr" : "roc";
  const title = typeof spec.title === "string" ? spec.title : "";

  // clean each curve: keep only finite point pairs (clamped to the [0,1] domain), preserve the
  // emitted order (the curve traversal), and carry the AUC through only if it is a finite number.
  type Curve = { name: string; auc?: number; pts: [number, number][] };
  const curves: Curve[] = spec.curves
    .filter((c): c is CurveIn => !!c && typeof c === "object" && Array.isArray(c.points))
    .map((c) => {
      const pts = (c.points as number[][])
        .filter(
          (p) =>
            Array.isArray(p) &&
            p.length >= 2 &&
            typeof p[0] === "number" &&
            typeof p[1] === "number" &&
            isFinite(p[0]) &&
            isFinite(p[1]),
        )
        .map((p) => [clamp01(p[0]), clamp01(p[1])] as [number, number]);
      return {
        name: typeof c.name === "string" ? c.name : "",
        auc: typeof c.auc === "number" && isFinite(c.auc) ? c.auc : undefined,
        pts,
      };
    })
    .filter((c) => c.pts.length >= 1);

  if (curves.length === 0) {
    return <div className="ax-fig__err">ROC 需要有效的 curves（每条含 points:[[x,y],…]）</div>;
  }

  const dark = isDark();
  const color = (i: number) => catColor(i, dark);

  // axis semantics differ between the two kinds; the domain is fixed to [0,1] for both.
  const xName = kind === "roc" ? "False positive rate" : "Recall";
  const yName = kind === "roc" ? "True positive rate" : "Precision";
  const xShort = kind === "roc" ? "FPR" : "recall";
  const yShort = kind === "roc" ? "TPR" : "precision";

  // geometry — a square plot so a diagonal reads at 45°.
  const padL = 46;
  const padR = 132; // legend column (name + AUC)
  const padT = title ? 30 : 16;
  const padB = 44;
  const plot = 300;

  // legend rows grow with the curve count; size the canvas so many curves are never clipped.
  const legRowH = 30;
  const legTop = padT + 6;
  const chanceExtra = kind === "roc" ? 14 : 0;
  const legendBottom = legTop + curves.length * legRowH + chanceExtra;

  const w = padL + plot + padR;
  const h = Math.max(padT + plot + padB, legendBottom + 12);
  const x0 = padL;
  const y0 = padT;
  const xTo = (x: number) => x0 + clamp01(x) * plot;
  const yTo = (y: number) => y0 + plot - clamp01(y) * plot; // invert: 0 at bottom

  const ticks = [0, 0.2, 0.4, 0.6, 0.8, 1];
  const fmtT = (v: number) => (v === 0 || v === 1 ? String(v) : v.toFixed(1));

  const lx = w - padR + 12;

  return (
    <figure className="ax-fig">
      <div className="ax-fig__frame" style={{ overflowX: "auto", justifyContent: "flex-start" }}>
        <svg
          viewBox={`0 0 ${w} ${h}`}
          width={w}
          height={h}
          role="img"
          aria-label={caption || title || (kind === "roc" ? "ROC curve" : "precision-recall curve")}
          style={{ maxWidth: "100%" }}
        >
          {title && (
            <text x={padL} y={16} fontSize="12" fontWeight="600" fill="var(--ax-ink)">
              {title}
            </text>
          )}

          {/* grid at each tick */}
          {ticks.map((t, i) => (
            <line key={`gx${i}`} x1={xTo(t)} y1={y0} x2={xTo(t)} y2={y0 + plot} stroke="var(--ax-line-soft)" strokeWidth="0.5" />
          ))}
          {ticks.map((t, i) => (
            <line key={`gy${i}`} x1={x0} y1={yTo(t)} x2={x0 + plot} y2={yTo(t)} stroke="var(--ax-line-soft)" strokeWidth="0.5" />
          ))}

          {/* plot border box */}
          <rect x={x0} y={y0} width={plot} height={plot} fill="none" stroke="var(--ax-line)" strokeWidth="1" />

          {/* chance diagonal (ROC only): TPR = FPR */}
          {kind === "roc" && (
            <line
              x1={xTo(0)}
              y1={yTo(0)}
              x2={xTo(1)}
              y2={yTo(1)}
              stroke="var(--ax-ink-faint)"
              strokeWidth="1"
              strokeDasharray="4 3"
            />
          )}

          {/* curves as polylines, in emitted order — dash cycle gives a redundant (non-hue)
              encoding so multiple curves stay separable in grayscale / for dichromats */}
          {curves.map((c, ci) => (
            <polyline
              key={`pl${ci}`}
              points={c.pts.map((p) => `${xTo(p[0])},${yTo(p[1])}`).join(" ")}
              fill="none"
              stroke={color(ci)}
              strokeWidth="2"
              strokeLinejoin="round"
              strokeLinecap="round"
              strokeDasharray={curves.length > 1 ? DASH_CYCLE[ci % DASH_CYCLE.length] || undefined : undefined}
            />
          ))}

          {/* vertex markers with native tooltips — only for curated/sparse curves, so a dense
              threshold sweep does not turn the line into an unreadable scatter. */}
          {curves.map((c, ci) =>
            c.pts.length <= 24
              ? c.pts.map((p, pi) => (
                  <circle key={`v${ci}-${pi}`} cx={xTo(p[0])} cy={yTo(p[1])} r={2} fill={color(ci)} stroke="var(--ax-surface)" strokeWidth="0.5">
                    <title>{`${c.name || `curve ${ci + 1}`} — ${xShort} ${p[0].toFixed(2)} · ${yShort} ${p[1].toFixed(2)}`}</title>
                  </circle>
                ))
              : null,
          )}

          {/* x ticks + axis title */}
          {ticks.map((t, i) => (
            <text key={`xt${i}`} x={xTo(t)} y={y0 + plot + 13} fontSize="9" textAnchor="middle" fill="var(--ax-ink-faint)">
              {fmtT(t)}
            </text>
          ))}
          <text x={x0 + plot / 2} y={y0 + plot + 34} fontSize="10" textAnchor="middle" fill="var(--ax-ink-soft)">
            {xName}
          </text>

          {/* y ticks + axis title */}
          {ticks.map((t, i) => (
            <text key={`yt${i}`} x={x0 - 6} y={yTo(t) + 3} fontSize="9" textAnchor="end" fill="var(--ax-ink-faint)">
              {fmtT(t)}
            </text>
          ))}
          <text x={13} y={y0 + plot / 2} fontSize="10" textAnchor="middle" fill="var(--ax-ink-soft)" transform={`rotate(-90 13 ${y0 + plot / 2})`}>
            {yName}
          </text>

          {/* legend — one entry per curve, AUC shown verbatim when supplied */}
          {curves.map((c, ci) => {
            const by = legTop + ci * legRowH;
            return (
              <g key={`lg${ci}`}>
                <line x1={lx} y1={by + 4} x2={lx + 16} y2={by + 4} stroke={color(ci)} strokeWidth="2.5" strokeLinecap="round" strokeDasharray={curves.length > 1 ? DASH_CYCLE[ci % DASH_CYCLE.length] || undefined : undefined} />
                <circle cx={lx + 8} cy={by + 4} r={2} fill={color(ci)} />
                <text x={lx + 22} y={by + 7} fontSize="9.5" fill="var(--ax-ink-soft)">
                  {c.name || `curve ${ci + 1}`}
                </text>
                {c.auc !== undefined && (
                  <text x={lx + 22} y={by + 18} fontSize="8.5" fill="var(--ax-ink-faint)">
                    {`AUC ${c.auc.toFixed(2)}`}
                  </text>
                )}
              </g>
            );
          })}
          {/* chance-line key (ROC only) */}
          {kind === "roc" && (
            <g>
              <line
                x1={lx}
                y1={legTop + curves.length * legRowH + 4}
                x2={lx + 16}
                y2={legTop + curves.length * legRowH + 4}
                stroke="var(--ax-ink-faint)"
                strokeWidth="1"
                strokeDasharray="4 3"
              />
              <text x={lx + 22} y={legTop + curves.length * legRowH + 7} fontSize="8.5" fill="var(--ax-ink-faint)">
                chance
              </text>
            </g>
          )}
        </svg>
      </div>
      <figcaption className="ax-fig__cap">
        <span className="ax-fig__kind">ROC · roc</span> {caption || title || ""}
      </figcaption>
    </figure>
  );
}
