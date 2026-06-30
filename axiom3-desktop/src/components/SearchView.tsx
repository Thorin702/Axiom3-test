// 检索 / Global search surface — retrieved evidence across sources, ranked by similarity (never an answer).
import { Search, Filter, FileText, Table2, Sigma, Image as ImageIcon } from "lucide-react";
import type { Lang } from "../i18n";

const S = {
  zh: {
    title: "检索 · 全局检索",
    sub: "检索只返回带来源与位置的证据片段,按相似度排序 —— 这不是答案,请自行核对。",
    placeholder: "搜索来源、片段、过往校对…",
    filters: "筛选",
    chips: ["文本", "图片", "表格", "公式", "来源:草稿", "来源:参考"],
    results: "检索结果 / Results",
    emptyTitle: "输入关键词开始检索",
    emptySub: "结果仅为按相似度召回的证据,附带来源与位置,供你判断,不代表结论。",
    rows: [
      { title: "样本量在 §2.1 处由 120 调整为 96", sub: "draft.pdf · 第3页", meta: "相似度 0.82", kind: "text" as const },
      { title: "图 4:对照组与处理组的误差棒重叠区间", sub: "draft.pdf · 图4", meta: "相似度 0.77", kind: "image" as const },
      { title: "表 2 中 p 值列与正文表述存在出入", sub: "ref_02.pdf · §2.1", meta: "片段", kind: "table" as const },
      { title: "回归方程的自由度与脚注不一致", sub: "ref_02.pdf · 第7页", meta: "相似度 0.71", kind: "formula" as const },
      { title: "引用编号 [12] 指向的来源未在参考列表中", sub: "draft.pdf · §4.3", meta: "相似度 0.65", kind: "text" as const },
    ],
  },
  en: {
    title: "Search · Across sources",
    sub: "Search returns evidence snippets with their source and location, ranked by similarity — not an answer; verify yourself.",
    placeholder: "Search sources, snippets, prior checks…",
    filters: "Filter",
    chips: ["Text", "Image", "Table", "Formula", "Source: draft", "Source: refs"],
    results: "检索结果 / Results",
    emptyTitle: "Type a query to start searching",
    emptySub: "Results are evidence recalled by similarity with source and location — for your judgement, not a conclusion.",
    rows: [
      { title: "Sample size revised from 120 to 96 at §2.1", sub: "draft.pdf · p.3", meta: "similarity 0.82", kind: "text" as const },
      { title: "Fig. 4: error bars overlap between control and treatment", sub: "draft.pdf · Fig. 4", meta: "similarity 0.77", kind: "image" as const },
      { title: "p-value column in Table 2 differs from the prose", sub: "ref_02.pdf · §2.1", meta: "snippet", kind: "table" as const },
      { title: "Degrees of freedom in regression disagree with footnote", sub: "ref_02.pdf · p.7", meta: "similarity 0.71", kind: "formula" as const },
      { title: "Citation [12] points to a source absent from references", sub: "draft.pdf · §4.3", meta: "similarity 0.65", kind: "text" as const },
    ],
  },
};

const ICONS = {
  text: FileText,
  image: ImageIcon,
  table: Table2,
  formula: Sigma,
};

export function SearchView({ lang }: { lang: Lang }) {
  const s = S[lang];
  return (
    <div className="ax-surface">
      <div className="ax-surface__head">
        <div>
          <h2 className="ax-surface__title">{s.title}</h2>
          <p className="ax-surface__sub">{s.sub}</p>
        </div>
        <div className="ax-surface__actions">
          <button className="ax-btn" disabled><Filter size={15} aria-hidden /> {s.filters}</button>
        </div>
      </div>

      <div className="ax-toolrow" style={{ marginBottom: 8 }}>
        <div className="ax-input" style={{ display: "flex", alignItems: "center", gap: 8, flex: 1 }}>
          <Search size={15} aria-hidden />
          <input
            className="ax-input"
            style={{ border: "none", padding: 0, flex: 1, background: "transparent" }}
            placeholder={s.placeholder}
            disabled
          />
        </div>
      </div>
      <div className="ax-toolrow" style={{ flexWrap: "wrap", gap: 6, marginBottom: 4 }}>
        {s.chips.map((c) => (
          <span key={c} className="ax-badge">{c}</span>
        ))}
      </div>

      <div className="ax-sec-label">{s.results}</div>
      <div className="ax-grid">
        {s.rows.map((r, i) => {
          const Icon = ICONS[r.kind];
          return (
            <div className="ax-row" key={i}>
              <span className="ax-lamp is-neutral" />
              <Icon size={15} aria-hidden />
              <div className="ax-row__main">
                <div className="ax-row__title">{r.title}</div>
                <div className="ax-row__sub">{r.sub}</div>
              </div>
              <span className="ax-row__meta">{r.meta}</span>
            </div>
          );
        })}
      </div>

      <div className="ax-empty" style={{ padding: "12px 14px", marginTop: 10 }}>
        <div className="ax-empty__title">{s.emptyTitle}</div>
        <div className="ax-empty__sub">{s.emptySub}</div>
      </div>
    </div>
  );
}
