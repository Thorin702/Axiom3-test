// Native distribution figure (```dist JSON): compare a numeric distribution across groups —
// box (quartiles + 1.5·IQR whiskers + outliers), optional seeded-jitter raw points, and an
// optional Gaussian-KDE violin (gated behind show:["violin"]). Quartiles/whiskers are computed
// client-side from the raw `values` (auditable); groups may also arrive pre-summarised.
// Spec: {"title"?, "ylabel"?, "show"?:["box","violin","points"], "groups":[{"label","values":[..]}
//   | {"label","q1","median","q3","whiskerLo"?,"whiskerHi"?,"outliers"?}]}
import type { ReactNode } from "react";

// data-viz categorical palette (validated, CVD-safe), light / dark by slot order.
const CAT_LIGHT = ["#2a78d6", "#1baf7a", "#eda100", "#008300", "#4a3aa7", "#e34948", "#e87ba4", "#eb6834"];
const CAT_DARK = ["#3987e5", "#199e70", "#c98500", "#008300", "#9085e9", "#e66767", "#d55181", "#d95926"];
function isDark(): boolean {
  const t = document.documentElement.dataset.theme;
  if (t === "dark") return true;
  if (t === "light") return false;
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? false;
}

type Stats = {
  label: string;
  q1: number;
  median: number;
  q3: number;
  whiskerLo: number;
  whiskerHi: number;
  outliers: number[];
  values: number[] | null; // present only when the group emitted raw values
  n: number;
};

// linear-interpolation quantile (numpy "type 7" default) over a sorted array.
function quantile(sorted: number[], p: number): number {
  const n = sorted.length;
  if (n === 0) return NaN;
  if (n === 1) return sorted[0];
  const idx = p * (n - 1);
  const lo = Math.floor(idx);
  const hi = Math.ceil(idx);
  if (lo === hi) return sorted[lo];
  return sorted[lo] * (hi - idx) + sorted[hi] * (idx - lo);
}

function isFiniteNum(x: unknown): x is number {
  return typeof x === "number" && Number.isFinite(x);
}

// Normalise a raw or pre-summarised group into Stats, or null if unusable.
function toStats(g: unknown): Stats | null {
  if (!g || typeof g !== "object") return null;
  const o = g as Record<string, unknown>;
  const label = typeof o.label === "string" ? o.label : "";

  if (Array.isArray(o.values)) {
    const vals = (o.values as unknown[]).filter(isFiniteNum);
    if (vals.length) {
      const s = [...vals].sort((a, b) => a - b);
      const q1 = quantile(s, 0.25);
      const median = quantile(s, 0.5);
      const q3 = quantile(s, 0.75);
      const iqr = q3 - q1;
      const loFence = q1 - 1.5 * iqr;
      const hiFence = q3 + 1.5 * iqr;
      const inl = s.filter((v) => v >= loFence && v <= hiFence);
      const whiskerLo = inl.length ? inl[0] : s[0];
      const whiskerHi = inl.length ? inl[inl.length - 1] : s[s.length - 1];
      const outliers = s.filter((v) => v < loFence || v > hiFence);
      return { label, q1, median, q3, whiskerLo, whiskerHi, outliers, values: vals, n: vals.length };
    }
  }

  // pre-summarised form
  const q1 = isFiniteNum(o.q1) ? o.q1 : null;
  const median = isFiniteNum(o.median) ? o.median : null;
  const q3 = isFiniteNum(o.q3) ? o.q3 : null;
  if (q1 === null || median === null || q3 === null) return null;
  const whiskerLo = isFiniteNum(o.whiskerLo) ? o.whiskerLo : q1;
  const whiskerHi = isFiniteNum(o.whiskerHi) ? o.whiskerHi : q3;
  const outliers = Array.isArray(o.outliers) ? (o.outliers as unknown[]).filter(isFiniteNum) : [];
  return { label, q1, median, q3, whiskerLo, whiskerHi, outliers, values: null, n: 0 };
}

// nice-number axis ticks (Heckbert).
function niceNum(range: number, round: boolean): number {
  const exp = Math.floor(Math.log10(range || 1));
  const f = (range || 1) / Math.pow(10, exp);
  let nf: number;
  if (round) nf = f < 1.5 ? 1 : f < 3 ? 2 : f < 7 ? 5 : 10;
  else nf = f <= 1 ? 1 : f <= 2 ? 2 : f <= 5 ? 5 : 10;
  return nf * Math.pow(10, exp);
}
function niceTicks(min: number, max: number, count = 5) {
  const range = niceNum(max - min, false);
  const step = niceNum(range / Math.max(1, count - 1), true) || 1;
  const niceMin = Math.floor(min / step) * step;
  const niceMax = Math.ceil(max / step) * step;
  const out: number[] = [];
  for (let v = niceMin; v <= niceMax + step * 0.5; v += step) out.push(Number(v.toFixed(10)));
  return { niceMin, niceMax, out };
}

function fmt(v: number): string {
  if (!Number.isFinite(v)) return "–";
  const a = Math.abs(v);
  const d = a >= 100 ? 0 : a >= 10 ? 1 : a >= 1 ? 2 : 3;
  return v.toFixed(d).replace(/\.0+$|(\.\d*?)0+$/, "$1");
}

// mulberry32 seeded PRNG → deterministic, reproducible jitter (audit-stable across renders).
function rng(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

// Gaussian-KDE bandwidth (Silverman, robust to spread). 0 → skip violin.
function bandwidth(vals: number[], q1: number, q3: number): number {
  const n = vals.length;
  if (n < 2) return 0;
  const mean = vals.reduce((s, v) => s + v, 0) / n;
  let ss = 0;
  for (const v of vals) ss += (v - mean) * (v - mean);
  const sigma = Math.sqrt(ss / (n - 1));
  const iqr = q3 - q1;
  const spread = iqr > 0 ? Math.min(sigma, iqr / 1.349) : sigma;
  const bw = 0.9 * (spread > 0 ? spread : sigma) * Math.pow(n, -1 / 5);
  return bw > 0 ? bw : 0;
}

// symmetric KDE outline for a violin, sampled across the shared [dMin,dMax] domain.
function violinPath(
  vals: number[],
  dMin: number,
  dMax: number,
  yScale: (v: number) => number,
  gx: number,
  half: number,
  bw: number,
): string {
  const K = 64;
  const invBw = 1 / bw;
  const norm = 1 / (vals.length * bw * Math.sqrt(2 * Math.PI));
  const dens: number[] = [];
  let maxD = 0;
  for (let k = 0; k <= K; k++) {
    const y = dMin + ((dMax - dMin) * k) / K;
    let sum = 0;
    for (const v of vals) {
      const z = (y - v) * invBw;
      sum += Math.exp(-0.5 * z * z);
    }
    const d = sum * norm;
    dens.push(d);
    if (d > maxD) maxD = d;
  }
  if (maxD <= 0) return "";
  const right: string[] = [];
  const left: string[] = [];
  for (let k = 0; k <= K; k++) {
    const y = dMin + ((dMax - dMin) * k) / K;
    const py = yScale(y);
    const wdt = (dens[k] / maxD) * half;
    right.push(`${(gx + wdt).toFixed(2)},${py.toFixed(2)}`);
    left.push(`${(gx - wdt).toFixed(2)},${py.toFixed(2)}`);
  }
  left.reverse();
  return `M${right.join(" L")} L${left.join(" L")} Z`;
}

type Spec = {
  title?: string;
  ylabel?: string;
  show?: string[];
  groups: unknown[];
};

const VALID_SHOW = ["box", "violin", "points"] as const;

export function Distribution({ spec: raw, caption }: { spec: string; caption?: string }) {
  let parsed: Spec;
  try {
    parsed = JSON.parse(raw.trim());
  } catch (e) {
    return <div className="ax-fig__err">分布图规范不是有效 JSON · {String((e as Error)?.message || e)}</div>;
  }
  if (!parsed || !Array.isArray(parsed.groups) || parsed.groups.length === 0) {
    return <div className="ax-fig__err">分布图需要非空的 groups 数组</div>;
  }

  const groups: Stats[] = [];
  for (const g of parsed.groups) {
    const s = toStats(g);
    if (s) groups.push(s);
  }
  if (groups.length === 0) {
    return <div className="ax-fig__err">分布图的每个 group 需要 values 或 q1/median/q3</div>;
  }

  let show = Array.isArray(parsed.show)
    ? parsed.show.filter((s): s is (typeof VALID_SHOW)[number] => (VALID_SHOW as readonly string[]).includes(s))
    : ["box", "points"];
  if (show.length === 0) show = ["box"];
  const wantBox = show.includes("box");
  const wantPoints = show.includes("points");
  const wantViolin = show.includes("violin");

  // shared numeric domain across every emitted value.
  let gMin = Infinity;
  let gMax = -Infinity;
  const acc = (v: number) => {
    if (v < gMin) gMin = v;
    if (v > gMax) gMax = v;
  };
  for (const s of groups) {
    if (s.values) for (const v of s.values) acc(v);
    else {
      acc(s.whiskerLo);
      acc(s.whiskerHi);
      acc(s.q1);
      acc(s.q3);
      acc(s.median);
    }
    for (const o of s.outliers) acc(o);
  }
  if (!(gMax > gMin)) {
    const c = Number.isFinite(gMin) ? gMin : 0;
    gMin = c - 1;
    gMax = c + 1;
  }
  const { niceMin: dMin, niceMax: dMax, out: ticks } = niceTicks(gMin, gMax, 5);

  const cat = isDark() ? CAT_DARK : CAT_LIGHT;
  const anyRaw = groups.some((s) => s.values && s.values.length);

  // geometry
  const band = 92;
  const boxHalf = 17;
  const jitterHalf = 13;
  const violinHalf = 26;
  const ptR = 2.3;
  const plotH = 250;
  const padL = 58;
  const padT = parsed.title ? 30 : 16;
  const padB = anyRaw ? 44 : 34;
  const padR = 18;
  const w = padL + groups.length * band + padR;
  const h = padT + plotH + padB;
  const yTop = padT;
  const yBot = padT + plotH;
  const span = dMax - dMin || 1;
  const yScale = (v: number) => yBot - ((v - dMin) / span) * plotH;
  const gx = (i: number) => padL + i * band + band / 2;

  return (
    <figure className="ax-fig">
      <div className="ax-fig__frame" style={{ overflowX: "auto", justifyContent: "flex-start" }}>
        <svg
          viewBox={`0 0 ${w} ${h}`}
          width={w}
          height={h}
          role="img"
          aria-label={caption || parsed.title || "distribution plot"}
          style={{ maxWidth: "100%" }}
        >
          {parsed.title && (
            <text x={padL} y={18} fontSize="12" fontWeight="600" fill="var(--ax-ink)">
              {parsed.title}
            </text>
          )}

          {/* y gridlines + tick labels */}
          {ticks.map((t, i) => {
            if (t < dMin - 1e-9 || t > dMax + 1e-9) return null;
            const y = yScale(t);
            return (
              <g key={`t${i}`}>
                <line x1={padL} y1={y} x2={w - padR} y2={y} stroke="var(--ax-line-soft)" strokeWidth="0.5" />
                <text x={padL - 6} y={y + 3} fontSize="9.5" textAnchor="end" fill="var(--ax-ink-faint)">
                  {fmt(t)}
                </text>
              </g>
            );
          })}
          {/* y axis */}
          <line x1={padL} y1={yTop} x2={padL} y2={yBot} stroke="var(--ax-line)" strokeWidth="1" />
          {parsed.ylabel && (
            <text
              x={14}
              y={yTop + plotH / 2}
              fontSize="10"
              textAnchor="middle"
              fill="var(--ax-ink-faint)"
              transform={`rotate(-90 14 ${yTop + plotH / 2})`}
            >
              {parsed.ylabel}
            </text>
          )}

          {/* per-group marks */}
          {groups.map((s, i) => {
            const x = gx(i);
            const color = cat[i % cat.length];
            const iqr = s.q3 - s.q1;
            const yQ1 = yScale(s.q1);
            const yQ3 = yScale(s.q3);
            const yMed = yScale(s.median);
            const yWl = yScale(s.whiskerLo);
            const yWh = yScale(s.whiskerHi);
            const parts: ReactNode[] = [];

            // violin (behind everything) — only for raw groups, gated by show:["violin"]
            let violinRendered = false;
            if (wantViolin && s.values && s.values.length >= 3) {
              const bw = bandwidth(s.values, s.q1, s.q3);
              if (bw > 0) {
                const path = violinPath(s.values, dMin, dMax, yScale, x, violinHalf, bw);
                if (path) {
                  parts.push(
                    <path key="violin" d={path} fill={color} fillOpacity={0.14} stroke={color} strokeOpacity={0.6} strokeWidth="1" />,
                  );
                  violinRendered = true;
                }
              }
            }

            // box + whiskers
            if (wantBox) {
              parts.push(
                <g key="box">
                  {/* whiskers */}
                  <line x1={x} y1={yWh} x2={x} y2={yQ3} stroke={color} strokeWidth="1" />
                  <line x1={x} y1={yQ1} x2={x} y2={yWl} stroke={color} strokeWidth="1" />
                  <line x1={x - boxHalf / 2} y1={yWh} x2={x + boxHalf / 2} y2={yWh} stroke={color} strokeWidth="1" />
                  <line x1={x - boxHalf / 2} y1={yWl} x2={x + boxHalf / 2} y2={yWl} stroke={color} strokeWidth="1" />
                  {/* box */}
                  <rect
                    x={x - boxHalf}
                    y={Math.min(yQ1, yQ3)}
                    width={boxHalf * 2}
                    height={Math.max(1, Math.abs(yQ1 - yQ3))}
                    fill={color}
                    fillOpacity={0.12}
                    stroke={color}
                    strokeWidth="1.2"
                  >
                    <title>{`${s.label} — median ${fmt(s.median)} · Q1 ${fmt(s.q1)} · Q3 ${fmt(s.q3)} · IQR ${fmt(iqr)}${s.n ? ` · n=${s.n}` : ""}`}</title>
                  </rect>
                  {/* median */}
                  <line x1={x - boxHalf} y1={yMed} x2={x + boxHalf} y2={yMed} stroke={color} strokeWidth="1.8" />
                </g>,
              );
            } else if (!violinRendered) {
              // no box requested and no violin actually rendered for this group
              // (not eligible, or degenerate zero-variance → bandwidth 0 → no path):
              // draw a minimal IQR + median mark so the slot isn't empty.
              parts.push(
                <g key="minbox">
                  <line x1={x} y1={yQ3} x2={x} y2={yQ1} stroke={color} strokeWidth="1.4" />
                  <line x1={x - boxHalf} y1={yMed} x2={x + boxHalf} y2={yMed} stroke={color} strokeWidth="1.8">
                    <title>{`${s.label} — median ${fmt(s.median)} · Q1 ${fmt(s.q1)} · Q3 ${fmt(s.q3)}`}</title>
                  </line>
                </g>,
              );
            }

            // raw jittered points (reveal n + shape, including any outliers)
            if (wantPoints && s.values && s.values.length) {
              const rand = rng(i * 100003 + s.values.length * 31 + 7);
              parts.push(
                <g key="pts">
                  {s.values.map((v, j) => {
                    const jx = x + (rand() * 2 - 1) * jitterHalf;
                    return (
                      <circle key={j} cx={jx} cy={yScale(v)} r={ptR} fill={color} fillOpacity={0.72} stroke="var(--ax-surface)" strokeWidth="0.4">
                        <title>{`${s.label}: ${fmt(v)}`}</title>
                      </circle>
                    );
                  })}
                </g>,
              );
            }

            // outlier marks — only when points aren't already showing every raw value
            const showOutlierMarks = s.outliers.length > 0 && (!wantPoints || s.values === null);
            if (showOutlierMarks) {
              parts.push(
                <g key="out">
                  {s.outliers.map((v, j) => (
                    <circle key={j} cx={x} cy={yScale(v)} r={2.4} fill="none" stroke={color} strokeWidth="1">
                      <title>{`${s.label}: ${fmt(v)} (outlier)`}</title>
                    </circle>
                  ))}
                </g>,
              );
            }

            return <g key={`g${i}`}>{parts}</g>;
          })}

          {/* group labels (direct-labelled — no separate colour legend needed) */}
          {groups.map((s, i) => (
            <g key={`lab${i}`}>
              <text x={gx(i)} y={yBot + 15} fontSize="10.5" textAnchor="middle" fill="var(--ax-ink-soft)">
                {s.label}
              </text>
              {s.n > 0 && (
                <text x={gx(i)} y={yBot + 27} fontSize="8.5" textAnchor="middle" fill="var(--ax-ink-faint)">
                  n={s.n}
                </text>
              )}
            </g>
          ))}
        </svg>
      </div>
      <figcaption className="ax-fig__cap">
        <span className="ax-fig__kind">分布 · distribution</span> {caption || parsed.title || ""}
      </figcaption>
    </figure>
  );
}
