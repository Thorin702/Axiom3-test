import { AlertTriangle, ShieldAlert, Bot } from "lucide-react";
import { Lang, t, codeLabel, findingDesc, verdictLabel, reasonLabel } from "../i18n";
import type { ClaimEntry } from "../lib/types";
import { attentionOf, lampClass, attentionKey } from "../lib/attention";

// A retrieved-literature passage is formatted "[retrieved · doi:…] Title (year). <abstract>"
// by the backend; split off the citation so the source is labeled, not buried in prose.
function parseEvidence(p: string): { src: "retrieved" | "user"; cite?: string; text: string } {
  const m = p.match(/^\[retrieved · ([^\]]+)\]\s*([\s\S]*)$/);
  if (m) return { src: "retrieved", cite: m[1], text: m[2] };
  return { src: "user", text: p };
}

export function ClaimDossier({ entry, lang }: { entry: ClaimEntry; lang: Lang }) {
  const att = attentionOf(entry);
  // Reason codes that AREN'T already shown as deterministic checks (those live in the
  // checks section) — i.e. the verification/evidence-level reasons (grounded_single_source,
  // cross_vendor_disagreement, fleet_cap_exceeded …). Avoids duplicating the checker codes.
  const extraReason = (entry.reason ?? "")
    .split(",")
    .map((c) => c.trim())
    .filter((c) => c && !entry.findings.some((f) => f.checker === c))
    .join(",");

  return (
    <div className="ax-dossier">
      <div className="ax-dossier__head">
        <p className="ax-claim">{entry.text}</p>
        <span className="ax-att">
          <span className={`ax-lamp ${lampClass[att]}`} />
          {t(lang, attentionKey[att])}
        </span>
      </div>

      {entry.findings.length > 0 && (
        <>
          <div className="ax-sep" />
          <p className="ax-sec-label">{t(lang, "sec_checks")}</p>
          {entry.findings.map((f, i) => (
            <div className="ax-check" key={i}>
              <AlertTriangle size={17} color="var(--ax-lamp-amber)" style={{ marginTop: 1, flex: "none" }} aria-hidden />
              <div>
                <p className="ax-check__title">{codeLabel(lang, f.checker)}</p>
                <p className="ax-check__detail">
                  {findingDesc(lang, f.checker)}
                  {f.excerpt ? ` · ${f.excerpt}` : ""}
                </p>
              </div>
            </div>
          ))}
        </>
      )}

      {entry.evidence.length > 0 && (
        <>
          <div className="ax-sep" />
          <p className="ax-sec-label">{t(lang, "sec_evidence")}</p>
          {entry.evidence.map((p, i) => {
            const e = parseEvidence(p);
            return (
              <div className="ax-evi" key={i}>
                <p className="ax-evi__meta">
                  <span className="ax-evi__src">
                    {e.src === "retrieved" ? t(lang, "src_retrieved") : t(lang, "src_user")}
                  </span>
                  {e.cite ? ` · ${e.cite}` : ""}
                </p>
                <p className="ax-evi__text">{e.text}</p>
              </div>
            );
          })}
        </>
      )}

      {entry.fleet && entry.fleet.vendors.length > 0 && (
        <>
          <div className="ax-sep" />
          <div className="ax-signal">
            <Bot size={16} color="var(--ax-ink-faint)" style={{ marginTop: 2, flex: "none" }} aria-hidden />
            <div>
              <p className="ax-signal__title">
                {t(lang, "sec_signal")} <span className="ax-signal__caveat">({t(lang, "signal_caveat")})</span>
              </p>
              {entry.fleet.vendors.map((v, i) => (
                <p className="ax-vote" key={i}>
                  <span className="ax-vote__family">{v.family}</span> · {verdictLabel(lang, v.status)}
                  {v.reasoning ? ` — ${v.reasoning}` : ""}
                </p>
              ))}
            </div>
          </div>
        </>
      )}

      {att === "neutralized" && (
        <>
          <div className="ax-sep" />
          <div className="ax-check">
            <ShieldAlert size={17} color="var(--ax-lamp-violet)" style={{ marginTop: 1, flex: "none" }} aria-hidden />
            <p className="ax-check__detail">{codeLabel(lang, "evidence_injection_neutralized")}</p>
          </div>
        </>
      )}

      {extraReason && <p className="ax-reason">{reasonLabel(lang, extraReason)}</p>}
    </div>
  );
}
