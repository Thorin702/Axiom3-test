// 舰队 · 编排 — the REAL fleet: the connected model families and a LIVE aggregate of the
// running orchestration (subscribes to run_event, like the Activity rail, but summarized).
// Per-agent detail lives in the Activity rail; this is the fleet overview.
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Boxes, Network, Activity, Users } from "lucide-react";
import type { Lang } from "../i18n";
import type { RunEvent } from "../lib/types";

const S = {
  zh: {
    title: "舰队 · 编排",
    sub: "每个任务临时组建一支智能体舰队,每个智能体绑定一个模型,任务结束即拆解。这里是连接的模型与实时编排的概览 —— 逐个智能体的细节在右侧活动栏。",
    models: "已连接模型", noModels: "未连接 · 在设置中添加",
    running: "运行中的智能体", done: "已完成", runs: "运行批次",
    live: "实时",
    modelsLabel: "模型家族(独立性)",
    connected: "已连接", off: "未连接",
    aggLabel: "编排(实时聚合)",
    hint: "跨厂商独立性:连接 ≥2 个独立模型家族,交叉核查才有意义。逐个智能体队列见活动栏。",
  },
  en: {
    title: "Fleet · Orchestration",
    sub: "Each task assembles a temporary fleet of agents, each bound to a model, torn down when done. This is the overview of connected models + live orchestration — per-agent detail is in the Activity rail.",
    models: "Models connected", noModels: "none · add in Settings",
    running: "Agents running", done: "Agents done", runs: "Runs",
    live: "live",
    modelsLabel: "Model families (independence)",
    connected: "connected", off: "not connected",
    aggLabel: "Orchestration (live aggregate)",
    hint: "Cross-vendor independence: connect ≥2 independent model families for cross-checks to mean anything. The per-agent queue is in the Activity rail.",
  },
};

const FAMILIES = ["deepseek", "qwen", "anthropic", "openai", "gemini"];

export function FleetMonitor({ lang }: { lang: Lang }) {
  const s = S[lang];
  const [connected, setConnected] = useState<Record<string, boolean>>({});
  const [agg, setAgg] = useState({ runs: 0, spawned: 0, finished: 0 });

  useEffect(() => {
    invoke<{ family: string; connected: boolean }[]>("provider_status")
      .then((ps) => setConnected(Object.fromEntries(ps.map((p) => [p.family, p.connected]))))
      .catch(() => {});
  }, []);

  useEffect(() => {
    const un = listen<RunEvent>("run_event", (e) => {
      const ev = e.payload;
      setAgg((a) =>
        ev.kind === "run_started"
          ? { ...a, runs: a.runs + 1 }
          : ev.kind === "agent_spawned"
          ? { ...a, spawned: a.spawned + 1 }
          : ev.kind === "agent_finished"
          ? { ...a, finished: a.finished + 1 }
          : a
      );
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  const nConnected = Object.values(connected).filter(Boolean).length;
  const running = Math.max(0, agg.spawned - agg.finished);

  return (
    <div className="ax-surface">
      <div className="ax-surface__head">
        <div>
          <h2 className="ax-surface__title">{s.title}</h2>
          <p className="ax-surface__sub">{s.sub}</p>
        </div>
      </div>

      <div className="ax-grid">
        <div className="ax-card ax-stat">
          <div className="ax-card__head"><Network size={15} /><span className="ax-card__title">{s.models}</span></div>
          <div className="ax-stat__num" style={{ color: nConnected >= 2 ? "var(--ax-accent)" : undefined }}>{nConnected}</div>
          <div className="ax-stat__label">{nConnected === 0 ? s.noModels : s.models}</div>
        </div>
        <div className="ax-card ax-stat">
          <div className="ax-card__head"><Activity size={15} /><span className="ax-card__title">{s.running}</span></div>
          <div className="ax-stat__num" style={{ color: running ? "var(--ax-lamp-amber)" : undefined }}>{running}</div>
          <div className="ax-stat__label">{s.running}</div>
        </div>
        <div className="ax-card ax-stat">
          <div className="ax-card__head"><Users size={15} /><span className="ax-card__title">{s.done}</span></div>
          <div className="ax-stat__num">{agg.finished}</div>
          <div className="ax-stat__label">{s.done}</div>
        </div>
        <div className="ax-card ax-stat">
          <div className="ax-card__head"><Boxes size={15} /><span className="ax-card__title">{s.runs}</span></div>
          <div className="ax-stat__num">{agg.runs}</div>
          <div className="ax-stat__label">{s.runs}</div>
        </div>
      </div>

      <div className="ax-sec-label" style={{ marginTop: 16 }}>{s.modelsLabel}</div>
      <div className="ax-results__list">
        {FAMILIES.map((f) => (
          <div className="ax-row" key={f}>
            <span className={`ax-lamp ${connected[f] ? "is-green" : "is-neutral"}`} />
            <div className="ax-row__main">
              <div className="ax-row__title" style={{ textTransform: "capitalize" }}>{f}</div>
            </div>
            <span className="ax-badge">{connected[f] ? s.connected : s.off}</span>
          </div>
        ))}
      </div>

      <div className="ax-hint" style={{ marginTop: 14 }}>{s.hint}</div>
    </div>
  );
}
