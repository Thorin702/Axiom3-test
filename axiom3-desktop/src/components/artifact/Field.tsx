// Native 2D scalar-field / contour map: a continuous quantity z(x,y) over a coordinate grid —
// temperature/stress/strain maps, potential fields, intensity, DOE sensitivity surfaces. One
// <rect> per sample cell (the DotPlot recipe scaled up), colored by the house blue ramp
// (`sequential`) or a signed pole-over-surface (`diverging`); an optional marching-squares pass
// draws iso-contour polylines for the data-specified `contours`. Pure SVG, no deps.
//
// Spec (```field JSON): {"title"?, "xlabel"?, "ylabel"?, "unit"?, "scale"?:"sequential"|"diverging",
//   "x":[...], "y":[...], "z":[[..row-major z[iy][ix]..]], "contours"?:[...], "domain"?:[min,max]}
import { useId } from "react";
import { sequential } from "./theme";

type Spec = {
  title?: string;
  xlabel?: string;
  ylabel?: string;
  unit?: string;
  scale?: "sequential" | "diverging";
  x: number[];
  y: number[];
  z: number[][];
  contours?: number[];
  domain?: [number, number];
};

type Pt = { x: number; y: number };

// Continuous colour follows the rubric: viridis for unsigned magnitude (perceptually uniform,
// CVD-safe) via the shared `sequential()`. Diverging keeps two RdBu poles over the theme surface
// (opacity carries magnitude; midpoint = theme surface underneath) — never a hue at 0.
const NEG = "#2166ac"; // RdBu blue pole
const POS = "#b2182b"; // RdBu red pole
const SEQ_STOPS = 8; // viridis samples for the sequential colorbar gradient

// cell boundaries at the midpoints between samples (handles non-uniform spacing); the two
// outer edges extend half a step so the field tiles the plot exactly.
function edges(vals: number[]): number[] {
  const n = vals.length;
  if (n === 1) return [vals[0] - 0.5, vals[0] + 0.5];
  const e = new Array<number>(n + 1);
  e[0] = vals[0] - (vals[1] - vals[0]) / 2;
  for (let i = 1; i < n; i++) e[i] = (vals[i - 1] + vals[i]) / 2;
  e[n] = vals[n - 1] + (vals[n - 1] - vals[n - 2]) / 2;
  return e;
}

function fmt(v: number): string {
  if (!isFinite(v)) return "";
  const a = Math.abs(v);
  if (a !== 0 && (a < 0.01 || a >= 100000)) return v.toExponential(1);
  if (a >= 100) return String(Math.round(v * 10) / 10);
  if (a >= 1) return String(Math.round(v * 100) / 100);
  return String(Math.round(v * 1000) / 1000);
}
const n2 = (v: number): number => Math.round(v * 100) / 100;

function interp(level: number, va: number, vb: number, pa: Pt, pb: Pt): Pt {
  const t = vb === va ? 0.5 : (level - va) / (vb - va);
  return { x: pa.x + t * (pb.x - pa.x), y: pa.y + t * (pb.y - pa.y) };
}

export function Field({ spec: raw, caption }: { spec: string; caption?: string }) {
  const uid = useId().replace(/:/g, "");
  const gradId = `axfield-${uid}`;
  const kindLabel = "场图 · field";

  let spec: Spec;
  try {
    spec = JSON.parse(raw.trim());
  } catch (e) {
    return <div className="ax-fig__err">场图规范不是有效 JSON · {String((e as Error)?.message ?? e)}</div>;
  }
  if (!spec || typeof spec !== "object") return <div className="ax-fig__err">场图规范为空或格式不对</div>;
  if (!Array.isArray(spec.x) || !Array.isArray(spec.y) || !Array.isArray(spec.z)) {
    return <div className="ax-fig__err">场图需要 x / y / z 三个数组（z 为 z[iy][ix] 的二维网格）</div>;
  }
  // Keep x / y aligned with z's indices — never drop a bad coordinate (that would silently
  // shift every downstream sample). Require the whole axis to be finite, else refuse to draw.
  const x = spec.x;
  const y = spec.y;
  const z = spec.z;
  const nx = x.length;
  const ny = y.length;
  if (nx < 1 || ny < 1 || !Array.isArray(z[0])) {
    return <div className="ax-fig__err">场图的 x / y 至少各需一个坐标，z 至少一行</div>;
  }
  if (!x.every((v) => typeof v === "number" && isFinite(v)) || !y.every((v) => typeof v === "number" && isFinite(v))) {
    return <div className="ax-fig__err">场图坐标轴 x / y 必须是有限数字（不能含空值或非数）</div>;
  }

  const diverging = spec.scale === "diverging";
  const unit = typeof spec.unit === "string" ? spec.unit : "";

  // ── color domain ──────────────────────────────────────────────────────────
  const flat: number[] = [];
  for (const row of z) if (Array.isArray(row)) for (const v of row) if (typeof v === "number" && isFinite(v)) flat.push(v);
  // only honour `domain` when both bounds are finite numbers — otherwise fall back to data extents
  const dom: [number, number] | null =
    Array.isArray(spec.domain) &&
    spec.domain.length === 2 &&
    typeof spec.domain[0] === "number" &&
    isFinite(spec.domain[0]) &&
    typeof spec.domain[1] === "number" &&
    isFinite(spec.domain[1])
      ? [spec.domain[0], spec.domain[1]]
      : null;
  let dmin: number;
  let dmax: number;
  let maxAbs = 1;
  if (diverging) {
    if (dom) {
      maxAbs = Math.max(Math.abs(dom[0]), Math.abs(dom[1]));
    } else if (flat.length) {
      maxAbs = Math.max(...flat.map((v) => Math.abs(v)));
    }
    maxAbs = maxAbs || 1;
    dmin = -maxAbs;
    dmax = maxAbs;
  } else if (dom) {
    dmin = dom[0];
    dmax = dom[1];
  } else if (flat.length) {
    dmin = Math.min(...flat);
    dmax = Math.max(...flat);
  } else {
    dmin = 0;
    dmax = 1;
  }
  const span = dmax - dmin || 1;

  // ── geometry (physical coordinates → pixels, aspect-preserving) ────────────
  const ex = edges(x);
  const ey = edges(y);
  const xMin = ex[0];
  const xMax = ex[nx];
  const yMin = ey[0];
  const yMax = ey[ny];
  const xExt = Math.abs(xMax - xMin) || 1;
  const yExt = Math.abs(yMax - yMin) || 1;
  const res = Math.max(nx, ny);
  const target = res <= 6 ? 300 : res <= 12 ? 360 : res <= 24 ? 420 : 480;
  const ppu = target / Math.max(xExt, yExt);
  const plotW = xExt * ppu;
  const plotH = yExt * ppu;

  const padT = spec.title ? 30 : 14;
  const padL = 56;
  const padB = 48;
  const gapBar = 26;
  const barW = 14;
  const padR = gapBar + barW + 50;
  const plotX0 = padL;
  const plotY0 = padT;

  const xDen = (xMax - xMin) || 1;
  const yDen = (yMax - yMin) || 1;
  const px = (v: number) => plotX0 + ((v - xMin) / xDen) * plotW;
  const py = (v: number) => plotY0 + plotH - ((v - yMin) / yDen) * plotH; // physical y up
  const gpx = (ix: number) => px(x[ix]);
  const gpy = (iy: number) => py(y[iy]);

  const w = padL + plotW + padR;
  const h = padT + plotH + padB;

  // colorbar layout
  const legendH = Math.min(plotH, 200);
  const barX = padL + plotW + gapBar;
  const barTop = padT + Math.max(0, (plotH - legendH) / 2);
  const yForVal = (v: number) =>
    diverging
      ? barTop + (1 - (v + maxAbs) / (2 * maxAbs)) * legendH
      : barTop + (1 - (v - dmin) / span) * legendH;

  // ── iso-contours via marching squares over the sample grid ─────────────────
  const contourLevels = (Array.isArray(spec.contours) ? spec.contours : []).filter(
    (c) => typeof c === "number" && isFinite(c),
  );
  const contourPaths = contourLevels
    .map((level) => {
      const parts: string[] = [];
      const seg = (a: Pt, b: Pt) => parts.push(`M${n2(a.x)} ${n2(a.y)}L${n2(b.x)} ${n2(b.y)}`);
      for (let iy = 0; iy < ny - 1; iy++) {
        for (let ix = 0; ix < nx - 1; ix++) {
          const vBL = z[iy]?.[ix];
          const vBR = z[iy]?.[ix + 1];
          const vTR = z[iy + 1]?.[ix + 1];
          const vTL = z[iy + 1]?.[ix];
          if (
            typeof vBL !== "number" || !isFinite(vBL) ||
            typeof vBR !== "number" || !isFinite(vBR) ||
            typeof vTR !== "number" || !isFinite(vTR) ||
            typeof vTL !== "number" || !isFinite(vTL)
          )
            continue;
          const pBL = { x: gpx(ix), y: gpy(iy) };
          const pBR = { x: gpx(ix + 1), y: gpy(iy) };
          const pTR = { x: gpx(ix + 1), y: gpy(iy + 1) };
          const pTL = { x: gpx(ix), y: gpy(iy + 1) };
          const aBL = vBL >= level;
          const aBR = vBR >= level;
          const aTR = vTR >= level;
          const aTL = vTL >= level;
          const cb = aBL !== aBR ? interp(level, vBL, vBR, pBL, pBR) : null; // bottom edge
          const cr = aBR !== aTR ? interp(level, vBR, vTR, pBR, pTR) : null; // right edge
          const ct = aTR !== aTL ? interp(level, vTR, vTL, pTR, pTL) : null; // top edge
          const cl = aTL !== aBL ? interp(level, vTL, vBL, pTL, pBL) : null; // left edge
          const present = [cb, cr, ct, cl].filter((p): p is Pt => p !== null);
          if (present.length === 2) {
            seg(present[0], present[1]);
          } else if (present.length === 4 && cb && cr && ct && cl) {
            // saddle: pair so the contour isolates corners differing from the cell centre.
            const centerAbove = (vBL + vBR + vTR + vTL) / 4 >= level;
            if (aBL === centerAbove) {
              seg(cb, cr);
              seg(ct, cl);
            } else {
              seg(cb, cl);
              seg(ct, cr);
            }
          }
        }
      }
      return { level, d: parts.join("") };
    })
    .filter((p) => p.d.length > 0);

  // axis tick indices (thin to ~8 labels on each axis)
  const xStep = Math.max(1, Math.ceil(nx / 8));
  const yStep = Math.max(1, Math.ceil(ny / 8));
  const xTicks: number[] = [];
  for (let i = 0; i < nx; i += xStep) xTicks.push(i);
  const yTicks: number[] = [];
  for (let i = 0; i < ny; i += yStep) yTicks.push(i);

  const withTitles = nx * ny <= 1200;
  const colorbarLabel = unit || "value";

  return (
    <figure className="ax-fig">
      <div className="ax-fig__frame" style={{ overflowX: "auto", justifyContent: "flex-start" }}>
        <svg
          viewBox={`0 0 ${w} ${h}`}
          width={w}
          height={h}
          role="img"
          aria-label={caption || spec.title || "field map"}
          style={{ maxWidth: "100%" }}
        >
          <defs>
            {diverging ? (
              <linearGradient id={gradId} x1="0" y1="0" x2="0" y2="1">
                <stop offset="0%" stopColor={POS} stopOpacity="1" />
                <stop offset="50%" stopColor={POS} stopOpacity="0" />
                <stop offset="50%" stopColor={NEG} stopOpacity="0" />
                <stop offset="100%" stopColor={NEG} stopOpacity="1" />
              </linearGradient>
            ) : (
              <linearGradient id={gradId} x1="0" y1="0" x2="0" y2="1">
                {Array.from({ length: SEQ_STOPS }, (_, i) => (
                  <stop key={i} offset={`${(i / (SEQ_STOPS - 1)) * 100}%`} stopColor={sequential(1 - i / (SEQ_STOPS - 1))} />
                ))}
              </linearGradient>
            )}
          </defs>

          {spec.title && (
            <text x={padL} y={18} fontSize="12" fontWeight="600" fill="var(--ax-ink)">
              {spec.title}
            </text>
          )}

          {/* surface under the field so low-opacity diverging cells blend to the theme background */}
          <rect x={plotX0} y={plotY0} width={plotW} height={plotH} fill="var(--ax-surface)" />

          {/* field cells */}
          {y.map((_, iy) =>
            x.map((_, ix) => {
              const v = z[iy]?.[ix];
              if (typeof v !== "number" || !isFinite(v)) return null;
              let fill: string;
              let opacity: number;
              if (diverging) {
                fill = v < 0 ? NEG : POS;
                opacity = Math.min(1, Math.abs(v) / maxAbs);
              } else {
                fill = sequential((v - dmin) / span);
                opacity = 1;
              }
              const cx0 = px(ex[ix]);
              const cx1 = px(ex[ix + 1]);
              const cy0 = py(ey[iy]);
              const cy1 = py(ey[iy + 1]);
              const rx = Math.min(cx0, cx1);
              const rw = Math.abs(cx1 - cx0);
              const ry = Math.min(cy0, cy1);
              const rh = Math.abs(cy1 - cy0);
              return (
                <rect
                  key={`${iy}-${ix}`}
                  x={rx}
                  y={ry}
                  width={rw}
                  height={rh}
                  fill={fill}
                  fillOpacity={opacity}
                  shapeRendering="crispEdges"
                >
                  {withTitles && (
                    <title>{`x=${fmt(x[ix])}, y=${fmt(y[iy])} — ${fmt(v)}${unit ? " " + unit : ""}`}</title>
                  )}
                </rect>
              );
            }),
          )}

          {/* iso-contours — a surface-colored halo under an ink line keeps them legible on both
              the palest and the darkest cells, in light and dark themes alike */}
          {contourPaths.map((c, i) => (
            <path
              key={`cth${i}`}
              d={c.d}
              fill="none"
              stroke="var(--ax-surface)"
              strokeOpacity="0.55"
              strokeWidth="2.4"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          ))}
          {contourPaths.map((c, i) => (
            <path
              key={`ct${i}`}
              d={c.d}
              fill="none"
              stroke="var(--ax-ink)"
              strokeOpacity="0.7"
              strokeWidth="0.9"
              strokeLinecap="round"
            >
              <title>{`${fmt(c.level)}${unit ? " " + unit : ""}`}</title>
            </path>
          ))}

          {/* plot border + axes */}
          <rect x={plotX0} y={plotY0} width={plotW} height={plotH} fill="none" stroke="var(--ax-line)" strokeWidth="0.75" />

          {/* x ticks + labels */}
          {xTicks.map((i) => (
            <g key={`xt${i}`}>
              <line x1={px(x[i])} y1={plotY0 + plotH} x2={px(x[i])} y2={plotY0 + plotH + 4} stroke="var(--ax-line)" strokeWidth="0.75" />
              <text x={px(x[i])} y={plotY0 + plotH + 15} fontSize="9" textAnchor="middle" fill="var(--ax-ink-faint)">
                {fmt(x[i])}
              </text>
            </g>
          ))}
          {/* y ticks + labels */}
          {yTicks.map((i) => (
            <g key={`yt${i}`}>
              <line x1={plotX0 - 4} y1={py(y[i])} x2={plotX0} y2={py(y[i])} stroke="var(--ax-line)" strokeWidth="0.75" />
              <text x={plotX0 - 7} y={py(y[i]) + 3} fontSize="9" textAnchor="end" fill="var(--ax-ink-faint)">
                {fmt(y[i])}
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
              x={16}
              y={plotY0 + plotH / 2}
              fontSize="10"
              textAnchor="middle"
              fill="var(--ax-ink-soft)"
              transform={`rotate(-90 16 ${plotY0 + plotH / 2})`}
            >
              {spec.ylabel}
            </text>
          )}

          {/* colorbar */}
          <text x={barX} y={barTop - 6} fontSize="9" fill="var(--ax-ink-faint)">
            {colorbarLabel}
          </text>
          {diverging && <rect x={barX} y={barTop} width={barW} height={legendH} fill="var(--ax-surface)" />}
          <rect x={barX} y={barTop} width={barW} height={legendH} fill={`url(#${gradId})`} />
          <rect x={barX} y={barTop} width={barW} height={legendH} fill="none" stroke="var(--ax-line)" strokeWidth="0.5" />

          {/* colorbar endpoints */}
          <text x={barX + barW + 5} y={barTop + 4} fontSize="8" fill="var(--ax-ink-faint)">
            {fmt(dmax)}
          </text>
          {diverging && (
            <text x={barX + barW + 5} y={barTop + legendH / 2 + 3} fontSize="8" fill="var(--ax-ink-faint)">
              0
            </text>
          )}
          <text x={barX + barW + 5} y={barTop + legendH} fontSize="8" fill="var(--ax-ink-faint)">
            {fmt(dmin)}
          </text>

          {/* contour levels marked on the colorbar (ties each iso-line to its color) */}
          {contourLevels
            .filter((c) => c >= Math.min(dmin, dmax) && c <= Math.max(dmin, dmax))
            .map((c, i) => {
              const cy = yForVal(c);
              return (
                <g key={`cbt${i}`}>
                  <line x1={barX} y1={cy} x2={barX + barW} y2={cy} stroke="var(--ax-ink)" strokeOpacity="0.5" strokeWidth="0.6" />
                  <line x1={barX + barW} y1={cy} x2={barX + barW + 3} y2={cy} stroke="var(--ax-ink-soft)" strokeWidth="0.75" />
                  <text x={barX + barW + 5} y={cy + 3} fontSize="8" fill="var(--ax-ink-soft)">
                    {fmt(c)}
                  </text>
                </g>
              );
            })}
        </svg>
      </div>
      <figcaption className="ax-fig__cap">
        <span className="ax-fig__kind">{kindLabel}</span> {caption || spec.title || ""}
      </figcaption>
    </figure>
  );
}
