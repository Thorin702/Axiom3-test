// The right-side ARTIFACT pane — Claude Science's center of gravity. A finished session's
// artifact lives here with its own tabs (产物 · 消息 · 审查 · 溯源), a version chip, and
// export — so the conversation stays in the center and the made thing sits beside it,
// carrying its own rigor. Scoped to workflow / coding sessions (the generated artifacts);
// draft-check sessions keep their claim-by-claim flow in the thread.
import { useEffect, useState } from "react";
import { X, FileDown, ScanSearch } from "lucide-react";
import type { Lang } from "../i18n";
import type { Task } from "../lib/types";
import { ArtifactBody } from "./artifact/ArtifactBody";
import { ArtifactProvenance } from "./ArtifactProvenance";
import { ArtifactInspector } from "./ArtifactInspector";

type Tab = "artifact" | "messages" | "review" | "provenance";

export function ArtifactPane({
  lang,
  task,
  forkedFromTitle,
  onExport,
  onClose,
  busy,
}: {
  lang: Lang;
  task: Task;
  forkedFromTitle?: string;
  onExport: () => void;
  onClose: () => void;
  busy?: boolean;
}) {
  const zh = lang === "zh";
  const steps = task.steps ?? [];
  const reviewFindings = task.review
    ? task.review.split("\n").map((l) => l.replace(/^\s*[-*•·]\s?/, "").trim()).filter((l) => l.length > 1)
    : [];

  const TABS: { key: Tab; zh: string; en: string; n?: number }[] = [
    { key: "artifact", zh: "产物", en: "Artifact" },
    { key: "messages", zh: "消息", en: "Messages", n: steps.length || undefined },
    { key: "review", zh: "审查", en: "Review", n: reviewFindings.length || undefined },
    { key: "provenance", zh: "溯源", en: "Provenance" },
  ];

  const [tab, setTab] = useState<Tab>("artifact");
  // reset to the artifact tab whenever a different session is shown
  useEffect(() => setTab("artifact"), [task.id]);

  return (
    <aside className="ax-artpane">
      <div className="ax-artpane__head">
        <span className="ax-artpane__name" title={task.title}>{task.title}</span>
        <span className="ax-artpane__ver" title={zh ? "版本(暂为单版)" : "version"}>v1</span>
        <div className="ax-artpane__actions">
          <button className="ax-iconbtn" onClick={onExport} disabled={busy} title={zh ? "导出溯源" : "Export provenance"} aria-label="export">
            <FileDown size={15} aria-hidden />
          </button>
          <button className="ax-iconbtn" onClick={onClose} title={zh ? "关闭" : "Close"} aria-label="close">
            <X size={16} aria-hidden />
          </button>
        </div>
      </div>

      <div className="ax-artpane__tabs" role="tablist">
        {TABS.map((tb) => (
          <button
            key={tb.key}
            role="tab"
            aria-selected={tab === tb.key}
            className={`ax-artpane__tab ${tab === tb.key ? "is-on" : ""}`}
            onClick={() => setTab(tb.key)}
          >
            {zh ? tb.zh : tb.en}
            {tb.n ? <span className="ax-artpane__badge">{tb.n}</span> : null}
          </button>
        ))}
      </div>

      <div className="ax-artpane__body">
        {tab === "artifact" && (
          task.synthesis ? <ArtifactBody text={task.synthesis} /> : <p className="ax-hint">{zh ? "无产物内容。" : "No artifact content."}</p>
        )}

        {tab === "messages" && (
          steps.length === 0 ? (
            <p className="ax-hint">{zh ? "此会话没有子智能体消息。" : "No sub-agent messages in this session."}</p>
          ) : (
            <div className="ax-artpane__msgs">
              {steps.map((s, i) => (
                <div className="ax-msg" key={i}>
                  <div className="ax-msg__head">
                    <span className={`ax-adot ${s.status === "ok" ? "is-fin" : s.status === "failed" ? "is-err" : "is-fin"}`} />
                    <span className="ax-msg__role">{s.role}</span>
                    {s.model && <span className="ax-msg__model">{s.model}</span>}
                  </div>
                  <div className="ax-msg__body">
                    <ArtifactBody text={s.output || (zh ? "(无输出)" : "(no output)")} />
                  </div>
                </div>
              ))}
            </div>
          )
        )}

        {tab === "review" && (
          reviewFindings.length === 0 ? (
            <div className="ax-reviewer__clear"><ScanSearch size={13} aria-hidden /> {zh ? "审查员未标记问题(仍需你自行核验)。" : "The reviewer flagged nothing (still verify yourself)."}</div>
          ) : (
            <div className="ax-artpane__review">
              <p className="ax-hint" style={{ marginBottom: 8 }}>
                {zh ? `审查员 · ${reviewFindings.length} 项发现 —— 对照产出检查一致性、缺口与无据主张;它减少但不消除错误,不判断真伪。`
                    : `Reviewer · ${reviewFindings.length} findings — consistency, gaps, unsupported claims. Reduces but doesn't eliminate errors; not a truth judgment.`}
              </p>
              {reviewFindings.map((f, i) => (
                <div className="ax-reviewer__finding" key={i}>{f}</div>
              ))}
            </div>
          )
        )}

        {tab === "provenance" && (
          <>
            <ArtifactProvenance lang={lang} task={task} forkedFromTitle={forkedFromTitle} onExport={onExport} busy={busy} />
            <ArtifactInspector lang={lang} task={task} />
          </>
        )}
      </div>
    </aside>
  );
}
