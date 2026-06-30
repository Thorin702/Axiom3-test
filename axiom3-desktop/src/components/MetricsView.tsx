import type { Lang } from "../i18n";
import { Gauge, BarChart3, Activity, Boxes, FileText, ShieldCheck } from "lucide-react";

const S = {
  zh: {
    title: "概览",
    sub: "本工作区活动的一张快照。这些数字描述的是已处理与已校验的量,而不是任何内容是否正确。",
    scopeLabel: "本工作区",
    sources: "导入来源",
    checks: "校对任务",
    review: "待复核",
    cross: "跨模型核查",
    agents: "活跃智能体",
    integrity: "完整性",
    intact: "连续",
    recentLabel: "近期活动",
    r1: "校对完成 · draft.pdf",
    r2: "护盾中和 1 处注入",
    r3: "导入 ref_02.pdf",
    r4: "跨模型核查就绪",
    t1: "2 分钟前",
    t2: "14 分钟前",
    t3: "1 小时前",
    t4: "今天",
    hint: "这些指标是描述性的——它们说明工作区处理与核查了多少,并不是一个质量评分。",
  },
  en: {
    title: "Overview",
    sub: "A snapshot of this workspace's activity. These numbers describe how much has been processed and checked, not whether anything is correct.",
    scopeLabel: "This workspace",
    sources: "Sources",
    checks: "Checks run",
    review: "Needs review",
    cross: "Cross-model checks",
    agents: "Active agents",
    integrity: "Integrity",
    intact: "Intact",
    recentLabel: "Recent activity",
    r1: "Check complete · draft.pdf",
    r2: "Shield neutralized 1 injection",
    r3: "Imported ref_02.pdf",
    r4: "Cross-model check ready",
    t1: "2 min ago",
    t2: "14 min ago",
    t3: "1 hr ago",
    t4: "Today",
    hint: "These metrics are descriptive — they tell you how much the workspace processed and checked, not a quality score.",
  },
};

export function MetricsView({ lang }: { lang: Lang }) {
  const s = S[lang];

  const stats = [
    { icon: FileText, label: s.sources, num: "18" },
    { icon: BarChart3, label: s.checks, num: "7" },
    { icon: Activity, label: s.review, num: "11" },
    { icon: Gauge, label: s.cross, num: "23" },
    { icon: Boxes, label: s.agents, num: "7" },
  ];

  const recent = [
    { lamp: "is-green", title: s.r1, meta: s.t1 },
    { lamp: "is-violet", title: s.r2, meta: s.t2 },
    { lamp: "is-neutral", title: s.r3, meta: s.t3 },
    { lamp: "is-green", title: s.r4, meta: s.t4 },
  ];

  return (
    <div className="ax-surface">
      <div className="ax-surface__head">
        <div>
          <h2 className="ax-surface__title">{s.title}</h2>
          <p className="ax-surface__sub">{s.sub}</p>
        </div>
      </div>

      <div className="ax-sec-label">{s.scopeLabel}</div>
      <div className="ax-grid">
        {stats.map((st) => {
          const Icon = st.icon;
          return (
            <div className="ax-card" key={st.label}>
              <div className="ax-card__head">
                <Icon size={15} />
                <span className="ax-card__title">{st.label}</span>
              </div>
              <div className="ax-stat">
                <div className="ax-stat__num">{st.num}</div>
                <div className="ax-stat__label">{st.label}</div>
              </div>
            </div>
          );
        })}
        <div className="ax-card">
          <div className="ax-card__head">
            <ShieldCheck size={15} />
            <span className="ax-card__title">{s.integrity}</span>
          </div>
          <div className="ax-stat">
            <div className="ax-stat__num">
              <span className="ax-lamp is-green" style={{ marginRight: 8 }} />
              {s.intact}
            </div>
            <div className="ax-stat__label">{s.integrity}</div>
          </div>
        </div>
      </div>

      <div className="ax-sec-label" style={{ marginTop: 18 }}>{s.recentLabel}</div>
      {recent.map((r, i) => (
        <div className="ax-row" key={i}>
          <span className={`ax-lamp ${r.lamp}`} />
          <div className="ax-row__main">
            <div className="ax-row__title">{r.title}</div>
          </div>
          <div className="ax-row__meta">{r.meta}</div>
        </div>
      ))}

      <div className="ax-hint" style={{ marginTop: 16 }}>{s.hint}</div>
    </div>
  );
}
