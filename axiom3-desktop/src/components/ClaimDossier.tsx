import { AlertTriangle, ShieldAlert, Bot, Check, ExternalLink } from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Lang, t, codeLabel, findingDesc, vendorSignalLabel, reasonLabel, indepLabel } from "../i18n";
import type { ClaimEntry } from "../lib/types";
import { attentionOf, lampClass, attentionKey } from "../lib/attention";

// A retrieved-literature passage is formatted "[retrieved · doi:…] Title (year). <abstract>"
// by the backend; split off the citation so the source is labeled, not buried in prose.
function parseEvidence(p: string): { src: "retrieved" | "user"; cite?: string; text: string } {
  const m = p.match(/^\[retrieved · ([^\]]+)\]\s*([\s\S]*)$/);
  if (m) return { src: "retrieved", cite: m[1], text: m[2] };
  return { src: "user", text: p };
}

// Turn a citation token into a resolver URL a researcher can open: DOIs → doi.org,
// OpenAlex work ids → openalex.org. Both are well-known, safe resolvers.
export function citationUrl(cite: string): string | null {
  const doi = cite.match(/10\.\d{4,9}\/[^\s"'<>]+/);
  if (doi) return `https://doi.org/${doi[0]}`;
  const oax = cite.match(/W\d{6,}/);
  if (oax) return `https://openalex.org/${oax[0]}`;
  return null;
}

export function ClaimDossier({
  entry,
  lang,
  reviewed,
  onReview,
}: {
  entry: ClaimEntry;
  lang: Lang;
  reviewed?: boolean;
  onReview?: () => void;
}) {
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
    <div className={`ax-dossier ${reviewed ? "is-reviewed" : ""}`}>
      <div className="ax-dossier__head">
        <p className="ax-claim">{entry.text}</p>
        <span style={{ display: "flex", alignItems: "center", gap: 8, flexShrink: 0 }}>
          <span className="ax-att">
            <span className={`ax-lamp ${lampClass[att]}`} />
            {t(lang, attentionKey[att])}
          </span>
          {onReview && (
            <button
              className={`ax-iconbtn ${reviewed ? "is-active" : ""}`}
              onClick={onReview}
              title={reviewed ? (lang === "zh" ? "已复核 — 点击取消" : "reviewed — click to undo") : lang === "zh" ? "标记已复核" : "mark reviewed"}
              aria-label="mark reviewed"
            >
              <Check size={15} aria-hidden />
            </button>
          )}
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
            const url = e.cite ? citationUrl(e.cite) : null;
            return (
              <div className="ax-evi" key={i}>
                <p className="ax-evi__meta">
                  <span className="ax-evi__src">
                    {e.src === "retrieved" ? t(lang, "src_retrieved") : t(lang, "src_user")}
                  </span>
                  {e.cite ? ` · ${e.cite}` : ""}
                  {url && (
                    <button
                      className="ax-linkbtn"
                      onClick={() => openUrl(url)}
                      title={lang === "zh" ? "打开来源" : "open source"}
                      style={{ marginLeft: 6 }}
                    >
                      <ExternalLink size={12} aria-hidden />
                    </button>
                  )}
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
              {/* The load-bearing conclusion: was the agreement INDEPENDENT across model
                  families, or discounted as correlated? This is the distinction the app exists
                  to draw — data already on entry.fleet, just never shown before. */}
              <p className="ax-vote" style={{ marginBottom: 6 }}>
                <span className="ax-badge">{indepLabel(lang, entry.fleet.independence)}</span>
                {entry.fleet.reason ? ` · ${reasonLabel(lang, entry.fleet.reason)}` : ""}
              </p>
              {entry.fleet.vendors.map((v, i) => (
                <p className="ax-vote" key={i}>
                  <span className="ax-vote__family">{v.family}</span> · {vendorSignalLabel(lang, v.status)}
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
