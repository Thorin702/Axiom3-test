// Native "series with uncertainty band" figure: a ```band JSON block → central estimate(s)
// plus interval, drawn as pure SVG (line polyline + filled ribbon + error caps). Universal
// uncertainty renderer for measured-vs-model, forecasts, GP posteriors, learning-curve ±std,
// I–V/decay/dispersion — optionally on log x/y axes. Colors reuse the data-viz categorical
// palette (fixed slot order, light/dark by theme) for cross-figure consistency.
//
// Spec (```band JSON): {"title"?, "xlabel"?, "ylabel"?, "xscale"?:"linear"|"log",
//   "yscale"?:"linear"|"log", "series":[{"name","dash"?,"points":[ [x,y] | [x,y,lo,hi] |
//   [x,y,err] ]}]}. Each point is self-describing: [x,y]=plain, [x,y,lo,hi]=ribbon,
//   [x,y,err]=symmetric error caps. dash:true for model/reference lines.

// data-viz reference categorical palette (validated), light / dark by slot order.
const CAT_LIGHT = ["#2a78d6", "#1baf7a", "#eda100", "#008300", "#4a3aa7", "#e34948", "#e87ba4", "#eb6834"];
const CAT_DARK = ["#3987e5", "#199e70", "#c98500", "#008300", "#9085e9", "#e66767", "#d55181", "#d95926"];
function isDark(): boolean {
  const t = document.documentElement.dataset.theme;
  if (t === "dark") return true;
  if (t === "light") return false;
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? false;
}

type SeriesSpec = { name?: string; dash?: boolean; points: number[][] };
type Spec = {
  title?: string;
  xlabel?: string;
  ylabel?: string;
  xscale?: "linear" | "log";
  yscale?: "linear" | "log";
  series: SeriesSpec[];
};

type Pt = { x: number; y: number; lo: number; hi: number; hasBand: boolean; hasErr: boolean };

// Parse one self-describing point from untrusted JSON (guard every element, never throw).
function parsePt(p: unknown): Pt | null {
  if (!Array.isArray(p) || p.length < 2) return null;
  const x = p[0];
  const y = p[1];
  if (typeof x !== "number" || typeof y !== "number" || !Number.isFinite(x) || !Number.isFinite(y)) return null;
  const lo = p[2];
  const hi = p[3];
  if (p.length >= 4 && typeof lo === "number" && typeof hi === "number" && Number.isFinite(lo) && Number.isFinite(hi)) {
    return { x, y, lo: Math.min(lo, hi), hi: Math.max(lo, hi), hasBand: true, hasErr: false };
  }
  if (p.length === 3 && typeof lo === "number" && Number.isFinite(lo)) {
    const e = Math.abs(lo);
    return { x, y, lo: y - e, hi: y + e, hasBand: false, hasErr: true };
  }
  return { x, y, lo: y, hi: y, hasBand: false, hasErr: false };
}

// nice linear domain + ticks (~count divisions on a 1/2/5×10ⁿ grid).
function linDomain(lo: number, hi: number, count = 5): { min: number; max: number; ticks: number[] } {
  if (!(hi > lo)) {
    const c = Number.isFinite(lo) ? lo : 0;
    lo = c - 1;
    hi = c + 1;
  }
  const span = hi - lo;
  const step0 = span / count;
  const mag = Math.pow(10, Math.floor(Math.log10(step0)));
  const norm = step0 / mag;
  const step = (norm < 1.5 ? 1 : norm < 3 ? 2 : norm < 7 ? 5 : 10) * mag;
  const min = Math.floor(lo / step) * step;
  const max = Math.ceil(hi / step) * step;
  const ticks: number[] = [];
  for (let v = min; v <= max + step * 1e-9; v += step) ticks.push(Number(v.toFixed(10)));
  return { min, max, ticks };
}

// log domain snapped to enclosing decades; ticks at every power of ten in range.
function logDomain(lo: number, hi: number): { min: number; max: number; ticks: number[] } {
  if (!(lo > 0)) lo = 1e-9;
  if (!(hi > lo)) hi = lo * 10;
  const eLo = Math.floor(Math.log10(lo));
  const eHi = Math.ceil(Math.log10(hi));
  const ticks: number[] = [];
  for (let e = eLo; e <= eHi; e++) ticks.push(Math.pow(10, e));
  return { min: Math.pow(10, eLo), max: Math.pow(10, eHi), ticks };
}

function fmt(v: number): string {
  if (!Number.isFinite(v)) return "";
  if (v === 0) return "0";
  const a = Math.abs(v);
  if (a >= 1e5 || a < 1e-3) return v.toExponential(0);
  return String(Number(v.toPrecision(6)));
}

export function Band({ spec: raw, caption }: { spec: string; caption?: string }) {
  let spec: Spec;
  try {
    spec = JSON.parse(raw.trim());
  } catch (e) {
    return <div className="ax-fig__err">区间图规范不是有效 JSON · {(e as Error).message}</div>;
  }
  if (!spec || !Array.isArray(spec.series) || spec.series.length === 0) {
    return <div className="ax-fig__err">区间图需要 series[]（每条含 points 数组）</div>;
  }

  const xlog = spec.xscale === "log";
  const ylog = spec.yscale === "log";
  const cat = isDark() ? CAT_DARK : CAT_LIGHT;

  const seriesData = spec.series.map((s, i) => {
    const pts = (Array.isArray(s.points) ? s.points : [])
      .map(parsePt)
      .filter((p): p is Pt => p !== null && (!xlog || p.x > 0) && (!ylog || p.y > 0))
      .sort((a, b) => a.x - b.x);
    return {
      name: (typeof s.name === "string" && s.name ? s.name : `series ${i + 1}`).slice(0, 48),
      dash: !!s.dash,
      color: cat[i % cat.length], // fixed slot order; modulo is a safe fallback for the rare >8-series case (legend labels disambiguate)
      pts,
      bandPts: pts.filter((p) => p.hasBand),
      errPts: pts.filter((p) => p.hasErr),
    };
  });

  const allPts = seriesData.flatMap((sd) => sd.pts);
  if (allPts.length === 0) {
    return <div className="ax-fig__err">区间图没有可绘制的数据点（检查 log 轴下的非正值）</div>;
  }

  // domains — y accounts for band/error extents so ribbons and caps never overflow the plot.
  const xs = allPts.map((p) => p.x);
  const ysRaw: number[] = [];
  allPts.forEach((p) => {
    ysRaw.push(p.y, p.lo, p.hi);
  });
  const ys = ylog ? ysRaw.filter((v) => v > 0) : ysRaw;
  const xd = xlog ? logDomain(Math.min(...xs), Math.max(...xs)) : linDomain(Math.min(...xs), Math.max(...xs));
  const yd = ylog ? logDomain(Math.min(...ys), Math.max(...ys)) : linDomain(Math.min(...ys), Math.max(...ys));

  // layout — legend wraps to as many rows as needed so long / many names never clip the viewBox.
  const w = 680;
  const h = 400;
  const padL = 60;
  const padR = 20;
  const showLegend = seriesData.length >= 2;
  const hasTitle = !!spec.title;
  const titleH = hasTitle ? 22 : 0;

  type Chip = { name: string; color: string; dash: boolean; x: number };
  const legendRows: Chip[][] = [];
  if (showLegend) {
    const maxX = w - padR;
    let row: Chip[] = [];
    let cursor = padL;
    for (const sd of seriesData) {
      const tw = sd.name.length * 6.2 + 32;
      if (cursor + tw > maxX && row.length > 0) {
        legendRows.push(row);
        row = [];
        cursor = padL;
      }
      row.push({ name: sd.name, color: sd.color, dash: sd.dash, x: cursor });
      cursor += tw;
    }
    if (row.length > 0) legendRows.push(row);
  }
  const legendH = legendRows.length * 16;

  const padT = Math.max(14, titleH + legendH + 8);
  const padB = spec.xlabel ? 50 : 34;
  const plotL = padL;
  const plotR = w - padR;
  const plotT = padT;
  const plotB = h - padB;

  const sx = (v: number): number => {
    if (xlog) {
      const lo = Math.log10(xd.min);
      const hi = Math.log10(xd.max);
      const t = (Math.log10(v > 0 ? v : xd.min) - lo) / (hi - lo || 1);
      return plotL + t * (plotR - plotL);
    }
    return plotL + ((v - xd.min) / (xd.max - xd.min || 1)) * (plotR - plotL);
  };
  const sy = (v: number): number => {
    if (ylog) {
      const lo = Math.log10(yd.min);
      const hi = Math.log10(yd.max);
      const t = (Math.log10(v > 0 ? v : yd.min) - lo) / (hi - lo || 1);
      return plotB - t * (plotB - plotT);
    }
    return plotB - ((v - yd.min) / (yd.max - yd.min || 1)) * (plotB - plotT);
  };

  const line = (pts: Pt[]): string => pts.map((p) => `${sx(p.x).toFixed(1)},${sy(p.y).toFixed(1)}`).join(" ");
  const ribbon = (pts: Pt[]): string => {
    if (pts.length < 2) return "";
    const up = pts.map((p) => `${sx(p.x).toFixed(1)},${sy(p.hi).toFixed(1)}`);
    const dn = [...pts].reverse().map((p) => `${sx(p.x).toFixed(1)},${sy(p.lo).toFixed(1)}`);
    return `M ${up.join(" L ")} L ${dn.join(" L ")} Z`;
  };

  return (
    <figure className="ax-fig">
      <div className="ax-fig__frame" style={{ overflowX: "auto", justifyContent: "flex-start" }}>
        <svg
          viewBox={`0 0 ${w} ${h}`}
          width={w}
          height={h}
          role="img"
          aria-label={caption || spec.title || "series with uncertainty band"}
          style={{ maxWidth: "100%" }}
        >
          {spec.title && (
            <text x={padL} y={15} fontSize="12" fontWeight="600" fill="var(--ax-ink)">
              {spec.title}
            </text>
          )}

          {/* legend (≥2 series): a line swatch + name, wrapped; never color-alone */}
          {legendRows.map((row, r) => {
            const ly = titleH + r * 16 + 8;
            return row.map((item, c) => (
              <g key={`lg${r}-${c}`}>
                <line
                  x1={item.x}
                  y1={ly}
                  x2={item.x + 16}
                  y2={ly}
                  stroke={item.color}
                  strokeWidth="2"
                  strokeDasharray={item.dash ? "4 3" : undefined}
                />
                <text x={item.x + 22} y={ly + 3.5} fontSize="10.5" fill="var(--ax-ink-soft)">
                  {item.name}
                </text>
              </g>
            ));
          })}

          {/* y grid + tick labels */}
          {yd.ticks.map((t, i) => {
            const y = sy(t);
            if (y < plotT - 0.5 || y > plotB + 0.5) return null;
            return (
              <g key={`yt${i}`}>
                <line x1={plotL} y1={y} x2={plotR} y2={y} stroke="var(--ax-line-soft)" strokeWidth="0.5" />
                <text x={plotL - 8} y={y + 3} fontSize="9.5" textAnchor="end" fill="var(--ax-ink-faint)">
                  {fmt(t)}
                </text>
              </g>
            );
          })}

          {/* x grid + tick labels */}
          {xd.ticks.map((t, i) => {
            const x = sx(t);
            if (x < plotL - 0.5 || x > plotR + 0.5) return null;
            return (
              <g key={`xt${i}`}>
                <line x1={x} y1={plotT} x2={x} y2={plotB} stroke="var(--ax-line-soft)" strokeWidth="0.5" />
                <text x={x} y={plotB + 14} fontSize="9.5" textAnchor="middle" fill="var(--ax-ink-faint)">
                  {fmt(t)}
                </text>
              </g>
            );
          })}

          {/* axis lines */}
          <line x1={plotL} y1={plotT} x2={plotL} y2={plotB} stroke="var(--ax-line)" strokeWidth="1" />
          <line x1={plotL} y1={plotB} x2={plotR} y2={plotB} stroke="var(--ax-line)" strokeWidth="1" />

          {/* uncertainty ribbons (behind every line) */}
          {seriesData.map((sd, i) =>
            sd.bandPts.length >= 2 ? (
              <path key={`bd${i}`} d={ribbon(sd.bandPts)} fill={sd.color} fillOpacity="0.15" stroke="none" />
            ) : null
          )}

          {/* per-series: error caps → central line → markers */}
          {seriesData.map((sd, i) => (
            <g key={`sr${i}`}>
              {sd.errPts.map((p, j) => {
                const x = sx(p.x);
                return (
                  <g key={`ec${i}-${j}`} stroke={sd.color} strokeWidth="1">
                    <line x1={x} y1={sy(p.lo)} x2={x} y2={sy(p.hi)} />
                    <line x1={x - 3} y1={sy(p.hi)} x2={x + 3} y2={sy(p.hi)} />
                    <line x1={x - 3} y1={sy(p.lo)} x2={x + 3} y2={sy(p.lo)} />
                  </g>
                );
              })}
              {sd.pts.length >= 2 && (
                <polyline
                  points={line(sd.pts)}
                  fill="none"
                  stroke={sd.color}
                  strokeWidth="1.75"
                  strokeDasharray={sd.dash ? "5 4" : undefined}
                  strokeLinejoin="round"
                  strokeLinecap="round"
                />
              )}
              {sd.pts.map((p, j) => (
                <circle
                  key={`mk${i}-${j}`}
                  cx={sx(p.x)}
                  cy={sy(p.y)}
                  r={sd.dash ? 2 : 2.6}
                  fill={sd.color}
                  stroke="var(--ax-surface)"
                  strokeWidth="0.75"
                >
                  <title>
                    {`${sd.name} · x=${fmt(p.x)}, y=${fmt(p.y)}${
                      p.hasBand ? ` [${fmt(p.lo)}, ${fmt(p.hi)}]` : p.hasErr ? ` ±${fmt((p.hi - p.lo) / 2)}` : ""
                    }`}
                  </title>
                </circle>
              ))}
            </g>
          ))}

          {/* axis titles */}
          {spec.xlabel && (
            <text x={(plotL + plotR) / 2} y={h - 8} fontSize="10.5" textAnchor="middle" fill="var(--ax-ink-soft)">
              {spec.xlabel}
            </text>
          )}
          {spec.ylabel && (
            <text
              x={16}
              y={(plotT + plotB) / 2}
              fontSize="10.5"
              textAnchor="middle"
              fill="var(--ax-ink-soft)"
              transform={`rotate(-90 16 ${(plotT + plotB) / 2})`}
            >
              {spec.ylabel}
            </text>
          )}
        </svg>
      </div>
      <figcaption className="ax-fig__cap">
        <span className="ax-fig__kind">区间 · band</span> {caption || spec.title || ""}
      </figcaption>
    </figure>
  );
}
