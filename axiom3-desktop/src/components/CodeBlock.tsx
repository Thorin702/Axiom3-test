// A Claude-Science-style code cell for the thinking chain: a dark card with a "LANG · ENV env"
// header, syntax-highlighted source, and a separately-collapsible "Show output" section. Mirrors
// the reference exactly — the code is shown expanded, its result hidden behind a Show-output toggle.
import { useState } from "react";
import { ChevronRight } from "lucide-react";
import type { Lang } from "../i18n";

const PY_KEYWORDS = new Set([
  "import", "from", "as", "def", "return", "for", "while", "if", "else", "elif", "in", "not", "and",
  "or", "with", "try", "except", "finally", "class", "lambda", "is", "break", "continue", "pass",
  "yield", "global", "nonlocal", "raise", "assert", "del", "async", "await", "None", "True", "False",
]);
const PY_BUILTINS = new Set([
  "print", "len", "range", "list", "dict", "set", "tuple", "int", "float", "str", "bool", "open",
  "enumerate", "zip", "map", "filter", "sorted", "sum", "min", "max", "abs", "round", "type", "isinstance",
  "super", "self", "np", "pd", "plt", "mpl",
]);

function esc(s: string): string {
  return s.replace(/[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" }[c] as string));
}

// Single-pass tokenizer → HTML with <span class="tk-*">. Order matters (comment/string before ident).
function highlightPython(code: string): string {
  const re =
    /(#[^\n]*)|('''[\s\S]*?'''|"""[\s\S]*?"""|'(?:\\.|[^'\\])*'|"(?:\\.|[^"\\])*")|(\b\d+\.?\d*\b)|(\b[A-Za-z_]\w*\b)|(@\w+)/g;
  let out = "";
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(code))) {
    out += esc(code.slice(last, m.index));
    last = re.lastIndex;
    if (m[1]) out += `<span class="tk-com">${esc(m[1])}</span>`;
    else if (m[2]) out += `<span class="tk-str">${esc(m[2])}</span>`;
    else if (m[3]) out += `<span class="tk-num">${esc(m[3])}</span>`;
    else if (m[4]) {
      const w = m[4];
      const cls = PY_KEYWORDS.has(w) ? "tk-kw" : PY_BUILTINS.has(w) ? "tk-bi" : "tk-id";
      // a name immediately followed by "(" reads as a call
      const call = code[re.lastIndex] === "(" && cls === "tk-id";
      out += `<span class="${call ? "tk-fn" : cls}">${esc(w)}</span>`;
    } else if (m[5]) out += `<span class="tk-dec">${esc(m[5])}</span>`;
  }
  out += esc(code.slice(last));
  return out;
}

export function CodeBlock({ code, output, lang: langName = "python", env = "python", uiLang }: {
  code: string;
  output?: string;
  lang?: string;
  env?: string;
  uiLang: Lang;
}) {
  const [showOut, setShowOut] = useState(false);
  const zh = uiLang === "zh";
  const hl = langName.toLowerCase() === "python" ? highlightPython(code) : esc(code);
  const hasOut = !!output && output.trim().length > 0;
  return (
    <div className="ax-code">
      <div className="ax-code__head">
        <span className="ax-code__lang">{langName.toUpperCase()}</span>
        <span className="ax-code__env">ENV {env}</span>
      </div>
      <pre className="ax-code__src"><code dangerouslySetInnerHTML={{ __html: hl }} /></pre>
      {hasOut && (
        <div className="ax-code__out">
          <button className="ax-code__outtoggle" onClick={() => setShowOut((o) => !o)} aria-expanded={showOut}>
            <ChevronRight size={13} className={`ax-code__chev ${showOut ? "is-open" : ""}`} aria-hidden />
            {showOut ? (zh ? "隐藏输出" : "Hide output") : (zh ? "显示输出" : "Show output")}
          </button>
          {showOut && <pre className="ax-code__outbody">{output}</pre>}
        </div>
      )}
    </div>
  );
}
