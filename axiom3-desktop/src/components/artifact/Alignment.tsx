// Native multiple-sequence-alignment figure: a ```msa JSON block → a monospace grid of
// residues (rows = sequences, columns = alignment positions), colored by a built-in Clustal
// chemistry map (protein) / nucleotide map (DNA) or by identity to the column consensus.
// The consensus row, the per-column conservation bar, and the '*/:/.' markers are all COMPUTED
// here from the raw sequences (never agent-supplied), so the figure surfaces evidence, not claims.
//
// Spec: {"title"?, "type":"dna"|"protein", "color":"clustal"|"identity", "start"?,
//        "seqs":[{"id":string,"seq":string}]}  — seqs are pre-aligned equal-length strings, '-' = gap.

import { sequential, autoContrastText } from "./theme";

type Seq = { id: string; seq: string };
type Spec = { title?: string; type?: string; color?: string; start?: number; seqs?: Seq[] };

// data-viz sequential ramp — used for conservation intensity + identity fill (unsigned magnitude,
// fraction 0→1, so it maps to the shared viridis `sequential(t)`). Fixed swatch stops for the legend.
const IDENTITY_STOPS = [0, 1, 2, 3, 4, 5, 6].map((i) => sequential(i / 6));

// Clustal-style amino-acid chemistry groups (fixed, domain-standard pastels).
const AA_GROUP: Record<string, string> = {
  A: "hydro", I: "hydro", L: "hydro", M: "hydro", F: "hydro", W: "hydro", V: "hydro",
  K: "pos", R: "pos",
  E: "neg", D: "neg",
  N: "polar", Q: "polar", S: "polar", T: "polar",
  C: "cys", G: "gly", P: "pro", H: "arom", Y: "arom",
};
const GROUP_COLOR: Record<string, string> = {
  hydro: "#80a0f0", pos: "#f05555", neg: "#c561c5", polar: "#3ec93e",
  cys: "#f0a0a0", gly: "#f0a848", pro: "#d8d020", arom: "#3bb6b6",
};
// Nucleotide chemistry map (DNA/RNA).
const NT_COLOR: Record<string, string> = {
  A: "#5cb85c", C: "#4a7fe0", G: "#f0a53a", T: "#e05a4a", U: "#e05a4a",
};

// CJK / full-width codepoints render ~1em wide; Latin/digits ~0.55em. Used to fit right-anchored
// sequence ids inside the left gutter by estimated rendered width (a char-count cap over-clips CJK).
const WIDE = /[ᄀ-ᅟ⺀-〾ぁ-㏿㐀-䶿一-鿿ꀀ-꓏가-힣豈-﫿︰-﹏＀-｠￠-￦]/;
function fitId(id: string, fs: number, budget: number): string {
  const cw = (ch: string) => (WIDE.test(ch) ? fs : fs * 0.55);
  let total = 0;
  for (const ch of id) total += cw(ch);
  if (total <= budget) return id;
  const ell = fs * 0.55; // width reserved for the trailing "…"
  let acc = 0, out = "";
  for (const ch of id) {
    if (acc + cw(ch) + ell > budget) break;
    acc += cw(ch);
    out += ch;
  }
  return out + "…";
}

const MAX_COLS = 160;
const MAX_ROWS = 40;

export function Alignment({ spec: raw, caption }: { spec: string; caption?: string }) {
  let spec: Spec;
  try {
    spec = JSON.parse(raw.trim());
  } catch (e) {
    return <div className="ax-fig__err">序列比对规范不是有效 JSON · {String(e instanceof Error ? e.message : e)}</div>;
  }
  if (!spec || typeof spec !== "object") return <div className="ax-fig__err">序列比对规范为空或格式不对</div>;
  if (!Array.isArray(spec.seqs) || spec.seqs.length === 0) {
    return <div className="ax-fig__err">序列比对需要 seqs 序列数组</div>;
  }
  // Only `seq` is required; a missing/non-string id gets a synthesized placeholder so the row is
  // still drawn and counted (dropping it would silently skew the computed consensus/conservation).
  const clean = spec.seqs
    .filter((s) => s && typeof s.seq === "string")
    .map((s, i) => ({ id: typeof s.id === "string" ? s.id : `seq${i + 1}`, seq: s.seq }));
  if (clean.length === 0) return <div className="ax-fig__err">seqs 需要 {"{ seq }"} 字段</div>;

  const maxLen = clean.reduce((m, s) => Math.max(m, s.seq.length), 0);
  if (maxLen === 0) return <div className="ax-fig__err">序列为空,无可比对的位点</div>;

  const type = spec.type === "dna" ? "dna" : "protein";
  const colorMode = spec.color === "identity" ? "identity" : "clustal";
  const start = typeof spec.start === "number" && Number.isFinite(spec.start) ? Math.trunc(spec.start) : 1;

  const L = Math.min(maxLen, MAX_COLS);
  const truncCols = maxLen > MAX_COLS;
  const truncRows = clean.length > MAX_ROWS;
  const rows = clean.slice(0, MAX_ROWS).map((s) => ({ id: s.id, seqU: s.seq.toUpperCase() }));

  // per-column consensus + conservation (computed from the shown sequences).
  type Col = { res: string; frac: number; marker: string };
  const cols: Col[] = [];
  for (let c = 0; c < L; c++) {
    const counts: Record<string, number> = {};
    for (const r of rows) {
      const ch = r.seqU[c] ?? "-";
      if (ch === "-" || ch === ".") continue;
      counts[ch] = (counts[ch] ?? 0) + 1;
    }
    let res = "-", best = 0;
    for (const k in counts) if (counts[k] > best) { best = counts[k]; res = k; }
    const frac = rows.length ? best / rows.length : 0;
    let marker = " ";
    if (res !== "-") marker = best === rows.length ? "*" : frac >= 0.8 ? ":" : frac >= 0.5 ? "." : " ";
    cols.push({ res, frac, marker });
  }

  function cellFill(ch: string, c: number): string | null {
    if (ch === "-" || ch === ".") return null;
    if (colorMode === "identity") {
      const col = cols[c];
      return col.res !== "-" && ch === col.res ? sequential(col.frac) : null;
    }
    if (type === "dna") return NT_COLOR[ch] ?? null;
    const g = AA_GROUP[ch];
    return g ? GROUP_COLOR[g] : null;
  }

  // geometry
  const cellW = 13, rowH = 15, padL = 108, padR = 16;
  const titleH = spec.title ? 22 : 4;
  const rulerTop = titleH;
  const rulerBaseY = rulerTop + 8;
  const consBarTop = rulerTop + 13;
  const consBarH = 24;
  const consBarBase = consBarTop + consBarH;
  const gridTop = consBarBase + 6;
  const gridBottom = gridTop + rows.length * rowH;
  const consRowTop = gridBottom + 5;
  const markerRowTop = consRowTop + rowH;
  const legendTop = markerRowTop + rowH + 12;
  const gridRight = padL + L * cellW;
  const cx = (c: number) => padL + c * cellW + cellW / 2;

  // ruler tick columns
  const step = L <= 30 ? 5 : L <= 80 ? 10 : 20;
  const ticks: number[] = [];
  for (let c = 0; c < L; c++) if (c === 0 || (start + c) % step === 0) ticks.push(c);

  // legend layout — wrap chemistry swatches inside a target width; identity uses a ramp bar.
  const wrapW = 360;
  const swatchItems: { color: string; label: string }[] =
    colorMode === "identity"
      ? []
      : type === "dna"
        ? ([["#5cb85c", "A"], ["#4a7fe0", "C"], ["#f0a53a", "G"], ["#e05a4a", "T / U"]] as [string, string][]).map(([color, label]) => ({ color, label }))
        : ([
            ["#80a0f0", "Hydrophobic"], ["#f05555", "Positive"], ["#c561c5", "Negative"], ["#3ec93e", "Polar"],
            ["#f0a0a0", "Cysteine"], ["#f0a848", "Glycine"], ["#d8d020", "Proline"], ["#3bb6b6", "Aromatic"],
          ] as [string, string][]).map(([color, label]) => ({ color, label }));
  let lx = 0, ly = 0, lineMax = 0;
  const placed = swatchItems.map((it) => {
    const iw = 36 + it.label.length * 5.7;
    if (lx > 0 && lx + iw > wrapW) { lx = 0; ly += 16; }
    const p = { ...it, x: lx, y: ly };
    lx += iw; lineMax = Math.max(lineMax, lx);
    return p;
  });
  const legendBlockW = colorMode === "identity" ? 168 : lineMax;
  const legendH = colorMode === "identity" ? 12 : (ly / 16 + 1) * 16 - 4;

  const kindLabel = colorMode === "identity" ? "identity" : type === "dna" ? "nucleotide" : "Clustal";
  const notes: string[] = [];
  if (truncRows) notes.push(`显示前 ${MAX_ROWS} / ${clean.length} 条序列`);
  if (truncCols) notes.push(`显示前 ${MAX_COLS} / ${maxLen} 个位点`);

  // Right edge must clear the widest of: grid, legend, and the two left-anchored <text> rows
  // (the title and the truncation note) — otherwise a long title on a short alignment gets
  // clipped at the SVG's own right edge. Estimated glyph widths, over-shoot is harmless.
  const titleW = spec.title ? 2 + spec.title.length * 9 + 6 : 0;
  const noteW = notes.length ? padL + notes.join(" · ").length * 9 : 0;
  const w = Math.ceil(Math.max(gridRight, padL + legendBlockW, titleW, noteW) + padR);
  const noteTop = legendTop + legendH + 6;
  const h = truncCols || truncRows ? noteTop + 6 : legendTop + legendH + 8;

  return (
    <figure className="ax-fig">
      <div className="ax-fig__frame" style={{ overflowX: "auto", justifyContent: "flex-start" }}>
        <svg viewBox={`0 0 ${w} ${h}`} width={w} height={h} role="img"
          aria-label={caption || spec.title || "multiple sequence alignment"} style={{ maxWidth: "none" }}>
          {spec.title && <text x={2} y={14} fontSize="12" fontWeight="600" fill="var(--ax-ink)">{spec.title}</text>}

          {/* coordinate ruler (alignment columns, numbered from `start`) */}
          {ticks.map((c) => (
            <g key={`t${c}`}>
              <line x1={cx(c)} y1={rulerBaseY + 2} x2={cx(c)} y2={consBarTop} stroke="var(--ax-line-soft)" strokeWidth="0.5" />
              <text x={cx(c)} y={rulerBaseY} fontSize="8" textAnchor="middle" fill="var(--ax-ink-faint)">{start + c}</text>
            </g>
          ))}

          {/* per-column conservation bar (computed) */}
          <text x={padL - 8} y={consBarTop + consBarH * 0.62} fontSize="8.5" textAnchor="end" fill="var(--ax-ink-faint)">保守度</text>
          <line x1={padL} y1={consBarBase + 0.5} x2={gridRight} y2={consBarBase + 0.5} stroke="var(--ax-line-soft)" strokeWidth="0.5" />
          {cols.map((col, c) => {
            const bh = col.frac * (consBarH - 2);
            if (bh <= 0) return null;
            return (
              <rect key={`b${c}`} x={padL + c * cellW + 1} y={consBarBase - bh} width={cellW - 2} height={bh} fill={sequential(col.frac)}>
                <title>{`位置 ${start + c} · 共识 ${col.res} · 保守度 ${(col.frac * 100).toFixed(0)}%`}</title>
              </rect>
            );
          })}

          {/* sequence rows */}
          {rows.map((r, i) => (
            <text key={`id${i}`} x={padL - 8} y={gridTop + i * rowH + rowH * 0.72} fontSize="9" textAnchor="end" fill="var(--ax-ink-soft)">
              {fitId(r.id, 9, padL - 16)}
              <title>{r.id}</title>
            </text>
          ))}
          {rows.map((r, i) =>
            cols.map((_, c) => {
              const ch = r.seqU[c] ?? "-";
              const fill = cellFill(ch, c);
              const txt = fill ? autoContrastText(fill) : ch === "-" || ch === "." ? "var(--ax-ink-faint)" : "var(--ax-ink)";
              return (
                <g key={`${i}-${c}`}>
                  {fill && <rect x={padL + c * cellW + 0.5} y={gridTop + i * rowH + 0.75} width={cellW - 1} height={rowH - 1.5} rx="1.5" fill={fill} />}
                  <text x={cx(c)} y={gridTop + i * rowH + rowH * 0.72} fontSize="11" textAnchor="middle"
                    fontFamily="var(--ax-mono, ui-monospace, Menlo, monospace)" fill={txt}>
                    {ch}
                    <title>{`${r.id} · 位置 ${start + c} · ${ch === "-" || ch === "." ? "gap" : ch}`}</title>
                  </text>
                </g>
              );
            })
          )}

          {/* divider + computed consensus row + conservation markers */}
          <line x1={padL} y1={gridBottom + 2} x2={gridRight} y2={gridBottom + 2} stroke="var(--ax-line)" strokeWidth="0.5" />
          <text x={padL - 8} y={consRowTop + rowH * 0.72} fontSize="8.5" textAnchor="end" fill="var(--ax-ink-faint)">共识</text>
          {cols.map((col, c) => (
            <text key={`cs${c}`} x={cx(c)} y={consRowTop + rowH * 0.72} fontSize="11" textAnchor="middle" fontWeight="600"
              fontFamily="var(--ax-mono, ui-monospace, Menlo, monospace)" fill={col.res === "-" ? "var(--ax-ink-faint)" : "var(--ax-ink)"}>
              {col.res}
            </text>
          ))}
          {cols.map((col, c) =>
            col.marker === " " ? null : (
              <text key={`mk${c}`} x={cx(c)} y={markerRowTop + rowH * 0.72} fontSize="11" textAnchor="middle"
                fontFamily="var(--ax-mono, ui-monospace, Menlo, monospace)" fill={col.marker === "*" ? "var(--ax-accent)" : "var(--ax-ink-faint)"}>
                {col.marker}
              </text>
            )
          )}

          {/* legend */}
          {colorMode === "identity" ? (
            <g>
              <text x={padL} y={legendTop + 9} fontSize="9" fill="var(--ax-ink-soft)">与共识一致</text>
              <text x={padL + 62} y={legendTop + 9} fontSize="9" textAnchor="end" fill="var(--ax-ink-faint)">低</text>
              {IDENTITY_STOPS.map((c, i) => (
                <rect key={`lr${i}`} x={padL + 66 + i * 12} y={legendTop} width="12" height="10" fill={c} />
              ))}
              <text x={padL + 66 + IDENTITY_STOPS.length * 12 + 4} y={legendTop + 9} fontSize="9" fill="var(--ax-ink-faint)">高</text>
            </g>
          ) : (
            placed.map((p, i) => (
              <g key={`lg${i}`}>
                <rect x={padL + p.x} y={legendTop + p.y} width="11" height="11" rx="2" fill={p.color} />
                <text x={padL + p.x + 15} y={legendTop + p.y + 9} fontSize="9" fill="var(--ax-ink-soft)">{p.label}</text>
              </g>
            ))
          )}

          {notes.length > 0 && (
            <text x={padL} y={noteTop + 2} fontSize="9" fill="var(--ax-ink-faint)">{notes.join(" · ")}</text>
          )}
        </svg>
      </div>
      <figcaption className="ax-fig__cap">
        <span className="ax-fig__kind">序列比对 · MSA</span>
        {caption || spec.title || `${rows.length} × ${L} · ${kindLabel}`}
      </figcaption>
    </figure>
  );
}
