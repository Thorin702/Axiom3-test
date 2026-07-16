// Native distribution figure (```dist JSON): compare a numeric distribution across groups —
// box (quartiles + 1.5·IQR whiskers + outliers), optional seeded-jitter raw points, and an
// optional Gaussian-KDE violin (gated behind show:["violin"]). Quartiles/whiskers are computed
// client-side from the raw `values` (auditable); groups may also arrive pre-summarised.
//
// SuperPlot (Lord et al., JCB 2020) — replicate honesty:
//   • optional spec.replicateType ∈ {"biological","technical"} → noted in the caption.
//   • optional per-group `replicateId` (array parallel to `values`) → SuperPlot style: raw points
//     coloured by replicate, per-replicate mean markers overlaid, and the group summary (mean ± SEM)
//     is computed over the *replicate means* — never over all raw points — so technical replicates
//     cannot inflate n. Displayed n switches to the number of replicates.
//   • no replicate fields → identical to the prior behaviour (graceful degradation).
//
// Spec: {"title"?, "ylabel"?, "show"?:["box","violin","points"], "whiskers"?:"tukey"|"5-95",
//   "replicateType"?:"biological"|"technical",
//   "groups":[{"label","values":[..],"replicateId"?:[..]}
//   | {"label","q1","median","q3","whiskerLo"?,"whiskerHi"?,"outliers"?,"n"?}]}
import type { ReactNode } from "react";
import { catColor, isDark } from "./theme";

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
  // SuperPlot replicate structure — all null / 0 unless a valid `replicateId` was supplied.
  replicateIds: (string | number)[] | null; // parallel to `values`: replicate label per raw point
  repLabels: (string | number)[] | null; // distinct replicate labels, first-seen order
  replicateMeans: number[] | null; // mean of each replicate, aligned to repLabels
  repMean: number | null; // mean of the replicate means (SuperPlot group centre)
  repSem: number | null; // SEM of the replicate means (SD / √k)
  nReplicates: number; // number of distinct replicates
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
// whiskerMode selects the whisker rule for raw groups: "tukey" (default,
// 1.5·IQR fences) or "5-95" (5th–95th-percentile clips). Both are computed
// from the raw values, so the caption can state exactly what the whiskers mean.
function toStats(g: unknown, whiskerMode: "tukey" | "5-95" = "tukey"): Stats | null {
  if (!g || typeof g !== "object") return null;
  const o = g as Record<string, unknown>;
  const label = typeof o.label === "string" ? o.label : "";

  if (Array.isArray(o.values)) {
    const rawVals = o.values as unknown[];
    // replicateId is honoured only when it is an array of matching length (else ignored → graceful).
    const rawReps =
      Array.isArray(o.replicateId) && (o.replicateId as unknown[]).length === rawVals.length
        ? (o.replicateId as unknown[])
        : null;
    const vals: number[] = [];
    const reps: (string | number)[] = [];
    for (let i = 0; i < rawVals.length; i++) {
      const v = rawVals[i];
      if (isFiniteNum(v)) {
        vals.push(v);
        if (rawReps) {
          const r = rawReps[i];
          reps.push(typeof r === "number" ? r : String(r));
        }
      }
    }
    if (vals.length) {
      const s = [...vals].sort((a, b) => a - b);
      const q1 = quantile(s, 0.25);
      const median = quantile(s, 0.5);
      const q3 = quantile(s, 0.75);
      let whiskerLo: number;
      let whiskerHi: number;
      let outliers: number[];
      if (whiskerMode === "5-95") {
        const loFence = quantile(s, 0.05);
        const hiFence = quantile(s, 0.95);
        const inl = s.filter((v) => v >= loFence && v <= hiFence);
        whiskerLo = inl.length ? inl[0] : s[0];
        whiskerHi = inl.length ? inl[inl.length - 1] : s[s.length - 1];
        outliers = s.filter((v) => v < loFence || v > hiFence);
      } else {
        const iqr = q3 - q1;
        const loFence = q1 - 1.5 * iqr;
        const hiFence = q3 + 1.5 * iqr;
        const inl = s.filter((v) => v >= loFence && v <= hiFence);
        whiskerLo = inl.length ? inl[0] : s[0];
        whiskerHi = inl.length ? inl[inl.length - 1] : s[s.length - 1];
        outliers = s.filter((v) => v < loFence || v > hiFence);
      }

      // SuperPlot replicate aggregation — mean of each replicate, then mean/SEM over those means.
      let replicateIds: (string | number)[] | null = null;
      let repLabels: (string | number)[] | null = null;
      let replicateMeans: number[] | null = null;
      let repMean: number | null = null;
      let repSem: number | null = null;
      let nReplicates = 0;
      if (rawReps && reps.length === vals.length) {
        const order: (string | number)[] = [];
        const buckets = new Map<string, number[]>();
        for (let i = 0; i < vals.length; i++) {
          const key = String(reps[i]);
          let b = buckets.get(key);
          if (!b) {
            b = [];
            buckets.set(key, b);
            order.push(reps[i]);
          }
          b.push(vals[i]);
        }
        replicateIds = reps;
        repLabels = order;
        replicateMeans = order.map((r) => {
          const b = buckets.get(String(r)) as number[];
          return b.reduce((a, v) => a + v, 0) / b.length;
        });
        nReplicates = replicateMeans.length;
        repMean = replicateMeans.reduce((a, v) => a + v, 0) / nReplicates;
        if (nReplicates >= 2) {
          let ss = 0;
          for (const m of replicateMeans) ss += (m - repMean) * (m - repMean);
          const sd = Math.sqrt(ss / (nReplicates - 1));
          repSem = sd / Math.sqrt(nReplicates);
        } else {
          repSem = 0;
        }
      }

      return {
        label,
        q1,
        median,
        q3,
        whiskerLo,
        whiskerHi,
        outliers,
        values: vals,
        n: vals.length,
        replicateIds,
        repLabels,
        replicateMeans,
        repMean,
        repSem,
        nReplicates,
      };
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
  // optional declared sample size for pre-summarised groups (no raw values to count)
  const nDecl = isFiniteNum(o.n) && o.n >= 0 ? Math.round(o.n) : 0;
  return {
    label,
    q1,
    median,
    q3,
    whiskerLo,
    whiskerHi,
    outliers,
    values: null,
    n: nDecl,
    replicateIds: null,
    repLabels: null,
    replicateMeans: null,
    repMean: null,
    repSem: null,
    nReplicates: 0,
  };
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

// CVD-safe marker: colour AND shape both encode the replicate (index 0..3 → ●▲■◆, then cycle),
// so replicates stay distinguishable in grayscale / for colour-blind readers.
function shapeMark(cx: number, cy: number, r: number, shapeIdx: number, fill: string, key: string, title: string): ReactNode {
  const s = ((shapeIdx % 4) + 4) % 4;
  const stroke = "var(--ax-surface)";
  const sw = 1.1;
  if (s === 1) {
    return (
      <polygon key={key} points={`${cx},${cy - r} ${cx + r},${cy + r * 0.85} ${cx - r},${cy + r * 0.85}`} fill={fill} stroke={stroke} strokeWidth={sw}>
        <title>{title}</title>
      </polygon>
    );
  }
  if (s === 2) {
    return (
      <rect key={key} x={cx - r * 0.9} y={cy - r * 0.9} width={r * 1.8} height={r * 1.8} fill={fill} stroke={stroke} strokeWidth={sw}>
        <title>{title}</title>
      </rect>
    );
  }
  if (s === 3) {
    return (
      <polygon key={key} points={`${cx},${cy - r} ${cx + r},${cy} ${cx},${cy + r} ${cx - r},${cy}`} fill={fill} stroke={stroke} strokeWidth={sw}>
        <title>{title}</title>
      </polygon>
    );
  }
  return (
    <circle key={key} cx={cx} cy={cy} r={r} fill={fill} stroke={stroke} strokeWidth={sw}>
      <title>{title}</title>
    </circle>
  );
}

type Spec = {
  title?: string;
  ylabel?: string;
  show?: string[];
  whiskers?: "tukey" | "5-95";
  replicateType?: "biological" | "technical";
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

  const whiskerMode: "tukey" | "5-95" = parsed.whiskers === "5-95" ? "5-95" : "tukey";
  const replicateType =
    parsed.replicateType === "biological" || parsed.replicateType === "technical" ? parsed.replicateType : undefined;

  const groups: Stats[] = [];
  for (const g of parsed.groups) {
    const s = toStats(g, whiskerMode);
    if (s) groups.push(s);
  }
  if (groups.length === 0) {
    return <div className="ax-fig__err">分布图的每个 group 需要 values 或 q1/median/q3</div>;
  }

  // SuperPlot active when at least one group carries a valid replicate structure.
  const superPlot = groups.some((s) => s.replicateMeans && s.nReplicates > 0);

  // Global replicate → (colour, shape) index, in first-seen order across every group, so the same
  // replicate reads with the same colour/shape in every condition (SuperPlot convention).
  const repIndex = new Map<string, number>();
  const repOrder: (string | number)[] = [];
  if (superPlot) {
    for (const s of groups) {
      if (s.replicateIds) {
        for (const r of s.replicateIds) {
          const k = String(r);
          if (!repIndex.has(k)) {
            repIndex.set(k, repOrder.length);
            repOrder.push(r);
          }
        }
      }
    }
  }
  const repOf = (r: string | number) => repIndex.get(String(r)) ?? 0;

  let show = Array.isArray(parsed.show)
    ? parsed.show.filter((s): s is (typeof VALID_SHOW)[number] => (VALID_SHOW as readonly string[]).includes(s))
    : superPlot
      ? ["points"] // SuperPlot default: swarm + replicate means + summary (raw-value box would imply cell-level n)
      : ["box", "points"];
  if (show.length === 0) show = ["box"];
  const wantBox = show.includes("box");
  const explicitPoints = show.includes("points");
  const wantViolin = show.includes("violin");

  const anyRaw = groups.some((s) => s.values && s.values.length);
  // Honesty: with ≤10 raw points a box hides the real spread — always reveal the points (additive).
  const maxRawN = groups.reduce((m, s) => Math.max(m, s.values ? s.values.length : 0), 0);
  const smallN = anyRaw && maxRawN <= 10;
  const forcedPoints = smallN && !explicitPoints;
  const wantPoints = explicitPoints || forcedPoints;

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
    if (s.repMean != null) {
      acc(s.repMean + (s.repSem || 0));
      acc(s.repMean - (s.repSem || 0));
    }
  }
  if (!(gMax > gMin)) {
    const c = Number.isFinite(gMin) ? gMin : 0;
    gMin = c - 1;
    gMax = c + 1;
  }
  const { niceMin: dMin, niceMax: dMax, out: ticks } = niceTicks(gMin, gMax, 5);

  const dark = isDark();

  // geometry
  const band = 92;
  const boxHalf = 17;
  const jitterHalf = 13;
  const violinHalf = 26;
  const meanSpread = 11; // horizontal spread of replicate-mean markers within a group
  const ptR = 2.3;
  const plotH = 250;
  const padL = 58;
  const padT = parsed.title ? 30 : 16;
  const legendH = repOrder.length > 0 ? 22 : 0;
  const padB = (anyRaw ? 44 : 34) + legendH;
  const padR = 18;
  const w = padL + groups.length * band + padR;
  const h = padT + plotH + padB;
  const yTop = padT;
  const yBot = padT + plotH;
  const span = dMax - dMin || 1;
  const yScale = (v: number) => yBot - ((v - dMin) / span) * plotH;
  const gx = (i: number) => padL + i * band + band / 2;

  // caption method notes — surface exactly what the marks mean (auditable, no fabrication).
  const notes: string[] = [];
  if ((wantBox || (!superPlot && anyRaw)) && anyRaw) {
    notes.push(whiskerMode === "5-95" ? "须状线:5–95 百分位" : "须状线:Tukey 1.5×IQR");
  }
  if (replicateType) notes.push(`重复类型:${replicateType === "biological" ? "生物学重复" : "技术重复"}`);
  if (superPlot) {
    notes.push("SuperPlot:原始点按重复上色,组均值±SEM 基于各重复的均值(n=重复数,非单点数)");
  } else if (replicateType === "technical") {
    notes.push("技术重复不代表独立样本,未据此膨胀 n");
  }
  if (forcedPoints) notes.push("n≤10,已显示全部原始点");

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
            const color = catColor(i, dark);
            const iqr = s.q3 - s.q1;
            const yQ1 = yScale(s.q1);
            const yQ3 = yScale(s.q3);
            const yMed = yScale(s.median);
            const yWl = yScale(s.whiskerLo);
            const yWh = yScale(s.whiskerHi);
            const hasSummary = s.repMean != null; // SuperPlot replicate summary for this group
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
            } else if (!violinRendered && !hasSummary) {
              // no box, no violin, no SuperPlot summary → draw a minimal IQR + median mark so the slot isn't empty.
              parts.push(
                <g key="minbox">
                  <line x1={x} y1={yQ3} x2={x} y2={yQ1} stroke={color} strokeWidth="1.4" />
                  <line x1={x - boxHalf} y1={yMed} x2={x + boxHalf} y2={yMed} stroke={color} strokeWidth="1.8">
                    <title>{`${s.label} — median ${fmt(s.median)} · Q1 ${fmt(s.q1)} · Q3 ${fmt(s.q3)}`}</title>
                  </line>
                </g>,
              );
            }

            // raw jittered points (reveal n + shape). In SuperPlot mode each point is coloured by replicate.
            if (wantPoints && s.values && s.values.length) {
              const rand = rng(i * 100003 + s.values.length * 31 + 7);
              parts.push(
                <g key="pts">
                  {s.values.map((v, j) => {
                    const jx = x + (rand() * 2 - 1) * jitterHalf;
                    const pc = s.replicateIds ? catColor(repOf(s.replicateIds[j]), dark) : color;
                    return (
                      <circle key={j} cx={jx} cy={yScale(v)} r={ptR} fill={pc} fillOpacity={0.72} stroke="var(--ax-surface)" strokeWidth="0.4">
                        <title>{s.replicateIds ? `${s.label} · ${String(s.replicateIds[j])}: ${fmt(v)}` : `${s.label}: ${fmt(v)}`}</title>
                      </circle>
                    );
                  })}
                </g>,
              );
            }

            // SuperPlot: per-replicate mean markers (colour + shape encode the replicate).
            if (s.replicateMeans && s.repLabels) {
              const K = s.replicateMeans.length;
              parts.push(
                <g key="repmeans">
                  {s.replicateMeans.map((m, k) => {
                    const gi = repOf((s.repLabels as (string | number)[])[k]);
                    const off = K > 1 ? (k / (K - 1) - 0.5) * 2 * meanSpread : 0;
                    return shapeMark(
                      x + off,
                      yScale(m),
                      5,
                      gi,
                      catColor(gi, dark),
                      `rm${k}`,
                      `${s.label} · ${String((s.repLabels as (string | number)[])[k])} mean ${fmt(m)}`,
                    );
                  })}
                </g>,
              );
            }

            // SuperPlot summary: mean ± SEM over the replicate means (drawn on top, neutral ink).
            if (hasSummary && s.repMean != null) {
              const yM = yScale(s.repMean);
              const sem = s.repSem || 0;
              const yHi = yScale(s.repMean + sem);
              const yLo = yScale(s.repMean - sem);
              parts.push(
                <g key="spsum">
                  <line x1={x} y1={yHi} x2={x} y2={yLo} stroke="var(--ax-ink)" strokeWidth="1.4" />
                  <line x1={x - 6} y1={yHi} x2={x + 6} y2={yHi} stroke="var(--ax-ink)" strokeWidth="1.4" />
                  <line x1={x - 6} y1={yLo} x2={x + 6} y2={yLo} stroke="var(--ax-ink)" strokeWidth="1.4" />
                  <line x1={x - boxHalf} y1={yM} x2={x + boxHalf} y2={yM} stroke="var(--ax-ink)" strokeWidth="2.4">
                    <title>{`${s.label} — mean ${fmt(s.repMean)} ± SEM ${fmt(sem)} · n=${s.nReplicates} 重复`}</title>
                  </line>
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
              {s.nReplicates > 0 ? (
                <text x={gx(i)} y={yBot + 27} fontSize="8.5" textAnchor="middle" fill="var(--ax-ink-faint)">
                  {`n=${s.nReplicates} 重复${s.n ? ` · ${s.n} 点` : ""}`}
                </text>
              ) : (
                s.n > 0 && (
                  <text x={gx(i)} y={yBot + 27} fontSize="8.5" textAnchor="middle" fill="var(--ax-ink-faint)">
                    n={s.n}
                  </text>
                )
              )}
            </g>
          ))}

          {/* replicate legend (SuperPlot) — colour + shape per replicate */}
          {repOrder.length > 0 &&
            (() => {
              const ly = h - 8;
              const items = repOrder.map((r) => String(r));
              // approximate widths → centre the row
              const widths = items.map((t) => 16 + t.length * 6.2 + 14);
              const total = widths.reduce((a, b) => a + b, 0);
              let cx = Math.max(padL, (w - total) / 2);
              return (
                <g key="replegend">
                  {repOrder.map((r, k) => {
                    const gi = repOf(r);
                    const mx = cx + 6;
                    const tx = cx + 16;
                    const node = (
                      <g key={`rl${k}`}>
                        {shapeMark(mx, ly - 3, 4.2, gi, catColor(gi, dark), `rls${k}`, String(r))}
                        <text x={tx} y={ly} fontSize="9" fill="var(--ax-ink-soft)">
                          {String(r)}
                        </text>
                      </g>
                    );
                    cx += widths[k];
                    return node;
                  })}
                </g>
              );
            })()}
        </svg>
      </div>
      <figcaption className="ax-fig__cap">
        <span className="ax-fig__kind">分布 · distribution</span> {caption || parsed.title || ""}
        {notes.length > 0 && (
          <span style={{ display: "block", marginTop: 2, color: "var(--ax-ink-faint)", fontSize: "0.86em" }}>{notes.join(" · ")}</span>
        )}
      </figcaption>
    </figure>
  );
}
