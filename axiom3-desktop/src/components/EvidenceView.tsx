import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Search, BookOpen, Quote, Link2, FileText, RefreshCw } from "lucide-react";
import type { Lang } from "../i18n";

const S = {
  zh: {
    title: "证据与文献",
    sub: "检索到的证据片段与引用记录汇集于此,仅供你自行判断——本工具只核验与呈现,不下结论。",
    refresh: "刷新",
    filterPh: "筛选证据、来源或 DOI…",
    retrieved: "检索到的证据",
    checks: "引用核验",
    checksNote: "以下为引用解析记录,非真伪判断——绿灯仅表示元数据解析成功。",
    relevance: "相关度",
    kinds: { journal: "期刊", preprint: "预印本", web: "网页" },
    parseOk: "解析成功",
    parsePending: "待核",
    parseIdle: "未核验",
    noProject: "未打开项目",
    noProjectSub: "打开一个项目后,检索到的证据与引用记录将显示在此。以下为示例内容。",
    chainOk: "证据链已核验",
    chainIdle: "证据链未核验",
    evidenceCount: "条证据",
  },
  en: {
    title: "Evidence & Literature",
    sub: "Retrieved snippets and citation records gather here for your own judgement — this tool only checks and surfaces, it never concludes.",
    refresh: "Refresh",
    filterPh: "Filter evidence, source or DOI…",
    retrieved: "Retrieved evidence",
    checks: "Citation checks",
    checksNote: "These are citation-resolution records, not truth judgements — green means metadata resolved, nothing more.",
    relevance: "relevance",
    kinds: { journal: "journal", preprint: "preprint", web: "web" },
    parseOk: "resolved",
    parsePending: "pending",
    parseIdle: "unchecked",
    noProject: "No project open",
    noProjectSub: "Open a project and retrieved evidence and citation records will appear here. Illustrative content shown below.",
    chainOk: "evidence chain checked",
    chainIdle: "evidence chain unchecked",
    evidenceCount: "evidence items",
  },
};

type Kind = "journal" | "preprint" | "web";
type Snippet = { title: { zh: string; en: string }; source: string; rel: number; kind: Kind };
type Check = { doi: string; state: "ok" | "pending" | "idle" };

const SNIPPETS: Snippet[] = [
  { title: { zh: "在 12 周随访中未观察到组间差异的显著变化。", en: "No significant change in between-group difference at 12-week follow-up." }, source: "Smith 2021 · p.4 · DOI:10.1038/s41586-021-03819", rel: 0.91, kind: "journal" },
  { title: { zh: "样本量(n=48)在附录方法部分有报告。", en: "Sample size (n=48) is reported in the supplementary methods." }, source: "Chen & Ito 2022 · §Methods · arXiv:2203.01234", rel: 0.84, kind: "preprint" },
  { title: { zh: "作者声明数据集在受限访问下可获取。", en: "Authors state the dataset is available under restricted access." }, source: "Data availability statement · project page", rel: 0.72, kind: "web" },
  { title: { zh: "效应量报告为 Cohen's d = 0.31(95% CI)。", en: "Effect size reported as Cohen's d = 0.31 (95% CI)." }, source: "Okafor 2020 · Table 2 · DOI:10.1016/j.cell.2020.05" , rel: 0.88, kind: "journal" },
  { title: { zh: "预印本第 2 版修订了统计方法一节。", en: "Preprint v2 revised the statistical-methods section." }, source: "Novak 2023 · v2 · bioRxiv:2023.04.556", rel: 0.66, kind: "preprint" },
];

const CHECKS: Check[] = [
  { doi: "10.1038/s41586-021-03819", state: "ok" },
  { doi: "10.1016/j.cell.2020.05.011", state: "pending" },
  { doi: "10.1101/2023.04.556 (bioRxiv)", state: "idle" },
];

const kindTone: Record<Kind, string> = { journal: "is-green", preprint: "is-violet", web: "is-neutral" };

export function EvidenceView({ lang }: { lang: Lang }) {
  const s = S[lang];
  const [query, setQuery] = useState("");
  const [hasProject, setHasProject] = useState(false);
  const [chainOk, setChainOk] = useState(false);
  const [eventCount, setEventCount] = useState<number | null>(null);

  useEffect(() => {
    invoke<{ event_count: number; chain_ok: boolean }>("project_status")
      .then((p) => { setHasProject(true); setEventCount(p.event_count); setChainOk(p.chain_ok); })
      .catch(() => { setHasProject(false); });
  }, []);

  const q = query.trim().toLowerCase();
  const rows = SNIPPETS.filter((r) =>
    !q || r.title[lang].toLowerCase().includes(q) || r.source.toLowerCase().includes(q)
  );

  return (
    <div className="ax-surface">
      <div className="ax-surface__head">
        <div>
          <h2 className="ax-surface__title">{s.title}</h2>
          <p className="ax-surface__sub">{s.sub}</p>
        </div>
        <div className="ax-surface__actions">
          {hasProject && (
            <span className="ax-badge">
              <span className={`ax-lamp ${chainOk ? "is-green" : "is-neutral"}`} />
              {eventCount ?? 0} {s.evidenceCount} · {chainOk ? s.chainOk : s.chainIdle}
            </span>
          )}
          <button className="ax-btn" type="button"><RefreshCw size={14} /> {s.refresh}</button>
        </div>
      </div>

      {!hasProject && (
        <div className="ax-empty" style={{ marginBottom: 16 }}>
          <div className="ax-empty__title">{s.noProject}</div>
          <div className="ax-empty__sub">{s.noProjectSub}</div>
        </div>
      )}

      <div className="ax-toolrow">
        <div className="ax-input" style={{ display: "flex", alignItems: "center", gap: 8, flex: 1 }}>
          <Search size={15} />
          <input
            className="ax-input"
            style={{ border: "none", padding: 0, background: "transparent", flex: 1 }}
            placeholder={s.filterPh}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
        </div>
      </div>

      <div className="ax-sec-label"><BookOpen size={14} /> {s.retrieved}</div>
      <div className="ax-card ax-card--pad">
        <div className="ax-card__body">
          {rows.map((r, i) => (
            <div className="ax-row" key={i}>
              <div className="ax-row__main">
                <div className="ax-row__title">{r.title[lang]}</div>
                <div className="ax-row__sub"><FileText size={12} /> {r.source}</div>
              </div>
              <div className="ax-row__meta">
                <span>{s.relevance} {r.rel.toFixed(2)}</span>
                <span className="ax-badge">
                  <span className={`ax-lamp ${kindTone[r.kind]}`} /> {s.kinds[r.kind]}
                </span>
              </div>
            </div>
          ))}
          {rows.length === 0 && (
            <div className="ax-hint">{lang === "zh" ? "没有匹配的证据。" : "No matching evidence."}</div>
          )}
        </div>
      </div>

      <div className="ax-sec-label"><Quote size={14} /> {s.checks}</div>
      <div className="ax-card ax-card--pad">
        <div className="ax-card__body">
          {CHECKS.map((c, i) => (
            <div className="ax-row" key={i}>
              <div className="ax-row__main">
                <div className="ax-row__title">
                  <span
                    className={`ax-lamp ${c.state === "ok" ? "is-green" : c.state === "pending" ? "is-amber" : "is-neutral"}`}
                    style={{ marginRight: 8 }}
                  />
                  DOI:{c.doi}
                </div>
                <div className="ax-row__sub">
                  {c.state === "ok" ? s.parseOk : c.state === "pending" ? s.parsePending : s.parseIdle}
                </div>
              </div>
              <div className="ax-row__meta">
                {c.state === "ok" ? <Link2 size={15} /> : <Quote size={15} />}
              </div>
            </div>
          ))}
          <div className="ax-hint" style={{ marginTop: 8 }}>{s.checksNote}</div>
        </div>
      </div>
    </div>
  );
}
