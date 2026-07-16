// Native slopegraph (```slope JSON): a paired before/after comparison across items — each row's
// two values sit on two vertical value axes (left = earlier, right = later) joined by a segment,
// so who rose vs fell is read directly from slope + colour. Rising uses the CAT green token,
// falling the CAT red token, flat a neutral ink; both endpoints are value-labelled (no invented
// verdict — direction is just to−from). Pure inline SVG, no dependency.
//
// Spec: {"title"?, "left"?, "right"?, "unit"?,
//   "rows":[{"label":string,"from":number,"to":number}, ...]}
import { catColor, isDark } from "./theme";

type Row = { label: string; from: number; to: number };
type Spec = { title?: string; left?: string; right?: string; unit?: string; rows?: unknown };

// Direction uses the shared CVD-safe palette: rose = green slot (2), fell = vermilion slot (3),
// flat = neutral ink var. Fixed categorical (not a continuous scale).

function isFiniteNum(x: unknown): x is number {
  return typeof x === "number" && Number.isFinite(x);
}
function fmt(v: number): string {
  if (!Number.isFinite(v)) return "—";
  const a = Math.abs(v);
  let s = a >= 100 ? v.toFixed(0) : a >= 10 ? v.toFixed(1) : a >= 1 ? v.toFixed(2) : v.toFixed(3);
  if (s.includes(".")) s = s.replace(/\.?0+$/, "");
  return s;
}
// CJK / full-width glyphs render ~1em; Latin ~0.6em. Measure text width in px at a given fontSize
// so gutters fit Chinese labels (the fixed 6.4px/char undercounts CJK ~2× and clips at negative x).
function isWide(ch: string): boolean {
  const c = ch.codePointAt(0) ?? 0;
  return (
    (c >= 0x1100 && c <= 0x115f) || // Hangul Jamo
    (c >= 0x2e80 && c <= 0x303e) || // CJK radicals · Kangxi · CJK punctuation
    (c >= 0x3041 && c <= 0x33ff) || // Hiragana · Katakana · CJK symbols
    (c >= 0x3400 && c <= 0x4dbf) || // CJK ext A
    (c >= 0x4e00 && c <= 0x9fff) || // CJK unified
    (c >= 0xac00 && c <= 0xd7a3) || // Hangul syllables
    (c >= 0xf900 && c <= 0xfaff) || // CJK compat ideographs
    (c >= 0xfe30 && c <= 0xfe4f) || // CJK compat forms
    (c >= 0xff00 && c <= 0xff60) || // full-width forms
    (c >= 0xffe0 && c <= 0xffe6) || // full-width signs
    (c >= 0x20000 && c <= 0x3fffd) // CJK ext B+
  );
}
function textW(s: string, fontSize: number): number {
  let w = 0;
  for (const ch of s) w += isWide(ch) ? fontSize : fontSize * 0.6;
  return w;
}
// Truncate to a pixel budget (CJK-aware), reserving room for the trailing ellipsis.
function truncateToW(s: string, maxW: number, fontSize: number): string {
  if (textW(s, fontSize) <= maxW) return s;
  const ellW = textW("…", fontSize);
  let w = 0;
  let out = "";
  for (const ch of s) {
    const cw = isWide(ch) ? fontSize : fontSize * 0.6;
    if (w + cw + ellW > maxW) break;
    out += ch;
    w += cw;
  }
  return out.length ? out + "…" : "…";
}

// Greedy label de-collision: keep every label ≥ gap apart within [top,bottom], nudging text
// only (the dots stay on their true value position). Positions returned in input order.
function declutter(desired: number[], gap: number, top: number, bottom: number): number[] {
  const n = desired.length;
  const placed = desired.slice();
  const order = [...Array(n).keys()].sort((a, b) => desired[a] - desired[b]);
  // forward pass — push down to enforce the gap
  let prev = -Infinity;
  for (const i of order) {
    const y = Math.max(placed[i], prev + gap);
    placed[i] = y;
    prev = y;
  }
  // backward pass — clamp to bottom and pull up where needed
  let next = Infinity;
  for (let k = order.length - 1; k >= 0; k--) {
    const i = order[k];
    const y = Math.min(placed[i], next - gap, bottom);
    placed[i] = y;
    next = y;
  }
  // final top clamp
  prev = -Infinity;
  for (const i of order) {
    const y = Math.max(placed[i], prev + gap, top);
    placed[i] = y;
    prev = y;
  }
  return placed;
}

export function Slope({ spec, caption }: { spec: string; caption?: string }) {
  let d: Spec;
  try {
    d = JSON.parse(spec.trim());
  } catch (e) {
    return <div className="ax-fig__err">斜率图规范不是有效 JSON · {String((e as Error)?.message || e)}</div>;
  }
  if (!d || typeof d !== "object") return <div className="ax-fig__err">斜率图规范为空或格式不对</div>;
  if (!Array.isArray(d.rows)) return <div className="ax-fig__err">斜率图需要 rows 数组（每项含 label / from / to）</div>;

  const rows: Row[] = d.rows
    .map((r) => (r && typeof r === "object" ? (r as Record<string, unknown>) : null))
    .filter(
      (o): o is Record<string, unknown> =>
        o !== null && typeof o.label === "string" && isFiniteNum(o.from) && isFiniteNum(o.to),
    )
    .map((o) => ({ label: o.label as string, from: o.from as number, to: o.to as number }));
  if (rows.length === 0) return <div className="ax-fig__err">斜率图无有效数据行（label / from / to）</div>;

  const unit = typeof d.unit === "string" ? d.unit : "";
  const leftName = typeof d.left === "string" ? d.left : "";
  const rightName = typeof d.right === "string" ? d.right : "";
  const dark = isDark();
  const UP = catColor(2, dark);
  const DOWN = catColor(3, dark);
  const FLAT = "var(--ax-ink-faint)";
  const dirColor = (r: Row) => (r.to > r.from ? UP : r.to < r.from ? DOWN : FLAT);
  const hasFlat = rows.some((r) => r.to === r.from);

  // ── value domain (shared across both axes), padded ──
  const vals: number[] = [];
  for (const r of rows) vals.push(r.from, r.to);
  let vMin = Math.min(...vals);
  let vMax = Math.max(...vals);
  let span = vMax - vMin;
  if (span <= 0) {
    const c = vMin;
    vMin = c - 1;
    vMax = c + 1;
    span = 2;
  }
  const pad = span * 0.08;
  vMin -= pad;
  vMax += pad;

  // ── layout ── (labels + values render at fontSize 11; measure at that size, CJK-aware)
  const labelFS = 11;
  const maxLabelPx = 210; // pixel budget for a row label before it gets ellipsized
  const shown = rows.map((r) => truncateToW(r.label, maxLabelPx, labelFS));
  const maxLabelW = Math.max(0, ...shown.map((s) => textW(s, labelFS)));
  const leftValStrs = rows.map((r) => `${fmt(r.from)}${unit}`);
  const rightValStrs = rows.map((r) => `${fmt(r.to)}${unit}`);
  const maxLeftValW = Math.max(0, ...leftValStrs.map((s) => textW(s, labelFS)));
  const maxRightValW = Math.max(0, ...rightValStrs.map((s) => textW(s, labelFS)));

  const leftGutter = Math.round(10 + maxLeftValW + 6 + maxLabelW + 6);
  const rightGutter = Math.round(10 + maxRightValW + 6 + maxLabelW + 6);
  const slopeW = 240;

  const titleH = d.title ? 24 : 8;
  const headerH = 20;
  const plotTop = titleH + headerH;
  const plotH = Math.max(240, Math.min(560, rows.length * 16));
  const plotBottom = plotTop + plotH;
  const legendH = 30;
  const h = plotBottom + legendH;

  const leftX = leftGutter;
  const rightX = leftX + slopeW;
  const bodyW = rightX + rightGutter;
  const titleW = d.title ? d.title.length * 6.7 + 16 : 0;
  const w = Math.max(bodyW, Math.ceil(titleW));

  const yScale = (v: number) => plotBottom - ((v - vMin) / (vMax - vMin)) * plotH;
  const leftDotY = rows.map((r) => yScale(r.from));
  const rightDotY = rows.map((r) => yScale(r.to));
  const leftLabY = declutter(leftDotY, 13, plotTop + 6, plotBottom - 2);
  const rightLabY = declutter(rightDotY, 13, plotTop + 6, plotBottom - 2);

  // legend entries (shape + colour — never colour-alone)
  const legend: { kind: "up" | "down" | "flat"; color: string; text: string }[] = [
    { kind: "up", color: UP, text: "上升 rose" },
    { kind: "down", color: DOWN, text: "下降 fell" },
  ];
  if (hasFlat) legend.push({ kind: "flat", color: FLAT, text: "持平 flat" });
  const legendY = plotBottom + 18;
  let lx = leftX;

  const label = caption || d.title || "slopegraph";

  return (
    <figure className="ax-fig">
      <div className="ax-fig__frame" style={{ overflowX: "auto", justifyContent: "flex-start" }}>
        <svg viewBox={`0 0 ${w} ${h}`} width={w} height={h} role="img" aria-label={label} style={{ maxWidth: "100%" }}>
          {d.title && (
            <text x={8} y={16} fontSize="12" fontWeight="600" fill="var(--ax-ink)">
              {d.title}
            </text>
          )}

          {/* column headers over each value axis */}
          {leftName && (
            <text x={leftX} y={plotTop - 7} fontSize="10.5" fontWeight="600" textAnchor="middle" fill="var(--ax-ink-soft)">
              {leftName}
            </text>
          )}
          {rightName && (
            <text x={rightX} y={plotTop - 7} fontSize="10.5" fontWeight="600" textAnchor="middle" fill="var(--ax-ink-soft)">
              {rightName}
            </text>
          )}

          {/* two vertical value axes */}
          <line x1={leftX} y1={plotTop} x2={leftX} y2={plotBottom} stroke="var(--ax-line)" strokeWidth="1" />
          <line x1={rightX} y1={plotTop} x2={rightX} y2={plotBottom} stroke="var(--ax-line)" strokeWidth="1" />

          {/* connecting segments (drawn first, under the endpoint dots) */}
          {rows.map((r, i) => {
            const c = dirColor(r);
            const delta = r.to - r.from;
            const sign = delta > 0 ? "+" : "";
            return (
              <line
                key={`seg${i}`}
                x1={leftX}
                y1={leftDotY[i]}
                x2={rightX}
                y2={rightDotY[i]}
                stroke={c}
                strokeWidth="2"
                strokeOpacity="0.9"
              >
                <title>{`${r.label}: ${fmt(r.from)}${unit} → ${fmt(r.to)}${unit} (Δ ${sign}${fmt(delta)}${unit})`}</title>
              </line>
            );
          })}

          {/* endpoint dots */}
          {rows.map((r, i) => {
            const c = dirColor(r);
            return (
              <g key={`dot${i}`}>
                <circle cx={leftX} cy={leftDotY[i]} r={3.5} fill={c} stroke="var(--ax-surface)" strokeWidth="0.8">
                  <title>{`${r.label} · ${leftName || "left"}: ${fmt(r.from)}${unit}`}</title>
                </circle>
                <circle cx={rightX} cy={rightDotY[i]} r={3.5} fill={c} stroke="var(--ax-surface)" strokeWidth="0.8">
                  <title>{`${r.label} · ${rightName || "right"}: ${fmt(r.to)}${unit}`}</title>
                </circle>
              </g>
            );
          })}

          {/* left-side labels: name (outer) + value (inner, hugging the axis) */}
          {rows.map((r, i) => {
            const vy = leftLabY[i];
            const dy = leftDotY[i];
            const valW = textW(leftValStrs[i], labelFS);
            return (
              <g key={`ll${i}`}>
                {Math.abs(vy - dy) > 7 && (
                  <line x1={leftX - 4} y1={dy} x2={leftX - 8} y2={vy} stroke="var(--ax-line-soft)" strokeWidth="0.5" />
                )}
                <text
                  x={leftX - 8}
                  y={vy + 3.5}
                  fontSize="11"
                  textAnchor="end"
                  fill="var(--ax-ink)"
                  style={{ fontVariantNumeric: "tabular-nums" }}
                >
                  {leftValStrs[i]}
                </text>
                <text x={leftX - 8 - valW - 6} y={vy + 3.5} fontSize="11" textAnchor="end" fill="var(--ax-ink-soft)">
                  {shown[i]}
                  {shown[i] !== r.label && <title>{r.label}</title>}
                </text>
              </g>
            );
          })}

          {/* right-side labels: value (inner, hugging the axis) + name (outer) */}
          {rows.map((r, i) => {
            const vy = rightLabY[i];
            const dy = rightDotY[i];
            const valW = textW(rightValStrs[i], labelFS);
            return (
              <g key={`rl${i}`}>
                {Math.abs(vy - dy) > 7 && (
                  <line x1={rightX + 4} y1={dy} x2={rightX + 8} y2={vy} stroke="var(--ax-line-soft)" strokeWidth="0.5" />
                )}
                <text
                  x={rightX + 8}
                  y={vy + 3.5}
                  fontSize="11"
                  textAnchor="start"
                  fill="var(--ax-ink)"
                  style={{ fontVariantNumeric: "tabular-nums" }}
                >
                  {rightValStrs[i]}
                </text>
                <text x={rightX + 8 + valW + 6} y={vy + 3.5} fontSize="11" textAnchor="start" fill="var(--ax-ink-soft)">
                  {shown[i]}
                  {shown[i] !== r.label && <title>{r.label}</title>}
                </text>
              </g>
            );
          })}

          {/* direction legend (shape + colour + label) */}
          {legend.map((it, i) => {
            const sx = lx;
            const gx = sx + 5;
            const gy = legendY - 3;
            const s = 4;
            const tx = sx + 14;
            lx = tx + it.text.length * 6.2 + 18;
            return (
              <g key={`lg${i}`}>
                {it.kind === "up" ? (
                  <polygon points={`${gx},${gy - s} ${gx - s},${gy + s} ${gx + s},${gy + s}`} fill={it.color} />
                ) : it.kind === "down" ? (
                  <polygon points={`${gx},${gy + s} ${gx - s},${gy - s} ${gx + s},${gy - s}`} fill={it.color} />
                ) : (
                  <line x1={gx - s} y1={gy} x2={gx + s} y2={gy} stroke={it.color} strokeWidth="2" />
                )}
                <text x={tx} y={legendY} fontSize="9.5" fill="var(--ax-ink-faint)">
                  {it.text}
                </text>
              </g>
            );
          })}
        </svg>
      </div>
      <figcaption className="ax-fig__cap">
        <span className="ax-fig__kind">斜率图 · slope</span> {caption || d.title || ""}
      </figcaption>
    </figure>
  );
}
