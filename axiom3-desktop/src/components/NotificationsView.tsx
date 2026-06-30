import type { Lang } from "../i18n";
import { Bell, CheckCircle2, AlertTriangle, ShieldCheck, Boxes } from "lucide-react";

const S = {
  zh: {
    title: "通知",
    sub: "任务、舰队与护盾的最新动态。",
    markAll: "全部已读",
    feed: "动态",
    isNew: "新",
    n1t: "校对完成", n1s: "draft.pdf · 3 处待复核", n1m: "2 分钟前",
    n2t: "抗投毒护盾", n2s: "中和 1 处注入 · ref_03.pdf", n2m: "14 分钟前",
    n3t: "跨模型核查就绪", n3s: "3 条主张已汇总倾向", n3m: "1 小时前",
    n4t: "舰队", n4s: "7 个智能体已组建 · 12 个排队", n4m: "1 小时前",
    n5t: "导入完成", n5s: "ref_02.pdf · 已分块", n5m: "今天",
    hint: "工作台只核查并呈现证据,从不裁定对错。已读后通知会归档保留 7 天。",
    emptyT: "暂无通知",
    emptyS: "任务、舰队与护盾有动态时会出现在这里。",
  },
  en: {
    title: "Notifications",
    sub: "Updates on tasks, the fleet, and the shield.",
    markAll: "Mark all read",
    feed: "Feed",
    isNew: "new",
    n1t: "Proofread complete", n1s: "draft.pdf · 3 to review", n1m: "2 min ago",
    n2t: "Anti-poison shield", n2s: "Neutralized 1 injection · ref_03.pdf", n2m: "14 min ago",
    n3t: "Cross-model check ready", n3s: "3 claims with summarized leaning", n3m: "1 hour ago",
    n4t: "Fleet", n4s: "7 agents assembled · 12 queued", n4m: "1 hour ago",
    n5t: "Import complete", n5s: "ref_02.pdf · chunked", n5m: "Today",
    hint: "The workbench only checks and surfaces evidence — it never judges right or wrong. Read items are archived for 7 days.",
    emptyT: "No notifications",
    emptyS: "Updates on tasks, the fleet, and the shield will appear here.",
  },
};

export function NotificationsView({ lang }: { lang: Lang }) {
  const s = S[lang];

  const rows = [
    { Icon: CheckCircle2, lamp: "is-green", title: s.n1t, sub: s.n1s, meta: s.n1m, isNew: true },
    { Icon: ShieldCheck, lamp: "is-violet", title: s.n2t, sub: s.n2s, meta: s.n2m, isNew: true },
    { Icon: CheckCircle2, lamp: "is-green", title: s.n3t, sub: s.n3s, meta: s.n3m, isNew: false },
    { Icon: Boxes, lamp: "is-neutral", title: s.n4t, sub: s.n4s, meta: s.n4m, isNew: false },
    { Icon: AlertTriangle, lamp: "is-neutral", title: s.n5t, sub: s.n5s, meta: s.n5m, isNew: false },
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
            <Bell size={14} /> {s.markAll}
          </button>
        </div>
      </div>

      <div className="ax-sec-label">{s.feed}</div>

      {rows.length > 0 ? (
        rows.map((r, i) => (
          <div className="ax-row" key={i}>
            <span className={`ax-lamp ${r.lamp}`} aria-hidden="true">
              <r.Icon size={14} />
            </span>
            <div className="ax-row__main">
              <div className="ax-row__title">
                {r.title}
                {r.isNew && <span className="ax-badge" style={{ marginLeft: 8 }}>{s.isNew}</span>}
              </div>
              <div className="ax-row__sub">{r.sub}</div>
            </div>
            <div className="ax-row__meta">{r.meta}</div>
          </div>
        ))
      ) : (
        <div className="ax-empty">
          <div className="ax-empty__title">{s.emptyT}</div>
          <div className="ax-empty__sub">{s.emptyS}</div>
        </div>
      )}

      <p className="ax-hint">{s.hint}</p>
    </div>
  );
}
