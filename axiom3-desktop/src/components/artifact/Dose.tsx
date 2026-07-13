// Native dose-response figure (pharmacology potency): response vs log10-dose with a fitted
// 4-parameter-logistic sigmoid per compound, raw points as dots, and a dashed IC50/EC50 marker
// at each curve's midpoint. The 4PL is evaluated client-side from the supplied {top,bottom,hill,
// ic50}; if a curve gives no fit params the renderer just draws a smooth spline through its
// points — it never invents a fit. Pure inline SVG, CSS-var theming, categorical colors.
//
// Spec (```dose JSON): {"title"?, "xlabel"?, "ylabel"?, "curves":[
//   {"name":<str>, "ic50"?:<log10>, "hill"?:1, "top"?, "bottom"?,
//    "points":[[<log10dose>,<response>], ...]} ]}
import { useId } from "react";

type RawCurve = {
  name?: string;
  ic50?: number;
  hill?: number;
  top?: number;
  bottom?: number;
  points?: [number, number][];
};
type Spec = {
  title?: string;
  xlabel?: string;
  ylabel?: string;
  curves: RawCurve[];
};

// data-viz reference categorical palette (validated, CVD-safe), light / dark by slot order.
const CAT_LIGHT = ["#2a78d6", "#1baf7a", "#eda100", "#008300", "#4a3aa7", "#e34948", "#e87ba4", "#eb6834"];
const CAT_DARK = ["#3987e5", "#199e70", "#c98500", "#008300", "#9085e9", "#e66767", "#d55181", "#d95926"];
function isDark(): boolean {
  const t = document.documentElement.dataset.theme;
  if (t === "dark") return true;
  if (t === "light") return false;
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? false;
}

// nice round tick step for a span, aiming for ~target divisions.
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
const fmtNum = (v: number) => Number(v.toFixed(2)).toString();
// IC50 given as log10(M) → molar concentration in exponential form.
function fmtConc(logv: number): string {
  const c = Math.pow(10, logv);
  if (!isFinite(c) || c <= 0) return String(c);
  return c.toExponential(1);
}

// 4-parameter logistic in log-dose space (GraphPad form): at x = ic50 the response is the
// (top+bottom)/2 midpoint. Overflow of 10^(...) collapses to bottom/top with no NaN.
function fourPL(x: number, top: number, bottom: number, hill: number, ic50: number): number {
  const denom = 1 + Math.pow(10, (ic50 - x) * hill);
  return bottom + (top - bottom) / denom;
}

// Catmull-Rom → cubic-Bézier smoothing for the no-fit fallback (screen-space points).
function smoothPath(p: [number, number][]): string {
  if (p.length === 0) return "";
  if (p.length === 1) return `M ${p[0][0]} ${p[0][1]}`;
  let d = `M ${p[0][0]} ${p[0][1]}`;
  for (let i = 0; i < p.length - 1; i++) {
    const p0 = p[i - 1] ?? p[i];
    const p1 = p[i];
    const p2 = p[i + 1];
    const p3 = p[i + 2] ?? p2;
    const c1x = p1[0] + (p2[0] - p0[0]) / 6;
    const c1y = p1[1] + (p2[1] - p0[1]) / 6;
    const c2x = p2[0] - (p3[0] - p1[0]) / 6;
    const c2y = p2[1] - (p3[1] - p1[1]) / 6;
    d += ` C ${c1x} ${c1y} ${c2x} ${c2y} ${p2[0]} ${p2[1]}`;
  }
  return d;
}

type Curve = {
  name: string;
  color: string;
  fit: { top: number; bottom: number; hill: number; ic50: number } | null;
  points: [number, number][]; // sorted by x, cleaned
};

export function Dose({ spec: raw, caption }: { spec: string; caption?: string }) {
  const uid = useId();
  const clipId = "dose-clip-" + uid.replace(/:/g, "");

  let spec: Spec;
  try {
    spec = JSON.parse(raw.trim());
  } catch (e) {
    return <div className="ax-fig__err">剂量反应规范不是有效 JSON · {String((e as Error)?.message || e)}</div>;
  }
  if (!spec || typeof spec !== "object" || !Array.isArray(spec.curves) || spec.curves.length === 0) {
    return <div className="ax-fig__err">剂量反应需要 curves 数组（每条含 points 或 ic50 拟合参数）</div>;
  }

  const cat = isDark() ? CAT_DARK : CAT_LIGHT;

  // clean each curve; keep it if it has plottable points OR a finite ic50 fit.
  const curves: Curve[] = [];
  spec.curves.forEach((c, i) => {
    if (!c || typeof c !== "object") return;
    const pts: [number, number][] = Array.isArray(c.points)
      ? c.points
          .filter(
            (p): p is [number, number] =>
              Array.isArray(p) && typeof p[0] === "number" && isFinite(p[0]) && typeof p[1] === "number" && isFinite(p[1]),
          )
          .map((p) => [p[0], p[1]] as [number, number])
          .sort((a, b) => a[0] - b[0])
      : [];
    const hasFit = typeof c.ic50 === "number" && isFinite(c.ic50);
    if (!hasFit && pts.length === 0) return;

    // fit endpoints default to the curve's own response range, then a neutral 0..100 fallback.
    const respVals = pts.map((p) => p[1]);
    const dataTop = respVals.length ? Math.max(...respVals) : 100;
    const dataBottom = respVals.length ? Math.min(...respVals) : 0;
    const fit = hasFit
      ? {
          ic50: c.ic50 as number,
          hill: typeof c.hill === "number" && isFinite(c.hill) ? c.hill : 1,
          top: typeof c.top === "number" && isFinite(c.top) ? c.top : dataTop,
          bottom: typeof c.bottom === "number" && isFinite(c.bottom) ? c.bottom : dataBottom,
        }
      : null;

    curves.push({
      name: String(c.name ?? `curve ${i + 1}`),
      color: i < cat.length ? cat[i] : "var(--ax-ink-soft)",
      fit,
      points: pts,
    });
  });

  if (curves.length === 0) {
    return <div className="ax-fig__err">剂量反应无有效曲线（需要数值 points 或 ic50）</div>;
  }

  // domains: x over all point log-doses + ic50s (± a span so a point-less fit still spreads);
  // y over all responses + fit endpoints. Everything derived from the emitted data.
  const xs: number[] = [];
  const ys: number[] = [];
  for (const c of curves) {
    for (const p of c.points) {
      xs.push(p[0]);
      ys.push(p[1]);
    }
    if (c.fit) {
      xs.push(c.fit.ic50 - 2, c.fit.ic50 + 2);
      ys.push(c.fit.top, c.fit.bottom);
    }
  }
  if (xs.length === 0 || ys.length === 0) {
    return <div className="ax-fig__err">剂量反应无有效数值范围</div>;
  }
  let xMin = Math.min(...xs);
  let xMax = Math.max(...xs);
  let yMin = Math.min(...ys);
  let yMax = Math.max(...ys);
  if (xMax === xMin) {
    xMin -= 1;
    xMax += 1;
  }
  if (yMax === yMin) {
    yMin -= 1;
    yMax += 1;
  }
  const xPad = (xMax - xMin) * 0.04;
  const yPad = (yMax - yMin) * 0.06;
  xMin -= xPad;
  xMax += xPad;
  yMin -= yPad;
  yMax += yPad;

  // geometry
  const padL = 54;
  const padR = 128;
  const padT = spec.title ? 30 : 16;
  const padB = 46;
  const plotW = 430;
  const plotH = 300;
  const w = padL + plotW + padR;
  // legend grows one ~30px row per curve (name + IC50 sub-line); size the canvas so many
  // curves are never clipped below the viewBox (cf. Roc.tsx).
  const h = Math.max(padT + plotH + padB, padT + 6 + curves.length * 30 + 12);
  const x0 = padL;
  const y0 = padT;
  const plotBottom = y0 + plotH;
  const xTo = (x: number) => x0 + ((x - xMin) / (xMax - xMin)) * plotW;
  const yTo = (y: number) => plotBottom - ((y - yMin) / (yMax - yMin)) * plotH;

  const xticks = axisTicks(xMin, xMax, 6);
  const yticks = axisTicks(yMin, yMax, 5);

  const xlabel = spec.xlabel || "log10[dose]";
  const ylabel = spec.ylabel || "response";

  // sample each fitted sigmoid across the x-domain (client-side 4PL) → polyline path.
  const SAMPLES = 120;
  const fitPath = (c: Curve): string => {
    if (!c.fit) return "";
    let d = "";
    for (let i = 0; i <= SAMPLES; i++) {
      const x = xMin + ((xMax - xMin) * i) / SAMPLES;
      const y = fourPL(x, c.fit.top, c.fit.bottom, c.fit.hill, c.fit.ic50);
      d += `${i === 0 ? "M" : "L"} ${xTo(x).toFixed(2)} ${yTo(y).toFixed(2)} `;
    }
    return d.trim();
  };

  const lx = w - padR + 10; // legend left edge

  return (
    <figure className="ax-fig">
      <div className="ax-fig__frame" style={{ overflowX: "auto", justifyContent: "flex-start" }}>
        <svg viewBox={`0 0 ${w} ${h}`} width={w} height={h} role="img" aria-label={caption || spec.title || "dose-response curve"} style={{ maxWidth: "100%" }}>
          {spec.title && <text x={padL} y={16} fontSize="12" fontWeight="600" fill="var(--ax-ink)">{spec.title}</text>}

          <defs>
            <clipPath id={clipId}>
              <rect x={x0} y={y0} width={plotW} height={plotH} />
            </clipPath>
          </defs>

          {/* grid */}
          {yticks.map((t, i) => (
            <line key={`gy${i}`} x1={x0} y1={yTo(t)} x2={x0 + plotW} y2={yTo(t)} stroke="var(--ax-line-soft)" strokeWidth="0.5" />
          ))}
          {xticks.map((t, i) => (
            <line key={`gx${i}`} x1={xTo(t)} y1={y0} x2={xTo(t)} y2={plotBottom} stroke="var(--ax-line-soft)" strokeWidth="0.5" />
          ))}

          {/* axis frame */}
          <line x1={x0} y1={y0} x2={x0} y2={plotBottom} stroke="var(--ax-line)" strokeWidth="1" />
          <line x1={x0} y1={plotBottom} x2={x0 + plotW} y2={plotBottom} stroke="var(--ax-line)" strokeWidth="1" />

          {/* x ticks + label */}
          {xticks.map((t, i) => (
            <text key={`xt${i}`} x={xTo(t)} y={plotBottom + 13} fontSize="9" textAnchor="middle" fill="var(--ax-ink-faint)">{fmtNum(t)}</text>
          ))}
          <text x={x0 + plotW / 2} y={h - 6} fontSize="10" textAnchor="middle" fill="var(--ax-ink-soft)">{xlabel}</text>

          {/* y ticks + label */}
          {yticks.map((t, i) => (
            <text key={`yt${i}`} x={x0 - 6} y={yTo(t) + 3} fontSize="9" textAnchor="end" fill="var(--ax-ink-faint)">{fmtNum(t)}</text>
          ))}
          <text x={14} y={y0 + plotH / 2} fontSize="10" textAnchor="middle" fill="var(--ax-ink-soft)"
            transform={`rotate(-90 14 ${y0 + plotH / 2})`}>{ylabel}</text>

          {/* curves (fit sigmoid or smooth spline), IC50 markers, then raw points — clipped to plot */}
          <g clipPath={`url(#${clipId})`}>
            {curves.map((c, ci) => {
              if (c.fit) {
                return (
                  <path key={`fit${ci}`} d={fitPath(c)} fill="none" stroke={c.color} strokeWidth="1.75">
                    <title>{`${c.name} — 4PL 拟合 · IC50/EC50 ${fmtConc(c.fit.ic50)} M · Hill ${fmtNum(c.fit.hill)}`}</title>
                  </path>
                );
              }
              if (c.points.length >= 2) {
                const screen = c.points.map((p) => [xTo(p[0]), yTo(p[1])] as [number, number]);
                return (
                  <path key={`sm${ci}`} d={smoothPath(screen)} fill="none" stroke={c.color} strokeWidth="1.75" strokeDasharray="5 3" opacity="0.9">
                    <title>{`${c.name} — 插值（无拟合参数）`}</title>
                  </path>
                );
              }
              return null;
            })}

            {/* IC50 / EC50 markers: dashed vertical at ic50 up to the fitted midpoint */}
            {curves.map((c, ci) => {
              if (!c.fit) return null;
              const mid = (c.fit.top + c.fit.bottom) / 2;
              const mx = xTo(c.fit.ic50);
              const my = yTo(mid);
              return (
                <g key={`ic${ci}`}>
                  <line x1={mx} y1={my} x2={mx} y2={plotBottom} stroke={c.color} strokeWidth="0.9" strokeDasharray="4 3" opacity="0.7" />
                  <circle cx={mx} cy={my} r={3.2} fill="var(--ax-surface)" stroke={c.color} strokeWidth="1.5">
                    <title>{`${c.name} — IC50/EC50 ${fmtConc(c.fit.ic50)} M (log ${fmtNum(c.fit.ic50)}) · Hill ${fmtNum(c.fit.hill)}`}</title>
                  </circle>
                </g>
              );
            })}

            {/* raw points on top */}
            {curves.map((c, ci) =>
              c.points.map((p, pi) => (
                <circle key={`pt${ci}-${pi}`} cx={xTo(p[0])} cy={yTo(p[1])} r={2.6} fill={c.color} stroke="var(--ax-surface)" strokeWidth="0.5">
                  <title>{`${c.name} — dose ${fmtConc(p[0])} M (log ${fmtNum(p[0])}) · ${ylabel} ${fmtNum(p[1])}`}</title>
                </circle>
              )),
            )}
          </g>

          {/* legend: color swatch + name (never color-alone → paired with text + IC50) */}
          {curves.map((c, i) => {
            const ly = padT + 6 + i * 30;
            return (
              <g key={`lg${i}`}>
                <line x1={lx} y1={ly + 4} x2={lx + 16} y2={ly + 4} stroke={c.color} strokeWidth="2" />
                <circle cx={lx + 8} cy={ly + 4} r={2.6} fill={c.color} stroke="var(--ax-surface)" strokeWidth="0.5" />
                <text x={lx + 22} y={ly + 7} fontSize="9.5" fill="var(--ax-ink-soft)">{c.name}</text>
                {c.fit ? (
                  <text x={lx} y={ly + 19} fontSize="8" fill="var(--ax-ink-faint)">{`IC50 ${fmtConc(c.fit.ic50)} M`}</text>
                ) : (
                  <text x={lx} y={ly + 19} fontSize="8" fill="var(--ax-ink-faint)">points only</text>
                )}
              </g>
            );
          })}
        </svg>
      </div>
      <figcaption className="ax-fig__cap"><span className="ax-fig__kind">剂量反应 · dose</span> {caption || spec.title || ""}</figcaption>
    </figure>
  );
}
