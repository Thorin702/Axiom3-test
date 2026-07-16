// Native phylogenetic tree / dendrogram: a rooted, horizontal rectangular tree. With branch
// lengths it draws a phylogram (x = cumulative length from root, plus a scale bar); without
// lengths it falls back to an evenly-spaced cladogram (topology only). Tips are labeled in an
// aligned column on the right with faint leader lines. Pure inline SVG, no dependency.
// Neutral by construction: it draws only the emitted topology + branch lengths, never a verdict.
//
// Spec (```tree JSON): {"title"?, "newick"?: "((A:0.1,B:0.2):0.05,C:0.3);",
//   "root"?: {"name"?, "length"?, "children":[ ...nested... ]}}  — provide newick OR root.

import { catColor, isDark } from "./theme";

type TreeNode = { name?: string; length?: number; support?: number; children: TreeNode[] };
type RawNode = { name?: unknown; length?: unknown; support?: unknown; children?: unknown };
type Spec = {
  title?: string;
  newick?: string;
  root?: RawNode;
  // optional support/rendering hints — all default to auto-detection when omitted
  support_type?: string; // "BS"|"bootstrap" | "PP"|"posterior" — disambiguates the support scale
  italicize_tips?: boolean; // force tip italics on/off; omitted → auto-detect latin binomials
  cladogram?: boolean; // force topology-only (ignore branch lengths) even if lengths are present
};

// ── ~40-line Newick parser (recursive descent over a position index) ──
function parseNewick(input: string): TreeNode | null {
  let s = input.trim();
  if (s.endsWith(";")) s = s.slice(0, -1);
  const n = s.length;
  let i = 0;
  let guard = 0;
  const ws = (): void => {
    while (i < n && /\s/.test(s[i])) i++;
  };
  function node(): TreeNode {
    if (++guard > 200000) throw new Error("newick too large");
    ws();
    const nd: TreeNode = { children: [] };
    if (s[i] === "(") {
      i++; // consume "("
      for (;;) {
        nd.children.push(node());
        ws();
        if (s[i] === ",") {
          i++;
          continue;
        }
        break;
      }
      ws();
      if (s[i] === ")") i++;
    }
    ws();
    // label: quoted, or bare up to a structural char
    let name = "";
    if (s[i] === "'") {
      i++;
      while (i < n && s[i] !== "'") name += s[i++];
      if (s[i] === "'") i++;
    } else {
      while (i < n && !":,()".includes(s[i])) name += s[i++];
    }
    name = name.trim().replace(/_/g, " ");
    if (name) nd.name = name;
    ws();
    if (s[i] === ":") {
      i++;
      let num = "";
      while (i < n && !",()".includes(s[i]) && !/\s/.test(s[i])) num += s[i++];
      const v = parseFloat(num);
      if (Number.isFinite(v)) nd.length = v;
    }
    return nd;
  }
  try {
    return node();
  } catch {
    return null;
  }
}

// Defensive: never throws on a malformed/nested root (null nodes, arrays, bad children).
function normalize(r: unknown, depth: number): TreeNode | null {
  if (depth > 4096) return null;
  if (!r || typeof r !== "object" || Array.isArray(r)) return null;
  const o = r as RawNode;
  const kids: TreeNode[] = [];
  if (Array.isArray(o.children)) {
    for (const c of o.children) {
      const nc = normalize(c, depth + 1);
      if (nc) kids.push(nc);
    }
  }
  return {
    name: typeof o.name === "string" ? o.name : undefined,
    length: typeof o.length === "number" && Number.isFinite(o.length) ? o.length : undefined,
    support: typeof o.support === "number" && Number.isFinite(o.support) ? o.support : undefined,
    children: kids,
  };
}

function fmt(v: number): string {
  if (!Number.isFinite(v)) return "—";
  const a = Math.abs(v);
  if (a === 0) return "0";
  if (a >= 100) return v.toFixed(0);
  if (a >= 10) return v.toFixed(1);
  if (a >= 1) return v.toFixed(2);
  if (a >= 0.001) return v.toFixed(3);
  return v.toPrecision(2); // tiny molecular branch lengths: keep 2 sig figs, never round to "0"
}
function trimZeros(s: string): string {
  return s.includes(".") ? s.replace(/\.?0+$/, "") : s;
}
function truncate(s: string, max: number): string {
  if (max <= 1) return s.length > 1 ? "…" : s;
  return s.length > max ? s.slice(0, max - 1) + "…" : s;
}
// CJK-aware pixel width at the 11px label font: full-width glyphs advance ~11px, others ~6.6px, so
// a Chinese taxon name isn't underestimated (which clipped it past the reserved label column).
const CJK_RE = /[⺀-鿿豈-﫿＀-￯　-〿]/;
function textPxWidth(s: string): number {
  let w = 0;
  for (const ch of s) w += CJK_RE.test(ch) ? 11 : 6.6;
  return w;
}
// Truncate to a pixel budget (not a char count), so a CJK label fits its column instead of clipping.
function truncateToWidth(s: string, maxW: number): string {
  if (textPxWidth(s) <= maxW) return s;
  let w = 0;
  let out = "";
  for (const ch of s) {
    const cw = CJK_RE.test(ch) ? 11 : 6.6;
    if (w + cw + 6 > maxW) break; // reserve ~6px for the ellipsis
    w += cw;
    out += ch;
  }
  return out + "…";
}
function niceRound(x: number): number {
  if (!(x > 0)) return 1;
  const exp = Math.floor(Math.log10(x));
  const f = x / Math.pow(10, exp);
  const nf = f < 1.5 ? 1 : f < 3 ? 2 : f < 7 ? 5 : 10;
  return nf * Math.pow(10, exp);
}

// A Newick internal-node label is a branch-support value (IQ-TREE "SH-aLRT/uBS", or a bootstrap %).
// Show it only when it actually looks like support — a number, or slash/parenthesis-joined numbers —
// so a genuine internal clade NAME isn't mistaken for support and vice-versa.
function isSupportLabel(s: string): boolean {
  return /^[\d]/.test(s.trim()) && /^[\d./\\|:()%\s-]+$/.test(s.trim());
}

// Parse a branch-support value into { value, kind, low } for graphical de-emphasis. Accepts a bare
// number (0–100 bootstrap % or a 0–1 posterior probability) or a joined label ("85/95", "0.98").
// Scale is auto-inferred (any number > 1 → bootstrap %, else posterior) unless spec.support_type
// overrides it. "Low" follows the field convention: BS < 70, or PP < 0.95 — those branches are
// dimmed/dashed so weakly-supported clades read as tentative, never as confident.
type Support = { value: number; kind: "BS" | "PP"; low: boolean; raw: string };
function parseSupport(raw: string | number | undefined, hint?: string): Support | null {
  let nums: number[] = [];
  let rawStr = "";
  if (typeof raw === "number" && Number.isFinite(raw)) {
    nums = [raw];
    rawStr = trimZeros(fmt(raw));
  } else if (typeof raw === "string") {
    rawStr = raw.trim();
    const m = rawStr.match(/\d+(?:\.\d+)?/g);
    if (m) nums = m.map(Number).filter((x) => Number.isFinite(x));
  }
  if (!nums.length) return null;
  const primary = nums[0];
  const h = (hint ?? "").toLowerCase();
  let kind: "BS" | "PP";
  if (h === "bs" || h === "bootstrap") kind = "BS";
  else if (h === "pp" || h === "posterior") kind = "PP";
  else kind = nums.some((x) => x > 1) ? "BS" : "PP";
  const low = kind === "BS" ? primary < 70 : primary < 0.95;
  return { value: primary, kind, low, raw: rawStr };
}

// A tip label that reads as a latin binomial (Genus species …) is italicised, per nomenclature
// convention. Conservative regex so codes/accessions ("OTU 12", "SEQ001") stay upright.
function looksBinomial(nm: string): boolean {
  return /^[A-Z][a-z]{2,}\s+[a-z]{2,}/.test(nm.trim());
}

// laid-out node (data + pixel coords), computed client-side so nothing is fabricated.
type Laid = {
  name?: string;
  isLeaf: boolean;
  cum: number; // cumulative branch length from root (lengths clamped ≥ 0)
  h: number; // height in edges to the furthest leaf below (0 for a leaf)
  support: Support | null; // parsed branch support (internal nodes only)
  children: Laid[];
  y: number;
  px: number;
};

export function Tree({ spec, caption }: { spec: string; caption?: string }) {
  let d: Spec;
  try {
    d = JSON.parse(spec.trim());
  } catch (e) {
    return <div className="ax-fig__err">进化树规范不是有效 JSON · {String((e as Error).message)}</div>;
  }
  if (!d || typeof d !== "object") return <div className="ax-fig__err">进化树规范为空或格式不对</div>;

  let root: TreeNode | null = null;
  if (typeof d.newick === "string" && d.newick.trim()) root = parseNewick(d.newick);
  else if (d.root && typeof d.root === "object" && !Array.isArray(d.root)) root = normalize(d.root, 0);
  if (!root) return <div className="ax-fig__err">进化树需要 newick 字符串或 root 嵌套对象（且可解析）</div>;
  // reject a degenerate parse (e.g. empty newick → a single nameless leaf)
  if (root.children.length === 0 && !root.name) {
    return <div className="ax-fig__err">进化树为空或无法解析（没有可显示的分类单元）</div>;
  }
  const safeRoot: TreeNode = root;

  // ── build the laid-out tree (cumulative length + height) ──
  function build(node: TreeNode, parentCum: number): Laid {
    const cum = parentCum + Math.max(0, node.length ?? 0);
    const children = node.children.map((c) => build(c, cum));
    const isLeaf = children.length === 0;
    const h = isLeaf ? 0 : 1 + Math.max(...children.map((c) => c.h));
    // support = explicit numeric `support` field, else a support-like Newick internal label.
    const supRaw =
      node.support !== undefined ? node.support : node.name && isSupportLabel(node.name) ? node.name : undefined;
    const support = isLeaf ? null : parseSupport(supRaw, d.support_type);
    return { name: node.name, isLeaf, cum, h, support, children, y: 0, px: 0 };
  }
  const laid = build(safeRoot, 0);

  const leaves: Laid[] = [];
  (function collect(l: Laid): void {
    if (l.isLeaf) leaves.push(l);
    else l.children.forEach(collect);
  })(laid);
  if (leaves.length === 0) return <div className="ax-fig__err">进化树为空（没有叶节点）</div>;

  function maxCumOf(l: Laid): number {
    return Math.max(l.cum, ...l.children.map(maxCumOf));
  }
  const maxCum = maxCumOf(laid);
  // any positive branch length → phylogram, else cladogram; spec.cladogram forces topology-only.
  const phylo = !d.cladogram && maxCum > 0;
  const H = laid.h; // tree height in edges (for cladogram x)

  // Auto-tint clades by the leading token of each leaf name (e.g. "Arch"/"Bact"/"Euk" domain
  // prefixes) — but ONLY when that token cleanly partitions the tree (2–6 groups, each ≥2 leaves,
  // covering ≥80% of tips), so ungrouped data (all-distinct first tokens) stays monochrome.
  const dark = isDark();
  const firstTok = (nm?: string): string => (nm ?? "").trim().split(/\s+/)[0] ?? "";
  const tokCount = new Map<string, number>();
  for (const lf of leaves) {
    const t = firstTok(lf.name);
    if (t) tokCount.set(t, (tokCount.get(t) ?? 0) + 1);
  }
  const groupTokens = [...tokCount.entries()].filter(([, c]) => c >= 2).map(([t]) => t).sort();
  const coveredByGroups = leaves.filter((lf) => tokCount.get(firstTok(lf.name))! >= 2 && groupTokens.includes(firstTok(lf.name))).length;
  const grouped = groupTokens.length >= 2 && groupTokens.length <= 6 && coveredByGroups >= 0.8 * leaves.length;
  const groupColor = new Map<string, string>();
  if (grouped) groupTokens.forEach((t, i) => groupColor.set(t, catColor(i, dark)));
  const leafColor = (nm?: string): string | null => (grouped ? groupColor.get(firstTok(nm)) ?? null : null);

  // ── layout geometry ──
  const L = leaves.length;
  const leafGap = 26;
  const topPad = d.title ? 30 : 14;
  const plotW = 360;
  const padL = 14;
  const labelX = padL + plotW + 6;
  // CJK-aware label column width (was `chars * 6.6`, which underestimated Chinese taxa names and
  // clipped them past the reserved column). Cap at 230px; truncation below is width-based too.
  const maxNameW = Math.max(20, ...leaves.map((l) => textPxWidth(l.name ?? "")));
  const labelWidth = Math.min(230, Math.max(30, maxNameW));
  const plotBottom = topPad + L * leafGap;
  const legendH = grouped ? 22 : 0;
  const bottomPad = (phylo ? 42 : 18) + legendH;
  // horizontal legend row (grouped trees only), measured so the canvas width includes it
  const legendItems: { token: string; x: number; color: string }[] = [];
  if (grouped) {
    let lx = padL;
    for (const t of groupTokens) {
      legendItems.push({ token: t, x: lx, color: groupColor.get(t)! });
      lx += 12 + Math.min(18, t.length) * 6.4 + 16;
    }
  }
  const legendRowW = legendItems.length
    ? legendItems[legendItems.length - 1].x + 12 + Math.min(18, groupTokens[groupTokens.length - 1].length) * 6.4 + 8
    : 0;
  const legendY = plotBottom + (phylo ? 50 : 30);
  const titleW = d.title ? d.title.length * 6.8 + 16 : 0;
  const w = Math.max(labelX + labelWidth + 8, Math.ceil(titleW), Math.ceil(legendRowW));
  const h = plotBottom + bottomPad;

  // assign y to leaves in traversal order, internal nodes = midpoint of children.
  let li = 0;
  (function assignY(l: Laid): number {
    if (l.isLeaf) {
      l.y = topPad + li * leafGap + leafGap / 2;
      li++;
      return l.y;
    }
    const ys = l.children.map(assignY);
    l.y = (Math.min(...ys) + Math.max(...ys)) / 2;
    return l.y;
  })(laid);

  // assign pixel x: phylogram uses cumulative length; cladogram uses (H − height) so tips align.
  (function assignPx(l: Laid): void {
    l.px = phylo
      ? padL + (maxCum > 0 ? l.cum / maxCum : 0) * plotW
      : padL + (H > 0 ? (H - l.h) / H : 0) * plotW;
    l.children.forEach(assignPx);
  })(laid);

  // flatten into drawable segments.
  type Seg = { x1: number; y1: number; x2: number; y2: number; low?: boolean };
  type VSeg = { x: number; y1: number; y2: number };
  type Tip = { px: number; y: number; name: string; cum: number };
  const hSegs: Seg[] = [];
  const vSegs: VSeg[] = [];
  const leaders: Seg[] = [];
  const tips: Tip[] = [];
  const supports: { x: number; y: number; text: string; low: boolean }[] = []; // internal-node branch support
  let lowCount = 0;
  (function walk(l: Laid, parentPx: number | null): void {
    // the branch leading INTO a weakly-supported node is dimmed/dashed (its support is the branch's).
    if (parentPx !== null) hSegs.push({ x1: parentPx, y1: l.y, x2: l.px, y2: l.y, low: l.support?.low });
    if (!l.isLeaf) {
      const ys = l.children.map((c) => c.y);
      vSegs.push({ x: l.px, y1: Math.min(...ys), y2: Math.max(...ys) });
      // A non-root internal node carries a branch-support value (bootstrap % / posterior / SH-aLRT
      // uBS) — render it (the whole point of a "with support" tree). Skip the root.
      if (parentPx !== null && l.support) {
        if (l.support.low) lowCount++;
        supports.push({ x: l.px, y: l.y, text: l.support.raw, low: l.support.low });
      }
      l.children.forEach((c) => walk(c, l.px));
    } else {
      if (l.px < labelX - 4) leaders.push({ x1: l.px, y1: l.y, x2: labelX - 4, y2: l.y });
      tips.push({ px: l.px, y: l.y, name: l.name ?? "", cum: l.cum });
    }
  })(laid, null);

  // scale bar (phylogram only): a round fraction of the max root-to-tip distance.
  const scaleLen = phylo ? niceRound(maxCum / 3) : 0;
  const scalePx = phylo && maxCum > 0 ? (scaleLen / maxCum) * plotW : 0;
  const scaleY = plotBottom + 22;

  const label = caption || d.title || "phylogenetic tree";

  // caption self-disclosure: a cladogram must not be read as distances; flag low-support de-emphasis.
  const capBase = caption || d.title || "";
  const notes: string[] = [];
  if (!phylo) notes.push("枝长不代表进化距离（分支拓扑）");
  if (lowCount > 0) notes.push("灰/虚线分支为低支持（BS<70 或 PP<0.95）");
  const capText = notes.length ? (capBase ? `${capBase} · ` : "") + notes.join("；") : capBase;

  return (
    <figure className="ax-fig">
      <div className="ax-fig__frame" style={{ overflowX: "auto", justifyContent: "flex-start" }}>
        <svg viewBox={`0 0 ${w} ${h}`} width={w} height={h} role="img" aria-label={label} style={{ maxWidth: "100%" }}>
          {d.title && (
            <text x={8} y={18} fontSize="12" fontWeight="600" fill="var(--ax-ink)">
              {d.title}
            </text>
          )}

          {/* leader lines from each tip to the aligned label column */}
          {leaders.map((s, i) => (
            <line
              key={`ld${i}`}
              x1={s.x1}
              y1={s.y1}
              x2={s.x2}
              y2={s.y2}
              stroke="var(--ax-ink-faint)"
              strokeWidth="0.7"
              strokeDasharray="1 2.5"
            />
          ))}

          {/* vertical connectors (parent bars) */}
          {vSegs.map((s, i) => (
            <line key={`v${i}`} x1={s.x} y1={s.y1} x2={s.x} y2={s.y2} stroke="var(--ax-ink)" strokeWidth="1.3" />
          ))}

          {/* internal-node branch support (bootstrap / posterior / SH-aLRT uBS), above-left of node;
              low support (BS<70 or PP<0.95) is greyed to read as tentative */}
          {supports.map((sp, i) => (
            <text
              key={`sup${i}`}
              x={sp.x - 3}
              y={sp.y - 3.5}
              fontSize="7.5"
              textAnchor="end"
              fill={sp.low ? "var(--ax-ink-faint)" : "var(--ax-ink-soft)"}
            >
              {truncate(sp.text, 11)}
              <title>{`支持值 ${sp.text}${sp.low ? "（低支持）" : ""}`}</title>
            </text>
          ))}
          {/* horizontal branches — low-support branches dimmed + dashed (colour+linetype redundant) */}
          {hSegs.map((s, i) => (
            <line
              key={`hb${i}`}
              x1={s.x1}
              y1={s.y1}
              x2={s.x2}
              y2={s.y2}
              stroke={s.low ? "var(--ax-ink-faint)" : "var(--ax-ink)"}
              strokeWidth="1.3"
              strokeLinecap="round"
              strokeDasharray={s.low ? "3 2" : undefined}
            />
          ))}

          {/* tips: node dot + aligned label, tinted by clade group when the tree partitions cleanly */}
          {tips.map((t, i) => {
            const col = leafColor(t.name) ?? "var(--ax-ink)";
            // latin binomials rendered in italic (nomenclature convention); spec.italicize_tips forces it.
            const ital =
              d.italicize_tips === false ? false : d.italicize_tips === true ? true : looksBinomial(t.name);
            return (
              <g key={`t${i}`}>
                <circle cx={t.px} cy={t.y} r={1.9} fill={col} />
                <text x={labelX} y={t.y + 3.5} fontSize="11" fill={col} fontStyle={ital ? "italic" : undefined}>
                  {truncateToWidth(t.name || "(未命名)", labelWidth)}
                  <title>{phylo ? `${t.name || "(未命名)"} · 根到叶 ${fmt(t.cum)}` : t.name || "(未命名)"}</title>
                </text>
              </g>
            );
          })}

          {/* scale bar (phylogram only) */}
          {phylo && scalePx > 0 && (
            <g>
              <line x1={padL} y1={scaleY} x2={padL + scalePx} y2={scaleY} stroke="var(--ax-line)" strokeWidth="1" />
              <line x1={padL} y1={scaleY - 3} x2={padL} y2={scaleY + 3} stroke="var(--ax-line)" strokeWidth="1" />
              <line
                x1={padL + scalePx}
                y1={scaleY - 3}
                x2={padL + scalePx}
                y2={scaleY + 3}
                stroke="var(--ax-line)"
                strokeWidth="1"
              />
              <text x={padL + scalePx / 2} y={scaleY + 13} fontSize="9" textAnchor="middle" fill="var(--ax-ink-faint)">
                {trimZeros(fmt(scaleLen))} 枝长
              </text>
            </g>
          )}
          {!phylo && (
            <text x={padL} y={plotBottom + 12} fontSize="9" fill="var(--ax-ink-faint)">
              分支拓扑（无枝长）
            </text>
          )}

          {/* clade legend (grouped trees only) — identity is never color-alone: swatch + token */}
          {legendItems.map((it, i) => (
            <g key={`lg${i}`}>
              <rect x={it.x} y={legendY - 7} width="9" height="9" rx="1.5" fill={it.color} />
              <text x={it.x + 13} y={legendY + 1} fontSize="9" fill="var(--ax-ink-soft)">
                {truncate(it.token, 18)}
              </text>
            </g>
          ))}
        </svg>
      </div>
      <figcaption className="ax-fig__cap">
        <span className="ax-fig__kind">进化树 · tree</span> {capText}
      </figcaption>
    </figure>
  );
}
