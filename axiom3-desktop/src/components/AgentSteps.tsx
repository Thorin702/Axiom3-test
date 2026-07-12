import { useState } from "react";
import { Check, X, ChevronRight } from "lucide-react";
import type { AgentStepView } from "../lib/types";
import type { Lang } from "../i18n";
import { ArtifactBody } from "./artifact/ArtifactBody";

// A Claude-Science-style "thinking chain": the agent's narration (reasoning) shown as plain
// paragraphs, interleaved with collapsible step cards — each a status icon + a short title + an
// output line-count, expandable to the code/command and its result.
export function AgentSteps({ steps, lang }: { steps: AgentStepView[]; lang: Lang }) {
  return (
    <div className="ax-chain">
      {steps.map((s, i) => (
        <AgentStepCard key={i} step={s} lang={lang} />
      ))}
    </div>
  );
}

function AgentStepCard({ step, lang }: { step: AgentStepView; lang: Lang }) {
  const [open, setOpen] = useState(false);
  const zh = lang === "zh";
  const title = step.title || step.role;
  const body = (step.output || "").trim();
  const hasBody = body.length > 0;
  const failed = step.status === "failed" || step.status === "err";
  // Count label: an explicit line count for tool steps, else fall back to the body's line count.
  const n = step.lines != null ? step.lines : hasBody ? body.split("\n").length : 0;
  const count =
    step.lines != null || hasBody
      ? n === 0
        ? zh ? "无输出" : "no output"
        : `${n} ${zh ? "行输出" : n === 1 ? "line" : "lines"}`
      : "";

  return (
    <div className="ax-chain__step">
      {step.reasoning && step.reasoning.trim() && (
        <p className="ax-chain__narr">{step.reasoning.trim()}</p>
      )}
      <button
        type="button"
        className={`ax-chain__card ${open ? "is-open" : ""} ${hasBody ? "" : "is-static"}`}
        onClick={() => hasBody && setOpen((o) => !o)}
        aria-expanded={hasBody ? open : undefined}
      >
        <span className={`ax-chain__ic ${failed ? "is-err" : "is-ok"}`}>
          {failed ? <X size={9} aria-hidden /> : <Check size={9} aria-hidden />}
        </span>
        <span className="ax-chain__title">{title}</span>
        {count && <span className="ax-chain__count">{count}</span>}
        {hasBody && <ChevronRight size={13} className={`ax-chain__chev ${open ? "is-open" : ""}`} aria-hidden />}
      </button>
      {open && hasBody && (
        <div className="ax-chain__body">
          <ArtifactBody text={body} />
        </div>
      )}
    </div>
  );
}
