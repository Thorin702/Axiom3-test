// Native phylogenetic tree / dendrogram: a rooted, horizontal rectangular tree. With branch
// lengths it draws a phylogram (x = cumulative length from root, plus a scale bar); without
// lengths it falls back to an evenly-spaced cladogram (topology only). Tips are labeled in an
// aligned column on the right with faint leader lines. Pure inline SVG, no dependency.
// Neutral by construction: it draws only the emitted topology + branch lengths, never a verdict.
//
// Spec (```tree JSON): {"title"?, "newick"?: "((A:0.1,B:0.2):0.05,C:0.3);",
//   "root"?: {"name"?, "length"?, "children":[ ...nested... ]}}  — provide newick OR root.

type TreeNode = { name?: string; length?: number; children: TreeNode[] };
type RawNode = { name?: unknown; length?: unknown; children?: unknown };
type Spec = { title?: string; newick?: string; root?: RawNode };

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
  return v.toFixed(3);
}
function trimZeros(s: string): string {
  return s.includes(".") ? s.replace(/\.?0+$/, "") : s;
}
function truncate(s: string, max: number): string {
  if (max <= 1) return s.length > 1 ? "…" : s;
  return s.length > max ? s.slice(0, max - 1) + "…" : s;
}
function niceRound(x: number): number {
  if (!(x > 0)) return 1;
  const exp = Math.floor(Math.log10(x));
  const f = x / Math.pow(10, exp);
  const nf = f < 1.5 ? 1 : f < 3 ? 2 : f < 7 ? 5 : 10;
  return nf * Math.pow(10, exp);
}

// laid-out node (data + pixel coords), computed client-side so nothing is fabricated.
type Laid = {
  name?: string;
  isLeaf: boolean;
  cum: number; // cumulative branch length from root (lengths clamped ≥ 0)
  h: number; // height in edges to the furthest leaf below (0 for a leaf)
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
    return { name: node.name, isLeaf, cum, h, children, y: 0, px: 0 };
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
  const phylo = maxCum > 0; // any positive branch length → phylogram, else cladogram
  const H = laid.h; // tree height in edges (for cladogram x)

  // ── layout geometry ──
  const L = leaves.length;
  const leafGap = 26;
  const topPad = d.title ? 30 : 14;
  const plotW = 360;
  const padL = 14;
  const labelX = padL + plotW + 6;
  const maxNameLen = Math.max(3, ...leaves.map((l) => (l.name ?? "").length));
  const labelWidth = Math.min(230, Math.max(30, maxNameLen * 6.6));
  const maxLabelChars = Math.max(4, Math.floor(labelWidth / 6.6));
  const plotBottom = topPad + L * leafGap;
  const bottomPad = phylo ? 42 : 18;
  const titleW = d.title ? d.title.length * 6.8 + 16 : 0;
  const w = Math.max(labelX + labelWidth + 8, Math.ceil(titleW));
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
  type Seg = { x1: number; y1: number; x2: number; y2: number };
  type VSeg = { x: number; y1: number; y2: number };
  type Tip = { px: number; y: number; name: string; cum: number };
  const hSegs: Seg[] = [];
  const vSegs: VSeg[] = [];
  const leaders: Seg[] = [];
  const tips: Tip[] = [];
  (function walk(l: Laid, parentPx: number | null): void {
    if (parentPx !== null) hSegs.push({ x1: parentPx, y1: l.y, x2: l.px, y2: l.y });
    if (!l.isLeaf) {
      const ys = l.children.map((c) => c.y);
      vSegs.push({ x: l.px, y1: Math.min(...ys), y2: Math.max(...ys) });
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
              stroke="var(--ax-line-soft)"
              strokeWidth="0.5"
              strokeDasharray="1 3"
            />
          ))}

          {/* vertical connectors (parent bars) */}
          {vSegs.map((s, i) => (
            <line key={`v${i}`} x1={s.x} y1={s.y1} x2={s.x} y2={s.y2} stroke="var(--ax-ink)" strokeWidth="1.3" />
          ))}
          {/* horizontal branches */}
          {hSegs.map((s, i) => (
            <line
              key={`hb${i}`}
              x1={s.x1}
              y1={s.y1}
              x2={s.x2}
              y2={s.y2}
              stroke="var(--ax-ink)"
              strokeWidth="1.3"
              strokeLinecap="round"
            />
          ))}

          {/* tips: node dot + aligned label with a native tooltip */}
          {tips.map((t, i) => (
            <g key={`t${i}`}>
              <circle cx={t.px} cy={t.y} r={1.9} fill="var(--ax-ink)" />
              <text x={labelX} y={t.y + 3.5} fontSize="11" fill="var(--ax-ink)">
                {truncate(t.name || "(未命名)", maxLabelChars)}
                <title>{phylo ? `${t.name || "(未命名)"} · 根到叶 ${fmt(t.cum)}` : t.name || "(未命名)"}</title>
              </text>
            </g>
          ))}

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
        </svg>
      </div>
      <figcaption className="ax-fig__cap">
        <span className="ax-fig__kind">进化树 · tree</span> {caption || d.title || ""}
      </figcaption>
    </figure>
  );
}
