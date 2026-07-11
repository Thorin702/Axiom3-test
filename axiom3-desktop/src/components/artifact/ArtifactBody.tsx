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
import { Chart } from "./Chart";
import { Protein } from "./Protein";
import { DotPlot } from "./DotPlot";
import { Forest } from "./Forest";
import { Heatmap } from "./Heatmap";
import { Confusion } from "./Confusion";
import { Distribution } from "./Distribution";
import { Volcano } from "./Volcano";
import { Manhattan } from "./Manhattan";
import { Band } from "./Band";
import { Alignment } from "./Alignment";
import { Reaction } from "./Reaction";
import { Field } from "./Field";
import { Network } from "./Network";
import { Track } from "./Track";
import { Pyfig } from "./Pyfig";

marked.use({ gfm: true, breaks: true });

type Kind =
  | "smiles" | "mermaid" | "math" | "chart" | "protein" | "dotplot"
  | "forest" | "heatmap" | "confusion" | "dist" | "volcano" | "manhattan"
  | "band" | "msa" | "rxn" | "field" | "network" | "track" | "pyfig";
type Seg = { t: "md"; body: string } | { t: Kind; body: string };

// Split the text at native-figure fences; everything else is markdown.
function segment(text: string): Seg[] {
  const re = /```(smiles|mermaid|math|chart|protein|dotplot|forest|heatmap|confusion|dist|volcano|manhattan|band|msa|rxn|field|network|track|pyfig)[^\n]*\n([\s\S]*?)```/g;
  const segs: Seg[] = [];
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text))) {
    if (m.index > last) segs.push({ t: "md", body: text.slice(last, m.index) });
    segs.push({ t: m[1] as Kind, body: m[2].trim() });
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
        if (s.t === "chart") return <Chart key={i} spec={s.body} />;
        if (s.t === "dotplot") return <DotPlot key={i} spec={s.body} />;
        if (s.t === "forest") return <Forest key={i} spec={s.body} />;
        if (s.t === "heatmap") return <Heatmap key={i} spec={s.body} />;
        if (s.t === "confusion") return <Confusion key={i} spec={s.body} />;
        if (s.t === "dist") return <Distribution key={i} spec={s.body} />;
        if (s.t === "volcano") return <Volcano key={i} spec={s.body} />;
        if (s.t === "manhattan") return <Manhattan key={i} spec={s.body} />;
        if (s.t === "band") return <Band key={i} spec={s.body} />;
        if (s.t === "msa") return <Alignment key={i} spec={s.body} />;
        if (s.t === "rxn") return <Reaction key={i} spec={s.body} />;
        if (s.t === "field") return <Field key={i} spec={s.body} />;
        if (s.t === "network") return <Network key={i} spec={s.body} />;
        if (s.t === "track") return <Track key={i} spec={s.body} />;
        if (s.t === "pyfig") return <Pyfig key={i} spec={s.body} />;
        if (s.t === "protein") return <Protein key={i} source={s.body} />;
        if (s.t === "math") return <div key={i} className="ax-md__math" dangerouslySetInnerHTML={{ __html: renderMath(s.body) }} />;
        // katex html is trusted (we generated it); markdown html is sanitized in renderMd.
        return <div key={i} dangerouslySetInnerHTML={{ __html: renderMd(s.body) }} />;
      })}
    </div>
  );
}
