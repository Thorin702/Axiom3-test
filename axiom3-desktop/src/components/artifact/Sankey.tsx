// Native Sankey flow diagram: quantities moving between stages (mass/energy balance, budgets,
// cohort funnels, sample disposition). Nodes are layered by longest-path depth, stacked per
// layer with bar height ∝ throughput; cubic-bezier ribbons have width ∝ value and take their
// SOURCE node's categorical color. Cycle/missing-node links are dropped, never drawn.
//
// Spec (```sankey JSON): {"title"?, "nodes":[string | {"id":string}, ...],
//   "links":[{"source":<name|idx>,"target":<name|idx>,"value":number}, ...]}
import { useMemo } from "react";

type Spec = { title?: unknown; nodes?: unknown; links?: unknown };

// categorical identity palette (fixed order; a source node keeps its slot, ribbons inherit it).
const CAT_LIGHT = ["#2a78d6", "#1baf7a", "#eda100", "#008300", "#4a3aa7", "#e34948", "#e87ba4", "#eb6834"];
const CAT_DARK = ["#3987e5", "#199e70", "#c98500", "#008300", "#9085e9", "#e66767", "#d55181", "#d95926"];
function isDark(): boolean {
  const t = document.documentElement.dataset.theme;
  if (t === "dark") return true;
  if (t === "light") return false;
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? false;
}

// compact numeric label: round to 2dp, drop trailing zeros, group the integer part with commas.
function fmtNum(v: number): string {
  if (!Number.isFinite(v)) return "—";
  const r = Math.round(v * 100) / 100;
  const s = Number.isInteger(r) ? String(r) : r.toFixed(2).replace(/\.?0+$/, "");
  const dot = s.indexOf(".");
  const intPart = dot < 0 ? s : s.slice(0, dot);
  const rest = dot < 0 ? "" : s.slice(dot);
  const sign = intPart.startsWith("-") ? "-" : "";
  const digits = sign ? intPart.slice(1) : intPart;
  return sign + digits.replace(/\B(?=(\d{3})+(?!\d))/g, ",") + rest;
}

// rough text width at a given font size (CJK ≈ full em, latin/digits ≈ 0.58 em).
function textW(s: string, size: number): number {
  let w = 0;
  for (const ch of s) w += ch.charCodeAt(0) > 0x2e80 ? size : size * 0.58;
  return w;
}
function clamp(v: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, v));
}

function resolveRef(ref: unknown, idIndex: Map<string, number>, n: number): number | undefined {
  if (typeof ref === "string") {
    const k = ref.trim();
    return idIndex.has(k) ? idIndex.get(k) : undefined;
  }
  if (typeof ref === "number" && Number.isInteger(ref) && ref >= 0 && ref < n) return ref;
  return undefined;
}

type RNode = {
  name: string;
  x: number;
  y: number;
  w: number;
  h: number;
  color: number; // palette slot (-1 = beyond palette → neutral)
  side: "L" | "R";
  through: number;
  inSum: number;
  outSum: number;
};
type RLink = { path: string; color: number; tip: string; area: number };
type Layout = {
  title: string;
  w: number;
  h: number;
  nodes: RNode[];
  links: RLink[];
};
type Result = { error: string } | Layout;

const NODE_W = 16;
const LINK_W = 150;

// can `from` already reach `to` through the accepted edges? (used to reject cycle-forming links)
function reaches(from: number, to: number, adj: number[][]): boolean {
  if (from === to) return true;
  const seen = new Set<number>([from]);
  const stack = [from];
  while (stack.length) {
    const u = stack.pop() as number;
    for (const v of adj[u]) {
      if (v === to) return true;
      if (!seen.has(v)) {
        seen.add(v);
        stack.push(v);
      }
    }
  }
  return false;
}

function build(raw: string): Result {
  let d: Spec;
  try {
    d = JSON.parse(raw.trim());
  } catch (e) {
    return { error: `桑基图规范不是有效 JSON · ${(e as Error).message}` };
  }
  if (!d || typeof d !== "object" || !Array.isArray(d.nodes) || !Array.isArray(d.links)) {
    return { error: "桑基图需要 nodes 数组与 links 数组" };
  }

  // ── nodes: accept "name" or {id}; dedupe by id, keep first appearance. ──
  const idIndex = new Map<string, number>();
  const names: string[] = [];
  for (const entry of d.nodes) {
    let id = "";
    if (typeof entry === "string") id = entry.trim();
    else if (entry && typeof entry === "object" && typeof (entry as { id?: unknown }).id === "string")
      id = (entry as { id: string }).id.trim();
    if (!id || idIndex.has(id)) continue;
    idIndex.set(id, names.length);
    names.push(id);
  }
  const n = names.length;
  if (n === 0) return { error: "桑基图无有效节点（每个节点需要非空 name/id）" };

  // ── links: drop malformed / self-loop / non-positive / missing-endpoint / cycle-forming. ──
  type VLink = { s: number; t: number; value: number };
  const links: VLink[] = [];
  const adj: number[][] = names.map(() => []);
  const inSum = new Array<number>(n).fill(0);
  const outSum = new Array<number>(n).fill(0);
  const preds: number[][] = names.map(() => []);
  for (const l of d.links) {
    if (!l || typeof l !== "object") continue;
    const s = resolveRef((l as { source?: unknown }).source, idIndex, n);
    const t = resolveRef((l as { target?: unknown }).target, idIndex, n);
    const value = Number((l as { value?: unknown }).value);
    if (s === undefined || t === undefined || s === t || !Number.isFinite(value) || value <= 0) continue;
    if (reaches(t, s, adj)) continue; // would introduce a cycle — this diagram is acyclic by construction
    links.push({ s, t, value });
    adj[s].push(t);
    preds[t].push(s);
    outSum[s] += value;
    inSum[t] += value;
  }
  if (links.length === 0)
    return { error: "桑基图无有效连接（links 需引用已声明的节点，且 value 为正数）" };

  // ── layer = longest path from a source (DAG, so a memoised DFS terminates). ──
  const layer = new Array<number>(n).fill(0);
  const done = new Array<boolean>(n).fill(false);
  const active = new Array<boolean>(n).fill(false);
  function depth(u: number): number {
    if (done[u]) return layer[u];
    if (active[u]) return 0; // safety net — cannot happen on an acyclic graph
    active[u] = true;
    let best = 0;
    for (const p of preds[u]) best = Math.max(best, depth(p) + 1);
    active[u] = false;
    done[u] = true;
    layer[u] = best;
    return best;
  }
  for (let i = 0; i < n; i++) depth(i);

  // throughput; only nodes touched by a valid link are drawn.
  const through = new Array<number>(n).fill(0);
  const visible: boolean[] = new Array<boolean>(n).fill(false);
  for (let i = 0; i < n; i++) {
    if (inSum[i] > 0 || outSum[i] > 0) {
      visible[i] = true;
      through[i] = Math.max(inSum[i], outSum[i]);
    }
  }

  // ── compact the used layer values into contiguous column indices. ──
  const usedLayers = Array.from(
    new Set(names.map((_, i) => (visible[i] ? layer[i] : -1)).filter((v) => v >= 0)),
  ).sort((a, b) => a - b);
  const colOf = new Map<number, number>();
  usedLayers.forEach((lv, ci) => colOf.set(lv, ci));
  const numCols = usedLayers.length;

  // column node lists (input order preserved by iterating 0..n).
  const colNodes: number[][] = Array.from({ length: numCols }, () => []);
  for (let i = 0; i < n; i++) {
    if (!visible[i]) continue;
    colNodes[colOf.get(layer[i]) as number].push(i);
  }

  const maxColCount = Math.max(...colNodes.map((c) => c.length));
  const colSum = colNodes.map((c) => c.reduce((a, i) => a + through[i], 0));
  const maxColSum = Math.max(...colSum, 1);

  // ── vertical scale: pick ky so every column (bars + gaps) fits the plot height. ──
  const plotH = clamp(maxColCount * 90, 260, 560);
  let gap = 18;
  if (maxColCount > 1 && (maxColCount - 1) * gap > plotH * 0.5) gap = (plotH * 0.5) / (maxColCount - 1);
  let ky = Infinity;
  colNodes.forEach((c, ci) => {
    if (colSum[ci] <= 0) return;
    const avail = plotH - (c.length - 1) * gap;
    ky = Math.min(ky, avail / colSum[ci]);
  });
  if (!Number.isFinite(ky) || ky <= 0) ky = plotH / maxColSum;

  // ── padding driven by first-column (left labels) and last-column (right labels) text. ──
  const labelWidth = (idx: number): number => Math.max(textW(names[idx], 11), textW(fmtNum(through[idx]), 9));
  const firstCol = colNodes[0];
  const lastCol = colNodes[numCols - 1];
  const padL = clamp(Math.max(0, ...firstCol.map(labelWidth)) + 12, 26, 220);
  const padR = clamp(Math.max(0, ...lastCol.map(labelWidth)) + 12, 32, 220);
  const title = typeof d.title === "string" ? d.title : "";
  const padT = title ? 30 : 14;
  const padB = 16;
  const plotTop = padT;

  // ── place nodes: x by column, y by centered stack within the column. ──
  const nodes: RNode[] = [];
  const nodeById = new Map<number, RNode>();
  const nx = (col: number) => padL + col * (NODE_W + LINK_W);
  colNodes.forEach((c, ci) => {
    const total = c.reduce((a, i) => a + through[i] * ky, 0) + (c.length - 1) * gap;
    let y = plotTop + (plotH - total) / 2;
    for (const i of c) {
      const h = through[i] * ky;
      const node: RNode = {
        name: names[i],
        x: nx(ci),
        y,
        w: NODE_W,
        h,
        color: i < CAT_LIGHT.length ? i : -1,
        side: ci === 0 ? "L" : "R",
        through: through[i],
        inSum: inSum[i],
        outSum: outSum[i],
      };
      nodes.push(node);
      nodeById.set(i, node);
      y += h + gap;
    }
  });

  // ── assign each link a band on its source's right edge and its target's left edge, ordered
  //    so ribbons enter/leave in the vertical order of the nodes they connect (fewer crossings). ──
  type Idx = { link: VLink; sy0: number; sy1: number; ty0: number; ty1: number };
  const banded: Idx[] = links.map((link) => ({ link, sy0: 0, sy1: 0, ty0: 0, ty1: 0 }));
  const outByNode = new Map<number, Idx[]>();
  const inByNode = new Map<number, Idx[]>();
  for (const b of banded) {
    (outByNode.get(b.link.s) ?? outByNode.set(b.link.s, []).get(b.link.s)!).push(b);
    (inByNode.get(b.link.t) ?? inByNode.set(b.link.t, []).get(b.link.t)!).push(b);
  }
  const centerY = (i: number): number => {
    const nd = nodeById.get(i);
    return nd ? nd.y + nd.h / 2 : 0;
  };
  outByNode.forEach((list, s) => {
    list.sort((a, b) => centerY(a.link.t) - centerY(b.link.t) || a.link.t - b.link.t);
    let top = (nodeById.get(s) as RNode).y;
    for (const b of list) {
      const th = b.link.value * ky;
      b.sy0 = top;
      b.sy1 = top + th;
      top += th;
    }
  });
  inByNode.forEach((list, t) => {
    list.sort((a, b) => centerY(a.link.s) - centerY(b.link.s) || a.link.s - b.link.s);
    let top = (nodeById.get(t) as RNode).y;
    for (const b of list) {
      const th = b.link.value * ky;
      b.ty0 = top;
      b.ty1 = top + th;
      top += th;
    }
  });

  // ── ribbon paths (cubic bezier, control points at the horizontal midpoint). ──
  const rlinks: RLink[] = banded.map((b) => {
    const src = nodeById.get(b.link.s) as RNode;
    const tgt = nodeById.get(b.link.t) as RNode;
    const sx = src.x + src.w;
    const tx = tgt.x;
    const xm = (sx + tx) / 2;
    const path =
      `M${sx.toFixed(2)},${b.sy0.toFixed(2)} ` +
      `C${xm.toFixed(2)},${b.sy0.toFixed(2)} ${xm.toFixed(2)},${b.ty0.toFixed(2)} ${tx.toFixed(2)},${b.ty0.toFixed(2)} ` +
      `L${tx.toFixed(2)},${b.ty1.toFixed(2)} ` +
      `C${xm.toFixed(2)},${b.ty1.toFixed(2)} ${xm.toFixed(2)},${b.sy1.toFixed(2)} ${sx.toFixed(2)},${b.sy1.toFixed(2)} Z`;
    return {
      path,
      color: src.color,
      tip: `${src.name} → ${tgt.name} · ${fmtNum(b.link.value)}`,
      area: b.link.value,
    };
  });
  // draw thick ribbons first so thin ones stay visible on top.
  rlinks.sort((a, b) => b.area - a.area);

  const contentW = padL + (numCols - 1) * (NODE_W + LINK_W) + NODE_W + padR;
  const w = Math.max(contentW, title ? Math.ceil(textW(title, 12)) + 20 : 0);
  const h = plotTop + plotH + padB;

  return { title, w, h, nodes, links: rlinks };
}

export function Sankey({ spec, caption }: { spec: string; caption?: string }) {
  const res = useMemo(() => build(spec), [spec]);
  if ("error" in res) return <div className="ax-fig__err">{res.error}</div>;

  const cat = isDark() ? CAT_DARK : CAT_LIGHT;
  const color = (slot: number): string => (slot >= 0 && slot < cat.length ? cat[slot] : "var(--ax-ink-faint)");
  const label = caption || res.title || "sankey diagram";

  return (
    <figure className="ax-fig">
      <div className="ax-fig__frame" style={{ overflowX: "auto", justifyContent: "flex-start" }}>
        <svg
          viewBox={`0 0 ${res.w} ${res.h}`}
          width={res.w}
          height={res.h}
          role="img"
          aria-label={label}
          style={{ maxWidth: "100%" }}
        >
          {res.title && (
            <text x={8} y={18} fontSize="12" fontWeight="600" fill="var(--ax-ink)">
              {res.title}
            </text>
          )}

          {/* ribbons (under the node bars) */}
          {res.links.map((l, i) => (
            <path key={`l${i}`} d={l.path} fill={color(l.color)} fillOpacity={0.42} stroke="none">
              <title>{l.tip}</title>
            </path>
          ))}

          {/* node bars */}
          {res.nodes.map((nd, i) => (
            <rect
              key={`n${i}`}
              x={nd.x}
              y={nd.y}
              width={nd.w}
              height={Math.max(nd.h, 1.5)}
              rx={2.5}
              fill={color(nd.color)}
              fillOpacity={0.92}
              stroke="var(--ax-surface)"
              strokeWidth={1}
            >
              <title>{`${nd.name} · 流入 ${fmtNum(nd.inSum)} · 流出 ${fmtNum(nd.outSum)}`}</title>
            </rect>
          ))}

          {/* node labels (name + throughput), haloed so they read over ribbons */}
          {res.nodes.map((nd, i) => {
            const cy = nd.y + nd.h / 2;
            const lx = nd.side === "L" ? nd.x - 7 : nd.x + nd.w + 7;
            const anchor = nd.side === "L" ? "end" : "start";
            return (
              <g key={`t${i}`}>
                <text
                  x={lx}
                  y={cy - 1}
                  fontSize="11"
                  textAnchor={anchor}
                  fill="var(--ax-ink)"
                  stroke="var(--ax-surface)"
                  strokeWidth={3}
                  strokeLinejoin="round"
                  style={{ paintOrder: "stroke" }}
                >
                  {nd.name}
                </text>
                <text
                  x={lx}
                  y={cy + 10}
                  fontSize="9"
                  textAnchor={anchor}
                  fill="var(--ax-ink-faint)"
                  stroke="var(--ax-surface)"
                  strokeWidth={3}
                  strokeLinejoin="round"
                  style={{ paintOrder: "stroke", fontVariantNumeric: "tabular-nums" }}
                >
                  {fmtNum(nd.through)}
                </text>
              </g>
            );
          })}
        </svg>
      </div>
      <figcaption className="ax-fig__cap">
        <span className="ax-fig__kind">桑基图 · sankey</span> {caption || res.title || ""}
      </figcaption>
    </figure>
  );
}
