// One FINISHED round of a multi-round session, kept permanently in the thread: the user's message for
// that round + its execution chain, collapsed by default. Starting a new round used to REPLACE the
// previous round's process (it was wiped, not hidden) — this keeps every round traceable.
import { useState } from "react";
import { ChevronRight, ChevronDown } from "lucide-react";
import { AgentSteps } from "./AgentSteps";
import type { AgentStepView } from "../lib/types";
import type { Lang } from "../i18n";

export function RoundHistory({
  lang,
  n,
  goal,
  steps,
}: {
  lang: Lang;
  n: number;
  goal: string;
  steps?: AgentStepView[];
}) {
  const zh = lang === "zh";
  const [open, setOpen] = useState(false);
  const count = steps?.length ?? 0;

  return (
    <div className="ax-round">
      {goal.trim() && (
        <div className="ax-usermsg">
          <span className="ax-usermsg__who">{zh ? "你" : "You"}</span>
          <p className="ax-usermsg__text">{goal}</p>
        </div>
      )}
      {count > 0 && (
        <div className="ax-round__exec">
          <button className="ax-round__head" onClick={() => setOpen((o) => !o)} aria-expanded={open}>
            {open ? <ChevronDown size={13} aria-hidden /> : <ChevronRight size={13} aria-hidden />}
            <span>{zh ? `第 ${n} 轮 · 执行过程 ${count} 步` : `Round ${n} · ${count} steps`}</span>
            <span className="ax-round__hint">{open ? (zh ? "收起" : "hide") : (zh ? "展开回溯" : "show")}</span>
          </button>
          {open && (
            <div className="ax-round__body">
              <AgentSteps steps={steps!} lang={lang} />
            </div>
          )}
        </div>
      )}
    </div>
  );
}
