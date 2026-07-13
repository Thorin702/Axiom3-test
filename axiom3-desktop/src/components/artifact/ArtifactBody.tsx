// The artifact rendered in its NATIVE form (Claude-Science-style): the agent's output is
// parsed into markdown prose interleaved with native science figures — `smiles` blocks
// become molecular structures, `mermaid` blocks become diagrams, `math` blocks become
// rendered equations. Everything else is GitHub-flavored markdown, SANITIZED before display
// (the same anti-poisoning posture as the rest of the product — imported/agent text is
// untrusted; links and remote images are stripped so a poisoned artifact can't navigate or
// beacon out).
import { useMemo, useState, type MouseEvent } from "react";
import { marked } from "marked";
import DOMPurify from "dompurify";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { CiteSource } from "../../lib/types";
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
import { Quiver } from "./Quiver";
import { Network } from "./Network";
import { Track } from "./Track";
import { Tree } from "./Tree";
import { Roc } from "./Roc";
import { SeqLogo } from "./SeqLogo";
import { Sankey } from "./Sankey";
import { Spectrum } from "./Spectrum";
import { Dose } from "./Dose";
import { Slope } from "./Slope";
import { Pyfig } from "./Pyfig";

marked.use({ gfm: true, breaks: true });

type Kind =
  | "smiles" | "mermaid" | "math" | "chart" | "protein" | "dotplot"
  | "forest" | "heatmap" | "confusion" | "dist" | "volcano" | "manhattan"
  | "band" | "msa" | "rxn" | "field" | "quiver" | "network" | "track" | "tree"
  | "roc" | "seqlogo" | "sankey" | "spectrum" | "dose" | "slope" | "pyfig";
type Seg = { t: "md"; body: string } | { t: Kind; body: string };

// Split the text at native-figure fences; everything else is markdown.
function segment(text: string): Seg[] {
  const re = /```(smiles|mermaid|math|chart|protein|dotplot|forest|heatmap|confusion|dist|volcano|manhattan|band|msa|rxn|field|quiver|network|track|tree|roc|seqlogo|sankey|spectrum|dose|slope|pyfig)[^\n]*\n([\s\S]*?)```/g;
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

// Prose/table tags that CANNOT load a remote resource. Agent/imported markdown is untrusted,
// so we ALLOWLIST (not denylist): anything not here — img, video, audio, svg, iframe, a, object,
// … — is dropped, so a poisoned artifact can neither navigate nor beacon out (a denylist missed
// <video poster>, <svg><image href>, <audio src>, etc.).
const MD_ALLOWED_TAGS = [
  "p", "br", "hr", "span", "div", "strong", "em", "b", "i", "u", "del", "s", "mark", "sub", "sup",
  "code", "pre", "kbd", "samp", "blockquote", "h1", "h2", "h3", "h4", "h5", "h6",
  "ul", "ol", "li", "dl", "dt", "dd", "table", "thead", "tbody", "tfoot", "tr", "th", "td", "caption",
];
function renderMd(md: string): string {
  const raw = marked.parse(md, { async: false }) as string;
  return DOMPurify.sanitize(raw, {
    ALLOWED_TAGS: MD_ALLOWED_TAGS,
    ALLOWED_ATTR: ["class", "colspan", "rowspan", "align"],
    ALLOW_DATA_ATTR: false,
  });
}

function renderMath(tex: string): string {
  try {
    return katex.renderToString(tex, { displayMode: true, throwOnError: false, output: "html" });
  } catch {
    return `<pre class="ax-md__pre">${tex.replace(/[<>&]/g, (c) => ({ "<": "&lt;", ">": "&gt;", "&": "&amp;" }[c] as string))}</pre>`;
  }
}

// Turn `[Sn]` markers into clickable citation superscripts — ONLY for ids we actually have a source
// for (an orphaned marker from a sub-agent stays plain text). Applied to already-sanitized HTML that
// we then set ourselves, so the injected <sup> survives (data-sid isn't re-sanitized away).
//
// The scan is TAG- and CODE-AWARE: a blind string replace would (a) corrupt a marker that lands
// inside a tag (`<td class="…[S3]…">`) and (b) wrongly "citify" a literal `[S3]` a report shows
// INSIDE a code sample. So we replace only in real text, never inside `<…>` or `<code>/<pre>`.
function injectCites(html: string, has: (id: number) => boolean): string {
  const lower = html.toLowerCase();
  let out = "";
  let i = 0;
  let inTag = false;   // between '<' and '>'
  let codeDepth = 0;   // nesting depth of <code>/<pre>
  while (i < html.length) {
    const ch = html[i];
    if (inTag) {
      out += ch;
      if (ch === ">") inTag = false;
      i++;
      continue;
    }
    if (ch === "<") {
      if (lower.startsWith("<code", i) || lower.startsWith("<pre", i)) codeDepth++;
      else if (lower.startsWith("</code", i) || lower.startsWith("</pre", i)) codeDepth = Math.max(0, codeDepth - 1);
      inTag = true;
      out += ch;
      i++;
      continue;
    }
    if (codeDepth === 0 && ch === "[" && html[i + 1] === "S") {
      const m = /^\[S(\d+)\]/.exec(html.slice(i, i + 12));
      if (m && has(Number(m[1]))) {
        out += `<sup class="ax-cite" data-sid="${m[1]}" title="点击溯源">[${m[1]}]</sup>`;
        i += m[0].length;
        continue;
      }
    }
    out += ch;
    i++;
  }
  return out;
}

export function ArtifactBody({ text, sources }: { text: string; sources?: CiteSource[] }) {
  const segs = useMemo(() => segment(text), [text]);
  const srcMap = useMemo(() => new Map((sources ?? []).map((s) => [s.id, s])), [sources]);
  const [cite, setCite] = useState<CiteSource | null>(null);
  const onClick = srcMap.size
    ? (e: MouseEvent) => {
        const el = (e.target as HTMLElement).closest?.(".ax-cite");
        if (el) {
          const s = srcMap.get(Number(el.getAttribute("data-sid")));
          if (s) setCite(s);
        }
      }
    : undefined;
  return (
    <div className="ax-md" onClick={onClick}>
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
        if (s.t === "quiver") return <Quiver key={i} spec={s.body} />;
        if (s.t === "network") return <Network key={i} spec={s.body} />;
        if (s.t === "track") return <Track key={i} spec={s.body} />;
        if (s.t === "tree") return <Tree key={i} spec={s.body} />;
        if (s.t === "roc") return <Roc key={i} spec={s.body} />;
        if (s.t === "seqlogo") return <SeqLogo key={i} spec={s.body} />;
        if (s.t === "sankey") return <Sankey key={i} spec={s.body} />;
        if (s.t === "spectrum") return <Spectrum key={i} spec={s.body} />;
        if (s.t === "dose") return <Dose key={i} spec={s.body} />;
        if (s.t === "slope") return <Slope key={i} spec={s.body} />;
        if (s.t === "pyfig") return <Pyfig key={i} spec={s.body} />;
        if (s.t === "protein") return <Protein key={i} source={s.body} />;
        if (s.t === "math") return <div key={i} className="ax-md__math" dangerouslySetInnerHTML={{ __html: renderMath(s.body) }} />;
        // katex html is trusted (we generated it); markdown html is sanitized in renderMd, then we
        // add citation superscripts on top of the already-sanitized string.
        const html = srcMap.size ? injectCites(renderMd(s.body), (id) => srcMap.has(id)) : renderMd(s.body);
        return <div key={i} dangerouslySetInnerHTML={{ __html: html }} />;
      })}
      {cite && <CitePopover source={cite} onClose={() => setCite(null)} />}
    </div>
  );
}

// Only ever hand http(s) URLs to the OS opener. Source urls are model-RETRIEVED (untrusted) data, and
// the Tauri opener capability has no pre-configured scheme scope — so a poisoned "file:///…" or a
// custom-scheme url would otherwise reach `open`. Returns the url iff it's a safe web link, else null.
function safeHttpUrl(u: string): string | null {
  try {
    const p = new URL(u);
    return p.protocol === "http:" || p.protocol === "https:" ? u : null;
  } catch {
    return null;
  }
}

// Click a [Sn] marker → this popover shows the original source (title, link, the retrieved passage).
function CitePopover({ source, onClose }: { source: CiteSource; onClose: () => void }) {
  const kind = source.kind === "paper" ? "📄 文献" : source.kind === "db" ? "🔬 数据库" : "🌐 网页";
  const href = safeHttpUrl(source.url);
  return (
    <>
      <div className="ax-cite__scrim" onClick={onClose} />
      <div className="ax-cite__pop" role="dialog" aria-label="来源溯源">
        <div className="ax-cite__head">
          <span className="ax-cite__kind">{kind} · 来源 [S{source.id}]</span>
          <button className="ax-cite__x" onClick={onClose} aria-label="关闭">×</button>
        </div>
        <div className="ax-cite__title">{source.title}</div>
        {href && (
          <button className="ax-cite__link" onClick={() => openUrl(href).catch(() => {})} title={href}>
            {href}　↗
          </button>
        )}
        <div className="ax-cite__excerpt">{source.excerpt}</div>
      </div>
    </>
  );
}
