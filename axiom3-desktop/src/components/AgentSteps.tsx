import { useState } from "react";
import { Check, X, ChevronRight } from "lucide-react";
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
const ROUTINE = new Set(["litsearch", "search", "search_skills", "load_skill", "read_file", "list_dir", "glob", "run", "job_status"]);

type Item = { kind: "group"; steps: AgentStepView[] } | { kind: "solo"; step: AgentStepView };

function group(steps: AgentStepView[]): Item[] {
  const out: Item[] = [];
  let run: AgentStepView[] = [];
  const flush = () => {
    if (run.length) { out.push({ kind: "group", steps: run }); run = []; }
  };
  for (const s of steps) {
    if (ROUTINE.has(s.role)) run.push(s);
    else { flush(); out.push({ kind: "solo", step: s }); }
  }
  flush();
  return out;
}

function summarize(steps: AgentStepView[], zh: boolean): string {
  let search = 0, cmd = 0, read = 0, load = 0;
  for (const s of steps) {
    if (s.role === "litsearch" || s.role === "search" || s.role === "search_skills") search++;
    else if (s.role === "run" || s.role === "job_status") cmd++;
    else if (s.role === "read_file" || s.role === "list_dir" || s.role === "glob") read++;
    else if (s.role === "load_skill") load++;
  }
  const parts: string[] = [];
  if (search) parts.push(zh ? `检索 ${search} 次` : `ran ${search} search${search > 1 ? "es" : ""}`);
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
  const items = group(steps);
  return (
    <div className="ax-chain">
      {items.map((it, i) =>
        it.kind === "group"
          ? <GroupRow key={i} steps={it.steps} lang={lang} />
          : <SoloRow key={i} step={it.step} lang={lang} />
      )}
    </div>
  );
}

function StatusIcon({ failed }: { failed: boolean }) {
  return (
    <span className={`ax-chain__ic ${failed ? "is-err" : "is-ok"}`}>
      {failed ? <X size={9} aria-hidden /> : <Check size={9} aria-hidden />}
    </span>
  );
}

function GroupRow({ steps, lang }: { steps: AgentStepView[]; lang: Lang }) {
  const [open, setOpen] = useState(false);
  const zh = lang === "zh";
  const failed = steps.some((s) => s.status === "failed" || s.status === "err");
  return (
    <div className="ax-chain__step">
      <button className={`ax-chain__card is-group ${open ? "is-open" : ""}`} onClick={() => setOpen((o) => !o)} aria-expanded={open}>
        <StatusIcon failed={failed} />
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
  const [open, setOpen] = useState(false);
  const zh = lang === "zh";
  const title = step.title || step.role;
  const body = (step.output || "").trim();
  const hasBody = body.length > 0;
  const failed = step.status === "failed" || step.status === "err";
  const isPy = step.role === "python";
  const figMeta = figureMeta(step, zh);
  const meta = figMeta ?? (step.lines != null ? `${step.lines} ${zh ? "行" : step.lines === 1 ? "line" : "lines"}` : "");

  return (
    <div className={`ax-chain__step ${nested ? "is-nested" : ""}`}>
      {step.reasoning && step.reasoning.trim() && <p className="ax-chain__narr">{step.reasoning.trim()}</p>}
      <button
        className={`ax-chain__card ${open ? "is-open" : ""} ${hasBody ? "" : "is-static"}`}
        onClick={() => hasBody && setOpen((o) => !o)}
        aria-expanded={hasBody ? open : undefined}
      >
        <StatusIcon failed={failed} />
        <span className="ax-chain__title">{title}</span>
        {meta && <span className="ax-chain__count">{meta}</span>}
        {hasBody && <ChevronRight size={13} className={`ax-chain__chev ${open ? "is-open" : ""}`} aria-hidden />}
      </button>
      {open && hasBody && (
        isPy
          ? (() => {
              const { code, output, figures } = parsePython(body);
              return (
                <>
                  <CodeBlock code={code} output={output} uiLang={lang} />
                  {figures.map((f, i) => <Pyfig key={i} spec={f} />)}
                </>
              );
            })()
          : <div className="ax-chain__body"><ArtifactBody text={body} /></div>
      )}
    </div>
  );
}
