// The in-thread live "thinking chain" — rendered ABOVE the composer in the main conversation
// (not tucked away in the right rail). It reads a live RunView from the module-level runStore
// and shows, as richly as the event stream allows: the phase progress, each spawned agent (with
// a launch animation + its name/model), and every step it takes — for coding/analysis runs that
// includes the VERBATIM code and its result (via AgentSteps), streamed live as the agent works.
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Square, Cpu } from "lucide-react";
import type { Lang } from "../i18n";
import type { RunView } from "../lib/types";
import { AgentSteps } from "./AgentSteps";
import { WorkflowStarting } from "./WorkflowStarting";
import { AskingCard } from "./AskingCard";
import { phaseLabel } from "../lib/phaseLabels";

function fmtDur(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m ${s % 60}s`;
}
function fmtTokens(n: number): string {
  return n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n);
}

export function ThreadActivity({ run, lang, onCancel }: { run: RunView; lang: Lang; onCancel?: () => void }) {
  const zh = lang === "zh";
  const live = run.status === "running";
  // tick for live per-agent durations while running
  const [, setTick] = useState(0);
  useEffect(() => {
    if (!live) return;
    const t = setInterval(() => setTick((n) => n + 1), 1000);
    return () => clearInterval(t);
  }, [live]);

  const idx = Math.max(0, run.phases.indexOf(run.phase));

  return (
    <div className="ax-tact">
      {/* ── phase progress ── */}
      {run.phases.length > 0 && (
        <div className="ax-tact__phases">
          {run.phases.map((p, i) => {
            const past = !live || i < idx;
            const now = live && i === idx;
            return (
              <span key={p} className={`ax-tact__ph ${now ? "is-now" : past ? "is-past" : "is-idle"}`}>
                <span className="ax-tact__phdot" />
                {phaseLabel(p, zh)}
              </span>
            );
          })}
          <span className="ax-tact__spacer" />
          <span className="ax-tact__runmeta">
            {run.agents.length} {zh ? "智能体" : "agents"}
            {" · "}
            {fmtDur((run.finishedMs ?? Date.now()) - run.startedMs)}
          </span>
          {live && onCancel && (
            <button className="ax-iconbtn ax-tact__cancel" onClick={onCancel} title={zh ? "取消" : "Cancel"} aria-label="cancel">
              <Square size={12} aria-hidden />
            </button>
          )}
        </div>
      )}

      {/* ── agents + their steps ── */}
      {run.agents.map((a) => {
        const dur = a.startedMs ? fmtDur((a.finishedMs ?? Date.now()) - a.startedMs) : "";
        const tok = a.tokensIn + a.tokensOut;
        const rich = a.rich ?? [];
        const dotClass = a.status === "running" ? "is-run" : a.status === "ok" ? "is-fin" : "is-err";
        return (
          <div className="ax-tact__agent" key={a.id}>
            <div className="ax-tact__ahead">
              <span className={`ax-adot ${dotClass}`} />
              <span className="ax-tact__arole">
                <Cpu size={11} aria-hidden />
                {zh ? "启动智能体" : "agent"} · {a.role || a.id}
              </span>
              {a.model && <span className="ax-tact__amodel">{a.model}{a.vendor ? ` · ${a.vendor}` : ""}</span>}
              <span className="ax-tact__spacer" />
              <span className="ax-tact__ameta">
                {tok > 0 ? `${fmtTokens(tok)} ${zh ? "令牌" : "tok"} · ` : ""}
                {dur}
              </span>
            </div>

            {rich.length > 0 ? (
              // coding / analysis: full structured steps — title + reasoning + verbatim code body
              <AgentSteps steps={rich} lang={lang} />
            ) : a.steps.length > 0 ? (
              // workflow sub-agents (no per-step structure): show the streamed notes
              <div className="ax-tact__notes">
                {a.steps.map((n, i) => (
                  <p className="ax-tact__note" key={i}>{n}</p>
                ))}
              </div>
            ) : a.summary ? (
              <p className="ax-tact__summary">{a.summary}</p>
            ) : a.status === "running" ? (
              <p className="ax-tact__note ax-tact__note--muted">{zh ? "思考中…" : "thinking…"}</p>
            ) : null}

            {a.error && <p className="ax-tact__err">{a.error}</p>}
          </div>
        );
      })}

      {run.agents.length === 0 && live && <WorkflowStarting lang={lang} />}

      {/* the agent is parked on a structured question — render the AskUserQuestion-style card */}
      {live && run.pendingAsk && (
        <AskingCard
          card={run.pendingAsk}
          lang={lang}
          onSubmit={(answer) => { invoke("answer_ask", { runid: run.id, answer }).catch(() => {}); }}
        />
      )}
    </div>
  );
}
