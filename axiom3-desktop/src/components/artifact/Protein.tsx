// Native 3D protein-structure rendering: a ```protein block → an interactive 3D viewer with a
// style switcher (Cartoon / Stick / Sphere / Surface / Line) + atom count, drawn with 3Dmol.js
// (lazy-loaded, WebGL). The block is either a 4-char PDB id ("1CRN") — fetched read-only from the
// public RCSB PDB — or inline PDB coordinates.
//
// RED LINE / safety: only a bare PDB id triggers a network fetch, and only to the fixed RCSB host
// (a public, read-only structure database) — never a URL from the artifact text.
import { useEffect, useRef, useState } from "react";

const PDB_ID = /^[0-9][a-zA-Z0-9]{3}$/;
const STYLES: { k: string; label: string }[] = [
  { k: "cartoon", label: "Cartoon" },
  { k: "stick", label: "Stick" },
  { k: "sphere", label: "Sphere" },
  { k: "surface", label: "Surface" },
  { k: "line", label: "Line" },
];

function bg(): string {
  const t = document.documentElement.dataset.theme;
  const dark = t === "dark" || (t !== "light" && window.matchMedia?.("(prefers-color-scheme: dark)").matches);
  return dark ? "#131312" : "#ffffff";
}

export function Protein({ source, caption }: { source: string; caption?: string }) {
  const ref = useRef<HTMLDivElement>(null);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const viewerRef = useRef<any>(null);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const molRef = useRef<any>(null);
  const [err, setErr] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [style, setStyle] = useState("cartoon");
  const [atoms, setAtoms] = useState<number | null>(null);
  const [themeTick, setThemeTick] = useState(0);
  const src = source.trim();
  const isId = PDB_ID.test(src);

  function applyStyle(s: string) {
    const v = viewerRef.current;
    const $3Dmol = molRef.current;
    if (!v) return;
    try {
      v.removeAllSurfaces?.();
      if (s === "stick") v.setStyle({}, { stick: {} });
      else if (s === "sphere") v.setStyle({}, { sphere: { scale: 0.3 } });
      else if (s === "line") v.setStyle({}, { line: {} });
      else {
        v.setStyle({}, { cartoon: { color: "spectrum" } });
        if (s === "surface" && $3Dmol) v.addSurface($3Dmol.SurfaceType.VDW, { opacity: 0.78, color: "white" });
      }
      v.render();
    } catch {
      /* ignore */
    }
  }

  useEffect(() => {
    let alive = true;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    let viewer: any = null;
    (async () => {
      setErr(null);
      setLoading(true);
      try {
        const mod = await import("3dmol");
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        const $3Dmol: any = (mod as any).default ?? mod;
        if (!alive || !ref.current) return;
        // release any prior canvas/WebGL context before mounting a new viewer,
        // otherwise 3Dmol stacks a second <canvas> in the same div and leaks a GL context
        ref.current.innerHTML = "";
        viewer = $3Dmol.createViewer(ref.current, { backgroundColor: bg() });
        let pdb = src;
        if (isId) {
          const resp = await fetch(`https://files.rcsb.org/download/${src.toUpperCase()}.pdb`);
          if (!resp.ok) throw new Error(`PDB ${src.toUpperCase()} not found`);
          pdb = await resp.text();
        }
        if (!alive) return;
        const model = viewer.addModel(pdb, "pdb");
        viewer.setStyle({}, { cartoon: { color: "spectrum" } });
        viewer.zoomTo();
        viewer.render();
        viewerRef.current = viewer;
        molRef.current = $3Dmol;
        try {
          setAtoms(model.selectedAtoms({}).length);
        } catch {
          /* count unavailable */
        }
        setStyle("cartoon");
        setLoading(false);
      } catch (e) {
        if (alive) {
          setErr(String((e as Error)?.message || e));
          setLoading(false);
        }
      }
    })();
    return () => {
      alive = false;
      try {
        viewer?.clear?.();
      } catch {
        /* ignore */
      }
      viewerRef.current = null;
    };
  }, [src, isId]);

  // watch for OS/app theme flips (3Dmol bakes the background color at createViewer time)
  useEffect(() => {
    if (typeof document === "undefined") return;
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

  // repaint the live viewer's background on theme change — no PDB re-fetch, no viewer rebuild
  useEffect(() => {
    const v = viewerRef.current;
    if (!v) return;
    try {
      v.setBackgroundColor(bg());
      v.render();
    } catch {
      /* ignore */
    }
  }, [themeTick]);

  return (
    <figure className="ax-fig">
      <div className="ax-fig__frame ax-fig__proteinframe">
        {err ? (
          <div className="ax-fig__err">结构无法渲染 · {err}</div>
        ) : (
          <>
            {/* ref div is owned exclusively by 3Dmol so it can be safely cleared on re-render;
                the loading overlay lives outside it (positioned against the relative frame) */}
            <div ref={ref} className="ax-fig__protein" />
            {loading && <span className="ax-fig__loading">加载结构…</span>}
          </>
        )}
      </div>
      {!err && (
        <div className="ax-mol__bar">
          <span className="ax-mol__label">Style</span>
          {STYLES.map((s) => (
            <button
              key={s.k}
              className={`ax-mol__btn ${style === s.k ? "is-on" : ""}`}
              disabled={loading}
              onClick={() => { setStyle(s.k); applyStyle(s.k); }}
            >
              {s.label}
            </button>
          ))}
          {atoms != null && <span className="ax-mol__atoms">{atoms.toLocaleString()} atoms</span>}
        </div>
      )}
      <figcaption className="ax-fig__cap">
        <span className="ax-fig__kind">结构 · 3D</span>
        {caption || (isId ? `PDB ${src.toUpperCase()} · RCSB` : "inline PDB")}
        <span className="ax-fig__hint">拖拽旋转 · 滚轮缩放</span>
      </figcaption>
    </figure>
  );
}
