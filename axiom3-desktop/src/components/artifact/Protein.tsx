// Native 3D protein-structure rendering: a ```protein block → an interactive 3D cartoon,
// drawn with 3Dmol.js (lazy-loaded, WebGL). The block is either a 4-char PDB id ("1CRN")
// — fetched read-only from the public RCSB PDB — or inline PDB coordinates.
//
// RED LINE / safety: only a bare PDB id triggers a network fetch, and only to the fixed
// RCSB host (a public, read-only structure database) — never a URL from the artifact text.
import { useEffect, useRef, useState } from "react";

const PDB_ID = /^[0-9][a-zA-Z0-9]{3}$/;

function bg(): string {
  const t = document.documentElement.dataset.theme;
  const dark = t === "dark" || (t !== "light" && window.matchMedia?.("(prefers-color-scheme: dark)").matches);
  return dark ? "#131312" : "#ffffff";
}

export function Protein({ source, caption }: { source: string; caption?: string }) {
  const ref = useRef<HTMLDivElement>(null);
  const [err, setErr] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const src = source.trim();
  const isId = PDB_ID.test(src);

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
        viewer = $3Dmol.createViewer(ref.current, { backgroundColor: bg() });

        let pdb = src;
        if (isId) {
          // read-only fetch of a public structure from the fixed RCSB host
          const resp = await fetch(`https://files.rcsb.org/download/${src.toUpperCase()}.pdb`);
          if (!resp.ok) throw new Error(`PDB ${src.toUpperCase()} not found`);
          pdb = await resp.text();
        }
        if (!alive) return;
        viewer.addModel(pdb, "pdb");
        viewer.setStyle({}, { cartoon: { color: "spectrum" } });
        viewer.zoomTo();
        viewer.render();
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
    };
  }, [src, isId]);

  return (
    <figure className="ax-fig">
      <div className="ax-fig__frame ax-fig__proteinframe">
        {err ? (
          <div className="ax-fig__err">结构无法渲染 · {err}</div>
        ) : (
          <div ref={ref} className="ax-fig__protein">
            {loading && <span className="ax-fig__loading">加载结构…</span>}
          </div>
        )}
      </div>
      <figcaption className="ax-fig__cap">
        <span className="ax-fig__kind">结构 · 3D</span>
        {caption || (isId ? `PDB ${src.toUpperCase()} · RCSB` : "inline PDB")}
        <span className="ax-fig__hint">拖拽旋转 · 滚轮缩放</span>
      </figcaption>
    </figure>
  );
}
