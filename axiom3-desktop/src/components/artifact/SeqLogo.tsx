// Native sequence logo (motif / PWM): per-position residue frequencies stacked and scaled by
// information content in bits (bits = log2|alphabet| − entropy), the classic TF-binding /
// conservation logo. Letters are scaled <text> (vertical transform = height); nucleotide /
// protein-chemistry color maps; y-axis in bits; a coordinate ruler numbered from `start`.
//
// Spec (```seqlogo JSON): {"title"?, "type":"dna"|"rna"|"protein", "start"?:int(default 1),
//   "columns":[{"A":0.9,"C":0.03,"G":0.04,"T":0.03}, ...]}  // each column need not sum to 1.

type Spec = { title?: unknown; type?: unknown; start?: unknown; columns?: unknown };
type StackItem = { res: string; p: number; bits: number };
type Column = { bits: number; stack: StackItem[] };

const DNA = ["A", "C", "G", "T"];
const RNA = ["A", "C", "G", "U"];
const PROTEIN = ["A", "R", "N", "D", "C", "Q", "E", "G", "H", "I", "L", "K", "M", "F", "P", "S", "T", "W", "Y", "V"];

function isDark(): boolean {
  const t = document.documentElement.dataset.theme;
  if (t === "dark") return true;
  if (t === "light") return false;
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? false;
}

// nucleotide colors (A green · C blue · G gold · T/U red), theme-adaptive by slot.
const NT_LIGHT: Record<string, string> = { A: "#008300", C: "#2a78d6", G: "#d99100", T: "#e34948", U: "#e34948" };
const NT_DARK: Record<string, string> = { A: "#199e70", C: "#3987e5", G: "#c98500", T: "#e66767", U: "#e66767" };

// protein "chemistry" scheme: polar green · neutral purple · basic blue · acidic red · hydrophobic ink.
const AA_GROUP: Record<string, "polar" | "neutral" | "basic" | "acidic" | "hydro"> = {
  G: "polar", S: "polar", T: "polar", Y: "polar", C: "polar",
  Q: "neutral", N: "neutral",
  K: "basic", R: "basic", H: "basic",
  D: "acidic", E: "acidic",
  A: "hydro", V: "hydro", L: "hydro", I: "hydro", P: "hydro", W: "hydro", F: "hydro", M: "hydro",
};
const GROUP_LIGHT: Record<string, string> = { polar: "#008300", neutral: "#4a3aa7", basic: "#2a78d6", acidic: "#e34948", hydro: "var(--ax-ink)" };
const GROUP_DARK: Record<string, string> = { polar: "#199e70", neutral: "#9085e9", basic: "#3987e5", acidic: "#e66767", hydro: "var(--ax-ink)" };
const GROUP_LABEL: Record<string, string> = { polar: "极性 polar", neutral: "中性 neutral", basic: "碱性 basic", acidic: "酸性 acidic", hydro: "疏水 hydrophobic" };

// glyph metrics: text drawn at font-size FS (in the pinned bold font below) has its cap-height ≈ CAP
// above the baseline; a vertical scale of height/CAP makes a capital letter exactly `height` px tall.
// The font is PINNED (not inherited) so the measured natural width and CAP stay consistent.
const FS = 100;
const CAP = 71; // Helvetica/Arial bold cap-height at FS=100 (≈0.717·em)
const LOGO_FONT_FAMILY = "'Helvetica Neue', Helvetica, Arial, sans-serif";

// Measure a glyph's NATURAL advance width at FS in the pinned font (memoized). A seqlogo letter must
// fill its cell width AND its information-content height by scaling BOTH axes from the glyph's true
// proportions — NOT by forcing textLength on a single glyph (which distorts the letter into a thin,
// stretched shape). Falls back to a typical cap width when canvas is unavailable (tests/SSR).
const glyphWCache = new Map<string, number>();
let measureCtx: CanvasRenderingContext2D | null | undefined;
function glyphWidth(ch: string): number {
  const cached = glyphWCache.get(ch);
  if (cached != null) return cached;
  if (measureCtx === undefined) {
    try {
      const cv = document.createElement("canvas");
      measureCtx = cv.getContext("2d");
      if (measureCtx) measureCtx.font = `700 ${FS}px ${LOGO_FONT_FAMILY}`;
    } catch {
      measureCtx = null;
    }
  }
  let w = 62; // fallback ≈ a typical uppercase advance at FS=100
  if (measureCtx) {
    measureCtx.font = `700 ${FS}px ${LOGO_FONT_FAMILY}`;
    const m = measureCtx.measureText(ch).width;
    if (Number.isFinite(m) && m > 1) w = m;
  }
  glyphWCache.set(ch, w);
  return w;
}

function computeColumn(col: unknown, alphabet: string[], maxBits: number): Column {
  const acc: Record<string, number> = {};
  if (col && typeof col === "object") {
    for (const [k, v] of Object.entries(col as Record<string, unknown>)) {
      const res = k.trim().toUpperCase();
      const num = typeof v === "number" ? v : Number(v);
      if (alphabet.includes(res) && isFinite(num) && num > 0) acc[res] = (acc[res] ?? 0) + num;
    }
  }
  const freqs = alphabet.map((r) => acc[r] ?? 0);
  const sum = freqs.reduce((s, f) => s + f, 0);
  if (sum <= 0) return { bits: 0, stack: [] };
  const ps = freqs.map((f) => f / sum);
  let H = 0;
  for (const p of ps) if (p > 0) H -= p * Math.log2(p);
  const R = Math.max(0, maxBits - H); // information content (bits) for the column
  const stack = alphabet
    .map((res, i) => ({ res, p: ps[i], bits: ps[i] * R }))
    .filter((s) => s.bits > 1e-6)
    .sort((a, b) => a.bits - b.bits); // ascending → largest residue ends on top of the stack
  return { bits: R, stack };
}

export function SeqLogo({ spec: raw, caption }: { spec: string; caption?: string }) {
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw.trim());
  } catch (e) {
    return <div className="ax-fig__err">序列标识规范不是有效 JSON · {String((e as Error)?.message || e)}</div>;
  }
  if (!parsed || typeof parsed !== "object") return <div className="ax-fig__err">序列标识规范为空或格式不对</div>;
  const spec = parsed as Spec;
  if (!Array.isArray(spec.columns) || spec.columns.length === 0) {
    return <div className="ax-fig__err">序列标识需要 columns 数组（每列为 残基→频率）</div>;
  }

  const kind = String(spec.type ?? "dna").trim().toLowerCase();
  const isProtein = kind === "protein";
  const alphabet = isProtein ? PROTEIN : kind === "rna" ? RNA : DNA;
  const maxBits = Math.log2(alphabet.length); // 2 for DNA/RNA, ≈4.32 for protein
  const start = typeof spec.start === "number" && isFinite(spec.start) ? Math.round(spec.start) : 1;
  const title = typeof spec.title === "string" ? spec.title : undefined;

  const cols = spec.columns.map((c) => computeColumn(c, alphabet, maxBits));
  if (!cols.some((c) => c.stack.length > 0)) {
    return <div className="ax-fig__err">序列标识需要至少一列有效的残基频率</div>;
  }

  const dark = isDark();
  const colorOf = (res: string): string => {
    if (isProtein) return (dark ? GROUP_DARK : GROUP_LIGHT)[AA_GROUP[res] ?? "hydro"];
    return (dark ? NT_DARK : NT_LIGHT)[res] ?? "var(--ax-ink)";
  };

  // geometry
  const padL = 46;
  const padR = 14;
  const padT = title ? 28 : 12;
  const plotH = 156;
  const rulerH = 20;
  const legendH = isProtein ? 22 : 20;
  const colPitch = 32;
  const letterW = 27; // fills most of the column so letters read wide, not thin & stretched
  const inset = (colPitch - letterW) / 2;
  const n = cols.length;
  const plotW = n * colPitch;
  const y0 = padT + plotH; // baseline (0 bits)
  const scaleY = plotH / maxBits; // px per bit
  const colLeft = (i: number) => padL + i * colPitch + inset;
  const colMid = (i: number) => padL + i * colPitch + colPitch / 2;

  // Legend groups for protein (identity legend row).
  const legendGroups = isProtein ? (["polar", "neutral", "basic", "acidic", "hydro"] as const) : null;

  // Total canvas width must fit whichever of {plot, legend row, title} is widest, so the outer
  // SVG viewport never clips the protein legend row or a long title (plot stays left-aligned).
  const legendPitch = 96;
  const baseW = padL + plotW + padR;
  const legendW = legendGroups ? padL + legendGroups.length * legendPitch : padL + alphabet.length * 30 + 12;
  const titleW = title ? padL + Math.ceil(title.length * 7) + padR : 0;
  const w = Math.max(baseW, legendW, titleW);
  const h = padT + plotH + rulerH + legendH;

  // y-axis tick values in bits (integers up to the theoretical max)
  const ticks: number[] = [];
  for (let t = 0; t <= maxBits + 1e-9; t += 1) ticks.push(t);

  // position ruler: label all columns when few, else thin out
  const labelStep = n <= 40 ? 1 : n <= 80 ? 2 : 5;

  return (
    <figure className="ax-fig">
      <div className="ax-fig__frame" style={{ overflowX: "auto", justifyContent: "flex-start" }}>
        <svg viewBox={`0 0 ${w} ${h}`} width={w} height={h} role="img" aria-label={caption || title || "sequence logo"} style={{ maxWidth: "100%" }}>
          {title && (
            <text x={padL} y={16} fontSize="12" fontWeight="600" fill="var(--ax-ink)">
              {title}
            </text>
          )}

          {/* y-axis gridlines + tick labels (bits) */}
          {ticks.map((t, i) => {
            const y = y0 - t * scaleY;
            return (
              <g key={`tk${i}`}>
                <line x1={padL} y1={y} x2={padL + plotW} y2={y} stroke="var(--ax-line-soft)" strokeWidth="0.5" />
                <text x={padL - 6} y={y + 3} fontSize="9" textAnchor="end" fill="var(--ax-ink-faint)">
                  {t}
                </text>
              </g>
            );
          })}
          {/* axes */}
          <line x1={padL} y1={padT} x2={padL} y2={y0} stroke="var(--ax-line)" strokeWidth="1" />
          <line x1={padL} y1={y0} x2={padL + plotW} y2={y0} stroke="var(--ax-line)" strokeWidth="1" />
          <text x={13} y={padT + plotH / 2} fontSize="10" textAnchor="middle" fill="var(--ax-ink-soft)" transform={`rotate(-90 13 ${padT + plotH / 2})`}>
            信息量 · bits
          </text>

          {/* stacked letters — largest residue on top; height = frequency × column info content */}
          {cols.map((c, i) => {
            let cum = 0; // bits accumulated from the baseline upward
            return (
              <g key={`col${i}`}>
                {c.stack.map((s, j) => {
                  const yBottom = y0 - cum * scaleY;
                  cum += s.bits;
                  const yTop = y0 - cum * scaleY;
                  const height = yBottom - yTop;
                  if (height < 0.5) return null;
                  // Fill the cell by scaling the glyph on BOTH axes from its true proportions:
                  // sx maps the glyph's natural advance to letterW, sy maps cap-height to `height`.
                  // No textLength → the letter stays a clean, undistorted glyph at any height.
                  const sx = letterW / glyphWidth(s.res);
                  const sy = height / CAP;
                  return (
                    <g
                      key={`l${i}_${j}`}
                      transform={`translate(${colLeft(i)}, ${yBottom}) scale(${sx}, ${sy})`}
                    >
                      <text
                        x={0}
                        y={0}
                        fontSize={FS}
                        fontWeight="700"
                        fontFamily={LOGO_FONT_FAMILY}
                        textAnchor="start"
                        fill={colorOf(s.res)}
                      >
                        <title>{`位置 ${start + i} · ${s.res} — 频率 ${(s.p * 100).toFixed(1)}% · ${s.bits.toFixed(2)} bits`}</title>
                        {s.res}
                      </text>
                    </g>
                  );
                })}
              </g>
            );
          })}

          {/* position ruler numbered from `start` */}
          {cols.map((_, i) =>
            i % labelStep === 0 ? (
              <text key={`p${i}`} x={colMid(i)} y={y0 + 14} fontSize="9" textAnchor="middle" fill="var(--ax-ink-faint)">
                {start + i}
              </text>
            ) : null,
          )}

          {/* color legend (identity, never color-alone → residue letters / group names) */}
          {legendGroups
            ? legendGroups.map((g, i) => {
                const lx = padL + i * legendPitch;
                const ly = y0 + rulerH + 8;
                return (
                  <g key={`lg${i}`}>
                    <rect x={lx} y={ly - 7} width="9" height="9" fill={(dark ? GROUP_DARK : GROUP_LIGHT)[g]} />
                    <text x={lx + 13} y={ly + 1} fontSize="8.5" fill="var(--ax-ink-soft)">
                      {GROUP_LABEL[g]}
                    </text>
                  </g>
                );
              })
            : alphabet.map((r, i) => {
                const lx = padL + i * 30;
                const ly = y0 + rulerH + 8;
                return (
                  <text key={`lg${i}`} x={lx} y={ly + 1} fontSize="12" fontWeight="700" fill={colorOf(r)}>
                    {r}
                  </text>
                );
              })}
        </svg>
      </div>
      <figcaption className="ax-fig__cap">
        <span className="ax-fig__kind">序列标识 · seqlogo</span> {caption || title || ""}
      </figcaption>
    </figure>
  );
}
