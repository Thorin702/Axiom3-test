// Native analytical spectrum: intensity vs a spectral axis. kind "ms"/"nmr" draws vertical
// sticks at each [x,intensity]; "ir"/"uv" draws a continuous polyline through the points.
// invertX flips the axis high→low (NMR/IR convention); labelTop annotates the N tallest peaks
// with their x value. Single accent color, recessive CSS-var axes, pure inline SVG.
//
// Spec (```spectrum JSON): {"title"?, "kind":"ms"|"nmr"|"ir"|"uv", "xlabel"?, "ylabel"?,
//   "invertX"?:boolean, "peaks":[[x,intensity],...], "labelTop"?:number}
type Kind = "ms" | "nmr" | "ir" | "uv";
type Spec = {
  title?: string;
  kind?: Kind;
  xlabel?: string;
  ylabel?: string;
  invertX?: boolean;
  peaks: [number, number][];
  labelTop?: number;
};

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
    out.push(Math.abs(v) < step * 1e-9 ? 0 : Math.round(v / step) * step);
  }
  return out;
}
// tick label precision derived from the step (integers when step ≥ 1).
function tickFmt(v: number, step: number): string {
  const dec = step >= 1 ? 0 : Math.min(4, Math.ceil(-Math.log10(step)));
  return v.toFixed(dec);
}
// compact peak-value label: trim trailing zeros (194 → "194", 7.26 → "7.26").
function trim(v: number): string {
  return String(Math.round(v * 1000) / 1000);
}

const KIND_X: Record<Kind, string> = {
  ms: "m/z",
  nmr: "chemical shift (ppm)",
  ir: "wavenumber (cm⁻¹)",
  uv: "wavelength (nm)",
};

export function Spectrum({ spec: raw, caption }: { spec: string; caption?: string }) {
  let spec: Spec;
  try {
    spec = JSON.parse(raw.trim());
  } catch (e) {
    return <div className="ax-fig__err">谱图规范不是有效 JSON · {String((e as Error)?.message || e)}</div>;
  }
  if (!spec || typeof spec !== "object" || !Array.isArray(spec.peaks)) {
    return <div className="ax-fig__err">谱图需要 peaks 数组（每项 [x, intensity]）</div>;
  }

  const kind: Kind = spec.kind === "nmr" || spec.kind === "ir" || spec.kind === "uv" ? spec.kind : "ms";
  const isLine = kind === "ir" || kind === "uv";
  const invertX = spec.invertX === true;
  // guard against non-string axis labels arriving from untrusted JSON (.trim() would throw).
  const xlabel = typeof spec.xlabel === "string" && spec.xlabel.trim() ? spec.xlabel : KIND_X[kind];
  const ylabel = typeof spec.ylabel === "string" && spec.ylabel.trim() ? spec.ylabel : "intensity";
  const title = typeof spec.title === "string" && spec.title.trim() ? spec.title : "";

  // clean: each peak must be [finite x, finite intensity]; anything else is dropped so a stray
  // value can never be misplaced. Nothing is precomputed — we only read the raw pairs.
  const pts = spec.peaks
    .filter(
      (p): p is [number, number] =>
        Array.isArray(p) && p.length >= 2 && typeof p[0] === "number" && isFinite(p[0]) && typeof p[1] === "number" && isFinite(p[1]),
    )
    .map((p) => ({ x: p[0], y: p[1] }));
  if (pts.length === 0) {
    return <div className="ax-fig__err">谱图无有效数据点（每项需为 [数值 x, 数值强度]）</div>;
  }

  // x domain (small padding so end marks aren't flush to the frame); guard a single-x spectrum.
  const xs = pts.map((p) => p.x);
  let xMin = Math.min(...xs);
  let xMax = Math.max(...xs);
  if (xMax === xMin) {
    xMin -= 1;
    xMax += 1;
  }
  const xPad = (xMax - xMin) * 0.03;
  const dMin = xMin - xPad;
  const dMax = xMax + xPad;

  // y domain: baseline at 0 (or below, if data dips negative), headroom on top for peak labels.
  const ys = pts.map((p) => p.y);
  const yFloor = Math.min(0, ...ys);
  let yPeak = Math.max(...ys);
  if (yPeak <= yFloor) yPeak = yFloor + 1;
  const ySpan = yPeak - yFloor;
  const yTopLim = yPeak + ySpan * 0.1;

  // geometry
  const padL = 50;
  const padR = 20;
  const padT = title ? 30 : 16;
  const padB = 48;
  const plotW = 520;
  const plotH = 300;
  const w = padL + plotW + padR;
  const h = padT + plotH + padB;
  const x0 = padL;
  const y0 = padT;
  const plotBottom = y0 + plotH;

  const xTo = (x: number) => {
    let f = (x - dMin) / (dMax - dMin);
    if (invertX) f = 1 - f;
    return x0 + f * plotW;
  };
  const yTo = (v: number) => plotBottom - ((v - yFloor) / (yTopLim - yFloor)) * plotH;

  const xStep = niceStep(dMax - dMin, 6);
  const xticks = axisTicks(dMin, dMax, 6);
  const yStep = niceStep(yPeak - yFloor, 5);
  const yticks = axisTicks(yFloor, yPeak, 5);

  // top-N peaks to annotate (by intensity, data-defined). Greedy vertical stagger by x so close
  // labels don't collide; every label still sits over its own peak.
  const N = typeof spec.labelTop === "number" && spec.labelTop > 0 ? Math.floor(spec.labelTop) : 0;
  const topPeaks = [...pts].sort((a, b) => b.y - a.y).slice(0, N);
  const ordered = topPeaks.map((p) => ({ p, px: xTo(p.x), py: yTo(p.y) })).sort((a, b) => a.px - b.px);
  let prevX = -1e9;
  let level = 0;
  const labels = ordered.map((d) => {
    if (d.px - prevX < 30) level += 1;
    else level = 0;
    prevX = d.px;
    const ly = Math.max(y0 + 8, d.py - 7 - level * 11);
    return { ...d, ly };
  });

  // polyline path for continuous spectra (sorted by data-x so the curve is a proper trace).
  const linePath = isLine
    ? [...pts]
        .sort((a, b) => a.x - b.x)
        .map((p, i) => `${i === 0 ? "M" : "L"}${xTo(p.x).toFixed(2)} ${yTo(p.y).toFixed(2)}`)
        .join(" ")
    : "";

  return (
    <figure className="ax-fig">
      <div className="ax-fig__frame" style={{ overflowX: "auto", justifyContent: "flex-start" }}>
        <svg viewBox={`0 0 ${w} ${h}`} width={w} height={h} role="img" aria-label={caption || title || "spectrum"} style={{ maxWidth: "100%" }}>
          {title && <text x={padL} y={16} fontSize="12" fontWeight="600" fill="var(--ax-ink)">{title}</text>}

          {/* horizontal grid + y ticks */}
          {yticks.map((t, i) => (
            <g key={`yt${i}`}>
              <line x1={x0} y1={yTo(t)} x2={x0 + plotW} y2={yTo(t)} stroke="var(--ax-line-soft)" strokeWidth="0.5" />
              <text x={x0 - 6} y={yTo(t) + 3} fontSize="8.5" textAnchor="end" fill="var(--ax-ink-faint)">{tickFmt(t, yStep)}</text>
            </g>
          ))}

          {/* axis frame */}
          <line x1={x0} y1={y0} x2={x0} y2={plotBottom} stroke="var(--ax-line)" strokeWidth="1" />
          <line x1={x0} y1={plotBottom} x2={x0 + plotW} y2={plotBottom} stroke="var(--ax-line)" strokeWidth="1" />

          {/* x ticks (marks + labels; no vertical grid so sticks stay legible) */}
          {xticks.map((t, i) => {
            const px = xTo(t);
            if (px < x0 - 0.5 || px > x0 + plotW + 0.5) return null;
            return (
              <g key={`xt${i}`}>
                <line x1={px} y1={plotBottom} x2={px} y2={plotBottom + 4} stroke="var(--ax-line)" strokeWidth="0.75" />
                <text x={px} y={plotBottom + 14} fontSize="8.5" textAnchor="middle" fill="var(--ax-ink-faint)">{tickFmt(t, xStep)}</text>
              </g>
            );
          })}

          {/* axis titles */}
          <text x={x0 + plotW / 2} y={h - 6} fontSize="10" textAnchor="middle" fill="var(--ax-ink-soft)">{xlabel}</text>
          <text x={13} y={y0 + plotH / 2} fontSize="10" textAnchor="middle" fill="var(--ax-ink-soft)"
            transform={`rotate(-90 13 ${y0 + plotH / 2})`}>{ylabel}</text>

          {/* data — sticks (ms/nmr) or a continuous trace (ir/uv), single accent color */}
          {isLine ? (
            <path d={linePath} fill="none" stroke="var(--ax-accent)" strokeWidth="1.4" strokeLinejoin="round" strokeLinecap="round" />
          ) : (
            pts.map((p, i) => (
              <line key={`st${i}`} x1={xTo(p.x)} y1={yTo(0)} x2={xTo(p.x)} y2={yTo(p.y)} stroke="var(--ax-accent)" strokeWidth="1.4" strokeLinecap="butt">
                <title>{`${xlabel} ${trim(p.x)} · ${ylabel} ${trim(p.y)}`}</title>
              </line>
            ))
          )}

          {/* invisible hover targets for the continuous trace (native tooltip per point) */}
          {isLine &&
            pts.map((p, i) => (
              <circle key={`hp${i}`} cx={xTo(p.x)} cy={yTo(p.y)} r={4} fill="var(--ax-accent)" fillOpacity={0}>
                <title>{`${xlabel} ${trim(p.x)} · ${ylabel} ${trim(p.y)}`}</title>
              </circle>
            ))}

          {/* top-N peak annotations (value at the peak x; connector when raised to clear overlap) */}
          {labels.map((d, i) => (
            <g key={`lb${i}`}>
              {d.ly < d.py - 8 && <line x1={d.px} y1={d.py - 3} x2={d.px} y2={d.ly + 1.5} stroke="var(--ax-line)" strokeWidth="0.5" />}
              <circle cx={d.px} cy={d.py} r={1.8} fill="var(--ax-accent)" />
              <text x={d.px} y={d.ly} fontSize="8.5" textAnchor="middle" fill="var(--ax-ink)">{trim(d.p.x)}</text>
            </g>
          ))}
        </svg>
      </div>
      <figcaption className="ax-fig__cap"><span className="ax-fig__kind">谱图 · spectrum</span> {caption || title}</figcaption>
    </figure>
  );
}
