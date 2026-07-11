// The artifact rendered in its NATIVE form (Claude-Science-style): the agent's output is
// parsed into markdown prose interleaved with native science figures — `smiles` blocks
// become molecular structures, `mermaid` blocks become diagrams, `math` blocks become
// rendered equations. Everything else is GitHub-flavored markdown, SANITIZED before display
// (the same anti-poisoning posture as the rest of the product — imported/agent text is
// untrusted; links and remote images are stripped so a poisoned artifact can't navigate or
// beacon out).
import { useMemo } from "react";
import { marked } from "marked";
import DOMPurify from "dompurify";
import katex from "katex";
import "katex/dist/katex.min.css";
import { Molecule } from "./Molecule";
import { Diagram } from "./Diagram";

marked.use({ gfm: true, breaks: true });

type Seg =
  | { t: "md"; body: string }
  | { t: "smiles"; body: string }
  | { t: "mermaid"; body: string }
  | { t: "math"; body: string };

// Split the text at ```smiles / ```mermaid / ```math fences; everything else is markdown.
function segment(text: string): Seg[] {
  const re = /```(smiles|mermaid|math)[^\n]*\n([\s\S]*?)```/g;
  const segs: Seg[] = [];
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text))) {
    if (m.index > last) segs.push({ t: "md", body: text.slice(last, m.index) });
    segs.push({ t: m[1] as "smiles" | "mermaid" | "math", body: m[2].trim() });
    last = re.lastIndex;
  }
  if (last < text.length) segs.push({ t: "md", body: text.slice(last) });
  return segs;
}

function renderMd(md: string): string {
  const raw = marked.parse(md, { async: false }) as string;
  // strip anything that could navigate or beacon; keep link/inline-code text.
  return DOMPurify.sanitize(raw, { FORBID_TAGS: ["a", "img", "iframe", "form", "input"], FORBID_ATTR: ["style", "onerror", "onload"] });
}

function renderMath(tex: string): string {
  try {
    return katex.renderToString(tex, { displayMode: true, throwOnError: false, output: "html" });
  } catch {
    return `<pre class="ax-md__pre">${tex.replace(/[<>&]/g, (c) => ({ "<": "&lt;", ">": "&gt;", "&": "&amp;" }[c] as string))}</pre>`;
  }
}

export function ArtifactBody({ text }: { text: string }) {
  const segs = useMemo(() => segment(text), [text]);
  return (
    <div className="ax-md">
      {segs.map((s, i) => {
        if (s.t === "smiles") return <Molecule key={i} smiles={s.body} />;
        if (s.t === "mermaid") return <Diagram key={i} code={s.body} />;
        if (s.t === "math") return <div key={i} className="ax-md__math" dangerouslySetInnerHTML={{ __html: renderMath(s.body) }} />;
        // katex html is trusted (we generated it); markdown html is sanitized in renderMd.
        return <div key={i} dangerouslySetInnerHTML={{ __html: renderMd(s.body) }} />;
      })}
    </div>
  );
}
