// Native molecular-structure rendering: a SMILES string → a 2D structure diagram, drawn
// client-side by smiles-drawer (no backend, no network). This is a "science artifact"
// rendered in its native form, like Claude Science's chemical-structure viewer.
import { useEffect, useRef, useState } from "react";
import SmilesDrawer from "smiles-drawer";

function isDark(): boolean {
  const t = document.documentElement.dataset.theme;
  if (t === "dark") return true;
  if (t === "light") return false;
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? false;
}

export function Molecule({ smiles, caption }: { smiles: string; caption?: string }) {
  const ref = useRef<SVGSVGElement>(null);
  const [err, setErr] = useState(false);
  const [themeTick, setThemeTick] = useState(0);

  // re-render the structure when the OS/app theme flips (smiles-drawer bakes colors at draw time)
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

  useEffect(() => {
    if (!ref.current) return;
    setErr(false);
    // clear any prior render
    ref.current.innerHTML = "";
    try {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const drawer = new (SmilesDrawer as any).SvgDrawer({ width: 340, height: 240, padding: 14, compactDrawing: false });
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      (SmilesDrawer as any).parse(
        smiles.trim(),
        (tree: unknown) => {
          try {
            drawer.draw(tree, ref.current, isDark() ? "dark" : "light");
          } catch {
            setErr(true);
          }
        },
        () => setErr(true)
      );
    } catch {
      setErr(true);
    }
  }, [smiles, themeTick]);

  return (
    <figure className="ax-fig">
      <div className="ax-fig__frame">
        {err ? (
          <div className="ax-fig__err">无法解析该 SMILES · unparseable SMILES</div>
        ) : (
          <svg ref={ref} className="ax-fig__svg" width={340} height={240} role="img" aria-label={caption || "molecular structure"} />
        )}
      </div>
      <figcaption className="ax-fig__cap">
        <span className="ax-fig__kind">分子 · molecule</span>
        <code className="ax-fig__smiles">{caption || smiles.trim()}</code>
      </figcaption>
    </figure>
  );
}
