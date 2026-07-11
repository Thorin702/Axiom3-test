// Native differential-expression volcano plot: x = log2 fold change, y = -log10(padj).
// Points are classified client-side by the DATA-supplied thresholds (up / down / n.s.) — the
// agent emits only the raw DE columns (gene, lfc, padj); this renderer computes -log10p, draws
// the ±lfc verticals and the p-cutoff horizontal, and labels the most extreme genes. Pure SVG.
//
// Spec (```volcano JSON): {"title"?, "lfcThreshold"?=1, "pThreshold"?=0.05,
//   "genes":[{"gene":<str>,"lfc":<log2FC>,"padj":<0..1>}]}
type Gene = { gene: string; lfc: number; padj: number };
type Spec = {
  title?: string;
  lfcThreshold?: number;
  pThreshold?: number;
  genes: Gene[];
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
const fmt = (v: number) => Number(v.toFixed(2)).toString();
function fmtP(p: number): string {
  if (!isFinite(p)) return String(p);
  if (p === 0) return "0";
  if (p < 1e-3) return p.toExponential(1);
  return p.toFixed(3);
}

export function Volcano({ spec: raw, caption }: { spec: string; caption?: string }) {
  let spec: Spec;
  try {
    spec = JSON.parse(raw.trim());
  } catch (e) {
    return <div className="ax-fig__err">火山图规范不是有效 JSON · {String((e as Error)?.message || e)}</div>;
  }
  if (!spec || typeof spec !== "object" || !Array.isArray(spec.genes)) {
    return <div className="ax-fig__err">火山图需要 genes 数组</div>;
  }

  const lfcT = typeof spec.lfcThreshold === "number" && spec.lfcThreshold > 0 ? spec.lfcThreshold : 1;
  const pT = typeof spec.pThreshold === "number" && spec.pThreshold > 0 && spec.pThreshold < 1 ? spec.pThreshold : 0.05;

  // clean + derive -log10(padj) client-side (nothing precomputed by the agent).
  // padj must be a finite, non-negative probability; anything else is dropped so an invalid
  // value can never be misplaced at the top of the plot or miscolored as significant.
  const clean = spec.genes.filter(
    (g): g is Gene =>
      !!g &&
      typeof g.lfc === "number" &&
      isFinite(g.lfc) &&
      typeof g.padj === "number" &&
      isFinite(g.padj) &&
      g.padj >= 0,
  );
  if (clean.length === 0) {
    return <div className="ax-fig__err">火山图需要有效的 genes（每项含数值 lfc 与 0..1 的 padj）</div>;
  }
  // padj === 0 -> -log10 diverges; represent as Infinity, later capped to the finite maximum.
  const rawY = clean.map((g) => (g.padj > 0 ? -Math.log10(g.padj) : Infinity));
  const finiteYs = rawY.filter((y) => isFinite(y) && y > 0);
  const yFiniteMax = finiteYs.length ? Math.max(...finiteYs) : 1;

  type Pt = { gene: string; lfc: number; padj: number; y: number; cls: "up" | "down" | "ns" };
  const pts: Pt[] = clean.map((g, i) => {
    const y = Math.max(0, isFinite(rawY[i]) ? rawY[i] : yFiniteMax);
    const sig = g.padj < pT;
    const cls: Pt["cls"] = sig && g.lfc > lfcT ? "up" : sig && g.lfc < -lfcT ? "down" : "ns";
    return { gene: String(g.gene ?? ""), lfc: g.lfc, padj: g.padj, y, cls };
  });

  const nUp = pts.filter((p) => p.cls === "up").length;
  const nDown = pts.filter((p) => p.cls === "down").length;
  const nNs = pts.filter((p) => p.cls === "ns").length;

  // domains: x symmetric around 0 (fold change), y from 0 up.
  const maxAbs = Math.max(...pts.map((p) => Math.abs(p.lfc)), lfcT);
  const xLim = Math.max(maxAbs * 1.05, lfcT * 1.4);
  const yThresh = -Math.log10(pT);
  const yLim = Math.max(yFiniteMax, yThresh) * 1.08 || 1;

  // colors: warm up-reg / cool down-reg / grey n.s. (grey = theme-adaptive ink token).
  const cat = isDark() ? CAT_DARK : CAT_LIGHT;
  const UP = cat[5]; // warm red
  const DOWN = cat[0]; // cool blue
  const NS = "var(--ax-ink-faint)";
  const clsColor = (c: Pt["cls"]) => (c === "up" ? UP : c === "down" ? DOWN : NS);

  // geometry
  const padL = 50;
  const padR = 108;
  const padT = spec.title ? 30 : 16;
  const padB = 46;
  const plotW = 440;
  const plotH = 320;
  const w = padL + plotW + padR;
  const h = padT + plotH + padB;
  const x0 = padL;
  const y0 = padT;
  const xTo = (lfc: number) => x0 + ((lfc + xLim) / (2 * xLim)) * plotW;
  const yTo = (y: number) => y0 + plotH - (y / yLim) * plotH;

  const xticks = axisTicks(-xLim, xLim, 6);
  const yticks = axisTicks(0, yLim, 5);

  // selective labels: most extreme significant genes (balance |lfc| and significance),
  // greedy non-overlap so every drawn label sits exactly on its point.
  type Box = { x0: number; y0: number; x1: number; y1: number };
  const placed: Box[] = [];
  const overlaps = (b: Box) =>
    placed.some((p) => b.x0 < p.x1 && b.x1 > p.x0 && b.y0 < p.y1 && b.y1 > p.y0);
  const ranked = pts
    .filter((p) => p.cls !== "ns" && p.gene)
    .map((p) => ({ p, score: Math.abs(p.lfc) / xLim + p.y / yLim }))
    .sort((a, b) => b.score - a.score)
    .slice(0, 12);
  const labels: { text: string; x: number; y: number; anchor: "start" | "end"; fill: string }[] = [];
  for (const { p } of ranked) {
    const px = xTo(p.lfc);
    const py = yTo(p.y);
    const left = px > x0 + plotW / 2; // label toward center to stay inside the plot
    const wBox = p.gene.length * 4.8 + 4;
    const bx0 = left ? px - 5 - wBox : px + 5;
    const bx1 = left ? px - 5 : px + 5 + wBox;
    const box: Box = { x0: bx0, y0: py - 7, x1: bx1, y1: py + 3 };
    if (overlaps(box)) continue;
    placed.push(box);
    labels.push({ text: p.gene, x: left ? px - 5 : px + 5, y: py + 3, anchor: left ? "end" : "start", fill: clsColor(p.cls) });
  }

  const lx = w - padR + 12;
  const legend: { c: string; label: string }[] = [
    { c: UP, label: `up (${nUp})` },
    { c: DOWN, label: `down (${nDown})` },
    { c: NS, label: `n.s. (${nNs})` },
  ];

  // point marker (shared by both draw passes) — a native <title> gives the hover tooltip.
  const renderPt = (p: Pt, key: string) => (
    <circle
      key={key}
      cx={xTo(p.lfc)}
      cy={yTo(p.y)}
      r={p.cls === "ns" ? 2 : 2.6}
      fill={clsColor(p.cls)}
      fillOpacity={p.cls === "ns" ? 0.45 : 0.85}
      stroke="var(--ax-surface)"
      strokeWidth="0.4"
    >
      <title>{`${p.gene} — log2FC ${p.lfc.toFixed(2)} · padj ${fmtP(p.padj)} · ${p.cls === "ns" ? "n.s." : p.cls}`}</title>
    </circle>
  );

  return (
    <figure className="ax-fig">
      <div className="ax-fig__frame" style={{ overflowX: "auto", justifyContent: "flex-start" }}>
        <svg viewBox={`0 0 ${w} ${h}`} width={w} height={h} role="img" aria-label={caption || spec.title || "volcano plot"} style={{ maxWidth: "100%" }}>
          {spec.title && <text x={padL} y={16} fontSize="12" fontWeight="600" fill="var(--ax-ink)">{spec.title}</text>}

          {/* grid */}
          {yticks.map((t, i) => (
            <line key={`gy${i}`} x1={x0} y1={yTo(t)} x2={x0 + plotW} y2={yTo(t)} stroke="var(--ax-line-soft)" strokeWidth="0.5" />
          ))}
          {xticks.map((t, i) => (
            <line key={`gx${i}`} x1={xTo(t)} y1={y0} x2={xTo(t)} y2={y0 + plotH} stroke="var(--ax-line-soft)" strokeWidth="0.5" />
          ))}

          {/* axis frame */}
          <line x1={x0} y1={y0} x2={x0} y2={y0 + plotH} stroke="var(--ax-line)" strokeWidth="1" />
          <line x1={x0} y1={y0 + plotH} x2={x0 + plotW} y2={y0 + plotH} stroke="var(--ax-line)" strokeWidth="1" />
          {/* x = 0 reference (no fold change) */}
          <line x1={xTo(0)} y1={y0} x2={xTo(0)} y2={y0 + plotH} stroke="var(--ax-line)" strokeWidth="0.75" />

          {/* data-defined thresholds */}
          <line x1={xTo(lfcT)} y1={y0} x2={xTo(lfcT)} y2={y0 + plotH} stroke="var(--ax-ink-faint)" strokeWidth="0.75" strokeDasharray="4 3" />
          <line x1={xTo(-lfcT)} y1={y0} x2={xTo(-lfcT)} y2={y0 + plotH} stroke="var(--ax-ink-faint)" strokeWidth="0.75" strokeDasharray="4 3" />
          <line x1={x0} y1={yTo(yThresh)} x2={x0 + plotW} y2={yTo(yThresh)} stroke="var(--ax-ink-faint)" strokeWidth="0.75" strokeDasharray="4 3" />
          <text x={x0 + plotW - 3} y={yTo(yThresh) - 3} fontSize="8" textAnchor="end" fill="var(--ax-ink-faint)">{`padj ${pT}`}</text>

          {/* points — draw n.s. underneath so the highlighted up/down genes are never occluded */}
          {pts.filter((p) => p.cls === "ns").map((p, i) => renderPt(p, `ns${i}`))}
          {pts.filter((p) => p.cls !== "ns").map((p, i) => renderPt(p, `sig${i}`))}

          {/* selective gene labels */}
          {labels.map((l, i) => (
            <text key={`lb${i}`} x={l.x} y={l.y} fontSize="8.5" textAnchor={l.anchor} fill={l.fill}>{l.text}</text>
          ))}

          {/* x ticks + label */}
          {xticks.map((t, i) => (
            <text key={`xt${i}`} x={xTo(t)} y={y0 + plotH + 13} fontSize="9" textAnchor="middle" fill="var(--ax-ink-faint)">{fmt(t)}</text>
          ))}
          <text x={x0 + plotW / 2} y={h - 6} fontSize="10" textAnchor="middle" fill="var(--ax-ink-soft)">log2 fold change</text>

          {/* y ticks + label */}
          {yticks.map((t, i) => (
            <text key={`yt${i}`} x={x0 - 6} y={yTo(t) + 3} fontSize="9" textAnchor="end" fill="var(--ax-ink-faint)">{fmt(t)}</text>
          ))}
          <text x={13} y={y0 + plotH / 2} fontSize="10" textAnchor="middle" fill="var(--ax-ink-soft)"
            transform={`rotate(-90 13 ${y0 + plotH / 2})`}>−log10 padj</text>

          {/* legend (data-defined classes + thresholds) */}
          {legend.map((g, i) => (
            <g key={`lg${i}`}>
              <circle cx={lx + 4} cy={padT + 8 + i * 17 + 3} r="4" fill={g.c} fillOpacity={g.c === NS ? 0.55 : 0.9} />
              <text x={lx + 14} y={padT + 8 + i * 17 + 6} fontSize="9.5" fill="var(--ax-ink-soft)">{g.label}</text>
            </g>
          ))}
          <text x={lx} y={padT + 8 + 3 * 17 + 12} fontSize="8" fill="var(--ax-ink-faint)">{`|log2FC| > ${lfcT}`}</text>
          <text x={lx} y={padT + 8 + 3 * 17 + 23} fontSize="8" fill="var(--ax-ink-faint)">{`padj < ${pT}`}</text>
        </svg>
      </div>
      <figcaption className="ax-fig__cap"><span className="ax-fig__kind">火山图 · volcano</span> {caption || spec.title || ""}</figcaption>
    </figure>
  );
}
