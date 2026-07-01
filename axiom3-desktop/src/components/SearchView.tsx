// 检索 · 全局检索 — REAL search across the workspace: check-result claims + their
// evidence, task titles/outputs, and the audit event log. Results are recalled matches
// for your judgement, never an answer/verdict. Supports a `type:` filter prefix.
import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Search, ListChecks, Sparkles, History } from "lucide-react";
import type { Lang } from "../i18n";
import type { Task, EventInfo } from "../lib/types";
import { attentionOf, lampClass } from "../lib/attention";

const S = {
  zh: {
    title: "检索 · 全局检索",
    sub: "在校对结果的主张与证据、任务、以及审计事件中检索 —— 这是召回的匹配项,供你核对,不是结论。",
    placeholder: "搜索主张、证据、任务、事件…  可用 type:claim / type:task / type:event",
    results: "检索结果",
    none: "没有匹配项。",
    emptyTitle: "输入关键词开始检索",
    emptySub: "在你运行过的校对与工作流结果中检索;结果附来源,供你判断。",
    all: "全部", claim: "主张", task: "任务", event: "事件",
    kClaim: "主张", kTask: "任务", kEvent: "事件",
  },
  en: {
    title: "Search · Across the workspace",
    sub: "Search claims + evidence from check results, tasks, and the audit event log — recalled matches for your judgement, not a conclusion.",
    placeholder: "Search claims, evidence, tasks, events…  try type:claim / type:task / type:event",
    results: "Results",
    none: "No matches.",
    emptyTitle: "Type a query to search",
    emptySub: "Searches across the checks and workflows you've run; results carry their source, for your judgement.",
    all: "All", claim: "Claims", task: "Tasks", event: "Events",
    kClaim: "claim", kTask: "task", kEvent: "event",
  },
};

const OP: Record<string, { zh: string; en: string }> = {
  "draft.checked": { zh: "校对草稿", en: "Checked a draft" },
  note: { zh: "记录事件", en: "Note" },
};
function opLabel(op: string, lang: Lang): string {
  return OP[op] ? OP[op][lang] : op;
}

type Hit = { kind: "task" | "claim" | "event"; title: string; sub: string; lamp: string; taskId?: string; key: string };

export function SearchView({ lang, tasks, onOpenTask }: { lang: Lang; tasks: Task[]; onOpenTask: (id: string) => void }) {
  const s = S[lang];
  const [query, setQuery] = useState("");
  const [events, setEvents] = useState<EventInfo[]>([]);

  useEffect(() => {
    invoke<EventInfo[]>("list_events", { limit: 500 }).then(setEvents).catch(() => {});
  }, []);

  const { typeFilter, text } = useMemo(() => {
    const m = query.match(/type:(claim|task|event)\b/i);
    return { typeFilter: m ? m[1].toLowerCase() : null, text: query.replace(/type:\S+/i, "").trim().toLowerCase() };
  }, [query]);

  const hits: Hit[] = useMemo(() => {
    if (!text) return [];
    const out: Hit[] = [];
    const want = (k: string) => !typeFilter || typeFilter === k;
    for (const t of tasks) {
      if (want("task") && (t.title.toLowerCase().includes(text) || (t.synthesis ?? "").toLowerCase().includes(text))) {
        out.push({ kind: "task", title: t.title, sub: `${s.kTask} · /${t.type}`, lamp: t.status === "done" ? "is-green" : "is-neutral", taskId: t.id, key: `t${t.id}` });
      }
      if (want("claim") && t.result) {
        for (const e of t.result.entries) {
          if (e.text.toLowerCase().includes(text) || e.evidence.some((ev) => ev.toLowerCase().includes(text))) {
            out.push({ kind: "claim", title: e.text, sub: `${s.kClaim} · ${t.title}`, lamp: lampClass[attentionOf(e)], taskId: t.id, key: `c${e.claim_id}` });
          }
        }
      }
    }
    if (want("event")) {
      for (const ev of events) {
        if (opLabel(ev.op, lang).toLowerCase().includes(text) || ev.op.toLowerCase().includes(text) || ev.actor.toLowerCase().includes(text)) {
          out.push({ kind: "event", title: opLabel(ev.op, lang), sub: `${s.kEvent} · #${ev.seq} · ${ev.actor}`, lamp: "is-green", key: `e${ev.seq}` });
        }
      }
    }
    return out.slice(0, 80);
  }, [text, typeFilter, tasks, events, lang, s.kTask, s.kClaim, s.kEvent]);

  const chips: { label: string; type: string | null }[] = [
    { label: s.all, type: null },
    { label: s.claim, type: "claim" },
    { label: s.task, type: "task" },
    { label: s.event, type: "event" },
  ];
  function setType(type: string | null) {
    const base = query.replace(/type:\S+\s*/i, "").trim();
    setQuery(type ? `type:${type} ${base}`.trim() : base);
  }
  const iconFor = (k: Hit["kind"]) => (k === "claim" ? ListChecks : k === "task" ? Sparkles : History);

  return (
    <div className="ax-surface">
      <div className="ax-surface__head">
        <div>
          <h2 className="ax-surface__title">{s.title}</h2>
          <p className="ax-surface__sub">{s.sub}</p>
        </div>
      </div>

      <div className="ax-input" style={{ display: "flex", alignItems: "center", gap: 8 }}>
        <Search size={15} aria-hidden />
        <input
          autoFocus
          className="ax-input"
          style={{ border: "none", padding: 0, flex: 1, background: "transparent" }}
          placeholder={s.placeholder}
          value={query}
          onChange={(e) => setQuery(e.currentTarget.value)}
        />
      </div>
      <div className="ax-toolrow" style={{ flexWrap: "wrap", gap: 6, marginTop: 8 }}>
        {chips.map((c) => (
          <button key={c.label} className={`ax-chip ${typeFilter === c.type ? "is-on" : ""}`} onClick={() => setType(c.type)} style={{ padding: "4px 11px" }}>
            {c.label}
          </button>
        ))}
      </div>

      {!text ? (
        <div className="ax-empty" style={{ minHeight: 0, padding: "28px 16px", marginTop: 12 }}>
          <div className="ax-empty__title">{s.emptyTitle}</div>
          <div className="ax-empty__sub">{s.emptySub}</div>
        </div>
      ) : (
        <>
          <div className="ax-sec-label" style={{ marginTop: 14 }}>{s.results} · {hits.length}</div>
          {hits.length === 0 ? (
            <div className="ax-hint">{s.none}</div>
          ) : (
            <div className="ax-results__list">
              {hits.map((h) => {
                const Icon = iconFor(h.kind);
                const clickable = !!h.taskId;
                return (
                  <button
                    key={h.key}
                    className="ax-row"
                    style={{ width: "100%", textAlign: "left", cursor: clickable ? "pointer" : "default" }}
                    onClick={() => h.taskId && onOpenTask(h.taskId)}
                  >
                    <span className={`ax-lamp ${h.lamp}`} />
                    <Icon size={15} aria-hidden />
                    <div className="ax-row__main">
                      <div className="ax-row__title" style={{ whiteSpace: "normal" }}>{h.title}</div>
                      <div className="ax-row__sub">{h.sub}</div>
                    </div>
                  </button>
                );
              })}
            </div>
          )}
        </>
      )}
    </div>
  );
}
