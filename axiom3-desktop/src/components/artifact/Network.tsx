// Native interaction / regulatory network: node-link biology (PPI, gene-regulatory circuits,
// co-expression modules, KG neighborhoods) that mermaid cannot draw. Layout is a DETERMINISTIC
// seeded Fruchterman-Reingold pass in a useMemo (fixed seed + fixed iterations → identical every
// render). Node radius = √value, fill by `group` (categorical palette + legend); edge stroke-width
// = weight; when `directed`, sign "+" draws an activation arrowhead ▸ and "-" a repression bar ⊣.
// It only lays out the supplied topology — it asserts nothing beyond the emitted nodes/edges.
//
// Spec (```network JSON): {"title"?, "directed"?: bool,
//   "nodes":[{"id":string,"group"?:string,"value"?:number}],
//   "edges":[{"source":string,"target":string,"weight"?:number,"sign"?:"+"|"-"}]}
import { useMemo } from "react";

type NodeSpec = { id?: unknown; group?: unknown; value?: unknown };
type EdgeSpec = { source?: unknown; target?: unknown; weight?: unknown; sign?: unknown };
type Spec = { title?: string; directed?: boolean; nodes?: NodeSpec[]; edges?: EdgeSpec[] };

// categorical identity palette (fixed order, indexed by group first-appearance; never cycled).
const CAT_LIGHT = ["#2a78d6", "#1baf7a", "#eda100", "#008300", "#4a3aa7", "#e34948", "#e87ba4", "#eb6834"];
const CAT_DARK = ["#3987e5", "#199e70", "#c98500", "#008300", "#9085e9", "#e66767", "#d55181", "#d95926"];
function isDark(): boolean {
  const t = document.documentElement.dataset.theme;
  if (t === "dark") return true;
  if (t === "light") return false;
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? false;
}

// deterministic PRNG (mulberry32) — fixed seed keeps the layout stable across re-renders.
function mulberry32(seed: number): () => number {
  let s = seed >>> 0;
  return () => {
    s = (s + 0x6d2b79f5) | 0;
    let t = Math.imul(s ^ (s >>> 15), 1 | s);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function fmt(v: number): string {
  if (!Number.isFinite(v)) return "—";
  const a = Math.abs(v);
  if (a >= 100) return v.toFixed(0);
  if (a >= 1) return String(Math.round(v * 100) / 100);
  return String(Math.round(v * 1000) / 1000);
}

type Head = "arrow" | "bar" | "none";
type LNode = { id: string; group: string; gi: number; value: number; x: number; y: number; r: number; degree: number };
type LEdge = { path: string; head: Head; hx: number; hy: number; dx: number; dy: number; sw: number; tip: string; loop: boolean };
type Result =
  | { error: string }
  | {
      title: string;
      w: number;
      h: number;
      padL: number;
      padT: number;
      drawW: number;
      drawH: number;
      nodes: LNode[];
      edges: LEdge[];
      groups: string[];
      directed: boolean;
      arrowLabel: string | null;
      hasRep: boolean;
    };

function headType(directed: boolean, sign: string): Head {
  if (!directed) return "none";
  const s = sign.trim().toLowerCase();
  if (s === "-" || s === "−" || s.startsWith("repress") || s.startsWith("inhibit") || s.startsWith("neg") || s === "|") return "bar";
  return "arrow"; // "+", "activate", or an unspecified directed edge → directional arrowhead
}

function build(raw: string): Result {
  let d: Spec;
  try {
    d = JSON.parse(raw.trim());
  } catch (e) {
    return { error: `网络图规范不是有效 JSON · ${(e as Error).message}` };
  }
  if (!d || typeof d !== "object" || !Array.isArray(d.nodes)) {
    return { error: "网络图需要 nodes 数组（每个节点含 id）" };
  }

  // ── validate nodes: id must be a non-empty string; dedupe by id (keep first). ──
  const idIndex = new Map<string, number>();
  const nodes: LNode[] = [];
  for (const ns of d.nodes) {
    if (!ns || typeof ns !== "object") continue;
    const id = typeof ns.id === "string" ? ns.id.trim() : "";
    if (!id || idIndex.has(id)) continue;
    const group = typeof ns.group === "string" && ns.group.trim() ? ns.group.trim() : "—";
    const value = Number.isFinite(ns.value) && (ns.value as number) > 0 ? (ns.value as number) : 1;
    idIndex.set(id, nodes.length);
    nodes.push({ id, group, gi: 0, value, x: 0, y: 0, r: 0, degree: 0 });
  }
  if (nodes.length === 0) return { error: "网络图无有效节点（每个节点需要非空 id）" };

  // ── group → color slot in first-appearance order ──
  const groups: string[] = [];
  for (const nd of nodes) {
    let gi = groups.indexOf(nd.group);
    if (gi < 0) {
      gi = groups.length;
      groups.push(nd.group);
    }
    nd.gi = gi;
  }

  const directed = d.directed === true;

  // ── validate edges: endpoints must resolve to real nodes; drop the rest. ──
  type VEdge = { s: number; t: number; weight: number; head: Head; sign: string };
  const rawEdges = Array.isArray(d.edges) ? d.edges : [];
  const vedges: VEdge[] = [];
  const dirKey = new Set<string>();
  let maxW = 0;
  let arrowSigned = false;
  let hasRep = false;
  for (const e of rawEdges) {
    if (!e || typeof e !== "object") continue;
    const su = typeof e.source === "string" ? e.source.trim() : "";
    const tu = typeof e.target === "string" ? e.target.trim() : "";
    const s = idIndex.get(su);
    const t = idIndex.get(tu);
    if (s === undefined || t === undefined) continue;
    const weight = Number.isFinite(e.weight) && (e.weight as number) > 0 ? (e.weight as number) : 0.6;
    const sign = typeof e.sign === "string" ? e.sign : "";
    const head = headType(directed, sign);
    if (head === "bar") hasRep = true;
    if (head === "arrow" && (sign.trim() === "+" || sign.trim().toLowerCase().startsWith("activ") || sign.trim().toLowerCase().startsWith("pos"))) arrowSigned = true;
    maxW = Math.max(maxW, weight);
    nodes[s].degree++;
    nodes[t].degree++;
    if (directed) dirKey.add(`${s} ${t}`);
    vedges.push({ s, t, weight, head, sign });
  }
  if (maxW <= 0) maxW = 1;

  // ── node radii: √value scaled into [rMin, rMax] (flat when no variation). ──
  const rMin = 7;
  const rMax = 21;
  const vs = nodes.map((n) => n.value);
  const vMin = Math.min(...vs);
  const vMax = Math.max(...vs);
  const sqLo = Math.sqrt(vMin);
  const sqHi = Math.sqrt(vMax);
  for (const nd of nodes) {
    nd.r = sqHi > sqLo ? rMin + ((Math.sqrt(nd.value) - sqLo) / (sqHi - sqLo)) * (rMax - rMin) : 12;
  }
  const maxR = Math.max(...nodes.map((n) => n.r));

  // ── deterministic Fruchterman-Reingold in an abstract sim box ──
  const n = nodes.length;
  const simW = 800;
  const simH = 600;
  const cx0 = simW / 2;
  const cy0 = simH / 2;
  const area = simW * simH;
  const k = 0.75 * Math.sqrt(area / n); // ideal edge length
  const rng = mulberry32(1337); // fixed seed → stable layout for a given topology
  const px = new Array<number>(n);
  const py = new Array<number>(n);
  const initR = Math.min(simW, simH) * 0.32;
  for (let i = 0; i < n; i++) {
    const ang = (2 * Math.PI * i) / n;
    const rr = initR * (0.55 + 0.9 * rng());
    px[i] = cx0 + Math.cos(ang) * rr;
    py[i] = cy0 + Math.sin(ang) * rr;
  }
  const ITER = n <= 40 ? 320 : 180;
  const temp0 = simW * 0.1;
  const dx = new Array<number>(n);
  const dy = new Array<number>(n);
  for (let it = 0; it < ITER; it++) {
    for (let i = 0; i < n; i++) {
      dx[i] = 0;
      dy[i] = 0;
    }
    // repulsion between every pair
    for (let i = 0; i < n; i++) {
      for (let j = i + 1; j < n; j++) {
        let ddx = px[i] - px[j];
        let ddy = py[i] - py[j];
        let dist = Math.hypot(ddx, ddy);
        if (dist < 0.01) {
          ddx = (rng() - 0.5) * 0.1;
          ddy = (rng() - 0.5) * 0.1;
          dist = 0.01;
        }
        const f = (k * k) / dist;
        const ux = ddx / dist;
        const uy = ddy / dist;
        dx[i] += ux * f;
        dy[i] += uy * f;
        dx[j] -= ux * f;
        dy[j] -= uy * f;
      }
    }
    // attraction along edges (heavier weight → slightly stronger pull; layout only)
    for (const e of vedges) {
      if (e.s === e.t) continue;
      let ddx = px[e.s] - px[e.t];
      let ddy = py[e.s] - py[e.t];
      let dist = Math.hypot(ddx, ddy);
      if (dist < 0.01) dist = 0.01;
      const f = ((dist * dist) / k) * (0.5 + 0.5 * (e.weight / maxW));
      const ux = (ddx / dist) * f;
      const uy = (ddy / dist) * f;
      dx[e.s] -= ux;
      dy[e.s] -= uy;
      dx[e.t] += ux;
      dy[e.t] += uy;
    }
    // mild gravity toward center so disconnected nodes stay in frame
    for (let i = 0; i < n; i++) {
      dx[i] += (cx0 - px[i]) * 0.035;
      dy[i] += (cy0 - py[i]) * 0.035;
    }
    // cap displacement by cooling temperature
    const temp = temp0 * (1 - it / ITER);
    for (let i = 0; i < n; i++) {
      const disp = Math.hypot(dx[i], dy[i]);
      if (disp > 0) {
        const lim = Math.min(disp, temp);
        px[i] += (dx[i] / disp) * lim;
        py[i] += (dy[i] / disp) * lim;
      }
    }
  }

  // ── fit the layout into the drawing box (preserve aspect ratio; margin for radii + labels) ──
  const drawW = Math.round(Math.min(760, Math.max(400, 86 * Math.sqrt(n))));
  const drawH = Math.round(Math.max(300, drawW * 0.7));
  const padL = 12;
  const padT = d.title ? 30 : 14;
  const padB = 14;
  const margin = maxR + 22; // room for the node circle + its label below
  const minX = Math.min(...px);
  const maxX = Math.max(...px);
  const minY = Math.min(...py);
  const maxY = Math.max(...py);
  const spanX = maxX - minX || 1;
  const spanY = maxY - minY || 1;
  const availW = drawW - 2 * margin;
  const availH = drawH - 2 * margin;
  const scale = Math.min(availW / spanX, availH / spanY);
  const offX = padL + margin + (availW - spanX * scale) / 2;
  const offY = padT + margin + (availH - spanY * scale) / 2;
  for (let i = 0; i < n; i++) {
    nodes[i].x = offX + (px[i] - minX) * scale;
    nodes[i].y = offY + (py[i] - minY) * scale;
  }

  // right-side legend column: width from the longest group label AND (when directed) the
  // edge-semantics rows; height must cover the whole legend so nothing clips at the viewBox edge.
  const arrowLabel = directed ? (arrowSigned ? "激活 activation" : "指向 directed") : null;
  const legendChars = Math.max(4, ...groups.map((g) => g.length));
  const edgeLegendMin = directed ? 138 : 0; // room for "激活 activation" / "抑制 repression"
  const padR = Math.round(Math.min(210, Math.max(96, legendChars * 6.6 + 30, edgeLegendMin)));
  const w = padL + drawW + padR;
  const groupTop = padT + 4;
  let legendBottom = groupTop + groups.length * 17 + 4;
  if (directed) {
    const edgeBase = groupTop + groups.length * 17 + 12;
    legendBottom = edgeBase + (hasRep ? 34 : arrowLabel ? 18 : 0);
  }
  const h = Math.max(padT + drawH + padB, legendBottom + padB);

  // ── edge geometry (reciprocal directed pairs bow apart; self-loops arc above) ──
  const edges: LEdge[] = vedges.map((e) => {
    const A = nodes[e.s];
    const B = nodes[e.t];
    const sw = 0.9 + (e.weight / maxW) * 2.4;
    const tipSign = e.sign ? ` · ${e.sign}` : "";
    const tip = `${A.id} ${directed ? "→" : "—"} ${B.id} · weight ${fmt(e.weight)}${tipSign}`;

    if (e.s === e.t) {
      // self-loop above the node
      const r = A.r;
      const a1 = (-125 * Math.PI) / 180;
      const a2 = (-55 * Math.PI) / 180;
      const sx = A.x + Math.cos(a1) * r;
      const sy = A.y + Math.sin(a1) * r;
      const ex = A.x + Math.cos(a2) * r;
      const ey = A.y + Math.sin(a2) * r;
      const c1x = A.x - r * 1.3;
      const c1y = A.y - r * 3 - 8;
      const c2x = A.x + r * 1.3;
      const c2y = A.y - r * 3 - 8;
      let hdx = A.x - ex;
      let hdy = A.y - ey;
      const hl = Math.hypot(hdx, hdy) || 1;
      hdx /= hl;
      hdy /= hl;
      return { path: `M${sx} ${sy} C${c1x} ${c1y} ${c2x} ${c2y} ${ex} ${ey}`, head: e.head, hx: ex, hy: ey, dx: hdx, dy: hdy, sw, tip, loop: true };
    }

    const reciprocal = directed && dirKey.has(`${e.t} ${e.s}`);
    const bow = reciprocal ? (e.s < e.t ? 16 : -16) : 0;
    const lx = B.x - A.x;
    const ly = B.y - A.y;
    const len = Math.hypot(lx, ly) || 1;
    const mx = (A.x + B.x) / 2;
    const my = (A.y + B.y) / 2;
    const perpx = -ly / len;
    const perpy = lx / len;
    const cx = mx + perpx * bow;
    const cy = my + perpy * bow;
    // start on A's boundary heading toward the control point
    let sax = cx - A.x;
    let say = cy - A.y;
    const sal = Math.hypot(sax, say) || 1;
    const startx = A.x + (sax / sal) * A.r;
    const starty = A.y + (say / sal) * A.r;
    // end on B's boundary heading toward the control point
    let ebx = cx - B.x;
    let eby = cy - B.y;
    const ebl = Math.hypot(ebx, eby) || 1;
    const endx = B.x + (ebx / ebl) * B.r;
    const endy = B.y + (eby / ebl) * B.r;
    // head direction = tangent from control into the target
    let hdx = endx - cx;
    let hdy = endy - cy;
    const hl = Math.hypot(hdx, hdy) || 1;
    hdx /= hl;
    hdy /= hl;
    return { path: `M${startx} ${starty} Q${cx} ${cy} ${endx} ${endy}`, head: e.head, hx: endx, hy: endy, dx: hdx, dy: hdy, sw, tip, loop: false };
  });

  const title = typeof d.title === "string" ? d.title : "";

  return { title, w, h, padL, padT, drawW, drawH, nodes, edges, groups, directed, arrowLabel, hasRep };
}

export function Network({ spec, caption }: { spec: string; caption?: string }) {
  const res = useMemo(() => build(spec), [spec]);

  if ("error" in res) return <div className="ax-fig__err">{res.error}</div>;

  const cat = isDark() ? CAT_DARK : CAT_LIGHT;
  const groupColor = (gi: number): string => (gi < cat.length ? cat[gi] : "var(--ax-lamp-neutral)");
  const label = caption || res.title || "network";

  const legendX = res.padL + res.drawW + 12;
  const headLen = 9;
  const headW = 8;
  const barHalf = 6;

  // legend rows: group swatches, then (if directed) edge-semantics glyphs.
  const groupTop = res.padT + 4;

  return (
    <figure className="ax-fig">
      <div className="ax-fig__frame" style={{ overflowX: "auto", justifyContent: "flex-start" }}>
        <svg viewBox={`0 0 ${res.w} ${res.h}`} width={res.w} height={res.h} role="img" aria-label={label} style={{ maxWidth: "100%" }}>
          {res.title && (
            <text x={res.padL - 4} y={18} fontSize="12" fontWeight="600" fill="var(--ax-ink)">
              {res.title}
            </text>
          )}

          {/* edges (drawn first so nodes sit on top) */}
          {res.edges.map((e, i) => {
            const nx = -e.dy;
            const ny = e.dx; // perpendicular to head direction
            const backx = e.hx - e.dx * headLen;
            const backy = e.hy - e.dy * headLen;
            return (
              <g key={`e${i}`}>
                <title>{e.tip}</title>
                <path d={e.path} fill="none" stroke="var(--ax-ink-faint)" strokeWidth={e.sw} strokeLinecap="round" opacity={0.85} />
                {e.head === "arrow" && (
                  <polygon
                    points={`${e.hx},${e.hy} ${backx + nx * (headW / 2)},${backy + ny * (headW / 2)} ${backx - nx * (headW / 2)},${backy - ny * (headW / 2)}`}
                    fill="var(--ax-ink-soft)"
                  />
                )}
                {e.head === "bar" && (
                  <line
                    x1={e.hx + nx * barHalf}
                    y1={e.hy + ny * barHalf}
                    x2={e.hx - nx * barHalf}
                    y2={e.hy - ny * barHalf}
                    stroke="var(--ax-ink-soft)"
                    strokeWidth={2}
                    strokeLinecap="round"
                  />
                )}
              </g>
            );
          })}

          {/* nodes */}
          {res.nodes.map((nd, i) => (
            <g key={`n${i}`}>
              <title>{`${nd.id} · ${nd.group} · value ${fmt(nd.value)} · degree ${nd.degree}`}</title>
              <circle cx={nd.x} cy={nd.y} r={nd.r} fill={groupColor(nd.gi)} stroke="var(--ax-surface)" strokeWidth={1.5} fillOpacity={0.92} />
              <text
                x={nd.x}
                y={nd.y + nd.r + 11}
                fontSize="10"
                textAnchor="middle"
                fill="var(--ax-ink)"
                stroke="var(--ax-surface)"
                strokeWidth={3}
                strokeLinejoin="round"
                style={{ paintOrder: "stroke" }}
              >
                {nd.id}
              </text>
            </g>
          ))}

          {/* group legend */}
          {res.groups.map((g, i) => (
            <g key={`lg${i}`}>
              <circle cx={legendX + 5} cy={groupTop + i * 17 + 4} r={5} fill={groupColor(i)} fillOpacity={0.92} />
              <text x={legendX + 15} y={groupTop + i * 17 + 7.5} fontSize="9.5" fill="var(--ax-ink-soft)">
                {g === "—" ? "（未分组）" : g}
              </text>
            </g>
          ))}

          {/* edge-semantics legend (only when directed) */}
          {res.directed &&
            (() => {
              const base = groupTop + res.groups.length * 17 + 12;
              return (
                <g>
                  <text x={legendX} y={base} fontSize="9" fill="var(--ax-ink-faint)">
                    边 edges
                  </text>
                  {res.arrowLabel && (
                    <g>
                      <line x1={legendX + 2} y1={base + 15} x2={legendX + 20} y2={base + 15} stroke="var(--ax-ink-faint)" strokeWidth={1.4} />
                      <polygon points={`${legendX + 22},${base + 15} ${legendX + 15},${base + 12} ${legendX + 15},${base + 18}`} fill="var(--ax-ink-soft)" />
                      <text x={legendX + 28} y={base + 18} fontSize="9" fill="var(--ax-ink-soft)">
                        {res.arrowLabel}
                      </text>
                    </g>
                  )}
                  {res.hasRep && (
                    <g>
                      <line x1={legendX + 2} y1={base + 31} x2={legendX + 20} y2={base + 31} stroke="var(--ax-ink-faint)" strokeWidth={1.4} />
                      <line x1={legendX + 22} y1={base + 27} x2={legendX + 22} y2={base + 35} stroke="var(--ax-ink-soft)" strokeWidth={2} />
                      <text x={legendX + 28} y={base + 34} fontSize="9" fill="var(--ax-ink-soft)">
                        抑制 repression
                      </text>
                    </g>
                  )}
                </g>
              );
            })()}
        </svg>
      </div>
      <figcaption className="ax-fig__cap">
        <span className="ax-fig__kind">网络 · network</span> {caption || res.title}
      </figcaption>
    </figure>
  );
}
