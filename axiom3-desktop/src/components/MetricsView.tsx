// 概览 — REAL workspace metrics from the backend (project_status + list_events +
// provider_status). Descriptive counts of what was processed/checked, never a quality score.
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { BarChart3, Activity, FileText, ShieldCheck, KeyRound } from "lucide-react";
import type { Lang } from "../i18n";
import type { EventInfo, ProjectStatus } from "../lib/types";

const S = {
  zh: {
    title: "概览",
    sub: "本工作区活动的一张快照。这些数字描述的是已处理与已校验的量,而不是任何内容是否正确。",
    scopeLabel: "本工作区",
    events: "事件",
    checks: "校对任务",
    providers: "已连接模型",
    integrity: "完整性",
    intact: "连续", broken: "异常", none: "—",
    recentLabel: "近期活动",
    empty: "还没有活动 — 打开项目并运行一次校对。",
    hint: "这些指标是描述性的——它们说明工作区处理与核查了多少,并不是一个质量评分。",
  },
  en: {
    title: "Overview",
    sub: "A snapshot of this workspace's activity. These numbers describe how much has been processed and checked, not whether anything is correct.",
    scopeLabel: "This workspace",
    events: "Events",
    checks: "Checks run",
    providers: "Models connected",
    integrity: "Integrity",
    intact: "Intact", broken: "Broken", none: "—",
    recentLabel: "Recent activity",
    empty: "No activity yet — open a project and run a check.",
    hint: "These metrics are descriptive — they tell you how much the workspace processed and checked, not a quality score.",
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
  const sec = Math.max(0, Math.floor((Date.now() - ms) / 1000));
  if (sec < 60) return lang === "zh" ? "刚刚" : "just now";
  const m = Math.floor(sec / 60);
  if (m < 60) return lang === "zh" ? `${m} 分钟前` : `${m} min ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return lang === "zh" ? `${h} 小时前` : `${h} hr ago`;
  return lang === "zh" ? `${Math.floor(h / 24)} 天前` : `${Math.floor(h / 24)} d ago`;
}

export function MetricsView({ lang }: { lang: Lang }) {
  const s = S[lang];
  const [status, setStatus] = useState<ProjectStatus | null>(null);
  const [events, setEvents] = useState<EventInfo[]>([]);
  const [providers, setProviders] = useState(0);

  useEffect(() => {
    invoke<ProjectStatus>("project_status").then(setStatus).catch(() => setStatus(null));
    invoke<EventInfo[]>("list_events", { limit: 8 }).then(setEvents).catch(() => setEvents([]));
    invoke<{ family: string; connected: boolean }[]>("provider_status")
      .then((ps) => setProviders(ps.filter((p) => p.connected).length))
      .catch(() => {});
  }, []);

  const checksRun = events.filter((e) => e.op === "draft.checked").length; // (from the limited window)
  const eventCount = status ? status.event_count : 0;

  const stats = [
    { icon: FileText, label: s.events, num: String(eventCount) },
    { icon: BarChart3, label: s.checks, num: String(checksRun) },
    { icon: KeyRound, label: s.providers, num: `${providers}` },
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
              <span className={`ax-lamp ${!status ? "is-neutral" : status.chain_ok ? "is-green" : "is-red"}`} style={{ marginRight: 8 }} />
              {!status ? s.none : status.chain_ok ? s.intact : s.broken}
            </div>
            <div className="ax-stat__label">{s.integrity}</div>
          </div>
        </div>
      </div>

      <div className="ax-sec-label" style={{ marginTop: 18 }}>{s.recentLabel}</div>
      {events.length === 0 ? (
        <div className="ax-hint">{s.empty}</div>
      ) : (
        events.slice(0, 5).map((e) => (
          <div className="ax-row" key={e.seq}>
            <span className="ax-lamp is-green" />
            <div className="ax-row__main">
              <div className="ax-row__title">
                {opLabel(e.op, lang)} <Activity size={12} style={{ opacity: 0.4, verticalAlign: "-1px" }} />
              </div>
            </div>
            <div className="ax-row__meta">{rel(e.ts, lang)}</div>
          </div>
        ))
      )}

      <div className="ax-hint" style={{ marginTop: 16 }}>{s.hint}</div>
    </div>
  );
}
