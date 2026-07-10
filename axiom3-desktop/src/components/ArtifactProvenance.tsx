// "How this was made" — the provenance footer every artifact carries, so a result can be
// validated and reproduced months later. Nothing here is a judgment of the content; it is
// a factual record of WHICH agents/models produced it and WHERE it sits in the audit chain.
import { FileDown, GitBranch, ShieldCheck } from "lucide-react";
import type { Lang } from "../i18n";
import type { Task } from "../lib/types";

const TYPE_LABEL: Record<string, { zh: string; en: string }> = {
  check_draft: { zh: "校对", en: "draft check" },
  workflow: { zh: "多智能体工作流", en: "multi-agent workflow" },
  code: { zh: "编程", en: "coding" },
  research: { zh: "调研", en: "research" },
  ppt: { zh: "PPT", en: "slides" },
};

export function ArtifactProvenance({
  lang,
  task,
  forkedFromTitle,
  onExport,
  busy,
}: {
  lang: Lang;
  task: Task;
  forkedFromTitle?: string;
  onExport: () => void;
  busy?: boolean;
}) {
  const zh = lang === "zh";

  // Which models actually produced this artifact (workflow sub-agents + cross-vendor fleet).
  const models = new Set<string>();
  task.steps?.forEach((s) => s.model && models.add(s.model));
  task.result?.entries.forEach((e) => e.fleet?.vendors.forEach((v) => models.add(v.model)));

  const specialists = task.steps?.length ?? 0;
  const rows: { k: string; v: string }[] = [
    {
      k: zh ? "会话类型" : "Session",
      v:
        (TYPE_LABEL[task.type] ? (zh ? TYPE_LABEL[task.type].zh : TYPE_LABEL[task.type].en) : task.type) +
        (task.deep ? (zh ? " · 深度搜索 (AB-MCTS)" : " · deep search (AB-MCTS)") : ""),
    },
    { k: zh ? "参与模型" : "Models", v: models.size ? [...models].join("  ·  ") : "—" },
  ];
  if (specialists > 0) {
    rows.push({
      k: zh ? "智能体" : "Agents",
      v: zh ? `${specialists} 个专家 + 1 个审查员` : `${specialists} specialists + 1 reviewer`,
    });
  }
  if (task.result) {
    rows.push({
      k: zh ? "审计事件" : "Audit event",
      v: zh ? `#${task.result.event_seq} · 哈希链` : `#${task.result.event_seq} · hash chain`,
    });
  }
  if (task.forkedFrom) {
    rows.push({ k: zh ? "分叉自" : "Forked from", v: forkedFromTitle ?? task.forkedFrom });
  }
  rows.push({ k: zh ? "时间" : "When", v: new Date(task.ts).toLocaleString() });

  return (
    <details className="ax-prov">
      <summary className="ax-prov__sum">
        <ShieldCheck size={13} aria-hidden />
        {zh ? "如何生成 · 可独立复核" : "How this was made · independently re-checkable"}
      </summary>
      <div className="ax-prov__body">
        <dl className="ax-prov__grid">
          {rows.map((r) => (
            <div className="ax-prov__row" key={r.k}>
              <dt>{r.k}</dt>
              <dd>
                {r.k === (zh ? "分叉自" : "Forked from") && <GitBranch size={11} style={{ verticalAlign: "-1px", marginRight: 4 }} aria-hidden />}
                {r.v}
              </dd>
            </div>
          ))}
        </dl>
        <div className="ax-toolrow" style={{ marginTop: 10 }}>
          <button className="ax-btn" onClick={onExport} disabled={busy}>
            <FileDown size={14} aria-hidden /> {zh ? "导出溯源" : "Export provenance"}
          </button>
        </div>
        <p className="ax-hint" style={{ marginTop: 8 }}>
          {zh
            ? "溯源记录锚定在只追加的哈希链上,可在工作台之外独立复核。它证明的是「这份产物如何被生成」,而非其中的主张为真。"
            : "Provenance is anchored to the append-only hash chain and can be re-verified outside the workspace. It records HOW this artifact was made — not that its claims are true."}
        </p>
      </div>
    </details>
  );
}
