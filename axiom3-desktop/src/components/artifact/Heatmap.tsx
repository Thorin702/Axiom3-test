// Native matrix heatmap: one color channel per cell for correlation / % identity / residue-contact /
// attention / z-scored marker panels (distinct from the sparse size+color dot-plot). `sequential`
// reuses the house blue ramp; `diverging` paints a signed pole over the theme surface so 0 reads as
// background and sign is instant. Optional agglomerative linkage (rowTree/colTree) reorders leaves
// and draws dendrogram brackets. Pure inline SVG, no deps.
//
// Spec (```heatmap JSON): {"title"?, "rows":[...], "cols":[...], "values":[[..row-major values[r][c]..]],
//   "scale"?:"sequential"|"diverging", "domain"?:[min,max], "colorLabel"?, "showValues"?,
//   "rowTree"?/"colTree"?: [[i,j,height],...] agglomerative merges}
type Spec = {
  title?: string;
  rows: string[];
  cols: string[];
  values: number[][];
  scale?: "sequential" | "diverging";
  domain?: [number, number];
  colorLabel?: string;
  showValues?: boolean;
  rowTree?: number[][];
  colTree?: number[][];
};

// data-viz sequential blue ramp (100 → 700), same accessor as the dot-plot.
const RAMP = ["#cde2fb", "#9ec5f4", "#6da7ec", "#3987e5", "#256abf", "#184f95", "#0d366b"];
function ramp(t: number): string {
  const x = Math.max(0, Math.min(1, t)) * (RAMP.length - 1);
  return RAMP[Math.min(Math.floor(x), RAMP.length - 1)];
}
// diverging poles (opacity carries magnitude; midpoint = theme surface underneath).
const NEG = "#2a78d6";
const POS = "#e34948";

function fmt(v: number): string {
  const a = Math.abs(v);
  if (a >= 100) return v.toFixed(0);
  if (a >= 10) return v.toFixed(1);
  return v.toFixed(2);
}

// ── agglomerative linkage → leaf order + dendrogram segments (in normalized depth/cross space) ──
type Layout = { order: number[]; segs: { d1: number; c1: number; d2: number; c2: number }[] };

function validTree(t: number[][] | undefined, nLeaves: number): t is number[][] {
  if (!Array.isArray(t) || nLeaves < 2 || t.length !== nLeaves - 1) return false;
  for (const row of t) {
    if (!Array.isArray(row) || row.length < 2) return false;
    const a = row[0];
    const b = row[1];
    if (typeof a !== "number" || typeof b !== "number") return false;
    if (!Number.isInteger(a) || !Number.isInteger(b) || a < 0 || b < 0) return false;
  }
  return true;
}

// Returns null (→ render as-is) on any malformedness; never throws.
function clusterLayout(tree: number[][], nLeaves: number): Layout | null {
  const m = tree.length; // == nLeaves - 1
  const total = nLeaves + m; // node ids: leaves 0..nLeaves-1, merges nLeaves..total-1
  const cross = new Array<number>(total).fill(NaN);
  const depth = new Array<number>(total).fill(0);
  const childA = new Array<number>(total).fill(-1);
  const childB = new Array<number>(total).fill(-1);
  const heights: number[] = [];
  let maxH = 0;
  let hValid = true;
  for (let k = 0; k < m; k++) {
    const row = tree[k];
    childA[nLeaves + k] = row[0];
    childB[nLeaves + k] = row[1];
    const hRaw = row.length > 2 ? row[2] : NaN;
    heights.push(hRaw);
    if (typeof hRaw !== "number" || !isFinite(hRaw)) hValid = false;
    else maxH = Math.max(maxH, hRaw);
  }
  if (!hValid || maxH <= 0) maxH = m; // fallback: rank merges by index

  const root = nLeaves + m - 1;
  const order: number[] = [];
  const stack: number[] = [root];
  const guard = new Set<number>();
  while (stack.length) {
    const node = stack.pop() as number;
    if (node < 0 || node >= total) return null;
    if (node < nLeaves) {
      order.push(node);
      continue;
    }
    if (guard.has(node)) return null; // cycle
    guard.add(node);
    stack.push(childB[node]); // push right first so left leaf pops first
    stack.push(childA[node]);
  }
  if (order.length !== nLeaves) return null;

  const seen = new Set<number>();
  for (let i = 0; i < order.length; i++) {
    const leaf = order[i];
    if (leaf < 0 || leaf >= nLeaves || seen.has(leaf)) return null;
    seen.add(leaf);
    cross[leaf] = i;
    depth[leaf] = 0;
  }
  // internal nodes: children always have smaller id in canonical linkage → resolve in order
  for (let k = 0; k < m; k++) {
    const id = nLeaves + k;
    const a = childA[id];
    const b = childB[id];
    if (a < 0 || a >= total || b < 0 || b >= total) return null;
    if (isNaN(cross[a]) || isNaN(cross[b])) return null;
    cross[id] = (cross[a] + cross[b]) / 2;
    depth[id] = hValid ? Math.max(0, Math.min(1, heights[k] / maxH)) : (k + 1) / m;
  }

  const segs: Layout["segs"] = [];
  for (let k = 0; k < m; k++) {
    const id = nLeaves + k;
    const a = childA[id];
    const b = childB[id];
    const dp = depth[id];
    segs.push({ d1: dp, c1: cross[a], d2: dp, c2: cross[b] }); // connector at merge depth
    segs.push({ d1: depth[a], c1: cross[a], d2: dp, c2: cross[a] }); // child a riser
    segs.push({ d1: depth[b], c1: cross[b], d2: dp, c2: cross[b] }); // child b riser
  }
  return { order, segs };
}

export function Heatmap({ spec: raw, caption }: { spec: string; caption?: string }) {
  let spec: Spec;
  try {
    spec = JSON.parse(raw.trim());
  } catch (e) {
    return <div className="ax-fig__err">热图规范不是有效 JSON · {String((e as Error)?.message ?? e)}</div>;
  }
  if (!spec || typeof spec !== "object") return <div className="ax-fig__err">热图规范为空或格式不对</div>;
  if (!Array.isArray(spec.rows) || !Array.isArray(spec.cols) || !Array.isArray(spec.values)) {
    return <div className="ax-fig__err">热图需要 rows / cols / values 三个数组</div>;
  }
  if (spec.rows.length === 0 || spec.cols.length === 0) {
    return <div className="ax-fig__err">热图的 rows / cols 不能为空</div>;
  }

  const diverging = spec.scale === "diverging";
  const nRows = spec.rows.length;
  const nCols = spec.cols.length;

  // optional clustering: reorder leaves + draw brackets (falls back to identity on any malformedness)
  const rowLayout = validTree(spec.rowTree, nRows) ? clusterLayout(spec.rowTree, nRows) : null;
  const colLayout = validTree(spec.colTree, nCols) ? clusterLayout(spec.colTree, nCols) : null;
  const rowOrder = rowLayout ? rowLayout.order : spec.rows.map((_, i) => i);
  const colOrder = colLayout ? colLayout.order : spec.cols.map((_, i) => i);

  // domain: honor an explicit domain, else derive from finite data (diverging is symmetric).
  const flat: number[] = [];
  for (const row of spec.values) {
    if (!Array.isArray(row)) continue;
    for (const x of row) if (typeof x === "number" && isFinite(x)) flat.push(x);
  }
  let dmin: number;
  let dmax: number;
  let maxAbs = 1;
  if (diverging) {
    if (Array.isArray(spec.domain) && spec.domain.length === 2) {
      maxAbs = Math.max(Math.abs(spec.domain[0]), Math.abs(spec.domain[1]));
    } else if (flat.length) {
      maxAbs = Math.max(...flat.map((v) => Math.abs(v)));
    }
    maxAbs = maxAbs || 1;
    dmin = -maxAbs;
    dmax = maxAbs;
  } else if (Array.isArray(spec.domain) && spec.domain.length === 2) {
    dmin = spec.domain[0];
    dmax = spec.domain[1];
  } else if (flat.length) {
    dmin = Math.min(...flat);
    dmax = Math.max(...flat);
  } else {
    dmin = 0;
    dmax = 1;
  }
  const span = dmax - dmin || 1;

  // geometry — square cells sized to the larger dimension so a big matrix still fits then scrolls.
  const n = Math.max(nRows, nCols);
  const cell = n <= 8 ? 32 : n <= 12 ? 26 : n <= 20 ? 20 : n <= 32 ? 14 : n <= 48 ? 10 : 7;
  const maxRowLen = Math.max(1, ...spec.rows.map((r) => String(r).length));
  const maxColLen = Math.max(1, ...spec.cols.map((c) => String(c).length));
  const labelL = Math.min(200, Math.max(38, 10 + maxRowLen * 6.6));
  const padTop = 10;
  const padB = Math.min(160, Math.max(30, 18 + maxColLen * 5));
  const padR = 82; // colorbar legend
  const dW = 42; // row dendrogram width
  const dH = 32; // col dendrogram height
  const dGap = 6;
  const dendroL = rowLayout ? dW + dGap : 0;
  const dendroT = colLayout ? dH + dGap : 0;

  const gridW = nCols * cell;
  const gridH = nRows * cell;
  const gridLeft = dendroL + labelL;
  const gridTop = padTop + dendroT;

  const gx = (c: number) => gridLeft + c * cell; // left edge of column c
  const gy = (r: number) => gridTop + r * cell; // top edge of row r
  const cxc = (c: number) => gridLeft + c * cell + cell / 2;
  const cyc = (r: number) => gridTop + r * cell + cell / 2;
  const xRowAt = (d: number) => dW * (1 - d); // row dendro: leaf (d=0) near labels, root (d=1) at edge
  const yColAt = (d: number) => padTop + dH * (1 - d); // col dendro: leaf near grid, root at top

  // legend colorbar layout
  const barX = gridLeft + gridW + padR - 68;
  const barTop = gridTop + 16;
  const swW = 13;
  const swH = 8;
  const DIV_STOPS = 13;
  const legendH = (diverging ? DIV_STOPS : RAMP.length) * swH;
  const legendBottom = barTop + legendH + 8;

  const w = gridLeft + gridW + padR;
  const h = Math.max(gridTop + gridH + padB, legendBottom + 12);

  const sep = cell >= 12; // hair of surface between cells to read as a grid
  const withVals = !!spec.showValues && nRows * nCols <= 120 && cell >= 20;
  const valFont = Math.min(11, cell * 0.36);
  const colLabelBaseY = gridTop + gridH + 12;
  const kindLabel = "热图 · heatmap";

  return (
    <figure className="ax-fig">
      <div className="ax-fig__frame" style={{ overflowX: "auto", justifyContent: "flex-start" }}>
        <svg
          viewBox={`0 0 ${w} ${h}`}
          width={w}
          height={h}
          role="img"
          aria-label={caption || spec.title || "heatmap"}
          style={{ maxWidth: "100%" }}
        >
          {/* surface under the grid so low-opacity diverging cells blend to the theme background */}
          <rect x={gridLeft} y={gridTop} width={gridW} height={gridH} fill="var(--ax-surface)" />

          {/* dendrograms (optional) */}
          {rowLayout &&
            rowLayout.segs.map((s, i) => (
              <line
                key={`rd${i}`}
                x1={xRowAt(s.d1)}
                y1={cyc(s.c1)}
                x2={xRowAt(s.d2)}
                y2={cyc(s.c2)}
                stroke="var(--ax-line)"
                strokeWidth="0.7"
              />
            ))}
          {colLayout &&
            colLayout.segs.map((s, i) => (
              <line
                key={`cd${i}`}
                x1={cxc(s.c1)}
                y1={yColAt(s.d1)}
                x2={cxc(s.c2)}
                y2={yColAt(s.d2)}
                stroke="var(--ax-line)"
                strokeWidth="0.7"
              />
            ))}

          {/* row labels (left) */}
          {spec.rows.map((_, i) => (
            <text key={`r${i}`} x={gridLeft - 6} y={cyc(i) + 3} fontSize="10" textAnchor="end" fill="var(--ax-ink-soft)">
              {spec.rows[rowOrder[i]]}
            </text>
          ))}
          {/* column labels (bottom, rotated) */}
          {spec.cols.map((_, i) => (
            <text
              key={`c${i}`}
              x={cxc(i)}
              y={colLabelBaseY}
              fontSize="9.5"
              fill="var(--ax-ink-soft)"
              transform={`rotate(55 ${cxc(i)} ${colLabelBaseY})`}
            >
              {spec.cols[colOrder[i]]}
            </text>
          ))}

          {/* cells */}
          {spec.rows.map((_, r) =>
            spec.cols.map((_, c) => {
              const srcRow = spec.values[rowOrder[r]] as number[] | undefined;
              const v = srcRow?.[colOrder[c]];
              if (v == null || !isFinite(v)) return null;
              let fill: string;
              let opacity: number;
              let t = 0;
              if (diverging) {
                fill = v < 0 ? NEG : POS;
                opacity = Math.min(1, Math.abs(v) / maxAbs);
              } else {
                t = (v - dmin) / span;
                fill = ramp(t);
                opacity = 1;
              }
              // contrast-aware on-cell label (fixed hex over a filled data cell, house convention)
              const labelColor = diverging
                ? opacity > 0.55
                  ? "#f7f8f9"
                  : "var(--ax-ink)"
                : t > 0.5
                  ? "#f7f8f9"
                  : "#123a5e";
              return (
                <g key={`${r}-${c}`}>
                  <rect
                    x={gx(c)}
                    y={gy(r)}
                    width={cell}
                    height={cell}
                    fill={fill}
                    fillOpacity={opacity}
                    stroke={sep ? "var(--ax-surface)" : "none"}
                    strokeWidth={sep ? 0.5 : 0}
                  >
                    <title>{`${spec.rows[rowOrder[r]]} · ${spec.cols[colOrder[c]]} — ${spec.colorLabel ?? "value"} ${fmt(v)}`}</title>
                  </rect>
                  {withVals && (
                    <text
                      x={cxc(c)}
                      y={cyc(r) + valFont * 0.35}
                      fontSize={valFont}
                      textAnchor="middle"
                      fill={labelColor}
                      style={{ pointerEvents: "none" }}
                    >
                      {fmt(v)}
                    </text>
                  )}
                </g>
              );
            }),
          )}

          {/* grid outline */}
          <rect x={gridLeft} y={gridTop} width={gridW} height={gridH} fill="none" stroke="var(--ax-line)" strokeWidth="0.75" />

          {/* colorbar legend (high at top for both scales) */}
          <text x={barX} y={barTop - 5} fontSize="9" fill="var(--ax-ink-faint)">
            {spec.colorLabel ?? (diverging ? "signed" : "value")}
          </text>
          {diverging ? (
            <>
              {/* surface behind the bar so the mid-swatch reads as background */}
              <rect x={barX} y={barTop} width={swW} height={legendH} fill="var(--ax-surface)" />
              {Array.from({ length: DIV_STOPS }, (_, i) => {
                const v = maxAbs * (1 - (2 * i) / (DIV_STOPS - 1));
                return (
                  <rect
                    key={`db${i}`}
                    x={barX}
                    y={barTop + i * swH}
                    width={swW}
                    height={swH}
                    fill={v < 0 ? NEG : POS}
                    fillOpacity={Math.min(1, Math.abs(v) / maxAbs)}
                  />
                );
              })}
              <rect x={barX} y={barTop} width={swW} height={legendH} fill="none" stroke="var(--ax-line)" strokeWidth="0.5" />
              <text x={barX + swW + 5} y={barTop + 6} fontSize="8.5" fill="var(--ax-ink-faint)">{fmt(maxAbs)}</text>
              <text x={barX + swW + 5} y={barTop + legendH / 2 + 3} fontSize="8.5" fill="var(--ax-ink-faint)">0</text>
              <text x={barX + swW + 5} y={barTop + legendH} fontSize="8.5" fill="var(--ax-ink-faint)">{fmt(-maxAbs)}</text>
            </>
          ) : (
            <>
              {[...RAMP].reverse().map((c, i) => (
                <rect key={`sb${i}`} x={barX} y={barTop + i * swH} width={swW} height={swH} fill={c} />
              ))}
              <rect x={barX} y={barTop} width={swW} height={legendH} fill="none" stroke="var(--ax-line)" strokeWidth="0.5" />
              <text x={barX + swW + 5} y={barTop + 6} fontSize="8.5" fill="var(--ax-ink-faint)">{fmt(dmax)}</text>
              <text x={barX + swW + 5} y={barTop + legendH} fontSize="8.5" fill="var(--ax-ink-faint)">{fmt(dmin)}</text>
            </>
          )}
        </svg>
      </div>
      <figcaption className="ax-fig__cap">
        <span className="ax-fig__kind">{kindLabel}</span> {caption || spec.title || ""}
      </figcaption>
    </figure>
  );
}
