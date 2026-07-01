// 阅读 · 摄入 — the source library. Import documents (PDF/DOCX/TXT/MD → text via the
// backend), keep them here, and open one into the /校对 composer. Ingest only parses +
// keeps the source; it draws no conclusions.
import { BookOpen, Upload, FileText, Trash2, ArrowUpRight } from "lucide-react";
import type { Lang } from "../i18n";
import type { Source } from "../lib/types";

const S = {
  zh: {
    title: "阅读 · 摄入",
    sub: "把论文、标书、草稿、数据表导入工作区。摄入只做解析与留存来源,不下结论。导入后可一键送入校对。",
    drop: "点击导入,或把文件拖到窗口任意处",
    dropHint: "PDF · DOCX · TXT · Markdown",
    import: "导入文件",
    library: "来源库",
    empty: "还没有导入任何来源",
    emptySub: "导入的文档会保留在这里,随时打开送入校对或检索。",
    chars: "字",
    open: "送入校对",
    remove: "移除",
  },
  en: {
    title: "Reading · Ingest",
    sub: "Bring papers, proposals, drafts and data tables into the workspace. Ingest only parses and keeps the source — it draws no conclusions. Open one straight into a check.",
    drop: "Click to import, or drop a file anywhere on the window",
    dropHint: "PDF · DOCX · TXT · Markdown",
    import: "Import file",
    library: "Source library",
    empty: "No sources imported yet",
    emptySub: "Imported documents stay here — open one into a check or search anytime.",
    chars: "chars",
    open: "Send to check",
    remove: "Remove",
  },
};

function rel(ts: number, lang: Lang): string {
  const s = Math.max(0, Math.floor((Date.now() - ts) / 1000));
  if (s < 60) return lang === "zh" ? "刚刚" : "just now";
  const m = Math.floor(s / 60);
  if (m < 60) return lang === "zh" ? `${m} 分钟前` : `${m} min ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return lang === "zh" ? `${h} 小时前` : `${h} hr ago`;
  return lang === "zh" ? `${Math.floor(h / 24)} 天前` : `${Math.floor(h / 24)} d ago`;
}

export function ReadingView({
  lang,
  sources,
  onImport,
  onOpen,
  onRemove,
}: {
  lang: Lang;
  sources: Source[];
  onImport: () => void;
  onOpen: (src: Source) => void;
  onRemove: (id: string) => void;
}) {
  const s = S[lang];
  return (
    <div className="ax-surface">
      <div className="ax-surface__head">
        <div>
          <h2 className="ax-surface__title">{s.title}</h2>
          <p className="ax-surface__sub">{s.sub}</p>
        </div>
        <div className="ax-surface__actions">
          <button className="ax-btn ax-btn--primary" onClick={onImport}>
            <Upload size={15} aria-hidden /> {s.import}
          </button>
        </div>
      </div>

      <div className="ax-drop" onClick={onImport} role="button">
        <BookOpen size={22} aria-hidden />
        <span>{s.drop}</span>
        <span className="ax-drop__hint">{s.dropHint}</span>
      </div>

      <div>
        <p className="ax-sec-label">{s.library} · {sources.length}</p>
        {sources.length === 0 ? (
          <div className="ax-empty" style={{ minHeight: 0, padding: "28px 16px" }}>
            <p className="ax-empty__title">{s.empty}</p>
            <p className="ax-empty__sub">{s.emptySub}</p>
          </div>
        ) : (
          <div className="ax-results__list">
            {sources.map((src) => (
              <div className="ax-row" key={src.id}>
                <FileText size={16} aria-hidden />
                <div className="ax-row__main">
                  <div className="ax-row__title">{src.name}</div>
                  <div className="ax-row__sub">
                    {src.chars.toLocaleString()} {s.chars} · {rel(src.ts, lang)}
                  </div>
                </div>
                <button className="ax-iconbtn" onClick={() => onOpen(src)} title={s.open} aria-label={s.open}>
                  <ArrowUpRight size={16} aria-hidden />
                </button>
                <button className="ax-iconbtn" onClick={() => onRemove(src.id)} title={s.remove} aria-label={s.remove}>
                  <Trash2 size={15} aria-hidden />
                </button>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
