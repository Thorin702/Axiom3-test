import type { Lang } from "../i18n";
import { History, GitBranch, Clock, Download, FileText } from "lucide-react";

const S = {
  zh: {
    title: "溯源 · 审计时间线",
    sub: "每个操作都按顺序连同它的哈希被记录下来,任何主张或导出都能被追溯到究竟是什么产生了它。",
    export: "导出溯源",
    chain: "操作链(追加式,逆时序)",
    researcher: "researcher",
    shield: "护盾",
    nodes: [
      { op: "导出溯源 JSON", actor: "researcher", time: "刚刚", hash: "#a91f…b2" },
      { op: "跨模型核查 3 条主张", actor: "researcher", time: "2 分钟前", hash: "#7f3a…d1" },
      { op: "校对草稿 draft.pdf", actor: "researcher", time: "14 分钟前", hash: "#3c08…9e" },
      { op: "抗投毒护盾中和 1 处注入", actor: "护盾", time: "1 小时前", hash: "#b5d2…44" },
      { op: "导入文档 ref_02.pdf", actor: "researcher", time: "3 小时前", hash: "#e60a…1c" },
      { op: '打开项目 "甲烷氧化"', actor: "researcher", time: "今天", hash: "#1d77…0f" },
    ],
    hint: "这条时间线是防篡改的(哈希链接),正是它让每个结果都能被独立复核 —— 它是过程的记录,而非对真伪的裁决。",
  },
  en: {
    title: "Provenance · Audit timeline",
    sub: "Every action is recorded in order with its hash, so any claim or export can be traced back to exactly what produced it.",
    export: "Export",
    chain: "Operation chain (append-only, reverse-chronological)",
    researcher: "researcher",
    shield: "shield",
    nodes: [
      { op: "Export provenance JSON", actor: "researcher", time: "just now", hash: "#a91f…b2" },
      { op: "Cross-model check of 3 claims", actor: "researcher", time: "2 min ago", hash: "#7f3a…d1" },
      { op: "Proofread draft draft.pdf", actor: "researcher", time: "14 min ago", hash: "#3c08…9e" },
      { op: "Anti-poison shield neutralized 1 injection", actor: "shield", time: "1 hr ago", hash: "#b5d2…44" },
      { op: "Imported document ref_02.pdf", actor: "researcher", time: "3 hr ago", hash: "#e60a…1c" },
      { op: 'Opened project "Methane oxidation"', actor: "researcher", time: "today", hash: "#1d77…0f" },
    ],
    hint: "The timeline is tamper-evident (hash-linked) and is what makes every result independently reviewable — it is a record of process, not a verdict on truth.",
  },
};

export function ProvenanceView({ lang }: { lang: Lang }) {
  const s = S[lang];
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
          <button className="ax-btn" disabled>
            <Download size={14} style={{ marginRight: 6, verticalAlign: "-2px" }} />
            {s.export}
          </button>
        </div>
      </div>

      <div className="ax-sec-label">
        <GitBranch size={13} style={{ marginRight: 6, verticalAlign: "-2px" }} />
        {s.chain}
      </div>

      <div className="ax-tl">
        {s.nodes.map((n) => (
          <div className="ax-tl__node" key={n.hash}>
            <div className="ax-tl__op">
              <FileText size={13} style={{ marginRight: 6, verticalAlign: "-2px" }} />
              {n.op}
              <span className="ax-lamp is-neutral" style={{ marginLeft: 8 }} />
            </div>
            <div className="ax-tl__meta">
              <Clock size={11} style={{ marginRight: 5, verticalAlign: "-1px" }} />
              {n.actor} · {n.time}
            </div>
            <div className="ax-tl__hash">{n.hash}</div>
          </div>
        ))}
      </div>

      <div className="ax-hint">{s.hint}</div>
    </div>
  );
}
