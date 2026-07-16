// Native publication-bias funnel plot (companion to the forest plot). x = effect size,
// y = standard error (INVERTED: high-precision / small-SE studies sit at the top, the apex).
// Around the pooled effect (or, absent that, the mean effect) we draw the PSEUDO 95% CI funnel:
// two straight boundaries effect ± 1.96·SE that open linearly with SE from the apex downward.
// Each study is one scatter point; if Egger's test p is supplied it is annotated (threshold 0.10).
// Nothing is fabricated: the funnel is a purely geometric construct from the supplied effects/SEs,
// and asymmetry is NEVER interpreted for the reader (the caption carries the mandatory caveats).
//
// Spec (```funnel JSON): {"title"?, "effectLabel"?,
//   "studies":[{"effect":<num>,"se":<num>0,"name"?:<str>}],
//   "pooledEffect"?:<num>,   // funnel centre; falls back to the mean effect when absent
//   "eggerP"?:<num 0..1>}    // Egger regression-intercept test p, annotated at threshold 0.10
// Every field except studies is OPTIONAL → graceful degradation, no fabricated numbers.
import { catColor, isDark } from "./theme";

type Study = { effect: number; se: number; name?: string };
type Spec = {
  title?: string;
  effectLabel?: string;
  studies: Study[];
  pooledEffect?: number;
  eggerP?: number;
};

// nice round tick step for a span, aiming for ~target divisions (same helper family as siblings).
function niceStep(span: number, target: number): number {
  if (!(span > 0) || !isFinite(span)) return 1;
  const raw = span / target;
  const mag = Math.pow(10, Math.floor(Math.log10(raw)));
  const norm = raw / mag;
  const step = norm < 1.5 ? 1 : norm < 3 ? 2 : norm < 7 ? 5 : 10;
  return step * mag;
}
function axisTicks(min: number, max: number, target: number): number[] {
  const step = niceStep(max - min, target);
  const start = Math.ceil(min / step) * step;
  const out: number[] = [];
  for (let v = start; v <= max + step * 1e-6; v += step) {
    out.push(Math.abs(v) < step * 1e-9 ? 0 : v);
  }
  return out;
}
const fmt = (v: number) => (Number.isFinite(v) ? Number(v.toFixed(2)).toString() : "—");
function fmtP(p: number): string {
  if (!isFinite(p)) return String(p);
  if (p === 0) return "0";
  if (p < 1e-3) return p.toExponential(1);
  return p.toFixed(3);
}

export function Funnel({ spec: raw, caption }: { spec: string; caption?: string }) {
  let spec: Spec;
  try {
    spec = JSON.parse(raw.trim());
  } catch (e) {
    return <div className="ax-fig__err">漏斗图规范不是有效 JSON · {String((e as Error)?.message || e)}</div>;
  }
  if (!spec || typeof spec !== "object" || !Array.isArray(spec.studies)) {
    return <div className="ax-fig__err">漏斗图需要 studies 数组（每项含 effect 与 se&gt;0）</div>;
  }

  // clean: effect finite, se finite & strictly positive. Malformed rows are dropped, never patched.
  const studies = spec.studies.filter(
    (s): s is Study =>
      !!s && typeof s.effect === "number" && isFinite(s.effect) && typeof s.se === "number" && isFinite(s.se) && s.se > 0,
  );
  if (studies.length === 0) {
    return <div className="ax-fig__err">漏斗图无有效研究（每项需数值 effect 与 se&gt;0）</div>;
  }

  const n = studies.length;
  // funnel centre: pooledEffect if supplied, else the (unweighted) mean effect — labelled accordingly.
  const hasPooled = typeof spec.pooledEffect === "number" && isFinite(spec.pooledEffect);
  const center = hasPooled ? (spec.pooledEffect as number) : studies.reduce((a, s) => a + s.effect, 0) / n;
  const centerLabel = hasPooled ? "合并效应" : "效应均值";

  const seMax = Math.max(...studies.map((s) => s.se));
  const seLim = seMax * 1.06 || 1;
  // funnel half-width at the widest SE, in effect units, defines the x-domain together with the data.
  const halfAtBottom = 1.96 * seLim;
  const effects = studies.map((s) => s.effect);
  let xMin = Math.min(center - halfAtBottom, ...effects);
  let xMax = Math.max(center + halfAtBottom, ...effects);
  const xSpan = xMax - xMin || Math.abs(center) * 0.2 || 1;
  xMin -= xSpan * 0.05;
  xMax += xSpan * 0.05;

  const dark = isDark();
  const dotColor = catColor(0, dark);
  const hasEgger = typeof spec.eggerP === "number" && isFinite(spec.eggerP) && spec.eggerP >= 0 && spec.eggerP <= 1;
  const eggerAsym = hasEgger && (spec.eggerP as number) < 0.1;

  // geometry
  const padL = 54;
  const padR = 118;
  const padT = spec.title ? 30 : 16;
  const padB = 46;
  const plotW = 420;
  const plotH = 320;
  const w = padL + plotW + padR;
  const h = padT + plotH + padB;
  const x0 = padL;
  const y0 = padT;
  const xTo = (v: number) => x0 + ((v - xMin) / (xMax - xMin)) * plotW;
  const yTo = (se: number) => y0 + (se / seLim) * plotH; // SE=0 at top (apex), seLim at bottom

  const xticks = axisTicks(xMin, xMax, 6);
  const seticks = axisTicks(0, seLim, 5);

  // pseudo-95%CI funnel boundaries: apex (center, SE=0) → (center ± 1.96·seLim, seLim).
  const apexX = xTo(center);
  const apexY = yTo(0);
  const lx = xTo(center - halfAtBottom);
  const rx = xTo(center + halfAtBottom);
  const botY = yTo(seLim);
  const funnelPts = `${apexX},${apexY} ${lx},${botY} ${rx},${botY}`;

  const legX = w - padR + 12;

  return (
    <figure className="ax-fig">
      <div className="ax-fig__frame" style={{ overflowX: "auto", justifyContent: "flex-start" }}>
        <svg
          viewBox={`0 0 ${w} ${h}`}
          width={w}
          height={h}
          role="img"
          aria-label={caption || spec.title || "funnel plot"}
          style={{ maxWidth: "100%" }}
        >
          {spec.title && (
            <text x={padL} y={16} fontSize="12" fontWeight="600" fill="var(--ax-ink)">
              {spec.title}
            </text>
          )}

          {/* grid */}
          {seticks.map((t, i) => (
            <line key={`gy${i}`} x1={x0} y1={yTo(t)} x2={x0 + plotW} y2={yTo(t)} stroke="var(--ax-line-soft)" strokeWidth="0.5" />
          ))}
          {xticks.map((t, i) => (
            <line key={`gx${i}`} x1={xTo(t)} y1={y0} x2={xTo(t)} y2={y0 + plotH} stroke="var(--ax-line-soft)" strokeWidth="0.5" />
          ))}

          {/* pseudo-95%CI funnel region + boundaries (geometric, data-defined) */}
          <polygon points={funnelPts} fill="var(--ax-ink-faint)" fillOpacity="0.06" stroke="none" />
          <line x1={apexX} y1={apexY} x2={lx} y2={botY} stroke="var(--ax-ink-faint)" strokeWidth="1" strokeDasharray="4 3" />
          <line x1={apexX} y1={apexY} x2={rx} y2={botY} stroke="var(--ax-ink-faint)" strokeWidth="1" strokeDasharray="4 3" />
          {/* centre line (pooled / mean effect) — solid, distinct from the dashed boundaries */}
          <line x1={apexX} y1={apexY} x2={apexX} y2={botY} stroke="var(--ax-line-strong)" strokeWidth="1" />
          <text x={apexX} y={y0 - 4} fontSize="8.5" textAnchor="middle" fill="var(--ax-ink-faint)">
            {`${centerLabel} ${fmt(center)}`}
          </text>

          {/* axis frame */}
          <line x1={x0} y1={y0} x2={x0} y2={y0 + plotH} stroke="var(--ax-line)" strokeWidth="1" />
          <line x1={x0} y1={y0 + plotH} x2={x0 + plotW} y2={y0 + plotH} stroke="var(--ax-line)" strokeWidth="1" />

          {/* study points */}
          {studies.map((s, i) => (
            <circle
              key={`p${i}`}
              cx={xTo(s.effect)}
              cy={yTo(s.se)}
              r="3"
              fill={dotColor}
              fillOpacity="0.85"
              stroke="var(--ax-surface)"
              strokeWidth="0.5"
            >
              <title>{`${s.name ? s.name + " — " : ""}effect ${fmt(s.effect)} · SE ${fmt(s.se)}`}</title>
            </circle>
          ))}

          {/* x ticks + label */}
          {xticks.map((t, i) => (
            <text key={`xt${i}`} x={xTo(t)} y={y0 + plotH + 13} fontSize="9" textAnchor="middle" fill="var(--ax-ink-faint)">
              {fmt(t)}
            </text>
          ))}
          <text x={x0 + plotW / 2} y={h - 6} fontSize="10" textAnchor="middle" fill="var(--ax-ink-soft)">
            {spec.effectLabel || "效应量 effect size"}
          </text>

          {/* SE ticks + label (inverted axis) */}
          {seticks.map((t, i) => (
            <text key={`yt${i}`} x={x0 - 6} y={yTo(t) + 3} fontSize="9" textAnchor="end" fill="var(--ax-ink-faint)">
              {fmt(t)}
            </text>
          ))}
          <text
            x={14}
            y={y0 + plotH / 2}
            fontSize="10"
            textAnchor="middle"
            fill="var(--ax-ink-soft)"
            transform={`rotate(-90 14 ${y0 + plotH / 2})`}
          >
            标准误 SE（精度高在上）
          </text>

          {/* legend + provenance-free annotations (n, Egger) */}
          <g>
            <circle cx={legX + 4} cy={padT + 8} r="3" fill={dotColor} fillOpacity="0.9" />
            <text x={legX + 14} y={padT + 11} fontSize="9.5" fill="var(--ax-ink-soft)">{`研究 (n=${n})`}</text>
            <line x1={legX} y1={padT + 24} x2={legX + 22} y2={padT + 24} stroke="var(--ax-ink-faint)" strokeWidth="1" strokeDasharray="4 3" />
            <text x={legX} y={padT + 38} fontSize="8.5" fill="var(--ax-ink-faint)">伪 95%CI 漏斗边界</text>
            <text x={legX} y={padT + 49} fontSize="8.5" fill="var(--ax-ink-faint)">effect ± 1.96·SE</text>
            {hasEgger && (
              <>
                <text x={legX} y={padT + 68} fontSize="9" fill="var(--ax-ink-soft)">{`Egger p = ${fmtP(spec.eggerP as number)}`}</text>
                <text x={legX} y={padT + 79} fontSize="8" fill="var(--ax-ink-faint)">
                  {eggerAsym ? "p<0.10：不对称信号" : "p≥0.10：无显著不对称"}
                </text>
              </>
            )}
            {n < 10 && (
              <text x={legX} y={padT + (hasEgger ? 96 : 68)} fontSize="8" fill="var(--ax-ink-faint)">
                n&lt;10 不判读
              </text>
            )}
          </g>
        </svg>
      </div>
      <figcaption className="ax-fig__cap">
        <span className="ax-fig__kind">漏斗图 · funnel plot</span> {caption || spec.title || ""}
        {" "}研究数&lt;10 不解读不对称性；不对称≠必然发表偏倚（亦可源于真实异质性、效应量与精度相关等）。
      </figcaption>
    </figure>
  );
}
