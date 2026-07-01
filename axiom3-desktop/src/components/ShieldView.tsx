// 抗投毒护盾 — reports what the checks actually NEUTRALIZED. Imported reference text can
// carry hidden "ignore previous instructions" injections; a claim whose evidence tripped
// the detector reaches the terminal InjectionNeutralized state (never Verified/Refuted).
// This surfaces those real events — it reports findings, it does not certify a doc safe.
import { ShieldCheck, ShieldAlert, FileText } from "lucide-react";
import type { Lang } from "../i18n";
import type { Task } from "../lib/types";

const S = {
  zh: {
    title: "抗投毒护盾",
    sub: "在任何模型读取导入材料之前,护盾会中和其中隐藏的注入指令,并标记可疑段落 —— 它报告的是自己中和了什么,而不是判定文档是否安全。",
    active: "护盾在线", policy: "对导入证据启用指令注入过滤 · 中和后终态为「已中和」,永不产出成立/驳斥。",
    scanned: "已检查主张", neutralized: "已中和注入", clean: "未触发",
    recentLabel: "已中和的注入",
    badgeNeutral: "已中和",
    empty: "尚未发现注入",
    emptySub: "运行带文献检索的 /校对 后,若导入证据里含注入指令,会在这里列出被中和的项。",
    from: "来自",
    note: "护盾报告它中和了什么;它从不断言某文档「安全」或「恶意」。",
  },
  en: {
    title: "Anti-poisoning shield",
    sub: "Before any model reads imported material, the shield neutralizes injected instructions hidden inside and flags suspicious passages — it reports what it neutralized, it does not certify a document safe.",
    active: "Shield active", policy: "Injection filtering on imported evidence · neutralized items terminate as “neutralized”, never Verified/Refuted.",
    scanned: "Claims checked", neutralized: "Injections neutralized", clean: "not triggered",
    recentLabel: "Neutralized injections",
    badgeNeutral: "neutralized",
    empty: "No injections found yet",
    emptySub: "Run a /check with retrieval on — if imported evidence carries injected instructions, the neutralized items appear here.",
    from: "from",
    note: "The shield reports what it neutralized; it never asserts a document is “safe” or “malicious”.",
  },
};

export function ShieldView({ lang, tasks }: { lang: Lang; tasks: Task[] }) {
  const s = S[lang];
  const neutralized = tasks.flatMap((t) =>
    (t.result?.entries ?? []).filter((e) => e.status === "injection_neutralized").map((e) => ({ text: e.text, task: t.title }))
  );
  const scanned = tasks.reduce((n, t) => n + (t.result?.total ?? 0), 0);

  return (
    <div className="ax-surface">
      <div className="ax-surface__head">
        <div>
          <h2 className="ax-surface__title">{s.title}</h2>
          <p className="ax-surface__sub">{s.sub}</p>
        </div>
      </div>

      <div className="ax-card ax-card--pad">
        <div className="ax-row" style={{ border: "none", padding: 0 }}>
          <span className="ax-lamp is-green" />
          <div className="ax-row__main">
            <div className="ax-row__title">{s.active}</div>
            <div className="ax-row__sub">{s.policy}</div>
          </div>
        </div>
      </div>

      <div className="ax-grid" style={{ marginTop: 12 }}>
        <div className="ax-card ax-stat">
          <div className="ax-stat__num">{scanned}</div>
          <div className="ax-stat__label">{s.scanned}</div>
        </div>
        <div className="ax-card ax-stat">
          <div className="ax-stat__num" style={{ color: neutralized.length ? "var(--ax-lamp-violet)" : undefined }}>{neutralized.length}</div>
          <div className="ax-stat__label">{s.neutralized}</div>
        </div>
        <div className="ax-card ax-stat">
          <div className="ax-stat__num">
            <span className="ax-lamp is-green" style={{ marginRight: 8 }} />
            {scanned - neutralized.length}
          </div>
          <div className="ax-stat__label">{s.clean}</div>
        </div>
      </div>

      <div className="ax-sec-label" style={{ marginTop: 16 }}>
        <ShieldAlert size={14} style={{ verticalAlign: "-2px", marginRight: 6 }} /> {s.recentLabel} · {neutralized.length}
      </div>
      {neutralized.length === 0 ? (
        <div className="ax-empty" style={{ minHeight: 0, padding: "26px 16px" }}>
          <p className="ax-empty__title">{s.empty}</p>
          <p className="ax-empty__sub">{s.emptySub}</p>
        </div>
      ) : (
        <div className="ax-results__list">
          {neutralized.slice(0, 40).map((n, i) => (
            <div className="ax-row" key={i}>
              <span className="ax-lamp is-violet" />
              <div className="ax-row__main">
                <div className="ax-row__title" style={{ whiteSpace: "normal" }}>{n.text}</div>
                <div className="ax-row__sub">
                  <FileText size={12} style={{ verticalAlign: "-2px", marginRight: 4 }} />
                  {s.from} {n.task}
                </div>
              </div>
              <span className="ax-badge">
                <span className="ax-lamp is-violet" /> {s.badgeNeutral}
              </span>
            </div>
          ))}
        </div>
      )}

      <div className="ax-hint" style={{ marginTop: 14 }}>
        <ShieldCheck size={12} style={{ verticalAlign: "-2px", marginRight: 4 }} />
        {s.note}
      </div>
    </div>
  );
}
