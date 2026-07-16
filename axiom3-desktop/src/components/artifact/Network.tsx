// Native interaction / regulatory network: node-link biology (PPI, gene-regulatory circuits,
// co-expression modules, KG neighborhoods, mechanism maps) that mermaid cannot draw. Layout is a
// DETERMINISTIC seeded Fruchterman-Reingold pass in a useMemo (fixed seed + fixed iterations →
// identical every render). Node radius = √value, fill by `group` (categorical palette + legend);
// edge stroke-width = weight. It only lays out the supplied topology — it asserts nothing beyond
// the emitted nodes/edges.
//
// SBGN-lite semantics (all OPTIONAL — a spec that omits them renders exactly as before):
//   • edge.kind ∈ activate|inhibit|catalyze|bind|convert|translocate — a fixed <defs> marker
//     carries the mechanism as a SHAPE (solid triangle / T-bar / hollow circle / diamond / open
//     chevron / double chevron), decoupled from colour so it survives CVD. Absent → the legacy
//     sign-based arrow ▸ / bar ⊣ (directed) or a plain link.
//   • edge.evidence ∈ direct|indirect|hypothesized → solid / dashed / dotted line; hypothesized
//     edges also get a small "inferred" tag. Absent → solid.
//   • node.entityType ∈ macromolecule|simpleChemical|complex|phenotype → rounded-rect / ellipse /
//     cut-corner rect / hexagon. Absent → the legacy circle. Edges terminate on the real shape
//     boundary so arrowheads stay visible.
// The legend lists only the kinds / entity shapes / line styles that actually occur. module→hue is
// catColor; red/green never encode activation vs inhibition (that is the marker's job).
//
// Spec (```network JSON): {"title"?, "directed"?: bool,
//   "nodes":[{"id":string,"group"?:string,"value"?:number,"entityType"?:string}],
//   "edges":[{"source":string,"target":string,"weight"?:number,"sign"?:"+"|"-",
//             "kind"?:string,"evidence"?:string}]}
import { useMemo, type ReactElement } from "react";
import { catColor, categorical, isDark } from "./theme";

// ── optional SBGN vocabularies (graceful: unknown values fall back to defaults) ──
type EdgeKind = "activate" | "inhibit" | "catalyze" | "bind" | "convert" | "translocate";
type Evidence = "direct" | "indirect" | "hypothesized";
type EntityType = "macromolecule" | "simpleChemical" | "complex" | "phenotype";

const KIND_LABELS: Record<EdgeKind, string> = {
  activate: "激活 activate",
  inhibit: "抑制 inhibit",
  catalyze: "催化 catalyze",
  bind: "结合 bind",
  convert: "转化 convert",
  translocate: "转位 translocate",
};
const ENTITY_LABELS: Record<EntityType, string> = {
  macromolecule: "大分子 macromolecule",
  simpleChemical: "小分子 simple chemical",
  complex: "复合物 complex",
  phenotype: "表型 phenotype",
};
const EVIDENCE_LABELS: Record<Evidence, string> = {
  direct: "直接 direct",
  indirect: "间接 indirect",
  hypothesized: "推测 hypothesized",
};
const KIND_SET = new Set<string>(Object.keys(KIND_LABELS));
const ENTITY_ALIAS: Record<string, EntityType> = {
  macromolecule: "macromolecule", protein: "macromolecule", gene: "macromolecule", rna: "macromolecule", enzyme: "macromolecule",
  simplechemical: "simpleChemical", chemical: "simpleChemical", metabolite: "simpleChemical", compound: "simpleChemical", drug: "simpleChemical", ligand: "simpleChemical", ion: "simpleChemical",
  complex: "complex",
  phenotype: "phenotype", process: "phenotype", outcome: "phenotype", condition: "phenotype",
};
const EVID_ALIAS: Record<string, Evidence> = {
  direct: "direct", indirect: "indirect", hypothesized: "hypothesized", hypothetical: "hypothesized", inferred: "hypothesized", predicted: "hypothesized", putative: "hypothesized",
};
function normKind(v: unknown): EdgeKind | undefined {
  if (typeof v !== "string") return undefined;
  const s = v.trim().toLowerCase();
  return KIND_SET.has(s) ? (s as EdgeKind) : undefined;
}
function normEntity(v: unknown): EntityType | undefined {
  if (typeof v !== "string") return undefined;
  return ENTITY_ALIAS[v.trim().toLowerCase()];
}
function normEvid(v: unknown): Evidence | undefined {
  if (typeof v !== "string") return undefined;
  return EVID_ALIAS[v.trim().toLowerCase()];
}
function evidDash(ev: Evidence | undefined): string | undefined {
  if (ev === "indirect") return "6 4";
  if (ev === "hypothesized") return "1.5 3.5";
  return undefined; // direct / undefined → solid
}

type NodeSpec = { id?: unknown; group?: unknown; value?: unknown; entityType?: unknown };
type EdgeSpec = { source?: unknown; target?: unknown; weight?: unknown; sign?: unknown; kind?: unknown; evidence?: unknown };
type Spec = { title?: string; directed?: boolean; nodes?: NodeSpec[]; edges?: EdgeSpec[] };

// categorical identity palette (fixed order, indexed by group first-appearance). Past the palette
// slots the colours CYCLE (gi % len) and wrapped groups gain a distinguishing ring (see groupDash)
// so identity-by-color survives instead of collapsing to one neutral swatch.

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

// CJK-aware legend text width: full-width glyphs advance ~11px, not ~6.6, so Chinese labels don't
// clip at the viewBox right edge (outer SVG overflow is hidden).
function legendTextW(s: string): number {
  let wpx = 0;
  for (const ch of s) wpx += /[⺀-鿿＀-￯　-〿]/.test(ch) ? 11 : 6.6;
  return wpx;
}

// ── outward boundary radius of a node shape along a unit direction (ux,uy) ──
// Edges terminate here (not at the raw circle radius) so arrowhead markers clear the drawn shape.
function boundaryR(entity: EntityType | undefined, r: number, ux: number, uy: number): number {
  const ax = Math.abs(ux) || 1e-6;
  const ay = Math.abs(uy) || 1e-6;
  switch (entity) {
    case "simpleChemical": {
      const rx = 1.2 * r;
      const ry = r;
      return 1 / Math.sqrt((ux / rx) ** 2 + (uy / ry) ** 2);
    }
    case "macromolecule":
      return Math.min((1.3 * r) / ax, r / ay);
    case "complex":
      return Math.min((1.2 * r) / ax, r / ay);
    case "phenotype":
      return Math.min((1.35 * r) / ax, r / ay);
    default:
      return r;
  }
}

type Head = "arrow" | "bar" | "none";
type LNode = { id: string; group: string; gi: number; value: number; entity?: EntityType; x: number; y: number; r: number; degree: number };
type LEdge = {
  path: string;
  head: Head;
  kind?: EdgeKind;
  dash?: string;
  evidence?: Evidence;
  hx: number; hy: number; dx: number; dy: number;
  midx: number; midy: number;
  sw: number; tip: string; loop: boolean;
};
type LegendRow =
  | { t: "group"; label: string; gi: number; y: number }
  | { t: "header"; label: string; y: number }
  | { t: "shape"; label: string; entity: EntityType; y: number }
  | { t: "kind"; label: string; kind: EdgeKind; y: number }
  | { t: "line"; label: string; evidence: Evidence; y: number }
  | { t: "arrow"; label: string; y: number }
  | { t: "bar"; label: string; y: number };
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
      rows: LegendRow[];
      kindsPresent: EdgeKind[];
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
    const entity = normEntity(ns.entityType);
    idIndex.set(id, nodes.length);
    nodes.push({ id, group, gi: 0, value, entity, x: 0, y: 0, r: 0, degree: 0 });
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
  // entity shapes present, in first-appearance order (legend only lists what occurs)
  const entitiesPresent: EntityType[] = [];
  for (const nd of nodes) if (nd.entity && !entitiesPresent.includes(nd.entity)) entitiesPresent.push(nd.entity);

  const directed = d.directed === true;

  // ── validate edges: endpoints must resolve to real nodes; drop the rest. ──
  type VEdge = { s: number; t: number; weight: number; head: Head; sign: string; kind?: EdgeKind; evidence?: Evidence };
  const rawEdges = Array.isArray(d.edges) ? d.edges : [];
  const vedges: VEdge[] = [];
  let maxW = 0;
  let arrowSigned = false; // legacy sign-based "+" activation arrow present (no explicit kind)
  let legacyArrow = false;
  let legacyBar = false;
  const kindsPresent: EdgeKind[] = [];
  const evidencesPresent: Evidence[] = [];
  for (const e of rawEdges) {
    if (!e || typeof e !== "object") continue;
    const su = typeof e.source === "string" ? e.source.trim() : "";
    const tu = typeof e.target === "string" ? e.target.trim() : "";
    const s = idIndex.get(su);
    const t = idIndex.get(tu);
    if (s === undefined || t === undefined) continue;
    const weight = Number.isFinite(e.weight) && (e.weight as number) > 0 ? (e.weight as number) : 0.6;
    const sign = typeof e.sign === "string" ? e.sign : "";
    const kind = normKind(e.kind);
    const evidence = normEvid(e.evidence);
    const head = headType(directed, sign);
    if (kind) {
      if (!kindsPresent.includes(kind)) kindsPresent.push(kind);
    } else {
      // legacy sign-based head only decorates edges without an explicit mechanism kind
      if (head === "bar") legacyBar = true;
      if (head === "arrow") {
        legacyArrow = true;
        if (sign.trim() === "+" || sign.trim().toLowerCase().startsWith("activ") || sign.trim().toLowerCase().startsWith("pos")) arrowSigned = true;
      }
    }
    if (evidence && !evidencesPresent.includes(evidence)) evidencesPresent.push(evidence);
    maxW = Math.max(maxW, weight);
    nodes[s].degree++;
    nodes[t].degree++;
    vedges.push({ s, t, weight, head, sign, kind, evidence });
  }
  if (maxW <= 0) maxW = 1;

  // an explicit kind implies direction → let kind edges bow apart even when directed:false
  const geomDir = directed || kindsPresent.length > 0;
  const dirKey = new Set<string>();
  if (geomDir) for (const e of vedges) dirKey.add(`${e.s} ${e.t}`);

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
  const margin = maxR + 24; // room for the (possibly wider) node shape + its label below
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

  // ── edge geometry (reciprocal directed pairs bow apart; self-loops arc above) ──
  const edges: LEdge[] = vedges.map((e) => {
    const A = nodes[e.s];
    const B = nodes[e.t];
    const sw = 0.9 + (e.weight / maxW) * 2.4;
    const tipSign = e.sign ? ` · ${e.sign}` : "";
    const tipKind = e.kind ? ` · ${e.kind}` : "";
    const tipEv = e.evidence ? ` · ${e.evidence}` : "";
    const tip = `${A.id} ${geomDir ? "→" : "—"} ${B.id} · weight ${fmt(e.weight)}${tipSign}${tipKind}${tipEv}`;
    const dash = evidDash(e.evidence);

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
      return { path: `M${sx} ${sy} C${c1x} ${c1y} ${c2x} ${c2y} ${ex} ${ey}`, head: e.head, kind: e.kind, dash, evidence: e.evidence, hx: ex, hy: ey, dx: hdx, dy: hdy, midx: A.x, midy: A.y - r * 3 - 12, sw, tip, loop: true };
    }

    const reciprocal = geomDir && dirKey.has(`${e.t} ${e.s}`);
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
    // start on A's shape boundary heading toward the control point
    const sax = cx - A.x;
    const say = cy - A.y;
    const sal = Math.hypot(sax, say) || 1;
    const sux = sax / sal;
    const suy = say / sal;
    const startx = A.x + sux * boundaryR(A.entity, A.r, sux, suy);
    const starty = A.y + suy * boundaryR(A.entity, A.r, sux, suy);
    // end on B's shape boundary heading toward the control point
    const ebx = cx - B.x;
    const eby = cy - B.y;
    const ebl = Math.hypot(ebx, eby) || 1;
    const eux = ebx / ebl;
    const euy = eby / ebl;
    const endx = B.x + eux * boundaryR(B.entity, B.r, eux, euy);
    const endy = B.y + euy * boundaryR(B.entity, B.r, eux, euy);
    // head direction = tangent from control into the target
    let hdx = endx - cx;
    let hdy = endy - cy;
    const hl = Math.hypot(hdx, hdy) || 1;
    hdx /= hl;
    hdy /= hl;
    // curve midpoint (t=0.5 of the quadratic) — anchor for the "inferred" tag
    const midx = 0.25 * startx + 0.5 * cx + 0.25 * endx;
    const midy = 0.25 * starty + 0.5 * cy + 0.25 * endy;
    return { path: `M${startx} ${starty} Q${cx} ${cy} ${endx} ${endy}`, head: e.head, kind: e.kind, dash, evidence: e.evidence, hx: endx, hy: endy, dx: hdx, dy: hdy, midx, midy, sw, tip, loop: false };
  });

  // ── legend rows (only what actually occurs) with pre-computed y baselines ──
  const arrowLabel = arrowSigned ? "激活 activation" : "指向 directed";
  const showEv = evidencesPresent.some((ev) => ev !== "direct");
  const rows: LegendRow[] = [];
  const groupTop = padT + 4;
  let cur = groupTop;
  const rowY = () => { const y = cur + 8; cur += 17; return y; };
  const pushHeader = (label: string) => { cur += 7; rows.push({ t: "header", label, y: cur + 9 }); cur += 16; };

  groups.forEach((g, i) => rows.push({ t: "group", label: g === "—" ? "（未分组）" : g, gi: i, y: rowY() }));
  if (entitiesPresent.length) {
    pushHeader("节点 nodes");
    entitiesPresent.forEach((en) => rows.push({ t: "shape", entity: en, label: ENTITY_LABELS[en], y: rowY() }));
  }
  if (kindsPresent.length) {
    pushHeader("边 · 语义 edges");
    kindsPresent.forEach((kd) => rows.push({ t: "kind", kind: kd, label: KIND_LABELS[kd], y: rowY() }));
  }
  if (legacyArrow || legacyBar) {
    if (!kindsPresent.length) pushHeader("边 edges");
    if (legacyArrow) rows.push({ t: "arrow", label: arrowLabel, y: rowY() });
    if (legacyBar) rows.push({ t: "bar", label: "抑制 repression", y: rowY() });
  }
  if (showEv) {
    pushHeader("证据 evidence");
    evidencesPresent.forEach((ev) => rows.push({ t: "line", evidence: ev, label: EVIDENCE_LABELS[ev], y: rowY() }));
  }
  const legendBottom = cur;

  // right-side legend column width from the widest row label (CJK-aware) + glyph indent
  const maxLabelW = Math.max(26, ...rows.map((r) => legendTextW(r.label)));
  const padR = Math.round(Math.min(240, Math.max(120, maxLabelW + 40)));
  const w = padL + drawW + padR;
  const h = Math.max(padT + drawH + padB, legendBottom + padB);

  const title = typeof d.title === "string" ? d.title : "";
  return { title, w, h, padL, padT, drawW, drawH, nodes, edges, rows, kindsPresent };
}

const MK = "var(--ax-ink-soft)"; // neutral marker ink — mechanism is carried by SHAPE, not colour

// fixed <defs> markers keyed by mechanism kind (SBGN-inspired). markerUnits=userSpaceOnUse keeps
// the glyph a constant size regardless of edge weight. IDs are identical across instances (the defs
// are byte-identical), so duplicate ids on a page resolve harmlessly to the same shape.
function KindMarker({ kind }: { kind: EdgeKind }) {
  const id = `ax-net-mk-${kind}`;
  switch (kind) {
    case "activate":
      return (
        <marker id={id} markerUnits="userSpaceOnUse" viewBox="0 0 10 8" markerWidth={10} markerHeight={8} refX={10} refY={4} orient="auto">
          <polygon points="0,0 10,4 0,8" fill={MK} />
        </marker>
      );
    case "inhibit":
      return (
        <marker id={id} markerUnits="userSpaceOnUse" viewBox="0 0 5 12" markerWidth={5} markerHeight={12} refX={2.5} refY={6} orient="auto">
          <line x1={2.5} y1={1} x2={2.5} y2={11} stroke={MK} strokeWidth={2} strokeLinecap="round" />
        </marker>
      );
    case "catalyze":
      return (
        <marker id={id} markerUnits="userSpaceOnUse" viewBox="0 0 11 10" markerWidth={11} markerHeight={10} refX={8.6} refY={5} orient="auto">
          <circle cx={5} cy={5} r={3.6} fill="none" stroke={MK} strokeWidth={1.4} />
        </marker>
      );
    case "bind":
      return (
        <marker id={id} markerUnits="userSpaceOnUse" viewBox="0 0 10 10" markerWidth={10} markerHeight={10} refX={9} refY={5} orient="auto">
          <polygon points="1,5 5,1 9,5 5,9" fill={MK} />
        </marker>
      );
    case "convert":
      return (
        <marker id={id} markerUnits="userSpaceOnUse" viewBox="0 0 10 8" markerWidth={10} markerHeight={8} refX={9} refY={4} orient="auto">
          <polyline points="1,0 9,4 1,8" fill="none" stroke={MK} strokeWidth={1.6} strokeLinejoin="round" strokeLinecap="round" />
        </marker>
      );
    case "translocate":
      return (
        <marker id={id} markerUnits="userSpaceOnUse" viewBox="0 0 11 8" markerWidth={11} markerHeight={8} refX={10} refY={4} orient="auto">
          <polyline points="1,0 5,4 1,8" fill="none" stroke={MK} strokeWidth={1.5} strokeLinejoin="round" strokeLinecap="round" />
          <polyline points="6,0 10,4 6,8" fill="none" stroke={MK} strokeWidth={1.5} strokeLinejoin="round" strokeLinecap="round" />
        </marker>
      );
  }
}

export function Network({ spec, caption }: { spec: string; caption?: string }) {
  const res = useMemo(() => build(spec), [spec]);

  if ("error" in res) return <div className="ax-fig__err">{res.error}</div>;

  const dark = isDark();
  const catLen = categorical(dark).length;
  // Cycle the palette past its slots (was: collapse to one neutral → 9+ groups indistinguishable);
  // wrapped groups (2nd+ cycle) get a dashed ring so a repeated hue is still tellable apart.
  const groupColor = (gi: number): string => (gi < 0 ? "var(--ax-lamp-neutral)" : catColor(gi, dark));
  const label = caption || res.title || "network";

  const legendX = res.padL + res.drawW + 12;
  const headLen = 9;
  const headW = 8;
  const barHalf = 6;

  // ── node shape by entityType (graceful: undefined → the legacy circle) ──
  const nodeShape = (nd: LNode) => {
    const fill = groupColor(nd.gi);
    const r = nd.r;
    const common = { fill, stroke: "var(--ax-surface)", strokeWidth: 1.5, fillOpacity: 0.92 };
    switch (nd.entity) {
      case "macromolecule":
        return <rect x={nd.x - 1.3 * r} y={nd.y - r} width={2.6 * r} height={2 * r} rx={0.5 * r} {...common} />;
      case "simpleChemical":
        return <ellipse cx={nd.x} cy={nd.y} rx={1.2 * r} ry={r} {...common} />;
      case "complex": {
        const hw = 1.2 * r, hh = r, c = 0.42 * r;
        const p = `${nd.x - hw + c},${nd.y - hh} ${nd.x + hw - c},${nd.y - hh} ${nd.x + hw},${nd.y - hh + c} ${nd.x + hw},${nd.y + hh - c} ${nd.x + hw - c},${nd.y + hh} ${nd.x - hw + c},${nd.y + hh} ${nd.x - hw},${nd.y + hh - c} ${nd.x - hw},${nd.y - hh + c}`;
        return <polygon points={p} {...common} />;
      }
      case "phenotype": {
        const hw = 1.35 * r, hh = r, nk = 0.5 * r;
        const p = `${nd.x - hw},${nd.y} ${nd.x - hw + nk},${nd.y - hh} ${nd.x + hw - nk},${nd.y - hh} ${nd.x + hw},${nd.y} ${nd.x + hw - nk},${nd.y + hh} ${nd.x - hw + nk},${nd.y + hh}`;
        return <polygon points={p} {...common} />;
      }
      default:
        return <circle cx={nd.x} cy={nd.y} r={r} {...common} />;
    }
  };

  // small legend glyph mirroring each node shape (neutral fill — shape carries the meaning)
  const miniShape = (entity: EntityType, cx: number, cy: number) => {
    const c = { fill: "var(--ax-surface)", stroke: MK, strokeWidth: 1.2 };
    switch (entity) {
      case "macromolecule":
        return <rect x={cx - 6} y={cy - 4} width={12} height={8} rx={3} {...c} />;
      case "simpleChemical":
        return <ellipse cx={cx} cy={cy} rx={6.5} ry={4} {...c} />;
      case "complex":
        return <polygon points={`${cx - 4.5},${cy - 4} ${cx + 4.5},${cy - 4} ${cx + 6.5},${cy} ${cx + 4.5},${cy + 4} ${cx - 4.5},${cy + 4} ${cx - 6.5},${cy}`} {...c} />;
      case "phenotype":
        return <polygon points={`${cx - 7},${cy} ${cx - 4},${cy - 4} ${cx + 4},${cy - 4} ${cx + 7},${cy} ${cx + 4},${cy + 4} ${cx - 4},${cy + 4}`} {...c} />;
    }
  };

  // legend glyph for a mechanism kind (line + terminal marker mimic)
  const kindGlyph = (kind: EdgeKind, x: number, y: number) => {
    const g = x + 16; // glyph anchor (line end)
    const line = <line x1={x} y1={y} x2={g - 1} y2={y} stroke="var(--ax-ink-faint)" strokeWidth={1.4} />;
    let mk: ReactElement | null = null;
    switch (kind) {
      case "activate":
        mk = <polygon points={`${g + 1},${y} ${g - 6},${y - 3.2} ${g - 6},${y + 3.2}`} fill={MK} />;
        break;
      case "inhibit":
        mk = <line x1={g} y1={y - 4} x2={g} y2={y + 4} stroke={MK} strokeWidth={2} strokeLinecap="round" />;
        break;
      case "catalyze":
        mk = <circle cx={g - 2.5} cy={y} r={3.1} fill="none" stroke={MK} strokeWidth={1.3} />;
        break;
      case "bind":
        mk = <polygon points={`${g - 6},${y} ${g - 3},${y - 3.2} ${g},${y} ${g - 3},${y + 3.2}`} fill={MK} />;
        break;
      case "convert":
        mk = <polyline points={`${g - 6},${y - 3.4} ${g},${y} ${g - 6},${y + 3.4}`} fill="none" stroke={MK} strokeWidth={1.5} strokeLinejoin="round" strokeLinecap="round" />;
        break;
      case "translocate":
        mk = (
          <g>
            <polyline points={`${g - 9},${y - 3.4} ${g - 5},${y} ${g - 9},${y + 3.4}`} fill="none" stroke={MK} strokeWidth={1.4} strokeLinejoin="round" strokeLinecap="round" />
            <polyline points={`${g - 5},${y - 3.4} ${g - 1},${y} ${g - 5},${y + 3.4}`} fill="none" stroke={MK} strokeWidth={1.4} strokeLinejoin="round" strokeLinecap="round" />
          </g>
        );
        break;
    }
    return (
      <g>
        {line}
        {mk}
      </g>
    );
  };

  return (
    <figure className="ax-fig">
      <div className="ax-fig__frame" style={{ overflowX: "auto", justifyContent: "flex-start" }}>
        <svg viewBox={`0 0 ${res.w} ${res.h}`} width={res.w} height={res.h} role="img" aria-label={label} style={{ maxWidth: "100%" }}>
          {res.kindsPresent.length > 0 && (
            <defs>
              {res.kindsPresent.map((kd) => (
                <KindMarker key={kd} kind={kd} />
              ))}
            </defs>
          )}

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
                <path
                  d={e.path}
                  fill="none"
                  stroke="var(--ax-ink-faint)"
                  strokeWidth={e.sw}
                  strokeLinecap="round"
                  strokeDasharray={e.dash}
                  opacity={0.85}
                  markerEnd={e.kind ? `url(#ax-net-mk-${e.kind})` : undefined}
                />
                {/* legacy sign-based heads only when no explicit mechanism kind */}
                {!e.kind && e.head === "arrow" && (
                  <polygon
                    points={`${e.hx},${e.hy} ${backx + nx * (headW / 2)},${backy + ny * (headW / 2)} ${backx - nx * (headW / 2)},${backy - ny * (headW / 2)}`}
                    fill="var(--ax-ink-soft)"
                  />
                )}
                {!e.kind && e.head === "bar" && (
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
                {e.evidence === "hypothesized" && (
                  <text
                    x={e.midx}
                    y={e.midy}
                    fontSize="7.5"
                    textAnchor="middle"
                    fill="var(--ax-ink-faint)"
                    stroke="var(--ax-surface)"
                    strokeWidth={2.5}
                    strokeLinejoin="round"
                    style={{ paintOrder: "stroke" }}
                  >
                    inferred
                  </text>
                )}
              </g>
            );
          })}

          {/* nodes */}
          {res.nodes.map((nd, i) => (
            <g key={`n${i}`}>
              <title>{`${nd.id} · ${nd.group}${nd.entity ? ` · ${nd.entity}` : ""} · value ${fmt(nd.value)} · degree ${nd.degree}`}</title>
              {nodeShape(nd)}
              {nd.gi >= catLen && (
                <circle cx={nd.x} cy={nd.y} r={nd.r + 2.2} fill="none" stroke={groupColor(nd.gi)} strokeWidth={1} strokeDasharray="3 2.5" />
              )}
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

          {/* legend (only rows that actually occur) */}
          {res.rows.map((row, i) => {
            if (row.t === "header") {
              return (
                <text key={`l${i}`} x={legendX} y={row.y} fontSize="9" fill="var(--ax-ink-faint)">
                  {row.label}
                </text>
              );
            }
            if (row.t === "group") {
              return (
                <g key={`l${i}`}>
                  <circle cx={legendX + 5} cy={row.y} r={5} fill={groupColor(row.gi)} fillOpacity={0.92} />
                  {row.gi >= catLen && <circle cx={legendX + 5} cy={row.y} r={6.5} fill="none" stroke={groupColor(row.gi)} strokeWidth={1} strokeDasharray="3 2.5" />}
                  <text x={legendX + 15} y={row.y + 3.5} fontSize="9.5" fill="var(--ax-ink-soft)">
                    {row.label}
                  </text>
                </g>
              );
            }
            if (row.t === "shape") {
              return (
                <g key={`l${i}`}>
                  {miniShape(row.entity, legendX + 6, row.y)}
                  <text x={legendX + 17} y={row.y + 3.5} fontSize="9" fill="var(--ax-ink-soft)">
                    {row.label}
                  </text>
                </g>
              );
            }
            if (row.t === "kind") {
              return (
                <g key={`l${i}`}>
                  {kindGlyph(row.kind, legendX + 2, row.y)}
                  <text x={legendX + 24} y={row.y + 3.5} fontSize="9" fill="var(--ax-ink-soft)">
                    {row.label}
                  </text>
                </g>
              );
            }
            if (row.t === "line") {
              return (
                <g key={`l${i}`}>
                  <line x1={legendX + 2} y1={row.y} x2={legendX + 20} y2={row.y} stroke="var(--ax-ink-soft)" strokeWidth={1.5} strokeDasharray={evidDash(row.evidence)} />
                  <text x={legendX + 24} y={row.y + 3.5} fontSize="9" fill="var(--ax-ink-soft)">
                    {row.label}
                  </text>
                </g>
              );
            }
            if (row.t === "arrow") {
              return (
                <g key={`l${i}`}>
                  <line x1={legendX + 2} y1={row.y} x2={legendX + 18} y2={row.y} stroke="var(--ax-ink-faint)" strokeWidth={1.4} />
                  <polygon points={`${legendX + 20},${row.y} ${legendX + 13},${row.y - 3} ${legendX + 13},${row.y + 3}`} fill="var(--ax-ink-soft)" />
                  <text x={legendX + 24} y={row.y + 3} fontSize="9" fill="var(--ax-ink-soft)">
                    {row.label}
                  </text>
                </g>
              );
            }
            // row.t === "bar"
            return (
              <g key={`l${i}`}>
                <line x1={legendX + 2} y1={row.y} x2={legendX + 18} y2={row.y} stroke="var(--ax-ink-faint)" strokeWidth={1.4} />
                <line x1={legendX + 20} y1={row.y - 4} x2={legendX + 20} y2={row.y + 4} stroke="var(--ax-ink-soft)" strokeWidth={2} />
                <text x={legendX + 24} y={row.y + 3} fontSize="9" fill="var(--ax-ink-soft)">
                  {row.label}
                </text>
              </g>
            );
          })}
        </svg>
      </div>
      <figcaption className="ax-fig__cap">
        <span className="ax-fig__kind">网络 · network</span> {caption || res.title}
      </figcaption>
    </figure>
  );
}
