// Native diagram rendering: a Mermaid spec (flowchart / architecture / sequence …) → SVG,
// rendered client-side with securityLevel:"strict" (Mermaid sanitizes its own output). This
// covers "架构图/流程图" from request #3, generated in real time from the agent's output.
import { useEffect, useRef, useState } from "react";
import mermaid from "mermaid";

let inited = false;
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

  useEffect(() => {
    if (!inited) {
      mermaid.initialize({ startOnLoad: false, securityLevel: "strict", theme: themeName(), fontFamily: "inherit" });
      inited = true;
    }
    let alive = true;
    const id = `ax-mmd-${counter++}`;
    setErr(null);
    mermaid
      .render(id, code.trim())
      .then(({ svg }) => {
        if (alive && ref.current) ref.current.innerHTML = svg; // mermaid strict-mode output
      })
      .catch((e) => {
        if (alive) setErr(String(e?.message || e).split("\n")[0]);
      });
    return () => {
      alive = false;
    };
  }, [code]);

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
