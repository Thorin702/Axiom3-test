// The per-session settings popover (Claude-style): a compact icon button in the composer that
// expands a panel of run controls — Delegation / Auto-review / Reviewer model / Memory /
// Specialist / Compute. Toggles are switches; the model/specialist/compute rows cycle through
// their options on click. Settings persist (the parent owns the state); `specialist` maps to the
// domain-skill registry and is applied to the next run.
import { useState } from "react";
import { SlidersHorizontal, ChevronRight } from "lucide-react";
import type { Lang } from "../i18n";

export type SessionCfg = {
  delegation: boolean;
  autoReview: boolean;
  reviewerModel: string; // "" = Default
  memory: boolean;
  specialist: string; // "" = None, else a skill key
  compute: string; // "local" | "cloud"
};

export const DEFAULT_CFG: SessionCfg = {
  delegation: false,
  autoReview: false,
  reviewerModel: "",
  memory: false,
  specialist: "",
  compute: "cloud", // default cloud — there is no local inference yet; local switches to on-device (Ollama) when available
};

function cycle(list: string[], cur: string): string {
  const i = list.indexOf(cur);
  return list[(i + 1) % list.length];
}

export function SessionSettings({
  cfg,
  onChange,
  lang,
  skills,
  models,
}: {
  cfg: SessionCfg;
  onChange: (c: SessionCfg) => void;
  lang: Lang;
  skills: { key: string; zh: string; en: string }[];
  models: string[]; // connected model labels (for the reviewer model row)
}) {
  const zh = lang === "zh";
  const [open, setOpen] = useState(false);
  const set = (patch: Partial<SessionCfg>) => onChange({ ...cfg, ...patch });

  const reviewerOpts = ["", ...models]; // "" = Default
  const specialistOpts = ["", ...skills.map((s) => s.key)]; // "" = None
  const specialistLabel = cfg.specialist
    ? (() => { const s = skills.find((x) => x.key === cfg.specialist); return s ? (zh ? s.zh : s.en) : cfg.specialist; })()
    : (zh ? "无" : "None");

  return (
    <div className="ax-cfg">
      <button
        className={`ax-iconbtn ${open ? "is-active" : ""}`}
        onClick={() => setOpen((o) => !o)}
        title={zh ? "会话设置" : "Session settings"}
        aria-label={zh ? "会话设置" : "Session settings"}
        aria-expanded={open}
      >
        <SlidersHorizontal size={15} aria-hidden />
      </button>
      {open && (
        <>
          <div className="ax-cfg__scrim" onClick={() => setOpen(false)} />
          <div className="ax-cfg__menu" role="menu">
            {/* Delegation is no longer a manual toggle: analysis is always single-agent, and RESEARCH
                mode delegates automatically (the agent opens a sub-team for complex subtasks). */}
            <Toggle label={zh ? "自动审查" : "Auto-review"} on={cfg.autoReview} onClick={() => set({ autoReview: !cfg.autoReview })} />
            <Select
              label={zh ? "审查模型" : "Reviewer model"}
              value={cfg.reviewerModel || (zh ? "默认" : "Default")}
              onClick={() => set({ reviewerModel: cycle(reviewerOpts, cfg.reviewerModel) })}
            />
            <Toggle label={zh ? "记忆" : "Memory"} on={cfg.memory} onClick={() => set({ memory: !cfg.memory })} />
            <div className="ax-cfg__sep" />
            <Select
              label={zh ? "专家" : "Specialist"}
              value={specialistLabel}
              onClick={() => set({ specialist: cycle(specialistOpts, cfg.specialist) })}
            />
            <Select
              label={zh ? "算力" : "Compute"}
              value={cfg.compute === "cloud" ? (zh ? "云端" : "Cloud") : (zh ? "本地" : "Local")}
              sub={cfg.compute === "cloud"
                ? (zh ? "数据在本地内核处理，仅摘要上传云端模型" : "data processed in local kernel; only summaries sent to the cloud model")
                : (zh ? "模型与数据全在本机，零上传（需 Ollama）" : "model + data stay on-device, zero upload (needs Ollama)")}
              onClick={() => set({ compute: cfg.compute === "cloud" ? "local" : "cloud" })}
            />
          </div>
        </>
      )}
    </div>
  );
}

function Toggle({ label, on, onClick }: { label: string; on: boolean; onClick: () => void }) {
  return (
    <button className="ax-cfg__row" role="menuitemcheckbox" aria-checked={on} onClick={onClick}>
      <span className="ax-cfg__label">{label}</span>
      <span className={`ax-switch ${on ? "is-on" : ""}`} aria-hidden>
        <span className="ax-switch__knob" />
      </span>
    </button>
  );
}

function Select({ label, value, sub, onClick }: { label: string; value: string; sub?: string; onClick: () => void }) {
  return (
    <button className="ax-cfg__row" onClick={onClick}>
      <span className="ax-cfg__label">
        {label}
        {sub && <span className="ax-cfg__sub">{sub}</span>}
      </span>
      <span className="ax-cfg__val">
        {value}
        <ChevronRight size={13} aria-hidden />
      </span>
    </button>
  );
}
