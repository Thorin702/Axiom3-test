// Native classifier confusion matrix: rows = ACTUAL class, cols = PREDICTED class. The agent
// emits RAW integer counts only; everything neutral (per-cell %, overall accuracy) is derived
// client-side. Color = sequential blue ramp mapped by `normalize` ("row"/"col" fraction, or raw
// magnitude for "none"); the correct-prediction diagonal gets a structural accent outline.
//
// Spec (```confusion JSON): {"title"?, "labels":[...], "matrix":[[...],...],
//   "normalize"?: "row"|"col"|"none"}  // matrix is a labels.length² square of counts.
type Norm = "row" | "col" | "none";
type Spec = {
  title?: string;
  labels: string[];
  matrix: number[][];
  normalize?: Norm;
};

// data-viz sequential blue ramp (100 → 700); light→dark = low→high.
const RAMP = ["#cde2fb", "#9ec5f4", "#6da7ec", "#3987e5", "#256abf", "#184f95", "#0d366b"];
function ramp(t: number): string {
  const x = Math.max(0, Math.min(1, t)) * (RAMP.length - 1);
  return RAMP[Math.min(Math.floor(x), RAMP.length - 1)];
}
// contrast-aware ink for text drawn ON a filled data cell (fixed colors: the cell fill is a fixed
// data color in either theme, so the label's contrast color must be fixed too, not a theme var).
function cellInk(t: number): string {
  const idx = Math.min(Math.floor(Math.max(0, Math.min(1, t)) * (RAMP.length - 1)), RAMP.length - 1);
  return idx >= 3 ? "#f2f7ff" : "#12233b";
}

export function Confusion({ spec: raw, caption }: { spec: string; caption?: string }) {
  let spec: Spec;
  try {
    spec = JSON.parse(raw.trim());
  } catch (e) {
    return <div className="ax-fig__err">混淆矩阵规范不是有效 JSON · {(e as Error).message}</div>;
  }
  if (!spec || typeof spec !== "object") return <div className="ax-fig__err">混淆矩阵规范为空或格式不对</div>;
  if (!Array.isArray(spec.labels) || spec.labels.length === 0 || !Array.isArray(spec.matrix)) {
    return <div className="ax-fig__err">混淆矩阵需要 labels 与 matrix</div>;
  }
  const labels = spec.labels.map((l) => String(l));
  const K = labels.length;
  if (spec.matrix.length !== K || spec.matrix.some((row) => !Array.isArray(row) || row.length !== K)) {
    return <div className="ax-fig__err">matrix 必须是 {K}×{K} 的整数计数方阵</div>;
  }

  // Coerce to finite numbers; derive every neutral quantity locally (nothing fabricated).
  const M = spec.matrix.map((row) => row.map((v) => (Number.isFinite(v) ? Number(v) : 0)));
  const rowSums = M.map((row) => row.reduce((a, b) => a + b, 0));
  const colSums = labels.map((_, c) => M.reduce((a, row) => a + row[c], 0));
  const total = rowSums.reduce((a, b) => a + b, 0);
  const diag = M.reduce((a, row, i) => a + row[i], 0);
  const maxCount = Math.max(1, ...M.flat());
  const accuracy = total ? diag / total : 0;
  const mode: Norm = spec.normalize === "row" || spec.normalize === "col" ? spec.normalize : "none";

  // color intensity (normalized per mode); frac = the % shown/tooltip'd.
  const colorT = (r: number, c: number): number => {
    const v = M[r][c];
    if (mode === "col") return colSums[c] ? v / colSums[c] : 0;
    if (mode === "row") return rowSums[r] ? v / rowSums[r] : 0;
    return v / maxCount;
  };
  const frac = (r: number, c: number): number => {
    const v = M[r][c];
    if (mode === "col") return colSums[c] ? v / colSums[c] : 0;
    if (mode === "row") return rowSums[r] ? v / rowSums[r] : 0;
    return total ? v / total : 0;
  };
  const shareOf = mode === "col" ? "该预测列" : mode === "row" ? "该真实行" : "全体";

  // layout — left/bottom padding adapt to the longest label so class names never clip off-canvas
  // (rows and cols share the same label set). ~6px/char at these font sizes; rotated 50° cols.
  const maxLab = Math.max(1, ...labels.map((l) => l.length));
  const cell = 48;
  const padL = Math.max(104, maxLab * 6 + 34); // actual-axis title + row labels
  const padT = spec.title ? 48 : 30; // title + "predicted" header
  const padB = Math.max(70, Math.round(maxLab * 6 * 0.77 + 26)); // rotated predicted labels
  const padR = 100; // colorbar legend
  const gridW = K * cell;
  const gridH = K * cell;
  const gridRight = padL + gridW;
  const gridBottom = padT + gridH;
  const w = padL + gridW + padR;
  const h = padT + gridH + padB;
  const cx = (c: number) => padL + c * cell + cell / 2;
  const cy = (r: number) => padT + r * cell + cell / 2;
  const legendX = gridRight + 16;
  const cbLabel = mode === "none" ? "count" : mode === "col" ? "% pred" : "% actual";
  const cbMin = mode === "none" ? "0" : "0%";
  const cbMax = mode === "none" ? String(maxCount) : "100%";

  return (
    <figure className="ax-fig">
      <div className="ax-fig__frame" style={{ overflowX: "auto", justifyContent: "flex-start" }}>
        <svg
          viewBox={`0 0 ${w} ${h}`}
          width={w}
          height={h}
          role="img"
          aria-label={caption || spec.title || "confusion matrix"}
          style={{ maxWidth: "100%" }}
        >
          {spec.title && (
            <text x={12} y={16} fontSize="12" fontWeight="600" fill="var(--ax-ink)">
              {spec.title}
            </text>
          )}
          {/* predicted axis header (top) */}
          <text x={padL + gridW / 2} y={padT - 10} fontSize="9.5" textAnchor="middle" fill="var(--ax-ink-soft)">
            预测 predicted →
          </text>
          {/* actual axis title (left, rotated) */}
          <text
            x={14}
            y={padT + gridH / 2}
            fontSize="9.5"
            textAnchor="middle"
            fill="var(--ax-ink-soft)"
            transform={`rotate(-90 14 ${padT + gridH / 2})`}
          >
            真实 actual
          </text>
          {/* actual (row) labels */}
          {labels.map((lab, r) => (
            <text key={`rl${r}`} x={padL - 8} y={cy(r) + 3.5} fontSize="10" textAnchor="end" fill="var(--ax-ink-soft)">
              {lab}
            </text>
          ))}
          {/* predicted (col) labels, rotated */}
          {labels.map((lab, c) => (
            <text
              key={`cl${c}`}
              x={cx(c)}
              y={gridBottom + 11}
              fontSize="9.5"
              fill="var(--ax-ink-soft)"
              transform={`rotate(50 ${cx(c)} ${gridBottom + 11})`}
            >
              {lab}
            </text>
          ))}
          {/* cells */}
          {M.map((row, r) =>
            row.map((v, c) => {
              const t = colorT(r, c);
              const ink = cellInk(t);
              const pct = frac(r, c) * 100;
              return (
                <g key={`cell${r}-${c}`}>
                  <rect
                    x={padL + c * cell}
                    y={padT + r * cell}
                    width={cell}
                    height={cell}
                    rx={1.5}
                    fill={ramp(t)}
                    stroke="var(--ax-surface)"
                    strokeWidth="1"
                  >
                    <title>{`真实 ${labels[r]} → 预测 ${labels[c]} · ${v} 例 (${pct.toFixed(1)}% / ${shareOf})`}</title>
                  </rect>
                  {v > 0 ? (
                    <>
                      <text x={cx(c)} y={cy(r) - 1} fontSize="12.5" fontWeight="600" textAnchor="middle" fill={ink}>
                        {v}
                      </text>
                      <text x={cx(c)} y={cy(r) + 11} fontSize="8" textAnchor="middle" fill={ink} opacity="0.72">
                        {pct.toFixed(1)}%
                      </text>
                    </>
                  ) : (
                    <text x={cx(c)} y={cy(r) + 3.5} fontSize="11" textAnchor="middle" fill={ink} opacity="0.42">
                      0
                    </text>
                  )}
                </g>
              );
            }),
          )}
          {/* correct-prediction diagonal emphasis (structural, not a verdict) */}
          {labels.map((_, i) => (
            <rect
              key={`dg${i}`}
              x={padL + i * cell + 0.75}
              y={padT + i * cell + 0.75}
              width={cell - 1.5}
              height={cell - 1.5}
              rx={2}
              fill="none"
              stroke="var(--ax-accent)"
              strokeWidth="1.5"
            />
          ))}
          {/* outer grid border */}
          <rect x={padL} y={padT} width={gridW} height={gridH} rx={2} fill="none" stroke="var(--ax-line)" strokeWidth="0.75" />
          {/* colorbar legend */}
          <text x={legendX} y={padT + 4} fontSize="9" fill="var(--ax-ink-faint)">
            {cbLabel}
          </text>
          {RAMP.map((c, i) => (
            <rect key={`cb${i}`} x={legendX} y={padT + 14 + i * 8} width="14" height="8" fill={c} />
          ))}
          <text x={legendX + 18} y={padT + 21} fontSize="8" fill="var(--ax-ink-faint)">
            {cbMin}
          </text>
          <text x={legendX + 18} y={padT + 14 + RAMP.length * 8} fontSize="8" fill="var(--ax-ink-faint)">
            {cbMax}
          </text>
        </svg>
      </div>
      <figcaption className="ax-fig__cap">
        <span className="ax-fig__kind">混淆矩阵 · confusion</span>
        {(caption || spec.title) && <span>{caption || spec.title}</span>}
        <span className="ax-fig__hint">
          准确率 accuracy {(accuracy * 100).toFixed(1)}% · n={total}
        </span>
      </figcaption>
    </figure>
  );
}
