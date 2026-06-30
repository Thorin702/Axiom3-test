import type { Lang } from "../i18n";
import { FileCheck, Fingerprint, Lock, ShieldCheck, History } from "lucide-react";

const S = {
  zh: {
    title: "完整性报告",
    sub: "工作台为每个操作保留一条只追加、哈希相连的记录,任何对轨迹的篡改都可被发现 —— 这证明的是记录本身未被改动,而非内容为真。",
    reverify: "重新校验",
    chainIntact: "哈希链连续",
    headLabel: "链头哈希",
    eventsLabel: "事件",
    sigLabel: "签名",
    lastLabel: "上次校验",
    lastValue: "2 分钟前",
    checks: "校验项",
    c1: "哈希链连续",
    c1s: "每个事件都链接上一个事件的哈希",
    c2: "事件签名有效",
    c2s: "每条记录由工作台密钥签名",
    c3: "导出可独立复核",
    c3s: "导出的日志可在工作台之外重新验证",
    c4: "时间戳单调递增",
    c4s: "事件时间不回退,顺序不可重排",
    hint: "完整性 = 这条轨迹没有被改动过;它从不断言其中记录的主张是否正确。内容的真假由校验流程单独评估。",
  },
  en: {
    title: "Integrity report",
    sub: "The workspace keeps an append-only, hash-linked record of every action, so any tampering with the trail is detectable — this proves the record itself was not altered, not that the content is true.",
    reverify: "Re-verify",
    chainIntact: "Chain intact",
    headLabel: "Chain head",
    eventsLabel: "Events",
    sigLabel: "Signatures",
    lastLabel: "Last verified",
    lastValue: "2 min ago",
    checks: "Checks",
    c1: "Hash chain is continuous",
    c1s: "Each event links to the previous event's hash",
    c2: "Event signatures valid",
    c2s: "Every record is signed by the workspace key",
    c3: "Export is independently re-checkable",
    c3s: "The exported log can be re-verified outside the workspace",
    c4: "Timestamps strictly increasing",
    c4s: "Event times never go backwards; order can't be reordered",
    hint: "Integrity = this trail was not altered; it never asserts whether the claims recorded in it are correct. The truth of the content is judged separately by the verification pipeline.",
  },
};

const HEAD_HASH = "sha256:7f3a91c4…b2108d";

export function IntegrityView({ lang }: { lang: Lang }) {
  const s = S[lang];
  const checks = [
    { icon: Lock, title: s.c1, sub: s.c1s },
    { icon: Fingerprint, title: s.c2, sub: s.c2s },
    { icon: FileCheck, title: s.c3, sub: s.c3s },
    { icon: History, title: s.c4, sub: s.c4s },
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
            <ShieldCheck size={15} /> {s.reverify}
          </button>
        </div>
      </div>

      <div className="ax-card ax-card--pad">
        <div className="ax-row">
          <span className="ax-lamp is-green" />
          <div className="ax-row__main">
            <div className="ax-row__title">{s.chainIntact}</div>
            <div className="ax-row__sub">
              {s.headLabel}:{" "}
              <span style={{ fontFamily: "ui-monospace, Menlo, monospace" }}>{HEAD_HASH}</span>
            </div>
          </div>
          <span className="ax-row__meta">342 {s.eventsLabel.toLowerCase()}</span>
        </div>
      </div>

      <div className="ax-grid" style={{ marginTop: 12 }}>
        <div className="ax-card ax-stat">
          <div className="ax-stat__num">342</div>
          <div className="ax-stat__label">{s.eventsLabel}</div>
        </div>
        <div className="ax-card ax-stat">
          <div className="ax-stat__num">342</div>
          <div className="ax-stat__label">{s.sigLabel}</div>
        </div>
        <div className="ax-card ax-stat">
          <div className="ax-stat__num">{s.lastValue}</div>
          <div className="ax-stat__label">{s.lastLabel}</div>
        </div>
      </div>

      <div className="ax-sec-label" style={{ marginTop: 16 }}>{s.checks}</div>
      {checks.map((c, i) => {
        const Icon = c.icon;
        return (
          <div className="ax-row ax-card" key={i}>
            <span className="ax-lamp is-green" />
            <div className="ax-row__main">
              <div className="ax-row__title">
                <Icon size={14} style={{ marginRight: 6, verticalAlign: "-2px" }} />
                {c.title}
              </div>
              <div className="ax-row__sub">{c.sub}</div>
            </div>
            <span className="ax-badge">OK</span>
          </div>
        );
      })}

      <div className="ax-hint" style={{ marginTop: 16 }}>{s.hint}</div>
    </div>
  );
}
