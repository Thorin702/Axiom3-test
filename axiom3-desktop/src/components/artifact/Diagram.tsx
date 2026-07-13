// Native diagram rendering: a Mermaid spec (flowchart / architecture / sequence …) → SVG,
// rendered client-side with securityLevel:"strict" (Mermaid sanitizes its own output). This
// covers "架构图/流程图" from request #3, generated in real time from the agent's output.
import { useEffect, useRef, useState } from "react";
import mermaid from "mermaid";

let counter = 0;

function themeName(): "neutral" | "dark" {
  const t = document.documentElement.dataset.theme;
  if (t === "dark") return "dark";
  if (t === "light") return "neutral";
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ? "dark" : "neutral";
}

export function Diagram({ code, caption }: { code: string; caption?: string }) {
  const ref = useRef<HTMLDivElement>(null);
  const [err, setErr] = useState<string | null>(null);
  const [themeTick, setThemeTick] = useState(0);

  // re-render the diagram when the OS/app theme flips (mermaid bakes its palette at initialize time)
  useEffect(() => {
    if (typeof window === "undefined") return;
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
    let alive = true;
    const id = `ax-mmd-${counter++}`;
    setErr(null);
    // re-apply the theme on every run so a light↔dark toggle repaints the SVG palette
    mermaid.initialize({ startOnLoad: false, securityLevel: "strict", theme: themeName(), fontFamily: "inherit" });
    mermaid
      .render(id, code.trim())
      .then(({ svg }) => {
        if (alive && ref.current) ref.current.innerHTML = svg; // mermaid strict-mode output
      })
      .catch((e) => {
        // mermaid appends a temp measurement node ("d"+id) to <body>; drop it if the parse threw
        document.getElementById("d" + id)?.remove();
        if (alive) setErr(String(e?.message || e).split("\n")[0]);
      });
    return () => {
      alive = false;
    };
  }, [code, themeTick]);

  return (
    <figure className="ax-fig">
      <div className="ax-fig__frame">
        {err ? (
          <div className="ax-fig__err">图形无法渲染 · {err}</div>
        ) : (
          <div ref={ref} className="ax-fig__mermaid" />
        )}
      </div>
      {caption && <figcaption className="ax-fig__cap"><span className="ax-fig__kind">图 · diagram</span> {caption}</figcaption>}
    </figure>
  );
}
