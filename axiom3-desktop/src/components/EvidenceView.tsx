// 证据与文献 — the REAL evidence surfaced by the checks you've run: retrieved passages
// and the DOIs/citations found in them. This tool only checks and surfaces — it never
// concludes. Green marks a passage that was retrieved as evidence, not that it is true.
import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Search, BookOpen, Quote, FileText, Link2, ExternalLink } from "lucide-react";
import type { Lang } from "../i18n";
import type { Task } from "../lib/types";
import { citationUrl } from "./ClaimDossier";

const S = {
  zh: {
    title: "证据与文献",
    sub: "你运行过的校对所召回的证据片段与其中的引用汇集于此,仅供自行判断 —— 本工具只核验与呈现,不下结论。",
    filterPh: "筛选证据、来源或 DOI…",
    retrieved: "检索到的证据",
    checks: "发现的引用 / DOI",
    checksNote: "这些是在证据中出现的 DOI,仅为定位,不代表对内容真伪的判断。",
    kinds: { retrieved: "检索", user: "用户提供" },
    empty: "还没有证据",
    emptySub: "运行一次带文献检索的 /校对 后,召回的证据会显示在这里。",
    from: "来自",
    none: "没有匹配的证据。",
    noDoi: "尚未在证据中发现 DOI。",
  },
  en: {
    title: "Evidence & Literature",
    sub: "Retrieved passages and their citations from the checks you've run gather here for your own judgement — this tool only checks and surfaces, it never concludes.",
    filterPh: "Filter evidence, source or DOI…",
    retrieved: "Retrieved evidence",
    checks: "Citations / DOIs found",
    checksNote: "These are DOIs that appear in the evidence — for locating sources only, not a judgement of the content.",
    kinds: { retrieved: "retrieved", user: "user-provided" },
    empty: "No evidence yet",
    emptySub: "Run a /check with literature retrieval on, and the recalled evidence shows up here.",
    from: "from",
    none: "No matching evidence.",
    noDoi: "No DOIs found in the evidence yet.",
  },
};

type Ev = { src: "retrieved" | "user"; cite?: string; text: string; claim: string };

function parseEv(raw: string, claim: string): Ev {
  const m = raw.match(/^\[retrieved · ([^\]]+)\]\s*([\s\S]*)$/);
  if (m) return { src: "retrieved", cite: m[1], text: m[2], claim };
  return { src: "user", text: raw, claim };
}

export function EvidenceView({ lang, tasks }: { lang: Lang; tasks: Task[] }) {
  const s = S[lang];
  const [query, setQuery] = useState("");
  const [chain, setChain] = useState<{ ok: boolean; events: number } | null>(null);

  useEffect(() => {
    invoke<{ event_count: number; chain_ok: boolean }>("project_status")
      .then((p) => setChain({ ok: p.chain_ok, events: p.event_count }))
      .catch(() => setChain(null));
  }, []);

  const all: Ev[] = useMemo(
    () =>
      tasks.flatMap((t) => (t.result?.entries ?? []).flatMap((e) => e.evidence.map((raw) => parseEv(raw, e.text)))),
    [tasks]
  );
  const q = query.trim().toLowerCase();
  const rows = all.filter((e) => !q || e.text.toLowerCase().includes(q) || (e.cite ?? "").toLowerCase().includes(q) || e.claim.toLowerCase().includes(q));
  const dois = useMemo(() => {
    const set = new Set<string>();
    for (const e of all) (e.cite ?? "").match(/10\.\d{4,}\/[^\s\]]+/g)?.forEach((d) => set.add(d));
    return [...set];
  }, [all]);

  return (
    <div className="ax-surface">
      <div className="ax-surface__head">
        <div>
          <h2 className="ax-surface__title">{s.title}</h2>
          <p className="ax-surface__sub">{s.sub}</p>
        </div>
        {chain && (
          <div className="ax-surface__actions">
            <span className="ax-badge">
              <span className={`ax-lamp ${chain.ok ? "is-green" : "is-amber"}`} />
              {chain.events} {lang === "zh" ? "事件" : "events"}
            </span>
          </div>
        )}
      </div>

      {all.length === 0 ? (
        <div className="ax-empty" style={{ minHeight: 0, padding: "28px 16px" }}>
          <div className="ax-empty__title">{s.empty}</div>
          <div className="ax-empty__sub">{s.emptySub}</div>
        </div>
      ) : (
        <>
          <div className="ax-input" style={{ display: "flex", alignItems: "center", gap: 8 }}>
            <Search size={15} />
            <input
              className="ax-input"
              style={{ border: "none", padding: 0, background: "transparent", flex: 1 }}
              placeholder={s.filterPh}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />
          </div>

          <div className="ax-sec-label" style={{ marginTop: 12 }}><BookOpen size={14} style={{ verticalAlign: "-2px", marginRight: 6 }} /> {s.retrieved} · {rows.length}</div>
          {rows.length === 0 ? (
            <div className="ax-hint">{s.none}</div>
          ) : (
            <div className="ax-results__list">
              {rows.slice(0, 60).map((e, i) => {
                const url = e.cite ? citationUrl(e.cite) : null;
                return (
                  <div className="ax-row" key={i}>
                    <span className={`ax-lamp ${e.src === "retrieved" ? "is-green" : "is-neutral"}`} />
                    <div className="ax-row__main">
                      <div className="ax-row__title" style={{ whiteSpace: "normal" }}>{e.text}</div>
                      <div className="ax-row__sub">
                        <FileText size={12} style={{ verticalAlign: "-2px", marginRight: 4 }} />
                        {e.cite ?? s.kinds[e.src]} · {s.from} {e.claim.slice(0, 40)}{e.claim.length > 40 ? "…" : ""}
                      </div>
                    </div>
                    {url ? (
                      <button className="ax-linkbtn" onClick={() => openUrl(url)} title={url}>
                        <ExternalLink size={13} aria-hidden />
                      </button>
                    ) : (
                      <span className="ax-badge">{s.kinds[e.src]}</span>
                    )}
                  </div>
                );
              })}
            </div>
          )}

          <div className="ax-sec-label" style={{ marginTop: 16 }}><Quote size={14} style={{ verticalAlign: "-2px", marginRight: 6 }} /> {s.checks} · {dois.length}</div>
          {dois.length === 0 ? (
            <div className="ax-hint">{s.noDoi}</div>
          ) : (
            <div className="ax-results__list">
              {dois.map((d) => (
                <button
                  className="ax-row"
                  key={d}
                  style={{ width: "100%", textAlign: "left", cursor: "pointer" }}
                  onClick={() => openUrl(`https://doi.org/${d}`)}
                  title={`https://doi.org/${d}`}
                >
                  <Link2 size={15} aria-hidden />
                  <div className="ax-row__main">
                    <div className="ax-row__title" style={{ fontFamily: "ui-monospace, Menlo, monospace", fontSize: 12 }}>{d}</div>
                  </div>
                  <span className="ax-badge"><ExternalLink size={11} style={{ verticalAlign: "-1px", marginRight: 3 }} />DOI</span>
                </button>
              ))}
            </div>
          )}
          <div className="ax-hint" style={{ marginTop: 8 }}>{s.checksNote}</div>
        </>
      )}
    </div>
  );
}
