// Native gene-expression dot-plot (the Tabula Sapiens figure): rows (cell types) × columns
// (genes/features), dot SIZE = fraction of cells, dot COLOR = mean expression. Pure SVG, no
// dependency. Color uses the data-viz sequential blue ramp (light→dark = low→high); size and
// color are two encodings of two measures, each with its own legend (never color-alone).
//
// Spec (```dotplot JSON): {"title"?, "rows":[...], "cols":[...],
//   "cells":[{"r":<rowIdx>,"c":<colIdx>,"size":0..1,"color":0..1}], "sizeLabel"?, "colorLabel"?}
type Spec = {
  title?: string;
  rows: string[];
  cols: string[];
  cells: { r: number; c: number; size: number; color: number }[];
  sizeLabel?: string;
  colorLabel?: string;
};

// data-viz sequential blue ramp (100 → 700).
const RAMP = ["#cde2fb", "#9ec5f4", "#6da7ec", "#3987e5", "#256abf", "#184f95", "#0d366b"];
function ramp(t: number): string {
  const x = Math.max(0, Math.min(1, t)) * (RAMP.length - 1);
  const i = Math.floor(x);
  return RAMP[Math.min(i, RAMP.length - 1)];
}

export function DotPlot({ spec: raw, caption }: { spec: string; caption?: string }) {
  let spec: Spec;
  try {
    spec = JSON.parse(raw.trim());
  } catch {
    return <div className="ax-fig__err">点图规范不是有效 JSON</div>;
  }
  if (!Array.isArray(spec.rows) || !Array.isArray(spec.cols) || !Array.isArray(spec.cells)) {
    return <div className="ax-fig__err">点图需要 rows / cols / cells</div>;
  }

  const padL = 96; // row labels
  const padB = 70; // col labels (rotated)
  const padT = spec.title ? 26 : 10;
  const padR = 92; // legends
  const cell = 24;
  const rMax = 9;
  const w = padL + spec.cols.length * cell + padR;
  const h = padT + spec.rows.length * cell + padB;
  const cx = (c: number) => padL + c * cell + cell / 2;
  const cy = (r: number) => padT + r * cell + cell / 2;

  return (
    <figure className="ax-fig">
      <div className="ax-fig__frame" style={{ overflowX: "auto", justifyContent: "flex-start" }}>
        <svg viewBox={`0 0 ${w} ${h}`} width={w} height={h} role="img" aria-label={caption || "dot plot"} style={{ maxWidth: "100%" }}>
          {spec.title && <text x={padL} y={16} fontSize="12" fontWeight="600" fill="var(--ax-ink)">{spec.title}</text>}
          {/* row labels */}
          {spec.rows.map((r, i) => (
            <text key={`r${i}`} x={padL - 8} y={cy(i) + 3} fontSize="10" textAnchor="end" fill="var(--ax-ink-soft)">{r}</text>
          ))}
          {/* column labels (rotated) */}
          {spec.cols.map((c, i) => (
            <text key={`c${i}`} x={cx(i)} y={padT + spec.rows.length * cell + 10} fontSize="9.5" fill="var(--ax-ink-soft)"
              transform={`rotate(55 ${cx(i)} ${padT + spec.rows.length * cell + 10})`}>{c}</text>
          ))}
          {/* faint grid */}
          {spec.rows.map((_, i) => (
            <line key={`gl${i}`} x1={padL} y1={cy(i)} x2={padL + spec.cols.length * cell} y2={cy(i)} stroke="var(--ax-line-soft)" strokeWidth="0.5" />
          ))}
          {/* dots */}
          {spec.cells.map((d, i) => {
            if (d.r < 0 || d.r >= spec.rows.length || d.c < 0 || d.c >= spec.cols.length) return null;
            const r = 1.5 + Math.max(0, Math.min(1, d.size)) * rMax;
            return (
              <circle key={i} cx={cx(d.c)} cy={cy(d.r)} r={r} fill={ramp(d.color)} stroke="var(--ax-surface)" strokeWidth="0.5">
                <title>{`${spec.rows[d.r]} · ${spec.cols[d.c]} — ${spec.sizeLabel ?? "fraction"} ${(d.size * 100).toFixed(0)}% · ${spec.colorLabel ?? "expr"} ${d.color.toFixed(2)}`}</title>
              </circle>
            );
          })}
          {/* size legend */}
          <text x={w - padR + 6} y={padT + 4} fontSize="9" fill="var(--ax-ink-faint)">{spec.sizeLabel ?? "fraction"}</text>
          {[0.25, 0.6, 1].map((s, i) => (
            <g key={`sz${i}`}>
              <circle cx={w - padR + 14} cy={padT + 20 + i * 18} r={1.5 + s * rMax} fill="none" stroke="var(--ax-ink-faint)" />
              <text x={w - padR + 30} y={padT + 23 + i * 18} fontSize="8.5" fill="var(--ax-ink-faint)">{Math.round(s * 100)}%</text>
            </g>
          ))}
          {/* color legend */}
          <text x={w - padR + 6} y={padT + 92} fontSize="9" fill="var(--ax-ink-faint)">{spec.colorLabel ?? "expr"}</text>
          {RAMP.map((c, i) => (
            <rect key={`cl${i}`} x={w - padR + 14} y={padT + 100 + i * 7} width="12" height="7" fill={c} />
          ))}
          <text x={w - padR + 30} y={padT + 106} fontSize="8" fill="var(--ax-ink-faint)">low</text>
          <text x={w - padR + 30} y={padT + 100 + RAMP.length * 7} fontSize="8" fill="var(--ax-ink-faint)">high</text>
        </svg>
      </div>
      {caption && <figcaption className="ax-fig__cap"><span className="ax-fig__kind">点图 · dot-plot</span> {caption}</figcaption>}
    </figure>
  );
}
