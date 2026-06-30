// 阅读 / 摄入 surface — import documents into the workspace and browse the source
// library. Presentational shell for now (the ingest pipeline wires in at step 2); it is
// the reference other surface components copy: self-contained bilingual `S`, `{ lang }`
// props, theme classes only, its own empty state. No backend calls yet.
import { BookOpen, Upload, FileText, Table2, Sigma, Image as ImageIcon } from "lucide-react";
import type { Lang } from "../i18n";

const S = {
  zh: {
    title: "阅读 · 摄入",
    sub: "把论文、标书、草稿、数据表导入工作区。摄入只做解析与切分,不下结论 —— 文本、图片、表格、公式都会被分块并留存来源。",
    drop: "拖入文件,或点击选择",
    dropHint: "PDF · DOCX · TXT · Markdown · 图片 / 表格 / 公式将分模态解析",
    import: "导入文件",
    library: "来源库",
    empty: "还没有导入任何来源",
    emptySub: "导入后,每个片段都会带上来源与位置,供校对与检索引用。",
    modalities: "模态覆盖",
    mText: "文本", mImage: "图片", mTable: "表格", mFormula: "公式",
    parsed: "已解析", pending: "排队中",
  },
  en: {
    title: "Reading · Ingest",
    sub: "Bring papers, proposals, drafts and data tables into the workspace. Ingest only parses and segments — it draws no conclusions. Text, images, tables and formulas are chunked with their source kept.",
    drop: "Drop files here, or click to choose",
    dropHint: "PDF · DOCX · TXT · Markdown · images / tables / formulas parsed per modality",
    import: "Import file",
    library: "Source library",
    empty: "No sources imported yet",
    emptySub: "Once imported, every fragment carries its source and location for checking and retrieval.",
    modalities: "Modality coverage",
    mText: "Text", mImage: "Images", mTable: "Tables", mFormula: "Formulas",
    parsed: "parsed", pending: "queued",
  },
};

export function ReadingView({ lang }: { lang: Lang }) {
  const s = S[lang];
  const mods = [
    { icon: FileText, label: s.mText },
    { icon: ImageIcon, label: s.mImage },
    { icon: Table2, label: s.mTable },
    { icon: Sigma, label: s.mFormula },
  ];
  return (
    <div className="ax-surface">
      <div className="ax-surface__head">
        <div>
          <h2 className="ax-surface__title">{s.title}</h2>
          <p className="ax-surface__sub">{s.sub}</p>
        </div>
        <div className="ax-surface__actions">
          <button className="ax-btn" disabled>
            <Upload size={15} aria-hidden /> {s.import}
          </button>
        </div>
      </div>

      <div className="ax-drop">
        <BookOpen size={22} aria-hidden />
        <span>{s.drop}</span>
        <span className="ax-drop__hint">{s.dropHint}</span>
      </div>

      <div>
        <p className="ax-sec-label">{s.modalities}</p>
        <div className="ax-grid">
          {mods.map((m) => (
            <div className="ax-card" key={m.label}>
              <div className="ax-card__head">
                <m.icon size={15} aria-hidden />
                <p className="ax-card__title">{m.label}</p>
              </div>
              <div className="ax-card__body">
                <span className="ax-badge"><span className="ax-lamp is-neutral" />{s.pending}</span>
              </div>
            </div>
          ))}
        </div>
      </div>

      <div>
        <p className="ax-sec-label">{s.library}</p>
        <div className="ax-empty" style={{ minHeight: 0, padding: "28px 16px" }}>
          <p className="ax-empty__title">{s.empty}</p>
          <p className="ax-empty__sub">{s.emptySub}</p>
        </div>
      </div>
    </div>
  );
}
