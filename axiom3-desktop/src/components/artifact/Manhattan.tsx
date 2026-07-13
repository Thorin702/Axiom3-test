// Native GWAS Manhattan plot: association signal (-log10 P) across chromosomes. X is banded
// per chromosome (proportional to genomic length, alternating tints); the renderer does the
// -log10 transform client-side and draws the genome-wide line at `threshold` (default 7.3 =
// 5e-8), labeling only loci above it. Emit curated lead loci + background — never fabricated
// thousands. Pure inline SVG, CSS-var theming, no dependency.
//
// Spec (```manhattan JSON): {"title"?, "threshold"?:number(-log10, default 7.3),
//   "points":[{"chr":19,"pos":11202306,"p":1e-250,"snp"?:"rs..","gene"?:"LDLR"}]}
type RawPoint = { chr: number | string; pos: number; p: number; snp?: string; gene?: string };
type Spec = { title?: string; threshold?: number; points: RawPoint[] };
type Pt = { chr: string; pos: number; p: number; y: number; snp?: string; gene?: string };
type Band = { chr: string; rank: number; len: number; maxPos: number; pts: Pt[]; width: number; offset: number };

// hg38 chromosome lengths (bp) — sizes the per-chromosome bands so short/long chrs read right.
const CHR_LEN: Record<string, number> = {
  "1": 248956422, "2": 242193529, "3": 198295559, "4": 190214555, "5": 181538259,
  "6": 170805979, "7": 159345973, "8": 145138636, "9": 138394717, "10": 133797422,
  "11": 135086622, "12": 133275309, "13": 114364328, "14": 107043718, "15": 101991189,
  "16": 90338345, "17": 83257441, "18": 80373285, "19": 58617616, "20": 64444167,
  "21": 46709983, "22": 50818468, X: 156040895, Y: 57227415, MT: 16569,
};

const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v));

function normChr(c: number | string): string {
  return String(c).trim().toUpperCase().replace(/^CHR/, "").replace(/^M$/, "MT");
}
function chrRank(c: string): number {
  const n = Number(c);
  if (Number.isFinite(n)) return n;
  if (c === "X") return 23;
  if (c === "Y") return 24;
  if (c === "MT") return 25;
  return 99;
}
function niceStep(range: number, count: number): number {
  const raw = range / Math.max(1, count);
  const mag = Math.pow(10, Math.floor(Math.log10(raw || 1)));
  const n = raw / mag;
  const s = n < 1.5 ? 1 : n < 3 ? 2 : n < 7 ? 5 : 10;
  return s * mag;
}

export function Manhattan({ spec: raw, caption }: { spec: string; caption?: string }) {
  let spec: Spec;
  try {
    spec = JSON.parse(raw.trim());
  } catch (e) {
    return <div className="ax-fig__err">曼哈顿图规范不是有效 JSON · {String((e as Error)?.message || e)}</div>;
  }
  if (!spec || typeof spec !== "object") return <div className="ax-fig__err">曼哈顿图规范为空或格式不对</div>;
  if (!Array.isArray(spec.points) || spec.points.length === 0) {
    return <div className="ax-fig__err">曼哈顿图需要 points 数组（每项 {"{chr,pos,p}"}）</div>;
  }

  const thr = typeof spec.threshold === "number" && isFinite(spec.threshold) && spec.threshold > 0 ? spec.threshold : 7.3;

  // Parse + -log10 transform (client-side; nothing fabricated). Guard non-object entries so a
  // stray null in `points` can never throw. Clamp p to a plottable range.
  const byChr = new Map<string, Band>();
  for (const rp of spec.points) {
    if (!rp || typeof rp !== "object") continue;
    const pos = Number(rp.pos);
    let p = Number(rp.p);
    if (!isFinite(pos) || Number.isNaN(p)) continue;
    if (p <= 0) p = 1e-320;
    p = clamp(p, 1e-320, 1);
    const chr = normChr(rp.chr);
    const pt: Pt = { chr, pos, p, y: -Math.log10(p), snp: rp.snp, gene: rp.gene };
    let b = byChr.get(chr);
    if (!b) {
      b = { chr, rank: chrRank(chr), len: 0, maxPos: 0, pts: [], width: 0, offset: 0 };
      byChr.set(chr, b);
    }
    b.pts.push(pt);
    b.maxPos = Math.max(b.maxPos, pos);
  }
  const bands = [...byChr.values()].sort((a, b) => a.rank - b.rank);
  if (bands.length === 0) return <div className="ax-fig__err">曼哈顿图无有效数据点（p 需为正数、pos 为数字）</div>;

  // Band widths ∝ genomic length (floor so short chrs stay legible), normalized to a min plot width.
  for (const b of bands) b.len = CHR_LEN[b.chr] ?? Math.max(b.maxPos, 1);
  const maxLen = Math.max(...bands.map((b) => b.len));
  for (const b of bands) b.width = Math.max(46, (b.len / maxLen) * 150);
  let sum = bands.reduce((s, b) => s + b.width, 0);
  if (sum < 360) {
    const f = 360 / sum;
    for (const b of bands) b.width *= f;
    sum = 360;
  }
  let acc = 0;
  for (const b of bands) {
    b.offset = acc;
    acc += b.width;
  }
  const plotW = sum;

  // tallest signal (loop, not spread — robust to large point counts).
  let maxVal = thr;
  for (const b of bands) for (const p of b.pts) if (p.y > maxVal) maxVal = p.y;
  const yTop = Math.max(maxVal, thr + 1) * 1.06;

  // geometry
  const padL = 46;
  const padR = 16;
  const plotH = 250;
  const titleH = spec.title ? 18 : 0;
  const padT = 8 + titleH + 18; // title line + legend row
  const legendY = padT - 7;
  const padB = 40;
  const w = padL + plotW + padR;
  const h = padT + plotH + padB;
  const plotBottom = padT + plotH;

  const inset = 4;
  const xOf = (b: Band, pos: number) => padL + b.offset + inset + clamp(pos / b.len, 0, 1) * (b.width - 2 * inset);
  const yOf = (v: number) => plotBottom - (v / yTop) * plotH;

  // y ticks
  const step = niceStep(yTop, 5);
  const ticks: number[] = [];
  for (let v = 0; v <= yTop + 1e-9; v += step) ticks.push(v);
  const tickFmt = (v: number) => (step >= 1 ? String(Math.round(v)) : v.toFixed(1));

  const pStr = Math.pow(10, -thr).toExponential(0); // genome-wide p, e.g. "5e-8"

  // above-threshold hits, labeled with gene||snp; greedy vertical stacking for close x.
  const hits = bands
    .flatMap((b) => b.pts.map((p) => ({ x: xOf(b, p.pos), y: yOf(p.y), label: p.gene || p.snp || "", p })))
    .filter((d) => d.p.y >= thr && d.label)
    .sort((a, b) => a.x - b.x);
  // Estimated half text width at 8.5px (CJK glyphs ≈ 1.8× a latin glyph) so labels never clip the edges.
  const halfW = (s: string) => {
    let wpx = 0;
    for (const ch of s) wpx += /[⺀-￯]/.test(ch) ? 4.3 : 2.4;
    return wpx;
  };
  const K = 4; // vertical stack slots before wrapping — caps the pile-up at the top clamp
  let prevX = -1e9;
  let run = 0;
  const labels = hits.map((d) => {
    if (d.x - prevX < 34) run += 1;
    else run = 0;
    prevX = d.x;
    const level = run % K; // wrap the vertical offset so labels don't all clamp to the same top y
    const wrap = Math.floor(run / K);
    const ly = clamp(d.y - 9 - level * 11, padT + 6, plotBottom);
    // On collisions, nudge sideways and grow the text away from the point (alternate start/end)
    // so labels sharing a row never sit on top of each other; first label stays centered.
    const dir = run % 2 === 0 ? 1 : -1;
    const anchor: "start" | "middle" | "end" = run === 0 ? "middle" : dir > 0 ? "start" : "end";
    const hw = halfW(d.label);
    const leftExt = anchor === "end" ? 2 * hw : anchor === "middle" ? hw : 0;
    const rightExt = anchor === "start" ? 2 * hw : anchor === "middle" ? hw : 0;
    const off = run === 0 ? 0 : dir * (3 + wrap * 5);
    const lx = clamp(d.x + off, padL + leftExt, padL + plotW - rightExt);
    return { ...d, ly, lx, anchor };
  });

  return (
    <figure className="ax-fig">
      <div className="ax-fig__frame" style={{ overflowX: "auto", justifyContent: "flex-start" }}>
        <svg viewBox={`0 0 ${w} ${h}`} width={w} height={h} role="img" aria-label={caption || spec.title || "manhattan plot"} style={{ maxWidth: "100%" }}>
          {spec.title && <text x={padL} y={16} fontSize="12" fontWeight="600" fill="var(--ax-ink)">{spec.title}</text>}

          {/* legend (2 categories; hits also carry text labels → never color-alone) */}
          <circle cx={padL + 4} cy={legendY - 3} r={3} fill="var(--ax-ink-faint)" />
          <text x={padL + 12} y={legendY} fontSize="9" fill="var(--ax-ink-faint)">background</text>
          <circle cx={padL + 96} cy={legendY - 3} r={3.4} fill="var(--ax-accent)" />
          <text x={padL + 104} y={legendY} fontSize="9" fill="var(--ax-ink-faint)">{`genome-wide P<${pStr}`}</text>

          {/* alternating chromosome band tints */}
          {bands.map((b, i) =>
            i % 2 === 1 ? (
              <rect key={`bg${b.chr}`} x={padL + b.offset} y={padT} width={b.width} height={plotH} fill="var(--ax-ink)" opacity="0.04" />
            ) : null
          )}

          {/* y grid + ticks */}
          {ticks.map((t, i) => (
            <g key={`yt${i}`}>
              <line x1={padL} y1={yOf(t)} x2={padL + plotW} y2={yOf(t)} stroke="var(--ax-line-soft)" strokeWidth="0.5" />
              <text x={padL - 6} y={yOf(t) + 3} fontSize="8.5" textAnchor="end" fill="var(--ax-ink-faint)">{tickFmt(t)}</text>
            </g>
          ))}

          {/* axes */}
          <line x1={padL} y1={padT} x2={padL} y2={plotBottom} stroke="var(--ax-line)" strokeWidth="0.75" />
          <line x1={padL} y1={plotBottom} x2={padL + plotW} y2={plotBottom} stroke="var(--ax-line)" strokeWidth="0.75" />
          <text x={12} y={padT + plotH / 2} fontSize="9.5" textAnchor="middle" fill="var(--ax-ink-faint)"
            transform={`rotate(-90 12 ${padT + plotH / 2})`}>-log10(P)</text>

          {/* genome-wide significance line */}
          <line x1={padL} y1={yOf(thr)} x2={padL + plotW} y2={yOf(thr)} stroke="var(--ax-accent)" strokeWidth="1" strokeDasharray="4 3" opacity="0.75" />
          <text x={padL + plotW - 4} y={yOf(thr) - 4} fontSize="8.5" textAnchor="end" fill="var(--ax-accent)">{`P<${pStr}`}</text>

          {/* chromosome labels */}
          {bands.map((b) => (
            <text key={`cl${b.chr}`} x={padL + b.offset + b.width / 2} y={plotBottom + 13} fontSize="9" textAnchor="middle" fill="var(--ax-ink-faint)">{b.chr}</text>
          ))}
          <text x={padL + plotW / 2} y={plotBottom + 30} fontSize="9.5" textAnchor="middle" fill="var(--ax-ink-faint)">chromosome</text>

          {/* background points first (alternating ink tints by chr) … */}
          {bands.map((b, i) =>
            b.pts
              .filter((p) => p.y < thr)
              .map((p, j) => (
                <circle key={`bp${b.chr}-${j}`} cx={xOf(b, p.pos)} cy={yOf(p.y)} r={2.2}
                  fill={i % 2 === 0 ? "var(--ax-ink-soft)" : "var(--ax-ink-faint)"}>
                  <title>{`${p.gene || p.snp || `chr${b.chr}`} — chr${b.chr}:${p.pos.toLocaleString()} · P=${p.p.toExponential(1)} · -log10 ${p.y.toFixed(1)}`}</title>
                </circle>
              ))
          )}

          {/* … then hits on top (accent, larger) so lead loci are never occluded */}
          {bands.map((b) =>
            b.pts
              .filter((p) => p.y >= thr)
              .map((p, j) => (
                <circle key={`hp${b.chr}-${j}`} cx={xOf(b, p.pos)} cy={yOf(p.y)} r={3.4}
                  fill="var(--ax-accent)" stroke="var(--ax-surface)" strokeWidth={0.6}>
                  <title>{`${p.gene || p.snp || `chr${b.chr}`} — chr${b.chr}:${p.pos.toLocaleString()} · P=${p.p.toExponential(1)} · -log10 ${p.y.toFixed(1)}`}</title>
                </circle>
              ))
          )}

          {/* labels for above-threshold loci only */}
          {labels.map((d, i) => (
            <g key={`lb${i}`}>
              <line x1={d.x} y1={d.y - 4} x2={d.lx} y2={d.ly + 1.5} stroke="var(--ax-line)" strokeWidth="0.5" />
              <text x={d.lx} y={d.ly} fontSize="8.5" textAnchor={d.anchor} fill="var(--ax-ink)">{d.label}</text>
            </g>
          ))}
        </svg>
      </div>
      <figcaption className="ax-fig__cap"><span className="ax-fig__kind">曼哈顿图 · manhattan</span> {caption || spec.title || ""}</figcaption>
    </figure>
  );
}
