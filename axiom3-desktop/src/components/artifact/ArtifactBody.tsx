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
import { FigureFrame } from "./FigureFrame";
import { PrismaFlow } from "./PrismaFlow";
import { Survival } from "./Survival";
import { Funnel } from "./Funnel";

marked.use({ gfm: true, breaks: true });

type Kind =
  | "smiles" | "mermaid" | "math" | "chart" | "protein" | "dotplot"
  | "forest" | "heatmap" | "confusion" | "dist" | "volcano" | "manhattan"
  | "band" | "msa" | "rxn" | "field" | "quiver" | "network" | "track" | "tree"
  | "roc" | "seqlogo" | "sankey" | "spectrum" | "dose" | "slope" | "pyfig"
  | "prisma" | "km" | "survival" | "funnel";
type Seg = { t: "md"; body: string } | { t: Kind; body: string };

// Split the text at native-figure fences; everything else is markdown.
function segment(text: string): Seg[] {
  const re = /```(smiles|mermaid|math|chart|protein|dotplot|forest|heatmap|confusion|dist|volcano|manhattan|band|msa|rxn|field|quiver|network|track|tree|roc|seqlogo|sankey|spectrum|dose|slope|pyfig|prisma|km|survival|funnel)[^\n]*\n([\s\S]*?)```/g;
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
function katexHtml(tex: string, display: boolean): string {
  try {
    return katex.renderToString(tex.trim(), { displayMode: display, throwOnError: false, output: "html" });
  } catch {
    return `<code>${tex.replace(/[<>&]/g, (c) => ({ "<": "&lt;", ">": "&gt;", "&": "&amp;" }[c] as string))}</code>`;
  }
}

// Render markdown, but FIRST pull out inline/display math ($$…$$, \[…\], $…$, \(…\)) and render it via
// KaTeX — analysis/research answers are equation-heavy and inline math left to the markdown renderer
// shows up as literal `$\frac{}{}` gibberish (the "乱码" users saw). Math is stashed behind private-use
// placeholders that survive marked + DOMPurify untouched, then the (trusted, self-generated) KaTeX HTML
// is spliced back in AFTER sanitize — the same after-sanitize splice pattern injectCites uses.
// Private-use delimiters around the stash index — inert text through marked + DOMPurify, can't collide
// with real content (built via fromCharCode so no literal PUA chars live in the source).
const MATH_OPEN = String.fromCharCode(0xe000);
const MATH_CLOSE = String.fromCharCode(0xe001);
const MATH_RE = new RegExp(MATH_OPEN + "(\\d+)" + MATH_CLOSE, "g");
function renderMd(md: string): string {
  const math: string[] = [];
  const stash = (tex: string, display: boolean): string => {
    const i = math.length;
    math.push(katexHtml(tex, display));
    return `${MATH_OPEN}${i}${MATH_CLOSE}`;
  };
  const src = md
    .replace(/\$\$([\s\S]+?)\$\$/g, (_m, t) => stash(t, true))
    .replace(/\\\[([\s\S]+?)\\\]/g, (_m, t) => stash(t, true))
    .replace(/\\\(([\s\S]+?)\\\)/g, (_m, t) => stash(t, false))
    // inline $…$: require non-space just inside the delimiters and forbid $/newline within, so prose
    // like "$5 and $10" (space before the closing $) is NOT treated as math.
    .replace(/(?<![\\$])\$(?!\s)([^\n$]+?)(?<!\s)\$(?!\$)/g, (_m, t) => stash(t, false));
  const raw = marked.parse(src, { async: false }) as string;
  let html = DOMPurify.sanitize(raw, {
    ALLOWED_TAGS: MD_ALLOWED_TAGS,
    ALLOWED_ATTR: ["class", "colspan", "rowspan", "align"],
    ALLOW_DATA_ATTR: false,
  });
  if (math.length) html = html.replace(MATH_RE, (_m, n) => math[Number(n)] ?? "");
  return html;
}

function renderMath(tex: string): string {
  return katexHtml(tex, true);
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
        // NATIVE SVG/canvas figures → wrapped in FigureFrame so every one is click-to-zoom +
        // downloadable (same UX as Pyfig). Pyfig/Protein bring their own controls; math/markdown aren't figures.
        const native: Partial<Record<Seg["t"], React.ReactNode>> = {
          smiles: <Molecule smiles={s.body} />,
          mermaid: <Diagram code={s.body} />,
          chart: <Chart spec={s.body} />,
          dotplot: <DotPlot spec={s.body} />,
          forest: <Forest spec={s.body} />,
          heatmap: <Heatmap spec={s.body} />,
          confusion: <Confusion spec={s.body} />,
          dist: <Distribution spec={s.body} />,
          volcano: <Volcano spec={s.body} />,
          manhattan: <Manhattan spec={s.body} />,
          band: <Band spec={s.body} />,
          msa: <Alignment spec={s.body} />,
          rxn: <Reaction spec={s.body} />,
          field: <Field spec={s.body} />,
          quiver: <Quiver spec={s.body} />,
          network: <Network spec={s.body} />,
          track: <Track spec={s.body} />,
          tree: <Tree spec={s.body} />,
          roc: <Roc spec={s.body} />,
          seqlogo: <SeqLogo spec={s.body} />,
          sankey: <Sankey spec={s.body} />,
          spectrum: <Spectrum spec={s.body} />,
          dose: <Dose spec={s.body} />,
          slope: <Slope spec={s.body} />,
          prisma: <PrismaFlow spec={s.body} />,
          km: <Survival spec={s.body} />,
          survival: <Survival spec={s.body} />,
          funnel: <Funnel spec={s.body} />,
        };
        if (native[s.t]) return <FigureFrame key={i} kind={s.t}>{native[s.t]}</FigureFrame>;
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
