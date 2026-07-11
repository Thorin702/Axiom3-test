// Native 2D vector-field / quiver plot: an arrow at each sample point (x,y) whose direction is
// the field components (u,v) and whose LENGTH is the field magnitude. Arrow length auto-scales
// to the median sample spacing so arrows do not overlap; color encodes magnitude via the house
// sequential blue ramp with a colorbar. Uniform pixels-per-unit keeps arrow angles true. Pure SVG.
//
// Spec (```quiver JSON): {"title"?, "xlabel"?, "ylabel"?, "unit"?,
//   "vectors":[{"x":<num>,"y":<num>,"u":<num>,"v":<num>}, ...]}
import { useId } from "react";

type Vec = { x: number; y: number; u: number; v: number };
type Spec = {
  title?: string;
  xlabel?: string;
  ylabel?: string;
  unit?: string;
  vectors: Vec[];
};

// data-viz sequential blue ramp (100 → 700), same accessor as the dot-plot / field map.
const RAMP = ["#cde2fb", "#9ec5f4", "#6da7ec", "#3987e5", "#256abf", "#184f95", "#0d366b"];
function ramp(t: number): string {
  const x = Math.max(0, Math.min(1, t)) * (RAMP.length - 1);
  return RAMP[Math.min(Math.floor(x), RAMP.length - 1)];
}

// nice round tick step for a span, aiming for ~target divisions (shared idiom with volcano/manhattan).
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
function fmt(v: number): string {
  if (!isFinite(v)) return "";
  const a = Math.abs(v);
  if (a !== 0 && (a < 0.01 || a >= 1e5)) return v.toExponential(1);
  if (a >= 100) return String(Math.round(v * 10) / 10);
  if (a >= 1) return String(Math.round(v * 100) / 100);
  return String(Math.round(v * 1000) / 1000);
}

// median nearest-neighbour distance in physical space → the natural arrow-length scale so the
// longest arrow spans a fraction of a cell and the field never turns into a hairball.
function medianSpacing(pts: { x: number; y: number }[]): number {
  const n = pts.length;
  if (n < 2) return 0;
  const ds: number[] = [];
  for (let i = 0; i < n; i++) {
    let best = Infinity;
    for (let j = 0; j < n; j++) {
      if (i === j) continue;
      const d = Math.hypot(pts[i].x - pts[j].x, pts[i].y - pts[j].y);
      if (d > 0 && d < best) best = d;
    }
    if (isFinite(best)) ds.push(best);
  }
  if (!ds.length) return 0;
  ds.sort((a, b) => a - b);
  const mid = Math.floor(ds.length / 2);
  return ds.length % 2 ? ds[mid] : (ds[mid - 1] + ds[mid]) / 2;
}

export function Quiver({ spec: raw, caption }: { spec: string; caption?: string }) {
  const uid = useId().replace(/:/g, "");
  const gradId = `axquiver-${uid}`;
  const kindLabel = "矢量场 · quiver";

  let spec: Spec;
  try {
    spec = JSON.parse(raw.trim());
  } catch (e) {
    return <div className="ax-fig__err">矢量场规范不是有效 JSON · {String((e as Error)?.message ?? e)}</div>;
  }
  if (!spec || typeof spec !== "object" || !Array.isArray(spec.vectors)) {
    return <div className="ax-fig__err">矢量场需要 vectors 数组（每项含 x / y / u / v）</div>;
  }

  // keep only fully-finite vectors; a bad component could otherwise place an arrow at NaN.
  const valid = spec.vectors.filter(
    (d): d is Vec =>
      !!d &&
      typeof d.x === "number" && isFinite(d.x) &&
      typeof d.y === "number" && isFinite(d.y) &&
      typeof d.u === "number" && isFinite(d.u) &&
      typeof d.v === "number" && isFinite(d.v),
  );
  if (valid.length === 0) {
    return <div className="ax-fig__err">矢量场需要有效的 vectors（每项 x / y / u / v 均为有限数字）</div>;
  }

  // magnitudes are derived client-side (nothing precomputed) → color domain.
  const mags = valid.map((d) => Math.hypot(d.u, d.v));
  const mMax = Math.max(...mags);
  const mMin = Math.min(...mags);
  const mSpan = mMax - mMin || 1;
  const unit = typeof spec.unit === "string" ? spec.unit : "";

  // physical domain from sample positions, padded by ~one cell so edge arrows are not clipped.
  const xs = valid.map((d) => d.x);
  const ys = valid.map((d) => d.y);
  let xMin = Math.min(...xs);
  let xMax = Math.max(...xs);
  let yMin = Math.min(...ys);
  let yMax = Math.max(...ys);
  const rawExt = Math.max(xMax - xMin, yMax - yMin);
  const spacingRaw = medianSpacing(valid.map((d) => ({ x: d.x, y: d.y })));
  const sp = spacingRaw > 0 ? spacingRaw : rawExt > 0 ? rawExt : 1; // single / coincident points fallback
  const pad = sp * 0.7;
  xMin -= pad;
  xMax += pad;
  yMin -= pad;
  yMax += pad;
  const xExt = xMax - xMin || 1;
  const yExt = yMax - yMin || 1;

  // aspect-preserving geometry: a single pixels-per-unit for both axes keeps arrow angles true.
  const targetPx = 380;
  const ppu = targetPx / Math.max(xExt, yExt);
  const plotW = xExt * ppu;
  const plotH = yExt * ppu;

  const padT = spec.title ? 30 : 14;
  const padL = 52;
  const padB = spec.xlabel ? 48 : 34;
  const gapBar = 26;
  const barW = 14;
  const padR = gapBar + barW + 46;
  const plotX0 = padL;
  const plotY0 = padT;
  const w = padL + plotW + padR;
  const h = padT + plotH + padB;

  const px = (x: number) => plotX0 + (x - xMin) * ppu;
  const py = (y: number) => plotY0 + plotH - (y - yMin) * ppu; // physical y up

  // pixels per magnitude unit: the largest vector spans ~60% of a cell — strictly less than the
  // 0.7-cell domain pad, so even a max-magnitude arrow at an edge sample stays inside the frame.
  const ARROW_FRAC = 0.6;
  const arrowScale = mMax > 0 ? (sp * ARROW_FRAC * ppu) / mMax : 0;

  const xticks = axisTicks(xMin, xMax, 6).filter((t) => t >= xMin - 1e-9 && t <= xMax + 1e-9);
  const yticks = axisTicks(yMin, yMax, 6).filter((t) => t >= yMin - 1e-9 && t <= yMax + 1e-9);

  // colorbar layout
  const legendH = Math.min(plotH, 200);
  const barX = plotX0 + plotW + gapBar;
  const barTop = padT + Math.max(0, (plotH - legendH) / 2);
  const colorbarLabel = unit ? `|·| (${unit})` : "magnitude";

  const HA = 0.42; // arrowhead half-angle (~24°)

  return (
    <figure className="ax-fig">
      <div className="ax-fig__frame" style={{ overflowX: "auto", justifyContent: "flex-start" }}>
        <svg
          viewBox={`0 0 ${w} ${h}`}
          width={w}
          height={h}
          role="img"
          aria-label={caption || spec.title || "vector field"}
          style={{ maxWidth: "100%" }}
        >
          <defs>
            <linearGradient id={gradId} x1="0" y1="0" x2="0" y2="1">
              {RAMP.map((_, i) => (
                <stop key={i} offset={`${(i / (RAMP.length - 1)) * 100}%`} stopColor={RAMP[RAMP.length - 1 - i]} />
              ))}
            </linearGradient>
          </defs>

          {spec.title && (
            <text x={padL} y={18} fontSize="12" fontWeight="600" fill="var(--ax-ink)">
              {spec.title}
            </text>
          )}

          {/* recessive grid */}
          {xticks.map((t, i) => (
            <line key={`gx${i}`} x1={px(t)} y1={plotY0} x2={px(t)} y2={plotY0 + plotH} stroke="var(--ax-line-soft)" strokeWidth="0.5" />
          ))}
          {yticks.map((t, i) => (
            <line key={`gy${i}`} x1={plotX0} y1={py(t)} x2={plotX0 + plotW} y2={py(t)} stroke="var(--ax-line-soft)" strokeWidth="0.5" />
          ))}

          {/* arrows */}
          {valid.map((d, i) => {
            const m = mags[i];
            const x1 = px(d.x);
            const y1 = py(d.y);
            const dxPx = d.u * arrowScale;
            const dyPx = -d.v * arrowScale; // physical y up
            const x2 = x1 + dxPx;
            const y2 = y1 + dyPx;
            const len = Math.hypot(dxPx, dyPx);
            const color = ramp((m - mMin) / mSpan);
            const title = (
              <title>{`x=${fmt(d.x)}, y=${fmt(d.y)} — (u ${fmt(d.u)}, v ${fmt(d.v)}) · |·| ${fmt(m)}${unit ? " " + unit : ""}`}</title>
            );
            // near-zero arrow: a dot marks the sample point (direction is undefined).
            if (len < 1.2) {
              return (
                <circle key={i} cx={x1} cy={y1} r={1.7} fill={color} stroke="var(--ax-surface)" strokeWidth="0.4">
                  {title}
                </circle>
              );
            }
            const ang = Math.atan2(dyPx, dxPx);
            const headLen = Math.min(Math.max(len * 0.32, 3), 7);
            const hx1 = x2 - headLen * Math.cos(ang - HA);
            const hy1 = y2 - headLen * Math.sin(ang - HA);
            const hx2 = x2 - headLen * Math.cos(ang + HA);
            const hy2 = y2 - headLen * Math.sin(ang + HA);
            return (
              <g key={i}>
                {title}
                <line x1={x1} y1={y1} x2={x2} y2={y2} stroke={color} strokeWidth="1.2" strokeLinecap="round" />
                <path
                  d={`M${x2.toFixed(2)} ${y2.toFixed(2)}L${hx1.toFixed(2)} ${hy1.toFixed(2)}L${hx2.toFixed(2)} ${hy2.toFixed(2)}Z`}
                  fill={color}
                />
              </g>
            );
          })}

          {/* plot border */}
          <rect x={plotX0} y={plotY0} width={plotW} height={plotH} fill="none" stroke="var(--ax-line)" strokeWidth="0.75" />

          {/* x ticks + labels */}
          {xticks.map((t, i) => (
            <g key={`xt${i}`}>
              <line x1={px(t)} y1={plotY0 + plotH} x2={px(t)} y2={plotY0 + plotH + 4} stroke="var(--ax-line)" strokeWidth="0.75" />
              <text x={px(t)} y={plotY0 + plotH + 15} fontSize="9" textAnchor="middle" fill="var(--ax-ink-faint)">
                {fmt(t)}
              </text>
            </g>
          ))}
          {/* y ticks + labels */}
          {yticks.map((t, i) => (
            <g key={`yt${i}`}>
              <line x1={plotX0 - 4} y1={py(t)} x2={plotX0} y2={py(t)} stroke="var(--ax-line)" strokeWidth="0.75" />
              <text x={plotX0 - 7} y={py(t) + 3} fontSize="9" textAnchor="end" fill="var(--ax-ink-faint)">
                {fmt(t)}
              </text>
            </g>
          ))}

          {/* axis titles */}
          {spec.xlabel && (
            <text x={plotX0 + plotW / 2} y={plotY0 + plotH + padB - 6} fontSize="10" textAnchor="middle" fill="var(--ax-ink-soft)">
              {spec.xlabel}
            </text>
          )}
          {spec.ylabel && (
            <text
              x={15}
              y={plotY0 + plotH / 2}
              fontSize="10"
              textAnchor="middle"
              fill="var(--ax-ink-soft)"
              transform={`rotate(-90 15 ${plotY0 + plotH / 2})`}
            >
              {spec.ylabel}
            </text>
          )}

          {/* magnitude colorbar */}
          <text x={barX} y={barTop - 6} fontSize="9" fill="var(--ax-ink-faint)">
            {colorbarLabel}
          </text>
          <rect x={barX} y={barTop} width={barW} height={legendH} fill={`url(#${gradId})`} />
          <rect x={barX} y={barTop} width={barW} height={legendH} fill="none" stroke="var(--ax-line)" strokeWidth="0.5" />
          <text x={barX + barW + 5} y={barTop + 4} fontSize="8" fill="var(--ax-ink-faint)">
            {fmt(mMax)}
          </text>
          <text x={barX + barW + 5} y={barTop + legendH} fontSize="8" fill="var(--ax-ink-faint)">
            {fmt(mMin)}
          </text>
        </svg>
      </div>
      <figcaption className="ax-fig__cap">
        <span className="ax-fig__kind">{kindLabel}</span> {caption || spec.title || ""}
      </figcaption>
    </figure>
  );
}
