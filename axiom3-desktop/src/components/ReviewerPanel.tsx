// The reviewer agent, made visible inline in the session — like Claude Science's
// "🔍 Reviewer · N finding" panel. This is our adversarial-verify critique, which we used
// to discard. It surfaces concrete things a careful reviewer would flag (gaps, unsupported
// claims, contradictions).
//
// RED LINE: findings are advisory notes on the WORK, never a verdict on whether a claim is
// true. Mirrors Claude Science's own caveat: "the reviewer reduces, but doesn't eliminate,
// errors… verify results before relying on them."
import { useState } from "react";
import { ScanSearch, ChevronRight, ChevronDown, AlertTriangle, Check } from "lucide-react";
import type { Lang } from "../i18n";

// Split a bullet/line-based critique into discrete findings. "The outputs are solid" style
// single-line all-clears collapse to zero findings.
function toFindings(review: string): string[] {
  const clean = review.trim();
  const lines = clean
    .split("\n")
    .map((l) => l.replace(/^\s*[-*•·]\s?/, "").trim())
    .filter((l) => l.length > 1);
  return lines;
}

const ALL_CLEAR = /^(no (issues|problems|concerns|critique)|the outputs are solid|looks (good|solid)|nothing to flag)/i;

export function ReviewerPanel({ lang, review }: { lang: Lang; review: string }) {
  const zh = lang === "zh";
  const [open, setOpen] = useState(true);
  const findings = toFindings(review);
  const allClear = findings.length <= 1 && ALL_CLEAR.test(review.trim());
  const n = allClear ? 0 : findings.length;

  return (
    <div className="ax-reviewer">
      <button className="ax-reviewer__head" onClick={() => setOpen((o) => !o)} aria-expanded={open}>
        <ScanSearch size={14} aria-hidden />
        <span className="ax-reviewer__title">{zh ? "审查员" : "Reviewer"}</span>
        <span className={`ax-reviewer__count ${n > 0 ? "has" : "clear"}`}>
          {n > 0 ? (zh ? `${n} 项发现` : `${n} finding${n > 1 ? "s" : ""}`) : zh ? "未见问题" : "no findings"}
        </span>
        <span className="ax-reviewer__chev">{open ? <ChevronDown size={13} /> : <ChevronRight size={13} />}</span>
      </button>

      {open && (
        <div className="ax-reviewer__body">
          {n === 0 ? (
            <div className="ax-reviewer__clear">
              <Check size={13} aria-hidden /> {zh ? "审查员未标记问题(仍需你自行核验)。" : "The reviewer flagged nothing (still verify yourself)."}
            </div>
          ) : (
            findings.map((f, i) => (
              <div className="ax-reviewer__finding" key={i}>
                <AlertTriangle size={14} aria-hidden />
                <span>{f}</span>
              </div>
            ))
          )}
          <p className="ax-hint" style={{ marginTop: 8 }}>
            {zh
              ? "审查员对照产出检查一致性、缺口与无据主张 —— 它减少但不消除错误,且不判断内容真伪。"
              : "The reviewer checks the output for consistency, gaps, and unsupported claims — it reduces but does not eliminate errors, and does not judge whether the content is true."}
          </p>
        </div>
      )}
    </div>
  );
}
