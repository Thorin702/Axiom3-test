import { useState } from "react";
import { Check, ChevronRight } from "lucide-react";
import type { AgentStepView } from "../lib/types";
import type { Lang } from "../i18n";
import { ArtifactBody } from "./artifact/ArtifactBody";
import { CodeBlock } from "./CodeBlock";
import { Pyfig } from "./artifact/Pyfig";

// The thinking chain, styled like Claude Science: the agent's narration as plain paragraphs, and its
// steps as collapsible rows — routine tool calls (searches, commands, reads) collapse into ONE
// "Ran N searches, ran a command · N steps" summary; a code/figure step is its own row that expands
// to a dark PYTHON code cell with a "Show output" toggle.

// Tools that fold into a run-summary group (vs. a code/plan/figure step that stands alone).
const ROUTINE = new Set(["litsearch", "web_search", "web_fetch", "pubchem", "uniprot", "pdb", "gene", "trials", "europepmc", "search", "search_skills", "load_skill", "read_file", "list_dir", "glob", "run", "job_status"]);

type Item =
  | { kind: "narr"; text: string }
  | { kind: "group"; steps: AgentStepView[] }
  | { kind: "solo"; step: AgentStepView };

function group(steps: AgentStepView[]): Item[] {
  const out: Item[] = [];
  let run: AgentStepView[] = [];
  const flush = () => {
    if (run.length) { out.push({ kind: "group", steps: run }); run = []; }
  };
  for (const s of steps) {
    // The agent's thinking for this step is surfaced as a narration paragraph in the MAIN flow so
    // the user follows the reasoning (like Claude Science). Routine actions (searches/reads) still
    // fold into ONE group — their narrations stream as a continuous train of thought ABOVE the
    // group, so we DON'T break the group per step (which would scatter it into many "检索 1 次"
    // rows). A solo step's narration sits right above that step, after the routine group is flushed.
    const narr = (s.reasoning || "").trim().replace(/^THINKING\s*[:：]\s*/i, "");
    if (ROUTINE.has(s.role)) {
      if (narr) out.push({ kind: "narr", text: narr });
      run.push(s);
    } else {
      flush();
      if (narr) out.push({ kind: "narr", text: narr });
      out.push({ kind: "solo", step: s });
    }
  }
  flush();
  return out;
}

function summarize(steps: AgentStepView[], zh: boolean): string {
  let search = 0, cmd = 0, read = 0, load = 0, web = 0;
  for (const s of steps) {
    if (s.role === "litsearch" || s.role === "search" || s.role === "search_skills" || s.role === "web_search") search++;
    else if (s.role === "web_fetch") web++;
    else if (s.role === "run" || s.role === "job_status") cmd++;
    else if (s.role === "read_file" || s.role === "list_dir" || s.role === "glob") read++;
    else if (s.role === "load_skill") load++;
  }
  const parts: string[] = [];
  if (search) parts.push(zh ? `检索 ${search} 次` : `ran ${search} search${search > 1 ? "es" : ""}`);
  if (web) parts.push(zh ? `读网页 ${web} 个` : `read ${web} page${web > 1 ? "s" : ""}`);
  if (cmd) parts.push(zh ? `执行命令 ${cmd} 次` : cmd > 1 ? `ran ${cmd} commands` : `ran a command`);
  if (read) parts.push(zh ? `读取 ${read} 项` : `read ${read} file${read > 1 ? "s" : ""}`);
  if (load) parts.push(zh ? `加载技能` : `loaded a skill`);
  const t = parts.join(zh ? "、" : ", ") || (zh ? "执行了若干步" : "ran a few steps");
  return zh ? t : t.charAt(0).toUpperCase() + t.slice(1);
}

// Pull the code, its text result, and any computed figures out of a python step's body:
// "```python\n<code>\n```\n\n```\n<out>\n```" followed by zero+ "```pyfig\n<base64>\n```" blocks.
function parsePython(body: string): { code: string; output: string; figures: string[] } {
  const code: string[] = [];
  const figures: string[] = [];
  const re = /```([^\n]*)\n([\s\S]*?)```/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(body))) {
    const lang = m[1].trim().toLowerCase();
    const content = m[2].replace(/\n$/, "");
    if (lang === "pyfig") figures.push(content.replace(/\s+/g, ""));
    else code.push(content);
  }
  return { code: code[0] ?? body, output: code[1] ?? "", figures };
}

function figureMeta(step: AgentStepView, zh: boolean): string | null {
  const m = step.output.match(/\[(\d+) figure/);
  if (m) return zh ? `${m[1]} 张图` : `${m[1]} figure${Number(m[1]) > 1 ? "s" : ""}`;
  return null;
}

export function AgentSteps({ steps, lang }: { steps: AgentStepView[]; lang: Lang }) {
  const items = group(steps.filter((s) => !isToolTagArtifact(s)));
  return (
    <div className="ax-chain">
      {items.map((it, i) =>
        it.kind === "narr"
          ? <p key={i} className="ax-chain__narr">{stripStatusGlyphs(it.text)}</p>
          : it.kind === "group"
            ? <GroupRow key={i} steps={it.steps} lang={lang} />
            : <SoloRow key={i} step={it.step} lang={lang} />
      )}
    </div>
  );
}

// A calm, uniform step marker: the thinking chain shows the agent's PROCESS, so every step gets the
// same MUTED GRAY check — no colored status badge at all (no green ✓, no red ✗); genuine run failures
// surface at the run/task level, not as alarming per-step icons.
function StatusIcon() {
  return (
    <span className="ax-chain__ic">
      <Check size={11} aria-hidden />
    </span>
  );
}

// Models sometimes pepper their narration/titles with colored status glyphs (✓ ✗ ❌ ✅ …) to mark
// tried/failed sub-steps. Those render as loud green/red marks in the chain, which we don't want —
// strip them so the process reads as one calm gray train of thought.
function stripStatusGlyphs(s: string): string {
  return s
    .replace(/[✓✔✅☑✗✘✕✖❌❎]️?/g, "")
    .replace(/[ \t]{2,}/g, " ")
    .replace(/[ \t]+([,，。;；:：])/g, "$1")
    .trim();
}

// Some models emit raw `<tool_call>…` tags that leak into a step's title. Strip the wrapper, and if a
// step is nothing but such a tag (no real output), it's a parse artifact — drop it from the chain.
function cleanTitle(step: AgentStepView): string {
  const t = stripStatusGlyphs(
    (step.title || "")
      .replace(/<\/?\s*tool_call[^>]*>/gi, "")
      .replace(/^\s*<\|?\s*tool[_a-z]*\|?>?/i, "")
  );
  return t || step.role || "";
}
function isToolTagArtifact(step: AgentStepView): boolean {
  const t = (step.title || "").trim();
  const looksLikeTag = /^<\/?\s*tool_call/i.test(t) || /^<\|?\s*tool/i.test(t);
  return looksLikeTag && !(step.output || "").trim();
}

function GroupRow({ steps, lang }: { steps: AgentStepView[]; lang: Lang }) {
  const [open, setOpen] = useState(false);
  const zh = lang === "zh";
  return (
    <div className="ax-chain__step">
      <button className={`ax-chain__card is-group ${open ? "is-open" : ""}`} onClick={() => setOpen((o) => !o)} aria-expanded={open}>
        <StatusIcon />
        <span className="ax-chain__title">{summarize(steps, zh)}</span>
        <span className="ax-chain__count">{steps.length} {zh ? "步" : steps.length === 1 ? "step" : "steps"}</span>
        <ChevronRight size={13} className={`ax-chain__chev ${open ? "is-open" : ""}`} aria-hidden />
      </button>
      {open && (
        <div className="ax-chain__sub">
          {steps.map((s, i) => <SoloRow key={i} step={s} lang={lang} nested />)}
        </div>
      )}
    </div>
  );
}

function SoloRow({ step, lang, nested }: { step: AgentStepView; lang: Lang; nested?: boolean }) {
  const zh = lang === "zh";
  const title = cleanTitle(step);
  const body = (step.output || "").trim();
  const hasBody = body.length > 0;
  const isPy = step.role === "python";
  // Parse a python step up-front so its computed FIGURES can render ALWAYS (a real-time plot is the
  // point of the thinking chain), while the code+output stay behind the collapsible toggle.
  const py = isPy && hasBody ? parsePython(body) : null;
  const figures = py?.figures ?? [];
  const hasFigures = figures.length > 0;
  // Figures show by default; the code/output cell (and non-figure bodies) default to COLLAPSED.
  const [open, setOpen] = useState(false);
  const figMeta = figureMeta(step, zh);
  const meta = figMeta ?? (step.lines != null ? `${step.lines} ${zh ? "行" : step.lines === 1 ? "line" : "lines"}` : "");

  return (
    <div className={`ax-chain__step ${nested ? "is-nested" : ""}`}>
      {/* narration is now hoisted into the main flow as a `narr` item (see group()), not per-row */}
      <button
        className={`ax-chain__card ${open ? "is-open" : ""} ${hasBody ? "" : "is-static"}`}
        onClick={() => hasBody && setOpen((o) => !o)}
        aria-expanded={hasBody ? open : undefined}
      >
        <StatusIcon />
        <span className="ax-chain__title">{title}</span>
        {meta && <span className="ax-chain__count">{meta}</span>}
        {hasBody && <ChevronRight size={13} className={`ax-chain__chev ${open ? "is-open" : ""}`} aria-hidden />}
      </button>
      {/* real-time figures — ALWAYS visible under the step, not hidden behind the toggle */}
      {hasFigures && (
        <div className="ax-chain__figs">
          {figures.map((f, i) => <Pyfig key={i} spec={f} />)}
        </div>
      )}
      {/* collapsible detail: code + text output (python), or the markdown body (other steps) */}
      {open && hasBody && (
        isPy && py
          ? <CodeBlock code={py.code} output={py.output} uiLang={lang} />
          : <div className="ax-chain__body"><ArtifactBody text={body} /></div>
      )}
    </div>
  );
}
