// 溯源 · 审计时间线 — the REAL append-only, hash-linked audit trail of the open project
// (from list_events). A record of process, never a verdict on truth.
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import { History, GitBranch, Clock, Download, FileText } from "lucide-react";
import type { Lang } from "../i18n";
import type { EventInfo, ExportSummary } from "../lib/types";

const S = {
  zh: {
    title: "溯源 · 审计时间线",
    sub: "每个操作都按顺序连同它的哈希被记录下来,任何主张或导出都能被追溯到究竟是什么产生了它。",
    export: "导出溯源",
    chain: "操作链(追加式,逆时序)",
    loading: "读取审计链…",
    empty: "尚无审计事件",
    emptySub: "打开一个项目并运行一次校对后,操作会按顺序记录在这条防篡改的链上。",
    hint: "这条时间线是防篡改的(哈希链接),正是它让每个结果都能被独立复核 —— 它是过程的记录,而非对真伪的裁决。",
  },
  en: {
    title: "Provenance · Audit timeline",
    sub: "Every action is recorded in order with its hash, so any claim or export can be traced back to exactly what produced it.",
    export: "Export",
    chain: "Operation chain (append-only, reverse-chronological)",
    loading: "Reading the audit chain…",
    empty: "No audit events yet",
    emptySub: "Open a project and run a check — actions get recorded in order on this tamper-evident chain.",
    hint: "The timeline is tamper-evident (hash-linked) and is what makes every result independently reviewable — it is a record of process, not a verdict on truth.",
  },
};

const OP: Record<string, { zh: string; en: string }> = {
  "draft.checked": { zh: "校对草稿", en: "Checked a draft" },
  note: { zh: "记录事件", en: "Note" },
};
function opLabel(op: string, lang: Lang): string {
  return OP[op] ? OP[op][lang] : op;
}
function rel(ts: number, lang: Lang): string {
  const ms = ts < 1e12 ? ts * 1000 : ts;
  const s = Math.max(0, Math.floor((Date.now() - ms) / 1000));
  if (s < 60) return lang === "zh" ? "刚刚" : "just now";
  const m = Math.floor(s / 60);
  if (m < 60) return lang === "zh" ? `${m} 分钟前` : `${m} min ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return lang === "zh" ? `${h} 小时前` : `${h} hr ago`;
  return lang === "zh" ? `${Math.floor(h / 24)} 天前` : `${Math.floor(h / 24)} d ago`;
}

export function ProvenanceView({ lang }: { lang: Lang }) {
  const s = S[lang];
  const [events, setEvents] = useState<EventInfo[] | null>(null);

  useEffect(() => {
    invoke<EventInfo[]>("list_events", { limit: 200 }).then(setEvents).catch(() => setEvents([]));
  }, []);

  async function doExport() {
    const path = await save({ defaultPath: "axiom3-provenance.json", filters: [{ name: "JSON", extensions: ["json"] }] });
    if (typeof path === "string") await invoke<ExportSummary>("export_provenance", { path }).catch(() => {});
  }

  return (
    <div className="ax-surface">
      <div className="ax-surface__head">
        <div>
          <h2 className="ax-surface__title">
            <History size={18} style={{ marginRight: 8, verticalAlign: "-3px" }} />
            {s.title}
          </h2>
          <p className="ax-surface__sub">{s.sub}</p>
        </div>
        <div className="ax-surface__actions">
          <button className="ax-btn" onClick={doExport} disabled={!events || events.length === 0}>
            <Download size={14} style={{ marginRight: 6, verticalAlign: "-2px" }} />
            {s.export}
          </button>
        </div>
      </div>

      <div className="ax-sec-label">
        <GitBranch size={13} style={{ marginRight: 6, verticalAlign: "-2px" }} />
        {s.chain}
      </div>

      {events === null ? (
        <div className="ax-hint">{s.loading}</div>
      ) : events.length === 0 ? (
        <div className="ax-empty" style={{ minHeight: 0, padding: "28px 16px" }}>
          <p className="ax-empty__title">{s.empty}</p>
          <p className="ax-empty__sub">{s.emptySub}</p>
        </div>
      ) : (
        <div className="ax-tl">
          {events.map((e) => (
            <div className="ax-tl__node" key={e.seq}>
              <div className="ax-tl__op">
                <FileText size={13} style={{ marginRight: 6, verticalAlign: "-2px" }} />
                {opLabel(e.op, lang)}
                <span className="ax-lamp is-green" style={{ marginLeft: 8 }} />
              </div>
              <div className="ax-tl__meta">
                <Clock size={11} style={{ marginRight: 5, verticalAlign: "-1px" }} />
                {e.actor} · {rel(e.ts, lang)} · #{e.seq}
              </div>
              <div className="ax-tl__hash">{e.entry_hash.slice(0, 16)}…</div>
            </div>
          ))}
        </div>
      )}

      <div className="ax-hint">{s.hint}</div>
    </div>
  );
}
