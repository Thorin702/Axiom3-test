// 概览 — a usage DASHBOARD (看板) over everything the workspace has run: how many sessions, the
// research/analysis split, output volume (figures / sources / words / an approximate token spend),
// and a time distribution. Descriptive counts of activity — never a quality score.
import { Gauge, FlaskConical, Microscope, ScanSearch, Image, Link2, FileText, Coins } from "lucide-react";
import type { Lang } from "../i18n";
import type { Task } from "../lib/types";

function fig(s?: string): number {
  if (!s) return 0;
  return (s.match(/```(pyfig|mermaid|network|reaction)/g) || []).length;
}
// A rough token estimate from produced text (CJK ≈ 1.6 chars/token) — an order-of-magnitude gauge,
// labelled as approximate, not a billing figure.
function estTokens(t: Task): number {
  const body = (t.synthesis?.length ?? 0) + (t.review?.length ?? 0) + (t.steps ?? []).reduce((a, s) => a + (s.output?.length ?? 0), 0);
  return Math.round(body / 1.6);
}

export function MetricsView({ lang, tasks }: { lang: Lang; tasks: Task[] }) {
  const zh = lang === "zh";
  const done = tasks.filter((t) => t.type === "research" || t.type === "code" || t.type === "workflow");
  const research = done.filter((t) => t.type === "research").length;
  const analysis = done.filter((t) => t.type === "code").length;
  const reviewed = done.filter((t) => t.review && t.review.trim()).length;
  const figures = done.reduce((a, t) => a + fig(t.synthesis) + (t.versions ?? []).reduce((b, v) => b + fig(v.synthesis), 0), 0);
  const sources = done.reduce((a, t) => a + (t.sources?.length ?? 0), 0);
  const words = done.reduce((a, t) => a + (t.synthesis?.length ?? 0), 0);
  const tokens = done.reduce((a, t) => a + estTokens(t), 0);
  const rounds = done.reduce((a, t) => a + (t.versions?.length ?? 1), 0);

  const cards = [
    { icon: Gauge, label: zh ? "课题总数" : "Sessions", num: done.length, sub: zh ? `${rounds} 轮执行` : `${rounds} rounds` },
    { icon: FlaskConical, label: zh ? "研究" : "Research", num: research, sub: zh ? "深度联网" : "deep" },
    { icon: Microscope, label: zh ? "分析" : "Analysis", num: analysis, sub: zh ? "本地数据" : "local" },
    { icon: ScanSearch, label: zh ? "已审查" : "Reviewed", num: reviewed, sub: done.length ? `${Math.round((100 * reviewed) / done.length)}%` : "—" },
    { icon: Image, label: zh ? "图形" : "Figures", num: figures, sub: zh ? "累计产出" : "total" },
    { icon: Link2, label: zh ? "引用来源" : "Sources", num: sources, sub: zh ? "累计" : "total" },
    { icon: FileText, label: zh ? "报告字数" : "Words", num: words.toLocaleString(), sub: zh ? "正文合计" : "body" },
    { icon: Coins, label: zh ? "估算 Token" : "Est. tokens", num: tokens >= 1000 ? `${(tokens / 1000).toFixed(1)}k` : String(tokens), sub: zh ? "约数" : "approx" },
  ];

  // last-14-days activity histogram (by local day)
  const days: { key: string; label: string; n: number }[] = [];
  const now = new Date();
  for (let i = 13; i >= 0; i--) {
    const d = new Date(now.getFullYear(), now.getMonth(), now.getDate() - i);
    const key = `${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`;
    days.push({ key, label: `${d.getMonth() + 1}/${d.getDate()}`, n: 0 });
  }
  const dmap = new Map(days.map((d) => [d.key, d]));
  for (const t of done) {
    const d = new Date(t.ts);
    const k = `${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`;
    const e = dmap.get(k);
    if (e) e.n += 1;
  }
  const peak = Math.max(1, ...days.map((d) => d.n));

  return (
    <div className="ax-panel ax-dash">
      <div className="ax-panel__head">
        <span className="ax-panel__title"><Gauge size={16} aria-hidden /> {zh ? "概览 · 使用看板" : "Overview · usage"}</span>
      </div>
      <p className="ax-hint" style={{ margin: "0 0 14px" }}>
        {zh ? "工作区活动的一张快照——做了多少、产出多少、时间怎么分布。描述性数字,不是质量评分。" : "A snapshot of activity — how much was run and produced. Descriptive, not a quality score."}
      </p>

      <div className="ax-dash__cards">
        {cards.map((c) => {
          const Icon = c.icon;
          return (
            <div className="ax-dash__card" key={c.label}>
              <div className="ax-dash__chead"><Icon size={14} aria-hidden /> {c.label}</div>
              <div className="ax-dash__num">{c.num}</div>
              <div className="ax-dash__sub">{c.sub}</div>
            </div>
          );
        })}
      </div>

      <div className="ax-sec-label" style={{ marginTop: 20 }}>{zh ? "近 14 天活动" : "Last 14 days"}</div>
      <div className="ax-dash__hist">
        {days.map((d) => (
          <div className="ax-dash__bar" key={d.key} title={`${d.label} · ${d.n}`}>
            <div className="ax-dash__barfill" style={{ height: `${Math.round((d.n / peak) * 100)}%`, opacity: d.n ? 1 : 0.25 }} />
            <span className="ax-dash__barlabel">{d.label}</span>
          </div>
        ))}
      </div>
      {done.length === 0 && <p className="ax-hint" style={{ marginTop: 12 }}>{zh ? "还没有课题 — 跑一个研究或分析后这里就有数据了。" : "No sessions yet."}</p>}
    </div>
  );
}
