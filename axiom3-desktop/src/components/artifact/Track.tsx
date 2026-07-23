// Native protein/gene lollipop track (cBioPortal / MutationMapper staple): a length-scaled ruler
// with UniProt-style domain rectangles on the backbone and mutation lollipops rising above it —
// stem height + head size ∝ cohort count, head color = variant class. Two encodings, two legends
// (count size-legend + class color-legend); nothing derived is asserted beyond the emitted counts.
//
// Spec (```track JSON): {"title"?, "length":number, "unit"?:"aa"|"nt",
//   "domains":[{"name","from","to"}], "markers":[{"pos","label"?,"count","class"?}]}
//   class ∈ missense | nonsense | frameshift | splice (anything else → "other").
type Domain = { name: string; from: number; to: number };
type Marker = { pos: number; label?: string; count: number; class?: string };
type Spec = { title?: string; length: number; unit?: string; domains?: Domain[]; markers?: Marker[] };
import { catColor, isDark, autoContrastText } from "./theme";

const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v));

// fixed class → palette slot (distinct hues; stop-gain reads red, etc.). fallback "other" → orange.
const CLASS_SLOT: Record<string, number> = { missense: 0, nonsense: 5, frameshift: 2, splice: 4, other: 7 };
const CLASS_ORDER = ["missense", "nonsense", "frameshift", "splice", "other"];
function normClass(c?: string): string {
  const k = String(c ?? "").trim().toLowerCase();
  return k in CLASS_SLOT ? k : "other";
}

function niceStep(range: number, count: number): number {
  const raw = range / Math.max(1, count);
  const mag = Math.pow(10, Math.floor(Math.log10(raw || 1)));
  const n = raw / mag;
  const s = n < 1.5 ? 1 : n < 3 ? 2 : n < 7 ? 5 : 10;
  return s * mag;
}

// CJK-aware SVG text width estimate: full-width glyphs (CJK/Kana/Hangul) ≈ fontSize px,
// Latin/other ≈ 0.6·fontSize — so a "激酶结构域" label is not undercounted like Latin.
function isFullWidth(cp: number): boolean {
  return (
    (cp >= 0x1100 && cp <= 0x115f) || // Hangul Jamo
    (cp >= 0x2e80 && cp <= 0xa4cf) || // CJK radicals … Yi
    (cp >= 0xac00 && cp <= 0xd7a3) || // Hangul syllables
    (cp >= 0xf900 && cp <= 0xfaff) || // CJK compat ideographs
    (cp >= 0xfe30 && cp <= 0xfe4f) || // CJK compat forms
    (cp >= 0xff00 && cp <= 0xff60) || // fullwidth forms
    (cp >= 0xffe0 && cp <= 0xffe6) ||
    (cp >= 0x20000 && cp <= 0x3fffd) // CJK ext B+
  );
}
function textW(s: string, fontSize: number): number {
  let w = 0;
  for (const ch of s) w += isFullWidth(ch.codePointAt(0) ?? 0) ? fontSize : 0.6 * fontSize;
  return w;
}

export function Track({ spec: raw, caption }: { spec: string; caption?: string }) {
  let spec: Spec;
  try {
    spec = JSON.parse(raw.trim());
  } catch (e) {
    return <div className="ax-fig__err">结构域轨迹规范不是有效 JSON · {String((e as Error)?.message || e)}</div>;
  }
  if (!spec || typeof spec !== "object") return <div className="ax-fig__err">结构域轨迹规范为空或格式不对</div>;
  const length = Number(spec.length);
  if (!isFinite(length) || length <= 0) {
    return <div className="ax-fig__err">结构域轨迹需要正的 length（序列长度）</div>;
  }
  const domains = Array.isArray(spec.domains) ? spec.domains : [];
  const markersIn = Array.isArray(spec.markers) ? spec.markers : [];
  if (domains.length === 0 && markersIn.length === 0) {
    return <div className="ax-fig__err">结构域轨迹需要 domains 或 markers 数组</div>;
  }

  const unit = typeof spec.unit === "string" && spec.unit.trim() ? spec.unit.trim() : "aa";
  const dark = isDark();

  // geometry: horizontal scale by length; the frame scrolls if the ruler is wide.
  const padL = 46;
  const padR = 22;
  const plotW = clamp(length * 1.6, 420, 900);
  const resX = (r: number) => padL + (clamp(r, 0, length) / length) * plotW;

  const domainH = 16;
  const maxStem = 92;
  const minStem = 18;
  const rMax = 8.5;
  const rMin = 3;

  // collect valid markers FIRST (finite pos, non-negative count) so the scale is not inflated
  // by markers we then drop; maxCount derives only from what we actually draw.
  type VM = { pos: number; label: string; count: number; cls: string };
  const valid: VM[] = [];
  for (const m of markersIn) {
    // A stray null inside a JSON array is still valid JSON, and reading `.pos` off it throws
    // DURING RENDER — with no ErrorBoundary anywhere, that takes the whole app to a white screen
    // rather than just breaking this one figure. (Manhattan.tsx guards the same way.)
    if (!m || typeof m !== "object") continue;
    const pos = Number(m.pos);
    if (!isFinite(pos)) continue;
    const count = Math.max(0, Number(m.count) || 0);
    const cls = normClass(m.class);
    const label = typeof m.label === "string" && m.label.trim() ? m.label.trim() : String(pos);
    valid.push({ pos, label, count, cls });
  }
  const counts = valid.map((v) => v.count);
  const maxCount = Math.max(1, ...counts);
  const rFor = (c: number) => rMin + (clamp(c, 0, maxCount) / maxCount) * (rMax - rMin);

  // process markers: count → stem height + head radius; class → color.
  type PM = { x: number; pos: number; label: string; count: number; cls: string; color: string; stemH: number; rHead: number; row: number };
  const pm: PM[] = valid.map((v) => {
    const f = clamp(v.count, 0, maxCount) / maxCount;
    return {
      x: resX(v.pos - 0.5),
      pos: v.pos,
      label: v.label,
      count: v.count,
      cls: v.cls,
      color: catColor(CLASS_SLOT[v.cls], dark),
      stemH: minStem + f * (maxStem - minStem),
      rHead: rMin + f * (rMax - rMin),
      row: 0,
    };
  });
  const hasMarkers = pm.length > 0;

  // greedy label-row packing (labels sit in rows near the top, connected to heads by leaders).
  pm.sort((a, b) => a.x - b.x);
  const rowLastX: number[] = [];
  for (const m of pm) {
    const halfW = Math.max(11, textW(m.label, 8.5) / 2 + 4); // CJK-aware: full-width ~0.5·fs/char, Latin ~0.3·fs
    let row = 0;
    while (row < rowLastX.length && m.x - rowLastX[row] < halfW) row += 1;
    m.row = row;
    rowLastX[row] = m.x + halfW;
  }
  const maxRow = hasMarkers ? Math.max(...pm.map((m) => m.row)) : -1;

  const titleH = spec.title ? 20 : 0;
  const padTop = 8 + titleH;
  const rowBase = (r: number) => padTop + 10 + r * 11; // baseline of a label row (row 0 = topmost)
  const headTopMin = padTop + 10 + (maxRow >= 0 ? maxRow * 11 : 0) + 8;
  const backboneY = hasMarkers ? headTopMin + maxStem + rMax : padTop + 22;
  const rulerY = backboneY + domainH / 2 + 6;

  // y for a marker head center = backbone minus its stem.
  const headY = (m: PM) => backboneY - m.stemH;

  // ruler ticks (client-side nice step); explicit start (1) and end (length).
  const step = niceStep(length, 6);
  const midTicks: number[] = [];
  for (let v = step; v < length - step * 0.35; v += step) if (v > step * 0.35) midTicks.push(v);

  // present classes (fixed order) for the color legend.
  const present = CLASS_ORDER.filter((c) => pm.some((m) => m.cls === c));

  // size-legend sample counts (distinct, descending).
  const minCount = hasMarkers ? Math.min(...counts) : 0;
  const sampleSet = new Set<number>();
  if (hasMarkers) {
    sampleSet.add(maxCount);
    if (maxCount >= 4) sampleSet.add(Math.round(maxCount / 2));
    if (minCount > 0 && minCount !== maxCount) sampleSet.add(minCount);
  }
  const samples = [...sampleSet].sort((a, b) => b - a);

  // legend stack sits BELOW the ruler axis label (baseline rulerY+25) so nothing overlaps it.
  const legendY = rulerY + 40;
  const legendRow2 = legendY + 18;

  // class color-legend: running cursor sized per label so 5 classes never overflow / collide.
  const classLegend: { c: string; x: number }[] = [];
  {
    let x = padL;
    for (const c of present) {
      classLegend.push({ c, x });
      x += 22 + c.length * 5.6;
    }
  }

  const h = hasMarkers ? legendRow2 + 14 : rulerY + 32;
  const w = padL + plotW + padR;

  return (
    <figure className="ax-fig">
      <div className="ax-fig__frame" style={{ overflowX: "auto", justifyContent: "flex-start" }}>
        <svg viewBox={`0 0 ${w} ${h}`} width={w} height={h} role="img" aria-label={caption || spec.title || "mutation track"} style={{ maxWidth: "100%" }}>
          {spec.title && <text x={padL} y={16} fontSize="12" fontWeight="600" fill="var(--ax-ink)">{spec.title}</text>}

          {/* backbone bar (protein extent) */}
          <rect x={resX(0)} y={backboneY - 2.5} width={plotW} height={5} rx={2.5} fill="var(--ax-line)" />

          {/* domains */}
          {domains.map((dm, i) => {
            if (!dm || typeof dm !== "object") return null;
            const from = Number(dm.from);
            const to = Number(dm.to);
            if (!isFinite(from) || !isFinite(to) || to <= from) return null;
            const x = resX(from - 1);
            const dw = Math.max(2, resX(to) - x);
            const color = catColor(i, dark);
            const name = String(dm.name ?? "");
            const fits = dw - 6 >= textW(name, 9); // CJK-aware: full-width ~9px/char, Latin ~5.4px/char
            return (
              <g key={`dm${i}`}>
                <rect x={x} y={backboneY - domainH / 2} width={dw} height={domainH} rx={3} fill={color} stroke="var(--ax-surface)" strokeWidth={0.75}>
                  <title>{`${name} · ${from}–${to} ${unit}`}</title>
                </rect>
                {fits && (
                  <text x={x + dw / 2} y={backboneY + 3.2} fontSize="9" textAnchor="middle" fill={autoContrastText(color)} style={{ pointerEvents: "none" }}>{name}</text>
                )}
              </g>
            );
          })}

          {/* ruler axis + ticks */}
          <line x1={resX(0)} y1={rulerY} x2={resX(length)} y2={rulerY} stroke="var(--ax-line)" strokeWidth={0.75} />
          {[{ v: 1, t: "1" }, ...midTicks.map((v) => ({ v, t: String(Math.round(v)) })), { v: length, t: String(length) }].map((tk, i) => (
            <g key={`tk${i}`}>
              <line x1={resX(tk.v)} y1={rulerY} x2={resX(tk.v)} y2={rulerY + 4} stroke="var(--ax-line)" strokeWidth={0.75} />
              <text x={resX(tk.v)} y={rulerY + 13} fontSize="8.5" textAnchor="middle" fill="var(--ax-ink-faint)">{tk.t}</text>
            </g>
          ))}
          <text x={padL + plotW / 2} y={rulerY + 25} fontSize="9" textAnchor="middle" fill="var(--ax-ink-faint)">{`residue position · ${unit}`}</text>

          {/* lollipops: leader → stem → head, label in top row */}
          {pm.map((m, i) => {
            const hy = headY(m);
            return (
              <g key={`m${i}`}>
                <line x1={m.x} y1={hy - m.rHead} x2={m.x} y2={rowBase(m.row) + 2} stroke="var(--ax-line-soft)" strokeWidth={0.5} />
                <line x1={m.x} y1={backboneY} x2={m.x} y2={hy} stroke="var(--ax-line)" strokeWidth={0.75} />
                <circle cx={m.x} cy={hy} r={m.rHead} fill={m.color} stroke="var(--ax-surface)" strokeWidth={0.7}>
                  <title>{`${m.label} · ${unit}${m.pos} · ${m.cls} · n=${m.count}`}</title>
                </circle>
                <text x={m.x} y={rowBase(m.row)} fontSize="8.5" textAnchor="middle" fill="var(--ax-ink)">{m.label}</text>
              </g>
            );
          })}

          {/* color legend (variant class) — each swatch carries a text label, never color-alone */}
          {classLegend.map(({ c, x }) => (
            <g key={`lg${c}`}>
              <circle cx={x + 4} cy={legendY - 3} r={4} fill={catColor(CLASS_SLOT[c], dark)} />
              <text x={x + 13} y={legendY} fontSize="9" fill="var(--ax-ink-faint)">{c}</text>
            </g>
          ))}

          {/* size legend (cohort count) */}
          {hasMarkers && (
            <>
              <text x={padL} y={legendRow2} fontSize="9" fill="var(--ax-ink-faint)">count</text>
              {samples.map((c, i) => {
                const base = padL + 42;
                const gap = 46;
                const cx = base + i * gap;
                return (
                  <g key={`sz${i}`}>
                    <circle cx={cx} cy={legendRow2 - 3} r={rFor(c)} fill="none" stroke="var(--ax-ink-faint)" strokeWidth={0.9} />
                    <text x={cx + rMax + 4} y={legendRow2} fontSize="8.5" fill="var(--ax-ink-faint)">{c}</text>
                  </g>
                );
              })}
            </>
          )}
        </svg>
      </div>
      <figcaption className="ax-fig__cap"><span className="ax-fig__kind">结构域 · track</span> {caption || spec.title || ""}</figcaption>
    </figure>
  );
}
