// The inline "Asking" card — the agent's structured clarifying question, styled like Claude Code's
// AskUserQuestion: a header chip per question, the question, 2–4 option cards (bold label + a
// description + an optional field-standard tag), single- or multi-select, an "Other…" free-text
// row, and a submit. Research-grade: the agent asks on methodological forks / permissions / intent,
// and it NEVER pre-selects an option for a real fork — the human owns the choice. Answering calls
// answer_ask, which unblocks the parked run and feeds the choice back to the model.
import { useMemo, useState } from "react";
import { HelpCircle, Check, ChevronRight, CornerDownLeft, ShieldAlert } from "lucide-react";
import type { Lang } from "../i18n";
import type { AskCard, AskQuestion } from "../lib/types";

const KIND_META: Record<string, { zh: string; en: string; cls: string }> = {
  intent: { zh: "开场对齐", en: "Intake", cls: "is-intent" },
  fork: { zh: "方法学决策", en: "Decision", cls: "is-fork" },
  permission: { zh: "权限 · 安全", en: "Permission", cls: "is-warn" },
  integrity: { zh: "红线", en: "Red line", cls: "is-warn" },
  scope: { zh: "范围", en: "Scope", cls: "is-neutral" },
  cost: { zh: "成本", en: "Cost", cls: "is-neutral" },
};

function isAnswered(q: AskQuestion, sel: Set<string>, other: string): boolean {
  // `allowOther` defaults to true (matches the render gate) — otherwise a typed Other answer would
  // never satisfy the submit gate and the user gets stranded.
  return sel.size > 0 || ((q.allowOther ?? true) && other.trim().length > 0);
}

export function AskingCard({ card, lang, busy, onSubmit }: {
  card: AskCard;
  lang: Lang;
  busy?: boolean;
  onSubmit: (answer: string) => void;
}) {
  const zh = lang === "zh";
  const questions = card.questions.filter((q) => q && q.question);
  const [active, setActive] = useState(0);
  // per-question selection (option labels) + free-text
  const [sel, setSel] = useState<Record<number, Set<string>>>({});
  const [other, setOther] = useState<Record<number, string>>({});
  const [whyOpen, setWhyOpen] = useState(false);
  const [submitted, setSubmitted] = useState(false); // one answer only — guards double-fire clobber
  const locked = busy || submitted;

  const q = questions[Math.min(active, questions.length - 1)];
  const qSel = sel[active] ?? new Set<string>();
  const qOther = other[active] ?? "";
  const kind = KIND_META[q?.kind ?? "fork"] ?? KIND_META.fork;

  const allAnswered = useMemo(
    () => questions.every((qq, i) => isAnswered(qq, sel[i] ?? new Set(), other[i] ?? "")),
    [questions, sel, other]
  );

  function toggle(label: string) {
    if (locked) return;
    setSel((prev) => {
      const cur = new Set(prev[active] ?? []);
      if (q.multiSelect) {
        cur.has(label) ? cur.delete(label) : cur.add(label);
      } else {
        cur.clear();
        cur.add(label);
      }
      return { ...prev, [active]: cur };
    });
  }

  function submit() {
    if (locked) return; // guard double-fire so a stray click can't clobber the sent answer
    setSubmitted(true);
    const lines = questions.map((qq, i) => {
      const picks: string[] = [...(sel[i] ?? [])];
      const o = (other[i] ?? "").trim();
      if (o) picks.push(`${zh ? "其他" : "Other"}: ${o}`);
      const head = qq.header?.trim() || qq.question;
      return `- ${head}: ${picks.join("; ") || (zh ? "(未选)" : "(none)")}`;
    });
    onSubmit(lines.join("\n"));
  }

  function useDefaults() {
    if (locked) return;
    setSubmitted(true);
    onSubmit("[proceed with your suggested field-standard defaults and DISCLOSE each assumption in the report's methods/limitations]");
  }

  // A card with no renderable question would otherwise park the run invisibly — offer a way out.
  if (!q) {
    return (
      <div className="ax-ask" role="group">
        <div className="ax-ask__label">{zh ? "研究智能体在等待,但这张卡片没有可显示的问题。" : "The agent is waiting, but this card has no renderable question."}</div>
        <div className="ax-ask__foot">
          <button className="ax-ask__submit" onClick={useDefaults} disabled={locked}>
            {zh ? "按默认继续" : "Proceed with defaults"}
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="ax-ask" role="group" aria-label={zh ? "需要你确认" : "Needs your input"}>
      <div className="ax-ask__label">
        <HelpCircle size={12} aria-hidden />
        {zh ? "研究智能体请你确认 —— 你来定,它不替你判断" : "The agent needs your call — it won't decide this for you"}
      </div>

      {/* header-chip tabs when the card carries more than one question */}
      {questions.length > 1 && (
        <div className="ax-ask__tabs" role="tablist">
          {questions.map((qq, i) => {
            const answered = isAnswered(qq, sel[i] ?? new Set(), other[i] ?? "");
            const k = KIND_META[qq.kind ?? "fork"] ?? KIND_META.fork;
            return (
              <button
                key={i}
                role="tab"
                aria-selected={i === active}
                className={`ax-ask__chip ${k.cls} ${i === active ? "is-on" : ""}`}
                onClick={() => { setActive(i); setWhyOpen(false); }}
              >
                {answered && <Check size={11} aria-hidden />}
                {qq.header?.trim() || `Q${i + 1}`}
              </button>
            );
          })}
        </div>
      )}

      <div className={`ax-ask__kind ${kind.cls}`}>
        {(q.kind === "permission" || q.kind === "integrity") && <ShieldAlert size={11} aria-hidden />}
        {zh ? kind.zh : kind.en}
      </div>
      <p className="ax-ask__q">{q.question}</p>

      {q.why_it_matters && (
        <div className="ax-ask__why">
          <button className="ax-ask__whytoggle" onClick={() => setWhyOpen((o) => !o)} aria-expanded={whyOpen}>
            <ChevronRight size={12} className={`ax-ask__whychev ${whyOpen ? "is-open" : ""}`} aria-hidden />
            {zh ? "为什么问我?" : "Why am I being asked?"}
          </button>
          {whyOpen && <p className="ax-ask__whytext">{q.why_it_matters}</p>}
        </div>
      )}

      <div className="ax-ask__opts">
        {/* Never render an option with no readable label: it draws as a blank radio the user can
            click but cannot read, and selecting it sends an empty answer back to the agent. The
            backend now repairs/drops these, but a card persisted by an older build can still carry
            them. */}
        {(q.options ?? []).filter((o) => o && typeof o.label === "string" && o.label.trim()).map((opt, i) => {
          const on = qSel.has(opt.label);
          return (
            <button key={i} className={`ax-ask-opt ${on ? "sel" : ""}`} onClick={() => toggle(opt.label)} disabled={locked}>
              <span className="ax-ask-opt__tick">
                {q.multiSelect
                  ? <span className={`ax-ask-opt__box ${on ? "on" : ""}`}>{on && <Check size={11} aria-hidden />}</span>
                  : <span className={`ax-ask-opt__radio ${on ? "on" : ""}`} />}
              </span>
              <span className="ax-ask-opt__main">
                <span className="ax-ask-opt__label">
                  {opt.label}
                  {opt.convention && <span className="ax-ask-opt__conv">{zh ? "领域惯例" : "field-standard"}</span>}
                </span>
                {opt.description && <span className="ax-ask-opt__desc">{opt.description}</span>}
                {opt.consequence && <span className="ax-ask-opt__conseq">→ {opt.consequence}</span>}
              </span>
            </button>
          );
        })}
      </div>

      {(q.allowOther ?? true) && (
        <div className="ax-ask__other">
          <input
            className="ax-ask__otherinput"
            placeholder={zh ? "其他 —— 用你自己的话覆盖以上选项…" : "Other — override the options in your own words…"}
            value={qOther}
            disabled={locked}
            onChange={(e) => setOther((p) => ({ ...p, [active]: e.target.value }))}
          />
        </div>
      )}

      <div className="ax-ask__foot">
        <button className="ax-ask__defaults" onClick={useDefaults} disabled={locked} title={zh ? "让它按领域惯例默认继续,并在报告中如实披露" : "Proceed on field-standard defaults, disclosed in the report"}>
          {zh ? "按默认继续" : "Use defaults"}
        </button>
        <button className="ax-ask__submit" onClick={submit} disabled={locked || !allAnswered}>
          <CornerDownLeft size={13} aria-hidden />
          {questions.length > 1 ? (zh ? "确认全部并继续" : "Confirm all & run") : (zh ? "确认并继续" : "Confirm & run")}
        </button>
      </div>
    </div>
  );
}
