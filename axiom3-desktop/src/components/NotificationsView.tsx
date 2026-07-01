// 通知 — derived from real task state (completions, flags, neutralized injections),
// newest first. A finished check says "N to review", never "N errors".
import { Bell, CheckCircle2, AlertTriangle, ShieldCheck, Boxes, Sparkles } from "lucide-react";
import type { Lang } from "../i18n";
import type { Task } from "../lib/types";

const S = {
  zh: {
    title: "通知",
    sub: "任务、护盾与工作流的动态,最新在上。",
    empty: "还没有通知",
    emptySub: "运行任务后,完成与需要复核的提醒会显示在这里。",
    checkDone: "校对完成", wfDone: "工作流完成", codeDone: "编程完成",
    running: "运行中", failed: "任务未完成",
    claims: "条主张", review: "待复核", inj: "已中和注入",
  },
  en: {
    title: "Notifications",
    sub: "Updates on tasks, the shield, and workflows — newest first.",
    empty: "No notifications yet",
    emptySub: "Run a task and completions / things to review show up here.",
    checkDone: "Check complete", wfDone: "Workflow complete", codeDone: "Coding complete",
    running: "Running", failed: "Task didn't complete",
    claims: "claims", review: "to review", inj: "neutralized",
  },
};

function rel(ts: number, lang: Lang): string {
  const sec = Math.max(0, Math.floor((Date.now() - ts) / 1000));
  if (sec < 60) return lang === "zh" ? "刚刚" : "just now";
  const m = Math.floor(sec / 60);
  if (m < 60) return lang === "zh" ? `${m} 分钟前` : `${m} min ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return lang === "zh" ? `${h} 小时前` : `${h} hr ago`;
  return lang === "zh" ? `${Math.floor(h / 24)} 天前` : `${Math.floor(h / 24)} d ago`;
}

export function NotificationsView({ lang, tasks, onOpen }: { lang: Lang; tasks: Task[]; onOpen: (id: string) => void }) {
  const s = S[lang];

  const notes = [...tasks]
    .sort((a, b) => b.ts - a.ts)
    .map((t) => {
      const inj = (t.result?.entries ?? []).filter((e) => e.status === "injection_neutralized").length;
      if (t.status === "running") return { id: t.id, Icon: Sparkles, lamp: "is-amber", title: s.running, sub: t.title, ts: t.ts };
      if (t.status === "failed" || t.status === "cancelled") return { id: t.id, Icon: AlertTriangle, lamp: "is-neutral", title: s.failed, sub: t.title, ts: t.ts };
      if (t.type === "check_draft" && t.result) {
        const flagged = t.result.flagged;
        const parts = [`${t.result.total} ${s.claims}`, `${flagged} ${s.review}`];
        if (inj) parts.push(`${inj} ${s.inj}`);
        return { id: t.id, Icon: flagged ? AlertTriangle : CheckCircle2, lamp: flagged ? "is-amber" : "is-green", title: s.checkDone, sub: parts.join(" · "), ts: t.ts };
      }
      if (t.type === "workflow") return { id: t.id, Icon: Boxes, lamp: "is-green", title: s.wfDone, sub: t.title, ts: t.ts };
      if (t.type === "code") return { id: t.id, Icon: ShieldCheck, lamp: "is-green", title: s.codeDone, sub: t.title, ts: t.ts };
      return { id: t.id, Icon: CheckCircle2, lamp: "is-green", title: t.title, sub: "", ts: t.ts };
    });

  return (
    <div className="ax-surface">
      <div className="ax-surface__head">
        <div>
          <h2 className="ax-surface__title">
            <Bell size={18} style={{ marginRight: 8, verticalAlign: "-3px" }} />
            {s.title}
          </h2>
          <p className="ax-surface__sub">{s.sub}</p>
        </div>
      </div>

      {notes.length === 0 ? (
        <div className="ax-empty" style={{ minHeight: 0, padding: "28px 16px" }}>
          <p className="ax-empty__title">{s.empty}</p>
          <p className="ax-empty__sub">{s.emptySub}</p>
        </div>
      ) : (
        <div className="ax-results__list">
          {notes.map((n) => (
            <button
              key={n.id}
              className="ax-row"
              style={{ width: "100%", textAlign: "left", cursor: "pointer" }}
              onClick={() => onOpen(n.id)}
            >
              <span className={`ax-lamp ${n.lamp}`} />
              <n.Icon size={15} aria-hidden />
              <div className="ax-row__main">
                <div className="ax-row__title">{n.title}</div>
                {n.sub && <div className="ax-row__sub">{n.sub}</div>}
              </div>
              <div className="ax-row__meta">{rel(n.ts, lang)}</div>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
