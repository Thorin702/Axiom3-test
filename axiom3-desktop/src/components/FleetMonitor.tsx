import type { Lang } from "../i18n";
import { Boxes, Network, Cpu, Workflow, Layers, Activity, Users } from "lucide-react";

const S = {
  zh: {
    title: "舰队 · 编排",
    sub: "每个任务都会临时组建一支智能体舰队——少则数个、多则上百——每个智能体绑定一个模型(DeepSeek / Qwen / 本地),任务结束后即拆解回收。这里是这支舰队的实时视图(代表性示例)。",
    live: "实时监视",
    statsLabel: "概览",
    active: "活跃智能体",
    queued: "队列",
    vendors: "模型供应商",
    tokens: "本次令牌",
    agentsLabel: "智能体",
    queueLabel: "队列",
    running: "运行中",
    done: "已完成",
    queuedTag: "排队",
    cross: "跨模型核验",
    waiting: "等待",
    hint: "编排引擎是平台的脊柱:它动态拼装、调度并回收这支舰队。引擎落地后,本视图将切换为实时数据。状态灯只是中性信号(进行/完成/排队),不代表对内容真伪的判断。",
    roles: {
      retriever: "检索员",
      checker: "校验员",
      synth: "综述员",
      image: "图像生成",
      analyst: "数据分析",
      supervisor: "编排 supervisor",
    },
    q: {
      a: "为「跨厂商一致性」拉取候选证据段落",
      b: "把第 3 节草稿切成断言并排队送检",
      c: "为图表请求生成配图",
      d: "汇总各校验员结果,准备下一轮派发",
    },
  },
  en: {
    title: "Fleet · Orchestration",
    sub: "Every task spins up a temporary fleet of agents — anywhere from a handful to hundreds — each bound to a model (DeepSeek / Qwen / local), then torn down when the task finishes. This is the live view of that fleet (representative sample).",
    live: "Live monitor",
    statsLabel: "Overview",
    active: "Active agents",
    queued: "Queued",
    vendors: "Vendors",
    tokens: "Tokens this run",
    agentsLabel: "Agents",
    queueLabel: "Queue",
    running: "running",
    done: "done",
    queuedTag: "queued",
    cross: "cross-model",
    waiting: "waiting",
    hint: "The orchestration engine is the platform's spine: it assembles, schedules, and recycles this fleet on the fly. Once it lands, this view goes live. Status lamps are neutral signals (running / done / queued) — never a verdict on whether content is true.",
    roles: {
      retriever: "Retriever",
      checker: "Checker",
      synth: "Synthesizer",
      image: "Image",
      analyst: "Analyst",
      supervisor: "Supervisor",
    },
    q: {
      a: "Pull candidate evidence passages for cross-vendor consistency",
      b: "Split section 3 draft into claims and enqueue for checking",
      c: "Generate a figure for the chart request",
      d: "Aggregate checker results, stage the next dispatch round",
    },
  },
};

export function FleetMonitor({ lang }: { lang: Lang }) {
  const s = S[lang];

  const stats = [
    { icon: Activity, num: "7", label: s.active },
    { icon: Layers, num: "12", label: s.queued },
    { icon: Network, num: "3", label: s.vendors },
    { icon: Cpu, num: "48", unit: "k", label: s.tokens },
  ];

  const agents = [
    { role: s.roles.supervisor, model: "DeepSeek-V4", lamp: "is-violet", tag: s.cross },
    { role: s.roles.retriever, model: "DeepSeek-V4", lamp: "is-amber", tag: s.running },
    { role: s.roles.checker, model: "Qwen-Image", lamp: "is-amber", tag: s.running },
    { role: s.roles.synth, model: "DeepSeek-V4", lamp: "is-neutral", tag: s.queuedTag },
    { role: s.roles.image, model: "Qwen-Image", lamp: "is-green", tag: s.done },
    { role: s.roles.analyst, model: "本地 Ollama", lamp: "is-green", tag: s.done },
  ];

  const queue = [s.q.a, s.q.b, s.q.c, s.q.d];

  return (
    <div className="ax-surface">
      <div className="ax-surface__head">
        <h2 className="ax-surface__title">{s.title}</h2>
        <p className="ax-surface__sub">{s.sub}</p>
        <div className="ax-surface__actions">
          <span className="ax-badge">
            <span className="ax-lamp is-amber" /> {s.live}
          </span>
        </div>
      </div>

      <div className="ax-sec-label">{s.statsLabel}</div>
      <div className="ax-grid" style={{ marginBottom: 8 }}>
        {stats.map((st) => {
          const Icon = st.icon;
          return (
            <div className="ax-card ax-card--pad" key={st.label}>
              <div className="ax-stat">
                <div className="ax-stat__num">
                  {st.num}
                  {st.unit ? <small>{st.unit}</small> : null}
                </div>
                <div className="ax-stat__label">
                  <Icon size={13} style={{ verticalAlign: "-2px", marginRight: 4 }} />
                  {st.label}
                </div>
              </div>
            </div>
          );
        })}
      </div>

      <div className="ax-sec-label">
        <Boxes size={13} style={{ verticalAlign: "-2px", marginRight: 4 }} />
        {s.agentsLabel}
      </div>
      <div className="ax-grid" style={{ marginBottom: 8 }}>
        {agents.map((a) => (
          <div className="ax-card ax-card--pad" key={a.role}>
            <div className="ax-card__head">
              <div className="ax-card__title">{a.role}</div>
              <span className="ax-badge">
                <span className={`ax-lamp ${a.lamp}`} /> {a.tag}
              </span>
            </div>
            <div className="ax-card__body">{a.model}</div>
          </div>
        ))}
      </div>

      <div className="ax-sec-label">
        <Workflow size={13} style={{ verticalAlign: "-2px", marginRight: 4 }} />
        {s.queueLabel}
      </div>
      {queue.map((q, i) => (
        <div className="ax-row" key={i}>
          <div className="ax-row__main">
            <div className="ax-row__title">{q}</div>
          </div>
          <div className="ax-row__meta">
            <span className="ax-lamp is-neutral" /> {s.waiting}
          </div>
        </div>
      ))}

      <p className="ax-hint" style={{ marginTop: 10 }}>
        <Users size={13} style={{ verticalAlign: "-2px", marginRight: 4 }} />
        {s.hint}
      </p>
    </div>
  );
}
