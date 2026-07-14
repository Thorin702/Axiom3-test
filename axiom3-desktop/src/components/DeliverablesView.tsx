// 产出物库 — the durable, disk-backed library of everything produced across sessions (replaces the old
// "项目" surface). It is the BACKUP mechanism: the full report + figures live on disk (not the quota-
// bound localStorage), listed here with detailed provenance. Rows can be opened, renamed, or deleted.
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Database, Trash2, PenLine, Check, X, FileText, Users, ScanSearch, Image, Link2, Layers } from "lucide-react";
import type { Lang } from "../i18n";

type Meta = {
  id: string;
  title: string;
  kind: string;
  ts: number;
  reviewed: boolean;
  rounds: number;
  experts: number;
  figures: number;
  sources: number;
  chars: number;
  bytes: number;
};

function fmtDate(ms: number): string {
  if (!ms) return "—";
  const d = new Date(ms);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}
function fmtSize(b: number): string {
  return b >= 1_048_576 ? `${(b / 1_048_576).toFixed(1)} MB` : b >= 1024 ? `${(b / 1024).toFixed(0)} KB` : `${b} B`;
}
function kindLabel(k: string, zh: boolean): string {
  if (k === "research") return zh ? "研究" : "Research";
  if (k === "code") return zh ? "分析" : "Analysis";
  if (k === "workflow") return "Workflow";
  return k || "—";
}

export function DeliverablesView({ lang, onOpen }: { lang: Lang; onOpen: (id: string) => void }) {
  const zh = lang === "zh";
  const [items, setItems] = useState<Meta[] | null>(null);
  const [editing, setEditing] = useState<string | null>(null);
  const [draft, setDraft] = useState("");

  async function refresh() {
    try {
      setItems(await invoke<Meta[]>("deliverable_list"));
    } catch {
      setItems([]);
    }
  }
  useEffect(() => {
    refresh();
  }, []);

  async function del(id: string) {
    await invoke("deliverable_delete", { id }).catch(() => {});
    refresh();
  }
  async function rename(id: string) {
    const title = draft.trim();
    if (title) await invoke("deliverable_rename", { id, title }).catch(() => {});
    setEditing(null);
    refresh();
  }

  const total = items?.length ?? 0;
  const totalBytes = (items ?? []).reduce((s, m) => s + m.bytes, 0);

  return (
    <div className="ax-panel ax-deliv">
      <div className="ax-panel__head">
        <span className="ax-panel__title">
          <Database size={16} aria-hidden /> {zh ? "产出物库" : "Deliverables"}
        </span>
        <span className="ax-deliv__meta">
          {zh ? `${total} 项 · 本地库 ${fmtSize(totalBytes)}` : `${total} items · ${fmtSize(totalBytes)} local`}
        </span>
      </div>
      <p className="ax-hint" style={{ margin: "0 0 12px" }}>
        {zh
          ? "每个课题完成后，完整报告与图形自动落库到本地磁盘（不受浏览器缓存上限影响），作为产出的备份。将来可同步到云端。"
          : "Every finished session is auto-saved here on local disk (beyond the browser quota) as a durable backup. Cloud sync later."}
      </p>

      {items === null ? (
        <p className="ax-hint">{zh ? "读取中…" : "Loading…"}</p>
      ) : items.length === 0 ? (
        <div className="ax-deliv__empty">
          <Database size={26} aria-hidden />
          <p>{zh ? "还没有产出物。完成一个课题后会自动出现在这里。" : "No deliverables yet — finished sessions land here automatically."}</p>
        </div>
      ) : (
        <div className="ax-deliv__list">
          {items.map((m) => (
            <div className="ax-deliv__row" key={m.id}>
              <div className="ax-deliv__main" onClick={() => editing !== m.id && onOpen(m.id)}>
                {editing === m.id ? (
                  <span className="ax-deliv__rename" onClick={(e) => e.stopPropagation()}>
                    <input
                      className="ax-deliv__input"
                      value={draft}
                      autoFocus
                      onChange={(e) => setDraft(e.currentTarget.value)}
                      onKeyDown={(e) => { if (e.key === "Enter") rename(m.id); if (e.key === "Escape") setEditing(null); }}
                    />
                    <button className="ax-iconbtn" onClick={() => rename(m.id)} title={zh ? "保存" : "Save"}><Check size={14} /></button>
                    <button className="ax-iconbtn" onClick={() => setEditing(null)} title={zh ? "取消" : "Cancel"}><X size={14} /></button>
                  </span>
                ) : (
                  <span className="ax-deliv__title">{m.title}</span>
                )}
                <span className="ax-deliv__tags">
                  <span className={`ax-deliv__kind ${m.kind === "research" ? "is-research" : "is-analysis"}`}>{kindLabel(m.kind, zh)}</span>
                  <span className="ax-deliv__stat" title={zh ? "执行轮次 / 子课题" : "rounds"}><Layers size={12} />{m.rounds}</span>
                  <span className="ax-deliv__stat" title={zh ? "参与智能体(主+委派)" : "experts"}><Users size={12} />{m.experts}</span>
                  <span className={`ax-deliv__stat ${m.reviewed ? "is-on" : ""}`} title={zh ? "是否经过审查" : "reviewed"}><ScanSearch size={12} />{m.reviewed ? (zh ? "已审" : "yes") : (zh ? "未审" : "no")}</span>
                  <span className="ax-deliv__stat" title={zh ? "图形数" : "figures"}><Image size={12} />{m.figures}</span>
                  <span className="ax-deliv__stat" title={zh ? "引用来源数" : "sources"}><Link2 size={12} />{m.sources}</span>
                  <span className="ax-deliv__stat" title={zh ? "报告字数" : "chars"}><FileText size={12} />{m.chars.toLocaleString()}</span>
                  <span className="ax-deliv__date">{fmtDate(m.ts)}</span>
                </span>
              </div>
              <div className="ax-deliv__actions">
                <button className="ax-iconbtn" title={zh ? "重命名课题" : "Rename"} onClick={() => { setEditing(m.id); setDraft(m.title); }}>
                  <PenLine size={14} />
                </button>
                <button className="ax-iconbtn ax-deliv__del" title={zh ? "从库中删除" : "Delete"} onClick={() => del(m.id)}>
                  <Trash2 size={14} />
                </button>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
