// Native PRISMA 2020 flow diagram: a FIXED-topology systematic-review flow chart whose only
// variables are the count slots. The renderer never invents a number — a missing count shows
// "—" and, when a *core* count (an identification source / screened / sought / assessed /
// included) is absent, a corner badge flags "数据不全". Two source streams (databases &
// registers vs. other methods) converge into a single Included box. Pure inline SVG.
//
// Spec (```prisma JSON): {
//   "title"?,
//   "identification"?: {"databases"?, "registers"?, "duplicatesRemoved"?, "otherRemoved"?, "otherMethods"?},
//   "screening"?:      {"screened"?, "excluded"?},
//   "retrieval"?:      {"sought"?, "notRetrieved"?},
//   "eligibility"?:    {"assessed"?, "excluded"?, "excludedReasons"?: [{"reason": string, "n"?: number}]},
//   "included"?:       {"studies"?, "reports"?}
// }
// ALL fields optional — the five-stage box column always renders (fixed PRISMA layout); empty
// slots read "—". The right "other methods" stream renders only when identification.otherMethods
// is a finite number. No count is ever fabricated to fill a gap.
import { catColor, isDark } from "./theme";

type Reason = { reason: string; n?: number };
type Spec = {
  title?: string;
  identification?: {
    databases?: number;
    registers?: number;
    duplicatesRemoved?: number;
    otherRemoved?: number;
    otherMethods?: number;
  };
  screening?: { screened?: number; excluded?: number };
  retrieval?: { sought?: number; notRetrieved?: number };
  eligibility?: { assessed?: number; excluded?: number; excludedReasons?: Reason[] };
  included?: { studies?: number; reports?: number };
};

type Line = { text: string; strong?: boolean; muted?: boolean };
type Box = {
  x: number;
  y: number;
  w: number;
  h: number;
  header?: string;
  accent?: string;
  wrapped: Line[];
  padX: number;
  padY: number;
  lh: number;
  headH: number;
};

const num = (v: unknown): number | undefined =>
  typeof v === "number" && Number.isFinite(v) ? v : undefined;

// CJK-aware glyph width at an arbitrary font size (baseline calibrated to ~11px).
function charW(ch: string, fs: number): number {
  const base = /[⺀-鿿＀-￯　-〿]/.test(ch) ? 11 : 6.2;
  return base * (fs / 11);
}
function textW(s: string, fs: number): number {
  let w = 0;
  for (const c of s) w += charW(c, fs);
  return w;
}
// Greedy word/character wrap to a pixel budget (CJK breaks per-glyph; Latin prefers spaces).
function wrapLine(text: string, maxW: number, fs: number): string[] {
  const out: string[] = [];
  let cur = "";
  let curW = 0;
  const flush = () => {
    out.push(cur);
    cur = "";
    curW = 0;
  };
  const tokens =
    text.match(/[⺀-鿿＀-￯　-〿]|[^\s⺀-鿿＀-￯　-〿]+|\s+/g) || [];
  for (const tk of tokens) {
    if (cur === "" && /^\s+$/.test(tk)) continue; // no leading space on a fresh line
    const tkW = textW(tk, fs);
    if (curW > 0 && curW + tkW > maxW && tk.trim() !== "") flush();
    if (tkW > maxW) {
      // hard-break an oversize token
      for (const c of tk) {
        const cwv = charW(c, fs);
        if (curW + cwv > maxW && curW > 0) flush();
        cur += c;
        curW += cwv;
      }
      continue;
    }
    cur += tk;
    curW += tkW;
  }
  if (cur !== "" || out.length === 0) out.push(cur);
  return out;
}

const BODY_FS = 10;

function makeBox(x: number, y: number, w: number, header: string | undefined, lines: Line[], accent?: string): Box {
  const padX = 10;
  const padY = 8;
  const lh = 14;
  const headH = header ? 16 : 0;
  const innerW = w - padX * 2 - (accent ? 6 : 0);
  const wrapped: Line[] = [];
  for (const ln of lines) {
    const parts = wrapLine(ln.text, innerW, BODY_FS);
    for (const t of parts) wrapped.push({ text: t, strong: ln.strong, muted: ln.muted });
  }
  const h = padY + headH + wrapped.length * lh + padY;
  return { x, y, w, h, header, accent, wrapped, padX, padY, lh, headH };
}

export function PrismaFlow({ spec, caption }: { spec: string; caption?: string }) {
  let d: Spec;
  try {
    d = JSON.parse(spec.trim());
  } catch (e) {
    return <div className="ax-fig__err">PRISMA 流程图规范不是有效 JSON · {String((e as Error).message)}</div>;
  }
  if (!d || typeof d !== "object" || Array.isArray(d))
    return <div className="ax-fig__err">PRISMA 流程图规范为空或格式不对</div>;

  const dark = isDark();
  const cId = catColor(0, dark);
  const cScr = catColor(2, dark);
  const cInc = catColor(1, dark);
  const warn = catColor(3, dark);

  const id = d.identification || {};
  const scr = d.screening || {};
  const ret = d.retrieval || {};
  const el = d.eligibility || {};
  const inc = d.included || {};

  const db = num(id.databases);
  const reg = num(id.registers);
  const dup = num(id.duplicatesRemoved);
  const othRm = num(id.otherRemoved);
  const othMeth = num(id.otherMethods);
  const screened = num(scr.screened);
  const scrExcl = num(scr.excluded);
  const sought = num(ret.sought);
  const notRetr = num(ret.notRetrieved);
  const assessed = num(el.assessed);
  const elExcl = num(el.excluded);
  const reasons: Reason[] = Array.isArray(el.excludedReasons)
    ? el.excludedReasons.filter((r): r is Reason => !!r && typeof r.reason === "string")
    : [];
  const studies = num(inc.studies);
  const reports = num(inc.reports);

  // ── missing-count accounting: flag only CORE counts, never optional side counts ──
  let incomplete = false;
  const opt = (v?: number): string => (v == null ? "—" : v.toLocaleString("en-US"));
  const core = (v?: number): string => {
    if (v == null) incomplete = true;
    return opt(v);
  };
  const hasOther = othMeth != null;
  // an identification source is core: at least one of databases/registers (or other methods) must exist
  if (db == null && reg == null && !hasOther) incomplete = true;

  // ── layout constants ──
  const gutterW = 26;
  const boxW = 216;
  const sideW = 184;
  const hGap = 54;
  const vGap = 30;
  const leftMainX = gutterW + 10;
  const leftExclX = leftMainX + boxW + hGap;
  const otherGap = 60;
  const rightMainX = leftExclX + sideW + otherGap;
  const otherBoxW = 216;
  const rightPad = 14;
  const width = (hasOther ? rightMainX + otherBoxW : leftExclX + sideW) + rightPad;

  const topPad = d.title ? 34 : 16;

  // ── box content ──
  const l0lines: Line[] = [
    { text: "记录来源 Records identified from:", strong: true },
    { text: `· 数据库 Databases: ${opt(db)}` },
    { text: `· 注册库 Registers: ${opt(reg)}` },
  ];
  const s0lines: Line[] = [
    { text: "筛选前移除 Records removed before screening:", strong: true },
    { text: `· 重复记录 Duplicates: ${opt(dup)}`, muted: true },
    { text: `· 其他移除 Other reasons: ${opt(othRm)}`, muted: true },
  ];
  const l1lines: Line[] = [{ text: `记录筛选 Records screened: ${core(screened)}`, strong: true }];
  const s1lines: Line[] = [{ text: `排除记录 Records excluded: ${opt(scrExcl)}`, muted: true }];
  const l2lines: Line[] = [{ text: `获取全文 Reports sought for retrieval: ${core(sought)}`, strong: true }];
  const s2lines: Line[] = [{ text: `未获取全文 Reports not retrieved: ${opt(notRetr)}`, muted: true }];
  const l3lines: Line[] = [{ text: `全文评估 Reports assessed for eligibility: ${core(assessed)}`, strong: true }];

  const s3lines: Line[] = [{ text: "排除全文 Reports excluded:", strong: true }];
  if (elExcl != null) s3lines.push({ text: `合计 Total: ${elExcl.toLocaleString("en-US")}` });
  if (reasons.length) {
    for (const r of reasons) {
      const n = num(r.n);
      s3lines.push({ text: `· ${r.reason}${n != null ? ` (n=${n.toLocaleString("en-US")})` : ""}`, muted: true });
    }
  } else if (elExcl == null) {
    s3lines.push({ text: "· 原因未提供 —", muted: true });
  }

  const l4lines: Line[] = [
    { text: `纳入研究 Studies included: ${core(studies)}`, strong: true },
    { text: `纳入报告 Reports of included studies: ${opt(reports)}` },
  ];

  // ── position boxes stage by stage ──
  let y = topPad;
  const L0 = makeBox(leftMainX, y, boxW, "识别 · Identification", l0lines, cId);
  const S0 = makeBox(leftExclX, y, sideW, undefined, s0lines);
  y += Math.max(L0.h, S0.h) + vGap;

  const L1 = makeBox(leftMainX, y, boxW, "筛选 · Screening", l1lines, cScr);
  const S1 = makeBox(leftExclX, y, sideW, undefined, s1lines);
  y += Math.max(L1.h, S1.h) + vGap;

  const L2 = makeBox(leftMainX, y, boxW, "检索获取 · Retrieval", l2lines, cScr);
  const S2 = makeBox(leftExclX, y, sideW, undefined, s2lines);
  y += Math.max(L2.h, S2.h) + vGap;

  const L3 = makeBox(leftMainX, y, boxW, "合格性评估 · Eligibility", l3lines, cScr);
  const S3 = makeBox(leftExclX, y, sideW, undefined, s3lines);
  y += Math.max(L3.h, S3.h) + vGap;

  const L4 = makeBox(leftMainX, y, boxW, "纳入 · Included", l4lines, cInc);
  const height = L4.y + L4.h + 16;

  // other-methods stream (single identification box) — only when a count is supplied
  const other = hasOther
    ? makeBox(rightMainX, L0.y, otherBoxW, "其他来源 · Other methods", [
        { text: `其他方法识别 Records via other methods: ${opt(othMeth)}`, strong: true },
        { text: "· 网站 / 机构 / 引文检索", muted: true },
      ])
    : null;

  const mainBoxes: Box[] = [L0, L1, L2, L3, L4];
  const sideBoxes: Box[] = [S0, S1, S2, S3];

  const cx = leftMainX + boxW / 2;

  // vertical connectors down the main column
  const vLinks = [
    [L0, L1],
    [L1, L2],
    [L2, L3],
    [L3, L4],
  ].map(([a, b]) => ({ x1: cx, y1: a.y + a.h, x2: cx, y2: b.y }));

  // horizontal connectors to the exclusion boxes (at each main box's mid-height)
  const hLinks = [
    [L0, S0],
    [L1, S1],
    [L2, S2],
    [L3, S3],
  ].map(([a]) => {
    const my = a.y + a.h / 2;
    return { x1: leftMainX + boxW, y1: my, x2: leftExclX, y2: my };
  });

  // phase-band gutter ranges
  const bands = [
    { label: "识别", top: L0.y - 6, bottom: L1.y - vGap / 2, color: cId },
    { label: "筛选与合格性", top: L1.y - vGap / 2, bottom: L4.y - vGap / 2, color: cScr },
    { label: "纳入", top: L4.y - vGap / 2, bottom: L4.y + L4.h + 6, color: cInc },
  ];

  const ariaLabel = caption || d.title || "PRISMA flow diagram";

  return (
    <figure className="ax-fig">
      <div className="ax-fig__frame" style={{ overflowX: "auto", justifyContent: "flex-start" }}>
        <svg
          viewBox={`0 0 ${width} ${height}`}
          width={width}
          height={height}
          role="img"
          aria-label={ariaLabel}
          style={{ maxWidth: "100%" }}
        >
          <defs>
            <marker
              id="ax-prisma-ah"
              markerWidth="9"
              markerHeight="9"
              refX="6.5"
              refY="3"
              orient="auto"
              markerUnits="userSpaceOnUse"
            >
              <path d="M0,0 L6.5,3 L0,6 Z" fill="var(--ax-line-strong)" />
            </marker>
          </defs>

          {d.title && (
            <text x={8} y={18} fontSize="13" fontWeight="700" fill="var(--ax-ink)">
              {d.title}
            </text>
          )}

          {/* incomplete-data badge (never fabricates — only warns) */}
          {incomplete && (
            <g>
              <rect
                x={width - 174}
                y={4}
                width={166}
                height={17}
                rx={8.5}
                fill="none"
                stroke={warn}
                strokeWidth="1"
              />
              <text x={width - 91} y={16} fontSize="9.5" textAnchor="middle" fill={warn}>
                ⚠ 数据不全 · 核心计数缺失
              </text>
            </g>
          )}

          {/* phase gutter bands + rotated labels */}
          {bands.map((b, i) => {
            const bh = Math.max(0, b.bottom - b.top);
            const midY = (b.top + b.bottom) / 2;
            return (
              <g key={`band${i}`}>
                <rect x={4} y={b.top} width={gutterW - 6} height={bh} rx={4} fill={b.color} opacity={0.12} />
                <rect x={4} y={b.top} width={2.5} height={bh} rx={1.25} fill={b.color} opacity={0.7} />
                <text
                  x={4 + (gutterW - 6) / 2 + 1}
                  y={midY}
                  fontSize="10"
                  fontWeight="600"
                  textAnchor="middle"
                  fill="var(--ax-ink-soft)"
                  transform={`rotate(-90 ${4 + (gutterW - 6) / 2 + 1} ${midY})`}
                >
                  {b.label}
                </text>
              </g>
            );
          })}

          {/* connectors (drawn under boxes) */}
          {vLinks.map((l, i) => (
            <line
              key={`v${i}`}
              x1={l.x1}
              y1={l.y1}
              x2={l.x2}
              y2={l.y2}
              stroke="var(--ax-line-strong)"
              strokeWidth="1.4"
              markerEnd="url(#ax-prisma-ah)"
            />
          ))}
          {hLinks.map((l, i) => (
            <line
              key={`h${i}`}
              x1={l.x1}
              y1={l.y1}
              x2={l.x2}
              y2={l.y2}
              stroke="var(--ax-line-strong)"
              strokeWidth="1.4"
              markerEnd="url(#ax-prisma-ah)"
            />
          ))}
          {/* other-methods stream converging into Included */}
          {other && (
            <path
              d={`M ${rightMainX + otherBoxW / 2} ${other.y + other.h} V ${L4.y + L4.h / 2} H ${leftMainX + boxW}`}
              fill="none"
              stroke="var(--ax-line-strong)"
              strokeWidth="1.4"
              markerEnd="url(#ax-prisma-ah)"
            />
          )}

          {/* boxes */}
          {[...mainBoxes, ...sideBoxes, ...(other ? [other] : [])].map((b, i) => {
            const isSide = sideBoxes.includes(b);
            const tx = b.x + b.padX + (b.accent ? 6 : 0);
            return (
              <g key={`box${i}`}>
                <rect
                  x={b.x}
                  y={b.y}
                  width={b.w}
                  height={b.h}
                  rx={6}
                  fill="var(--ax-surface)"
                  stroke="var(--ax-line)"
                  strokeWidth="1"
                  strokeDasharray={isSide ? "4 3" : undefined}
                />
                {b.accent && (
                  <rect x={b.x + 1} y={b.y + 1} width={3} height={b.h - 2} rx={1.5} fill={b.accent} />
                )}
                {b.header && (
                  <text x={tx} y={b.y + b.padY + 11} fontSize="10" fontWeight="700" fill={b.accent || "var(--ax-ink)"}>
                    {b.header}
                  </text>
                )}
                {b.wrapped.map((wl, j) => (
                  <text
                    key={`t${j}`}
                    x={tx}
                    y={b.y + b.padY + b.headH + j * b.lh + 10}
                    fontSize={BODY_FS}
                    fontWeight={wl.strong ? 600 : 400}
                    fill={wl.muted ? "var(--ax-ink-soft)" : "var(--ax-ink)"}
                    style={{ fontVariantNumeric: "tabular-nums" }}
                  >
                    {wl.text}
                  </text>
                ))}
              </g>
            );
          })}
        </svg>
      </div>
      <figcaption className="ax-fig__cap">
        <span className="ax-fig__kind">PRISMA 流程图 · PRISMA flow</span> {caption || d.title || ""}
      </figcaption>
    </figure>
  );
}
