// Native forest plot: ranked point estimates with uncertainty intervals — the one clean gap
// vs Chart.js (which cannot draw a CI). Each row is a whisker lo→hi with a dot at the estimate,
// sharing one x-axis and a dashed reference guide; a right column echoes estimate [lo, hi].
// Neutral by construction: it draws only the interval + the data-defined reference, never a verdict.
//
// Spec (```forest JSON): {"title"?, "xlabel"?, "reference"?, "logx"?,
//   "rows":[{"label":string,"estimate":number,"lo":number,"hi":number}, ...]}
type Row = { label: string; estimate: number; lo: number; hi: number };
type Spec = {
  title?: string;
  xlabel?: string;
  reference?: number;
  logx?: boolean;
  rows: Row[];
};

// Single categorical identity token (CAT slot 0) for the mark — one series, no per-row coloring.
const DOT_LIGHT = "#2a78d6";
const DOT_DARK = "#3987e5";
function isDark(): boolean {
  const t = document.documentElement.dataset.theme;
  if (t === "dark") return true;
  if (t === "light") return false;
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? false;
}

function fmt(v: number): string {
  if (!Number.isFinite(v)) return "—";
  const a = Math.abs(v);
  if (a === 0) return "0";
  if (a >= 100) return v.toFixed(0);
  if (a >= 10) return v.toFixed(1);
  if (a >= 1) return v.toFixed(2);
  return v.toFixed(3);
}
function trimZeros(s: string): string {
  return s.includes(".") ? s.replace(/\.?0+$/, "") : s;
}
// CJK-aware glyph width at the label font size (~11px): CJK glyphs render near-square
// (~11px) while Latin/digits are ~6.4px, so char-count sizing badly underestimates 中文.
function cw(s: string): number {
  let w = 0;
  for (const c of s) w += /[⺀-鿿＀-￯　-〿]/.test(c) ? 11 : 6.4;
  return w;
}
// Truncate to a pixel budget (not a char count), appending "…" when the label overflows.
function truncateToWidth(s: string, budget: number): string {
  if (cw(s) <= budget) return s;
  const ellW = cw("…");
  let w = 0;
  let out = "";
  for (const c of s) {
    const cwc = cw(c);
    if (w + cwc + ellW > budget) break;
    out += c;
    w += cwc;
  }
  return out ? out + "…" : "…";
}

function niceNum(range: number, round: boolean): number {
  const exp = Math.floor(Math.log10(range));
  const f = range / Math.pow(10, exp);
  let nf: number;
  if (round) nf = f < 1.5 ? 1 : f < 3 ? 2 : f < 7 ? 5 : 10;
  else nf = f <= 1 ? 1 : f <= 2 ? 2 : f <= 5 ? 5 : 10;
  return nf * Math.pow(10, exp);
}
function linTicks(min: number, max: number, count: number): number[] {
  if (!(max > min)) return [min];
  const step = niceNum(niceNum(max - min, false) / (count - 1), true);
  const start = Math.floor(min / step) * step;
  const out: number[] = [];
  for (let v = start; v <= max + step * 0.5; v += step) out.push(Math.round(v / step) * step);
  return out;
}
function logTicks(min: number, max: number): number[] {
  const out: number[] = [];
  const kLo = Math.floor(Math.log10(min));
  const kHi = Math.ceil(Math.log10(max));
  for (let k = kLo; k <= kHi; k++) for (const m of [1, 2, 5]) out.push(m * Math.pow(10, k));
  return out;
}

export function Forest({ spec, caption }: { spec: string; caption?: string }) {
  let d: Spec;
  try {
    d = JSON.parse(spec.trim());
  } catch (e) {
    return <div className="ax-fig__err">森林图规范不是有效 JSON · {String((e as Error).message)}</div>;
  }
  if (!d || typeof d !== "object") return <div className="ax-fig__err">森林图规范为空或格式不对</div>;
  if (!Array.isArray(d.rows))
    return <div className="ax-fig__err">森林图需要 rows 数组（每项含 label / estimate / lo / hi）</div>;

  const logx = d.logx === true;
  // validate + normalize (lo/hi order-independent); drop malformed rows rather than throw.
  const rows = d.rows
    .filter(
      (r): r is Row =>
        !!r &&
        typeof r.label === "string" &&
        Number.isFinite(r.estimate) &&
        Number.isFinite(r.lo) &&
        Number.isFinite(r.hi) &&
        (!logx || (r.lo > 0 && r.hi > 0 && r.estimate > 0)),
    )
    .map((r) => ({ label: r.label, estimate: r.estimate, lo: Math.min(r.lo, r.hi), hi: Math.max(r.lo, r.hi) }));
  if (rows.length === 0)
    return (
      <div className="ax-fig__err">森林图无有效数据行{logx ? "（logx 需要 lo / hi / estimate 均为正数）" : ""}</div>
    );

  const hasRef = Number.isFinite(d.reference) && (!logx || (d.reference as number) > 0);
  const refVal = d.reference as number;
  const dotColor = isDark() ? DOT_DARK : DOT_LIGHT;

  // ── x-domain (in transformed space; log10 for ratios) ──
  const tx = (v: number) => (logx ? Math.log10(Math.max(v, 1e-9)) : v);
  const vals: number[] = [];
  for (const r of rows) vals.push(r.lo, r.hi, r.estimate);
  if (hasRef) vals.push(refVal);
  const origLo = Math.min(...vals);
  const origHi = Math.max(...vals);
  let tMin = tx(origLo);
  let tMax = tx(origHi);
  const span = tMax - tMin || Math.abs(tMin) * 0.1 || 1;
  const pad = span * 0.06;
  tMin -= pad;
  tMax += pad;

  // ── layout ──
  const n = rows.length;
  const rowH = 28;
  const dotR = 4.5;
  const capH = 4;
  const plotW = 340;
  const labelW = Math.max(6 * 6.4, ...rows.map((r) => cw(r.label)));
  const padL = Math.round(Math.min(280, Math.max(96, labelW + 14)));
  // labels are drawn at x=8; keep them inside the label gutter (never bleeding into the plot).
  const labelBudget = padL - 16;
  const echoStrs = rows.map((r) => `${fmt(r.estimate)}  [${fmt(r.lo)}, ${fmt(r.hi)}]`);
  const echoChars = Math.max(10, ...echoStrs.map((s) => s.length));
  const padR = Math.round(Math.min(220, Math.max(118, echoChars * 6.4 + 18)));
  const topPad = d.title ? 26 : 10;
  const colHeaderH = 15;
  const plotTop = topPad + colHeaderH;
  const plotBottom = plotTop + n * rowH;
  const axisY = plotBottom + 4;
  const titleW = d.title ? cw(d.title) + 16 : 0;
  const w = Math.max(padL + plotW + padR, Math.ceil(titleW));
  const h = plotBottom + 40;
  const x0 = padL;
  const x1 = padL + plotW;

  const xs = (v: number) => {
    if (tMax === tMin) return (x0 + x1) / 2;
    return x0 + ((tx(v) - tMin) / (tMax - tMin)) * (x1 - x0);
  };
  const cy = (i: number) => plotTop + i * rowH + rowH / 2;

  // ticks within the (padded, transformed) domain
  const rawTicks = logx ? logTicks(origLo, origHi) : linTicks(origLo, origHi, 6);
  const ticks = rawTicks.filter((v) => tx(v) >= tMin - 1e-9 && tx(v) <= tMax + 1e-9);

  const label = caption || d.title || "forest plot";

  return (
    <figure className="ax-fig">
      <div className="ax-fig__frame" style={{ overflowX: "auto", justifyContent: "flex-start" }}>
        <svg viewBox={`0 0 ${w} ${h}`} width={w} height={h} role="img" aria-label={label} style={{ maxWidth: "100%" }}>
          {d.title && (
            <text x={8} y={16} fontSize="12" fontWeight="600" fill="var(--ax-ink)">
              {d.title}
            </text>
          )}
          {/* column header over the numeric echo column */}
          <text x={x1 + 12} y={plotTop - 4} fontSize="9" fill="var(--ax-ink-faint)">
            estimate [lo, hi]
          </text>

          {/* gridlines at ticks */}
          {ticks.map((t, i) => (
            <line
              key={`g${i}`}
              x1={xs(t)}
              y1={plotTop}
              x2={xs(t)}
              y2={plotBottom}
              stroke="var(--ax-line-soft)"
              strokeWidth="0.5"
            />
          ))}

          {/* dashed reference guide */}
          {hasRef && (
            <g>
              <line
                x1={xs(refVal)}
                y1={plotTop}
                x2={xs(refVal)}
                y2={plotBottom}
                stroke="var(--ax-ink-faint)"
                strokeWidth="1"
                strokeDasharray="4 3"
              />
              <text x={xs(refVal)} y={plotTop - 4} fontSize="8.5" textAnchor="middle" fill="var(--ax-ink-faint)">
                {`ref ${trimZeros(fmt(refVal))}`}
              </text>
            </g>
          )}

          {/* rows: whisker lo→hi with end caps, dot at estimate */}
          {rows.map((r, i) => {
            const y = cy(i);
            const xl = xs(r.lo);
            const xh = xs(r.hi);
            const xe = xs(r.estimate);
            const shown = truncateToWidth(r.label, labelBudget);
            return (
              <g key={`row${i}`}>
                <text x={8} y={y + 3.5} fontSize="11" fill="var(--ax-ink)">
                  {shown}
                  {shown !== r.label && <title>{r.label}</title>}
                </text>
                <line x1={xl} y1={y} x2={xh} y2={y} stroke={dotColor} strokeWidth="1.4" />
                <line x1={xl} y1={y - capH} x2={xl} y2={y + capH} stroke={dotColor} strokeWidth="1.4" />
                <line x1={xh} y1={y - capH} x2={xh} y2={y + capH} stroke={dotColor} strokeWidth="1.4" />
                <circle cx={xe} cy={y} r={dotR} fill={dotColor} stroke="var(--ax-surface)" strokeWidth="0.6">
                  <title>{`${r.label} — ${fmt(r.estimate)} [${fmt(r.lo)}, ${fmt(r.hi)}]`}</title>
                </circle>
                <text
                  x={x1 + 12}
                  y={y + 3.5}
                  fontSize="10.5"
                  fill="var(--ax-ink-soft)"
                  style={{ fontVariantNumeric: "tabular-nums" }}
                >
                  {echoStrs[i]}
                </text>
              </g>
            );
          })}

          {/* x-axis baseline + ticks */}
          <line x1={x0} y1={axisY} x2={x1} y2={axisY} stroke="var(--ax-line)" strokeWidth="1" />
          {ticks.map((t, i) => (
            <g key={`t${i}`}>
              <line x1={xs(t)} y1={axisY} x2={xs(t)} y2={axisY + 4} stroke="var(--ax-line)" strokeWidth="1" />
              <text x={xs(t)} y={axisY + 15} fontSize="9" textAnchor="middle" fill="var(--ax-ink-faint)">
                {trimZeros(fmt(t))}
              </text>
            </g>
          ))}
          {d.xlabel && (
            <text x={(x0 + x1) / 2} y={h - 6} fontSize="10" textAnchor="middle" fill="var(--ax-ink-soft)">
              {logx ? `${d.xlabel} (log scale)` : d.xlabel}
            </text>
          )}
        </svg>
      </div>
      <figcaption className="ax-fig__cap">
        <span className="ax-fig__kind">森林图 · forest</span> {caption || d.title || ""}
      </figcaption>
    </figure>
  );
}
