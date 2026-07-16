// Native reaction-scheme figure: reactants joined by '+', a reaction arrow bearing
// reagents (`above`) and conditions (`below`), then products. Each species is drawn
// client-side by smiles-drawer (same dep as Molecule.tsx) into its own nested <svg>;
// the arrow, '+', labels and tooltips are hand-drawn so the whole scheme themes via CSS vars.
// Spec (```rxn JSON): {"title"?, "reactants":[smiles…], "products":[smiles…], "above"?, "below"?}
// Also accepts a reaction-SMILES body: "A.B>>C.D" or "A.B>agents>C.D".
import { useEffect, useId, useMemo, useRef, useState } from "react";
import SmilesDrawer from "smiles-drawer";
import { isDark } from "./theme";

type Model =
  | { ok: true; title?: string; reactants: string[]; products: string[]; above?: string; below?: string }
  | { ok: false; reason: string };

function parseSpec(raw: string): Model {
  const t = raw.trim();
  if (!t) return { ok: false, reason: "空规范" };

  // 1) preferred form — a JSON object with reactants / products arrays
  let json: unknown;
  try {
    json = JSON.parse(t);
  } catch {
    json = undefined;
  }
  if (json && typeof json === "object" && !Array.isArray(json)) {
    const o = json as Record<string, unknown>;
    const R = o.reactants;
    const P = o.products;
    if (Array.isArray(R) && Array.isArray(P)) {
      const clean = (a: unknown[]): string[] =>
        a.filter((x): x is string => typeof x === "string" && x.trim().length > 0).map((s) => s.trim());
      const reactants = clean(R);
      const products = clean(P);
      if (reactants.length === 0 || products.length === 0)
        return { ok: false, reason: "reactants / products 至少各需一个 SMILES" };
      return {
        ok: true,
        title: typeof o.title === "string" && o.title.trim() ? o.title.trim() : undefined,
        reactants,
        products,
        above: typeof o.above === "string" && o.above.trim() ? o.above.trim() : undefined,
        below: typeof o.below === "string" && o.below.trim() ? o.below.trim() : undefined,
      };
    }
    return { ok: false, reason: "需要 reactants 与 products 数组" };
  }

  // 2) fallback — a reaction-SMILES string ("A.B>>C.D" or "A.B>agents>C.D")
  if (t.includes(">")) {
    const parts = t.split(">");
    let left = "";
    let agents = "";
    let right = "";
    if (parts.length === 3) [left, agents, right] = parts;
    else if (parts.length === 2) [left, right] = parts;
    else return { ok: false, reason: "无法解析反应 SMILES" };
    const split = (s: string): string[] => s.split(".").map((x) => x.trim()).filter(Boolean);
    const reactants = split(left);
    const products = split(right);
    if (reactants.length === 0 || products.length === 0)
      return { ok: false, reason: "反应 SMILES 缺少反应物或产物" };
    const ag = split(agents).join(" + ");
    return { ok: true, reactants, products, above: ag || undefined };
  }

  return { ok: false, reason: "既不是反应 JSON,也不是反应 SMILES" };
}

// layout geometry (all in the parent svg's user-space; it scales via viewBox)
const MOL_W = 124;
const MOL_H = 108;
const PLUS_W = 24;
const ARROW_MIN = 128; // shortest arrow; widened to fit long reagent/condition labels
const PAD = 12;
const TOP = 12;
const BOT = 12;

// rough glyph advance so the arrow spans its labels instead of them bleeding
// over the neighbouring molecules. Latin averages ~0.56em, but full-width CJK
// glyphs are ~1em, so sum per-character advances instead of a flat average —
// otherwise Chinese reagent/condition labels overflow the arrow region.
const CJK = /[⺀-鿿豈-﫿＀-￯　-〿]/;
function estTextW(s: string, fontSize: number): number {
  let w = 0;
  for (const ch of s) w += fontSize * (CJK.test(ch) ? 1.0 : 0.56);
  return w;
}

type SpeciesBox = { key: string; smiles: string; x: number; side: "reactant" | "product" };

export function Reaction({ spec, caption }: { spec: string; caption?: string }) {
  const uid = useId();
  const parentRef = useRef<SVGSVGElement>(null);
  const [drawErr, setDrawErr] = useState<Record<string, boolean>>({});
  const [themeTick, setThemeTick] = useState(0);

  const model = useMemo(() => parseSpec(spec), [spec]);

  const layout = useMemo(() => {
    if (!model.ok) return null;
    const aboveW = model.above ? estTextW(model.above, 10) : 0;
    const belowW = model.below ? estTextW(model.below, 9.5) : 0;
    const arrowW = Math.max(ARROW_MIN, Math.max(aboveW, belowW) + 28);

    const species: SpeciesBox[] = [];
    const plusX: number[] = [];
    let x = PAD;
    model.reactants.forEach((sm, i) => {
      if (i > 0) {
        plusX.push(x + PLUS_W / 2);
        x += PLUS_W;
      }
      species.push({ key: `r${i}`, smiles: sm, x, side: "reactant" });
      x += MOL_W;
    });
    const arrowX = x;
    x += arrowW;
    model.products.forEach((sm, i) => {
      if (i > 0) {
        plusX.push(x + PLUS_W / 2);
        x += PLUS_W;
      }
      species.push({ key: `p${i}`, smiles: sm, x, side: "product" });
      x += MOL_W;
    });
    const W = x + PAD;
    const H = TOP + MOL_H + BOT;
    return { species, plusX, arrowX, arrowW, W, H, boxTop: TOP, yMid: TOP + MOL_H / 2 };
  }, [model]);

  // re-render molecules when the OS/app theme flips (smiles-drawer bakes colors at draw time)
  useEffect(() => {
    const bump = (): void => setThemeTick((n) => n + 1);
    const mo = new MutationObserver(bump);
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
    const mq = window.matchMedia?.("(prefers-color-scheme: dark)");
    mq?.addEventListener?.("change", bump);
    return () => {
      mo.disconnect();
      mq?.removeEventListener?.("change", bump);
    };
  }, []);

  // draw each species into its nested <svg>
  useEffect(() => {
    const parent = parentRef.current;
    if (!parent || !layout) return;
    const theme = isDark() ? "dark" : "light";
    const next: Record<string, boolean> = {};
    for (const sp of layout.species) {
      const el = parent.querySelector<SVGSVGElement>(`[data-sp="${sp.key}"]`);
      if (!el) continue;
      el.innerHTML = "";
      try {
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        const drawer = new (SmilesDrawer as any).SvgDrawer({ width: MOL_W, height: MOL_H, padding: 8, compactDrawing: false });
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        (SmilesDrawer as any).parse(
          sp.smiles,
          (tree: unknown) => {
            try {
              drawer.draw(tree, el, theme);
            } catch {
              next[sp.key] = true;
            }
          },
          () => {
            next[sp.key] = true;
          }
        );
      } catch {
        next[sp.key] = true;
      }
    }
    setDrawErr((prev) => {
      const pk = Object.keys(prev);
      const nk = Object.keys(next);
      if (pk.length === nk.length && nk.every((k) => prev[k])) return prev;
      return next;
    });
  }, [layout, themeTick]);

  if (!model.ok || !layout) {
    const reason = model.ok ? "布局失败" : model.reason;
    return <div className="ax-fig__err">反应式规范无效(需 reactants/products 或反应 SMILES) · {reason}</div>;
  }

  const { species, plusX, arrowX, arrowW, W, H, boxTop, yMid } = layout;
  const lineX0 = arrowX + 12;
  const lineX1 = arrowX + arrowW - 12;
  const arrowCx = arrowX + arrowW / 2;
  const label = caption || model.title || "反应式";
  const condTip = [model.above, model.below].filter(Boolean).join(" / ");

  return (
    <figure className="ax-fig">
      <div className="ax-fig__frame" style={{ overflowX: "auto", justifyContent: "flex-start" }}>
        <svg
          ref={parentRef}
          viewBox={`0 0 ${W} ${H}`}
          width={W}
          height={H}
          role="img"
          aria-label={label}
          style={{ maxWidth: "100%" }}
        >
          {/* each species: nested svg painted imperatively by smiles-drawer */}
          {species.map((sp) => (
            <svg
              key={`${uid}-${sp.key}`}
              data-sp={sp.key}
              x={sp.x}
              y={boxTop}
              width={MOL_W}
              height={MOL_H}
              preserveAspectRatio="xMidYMid meet"
              aria-hidden="true"
            />
          ))}

          {/* fallback text for any species smiles-drawer couldn't parse */}
          {species.map((sp) =>
            drawErr[sp.key] ? (
              <text
                key={`e-${sp.key}`}
                x={sp.x + MOL_W / 2}
                y={yMid + 3}
                textAnchor="middle"
                fontSize="9"
                fill="var(--ax-lamp-amber)"
              >
                {sp.smiles.length > 16 ? sp.smiles.slice(0, 15) + "…" : sp.smiles}
              </text>
            ) : null
          )}

          {/* '+' between species on each side */}
          {plusX.map((px, i) => (
            <text key={`plus-${i}`} x={px} y={yMid + 6} textAnchor="middle" fontSize="17" fill="var(--ax-ink-soft)">
              +
            </text>
          ))}

          {/* reaction arrow */}
          <line x1={lineX0} y1={yMid} x2={lineX1 - 6} y2={yMid} stroke="var(--ax-ink-soft)" strokeWidth="1.4" />
          <path
            d={`M ${lineX1 - 8} ${yMid - 4} L ${lineX1} ${yMid} L ${lineX1 - 8} ${yMid + 4} Z`}
            fill="var(--ax-ink-soft)"
          />
          {model.above && (
            <text x={arrowCx} y={yMid - 8} textAnchor="middle" fontSize="10" fill="var(--ax-ink-soft)">
              {model.above}
            </text>
          )}
          {model.below && (
            <text x={arrowCx} y={yMid + 17} textAnchor="middle" fontSize="9.5" fill="var(--ax-ink-faint)">
              {model.below}
            </text>
          )}

          {/* transparent hover targets carrying native tooltips (drawn last = on top) */}
          {species.map((sp) => (
            <rect key={`h-${sp.key}`} x={sp.x} y={boxTop} width={MOL_W} height={MOL_H} fill="transparent">
              <title>{`${sp.side === "reactant" ? "反应物" : "产物"} · ${sp.smiles}`}</title>
            </rect>
          ))}
          {condTip && (
            <rect x={arrowX} y={boxTop} width={arrowW} height={MOL_H} fill="transparent">
              <title>{`条件 · ${condTip}`}</title>
            </rect>
          )}
        </svg>
      </div>
      <figcaption className="ax-fig__cap">
        <span className="ax-fig__kind">反应式 · reaction</span> {caption || model.title || ""}
      </figcaption>
    </figure>
  );
}
