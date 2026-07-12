// The right-side ARTIFACT pane — Claude Science's center of gravity. A finished session's
// artifact lives here with its own tabs (产物 · 消息 · 审查 · 溯源), a version chip, and
// export — so the conversation stays in the center and the made thing sits beside it,
// carrying its own rigor. Scoped to workflow / coding sessions (the generated artifacts);
// draft-check sessions keep their claim-by-claim flow in the thread.
import { useEffect, useRef, useState } from "react";
import { X, FileDown, FileText, ScanSearch, ChevronLeft, ChevronRight, Copy, Check, Maximize2, Minimize2 } from "lucide-react";
import type { Lang } from "../i18n";
import type { Task } from "../lib/types";
import { ArtifactBody } from "./artifact/ArtifactBody";
import { AgentSteps } from "./AgentSteps";
import { ArtifactProvenance } from "./ArtifactProvenance";
import { ArtifactInspector } from "./ArtifactInspector";
import { buildReportHtml } from "../lib/exportReport";

type Tab = "artifact" | "messages" | "review" | "provenance";

export function ArtifactPane({
  lang,
  task,
  forkedFromTitle,
  onExport,
  onSaveReport,
  onClose,
  expanded,
  onToggleExpand,
  busy,
}: {
  lang: Lang;
  task: Task;
  forkedFromTitle?: string;
  onExport: () => void;
  onSaveReport?: (html: string, defaultName: string) => void;
  onClose: () => void;
  expanded?: boolean;
  onToggleExpand?: () => void;
  busy?: boolean;
}) {
  const zh = lang === "zh";
  const artifactRef = useRef<HTMLDivElement>(null);
  const [copied, setCopied] = useState(false);
  function exportReport() {
    if (!artifactRef.current || !onSaveReport) return;
    const theme = document.documentElement.dataset.theme || "light";
    const title = task.title || task.goal || (zh ? "研究报告" : "Report");
    const html = buildReportHtml(artifactRef.current, title, theme);
    const name = title.slice(0, 40).replace(/[^\p{L}\p{N} _-]/gu, "").trim() || "report";
    onSaveReport(html, name);
  }

  // Version history: a follow-up iterates the artifact (v1 → v2 …). Default to the latest.
  const versions = task.versions ?? [{ synthesis: task.synthesis, review: task.review, steps: task.steps, ts: task.ts, goal: task.goal ?? "" }];
  const [vIdx, setVIdx] = useState(versions.length - 1);
  useEffect(() => setVIdx(versions.length - 1), [task.id, versions.length]);
  const ver = versions[Math.min(vIdx, versions.length - 1)];
  async function copyArtifact() {
    const text = ver.synthesis ?? "";
    if (!text) return;
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      setTimeout(() => setCopied(false), 1400);
    } catch {
      /* clipboard unavailable */
    }
  }

  const steps = ver.steps ?? [];
  const reviewFindings = ver.review
    ? ver.review.split("\n").map((l) => l.replace(/^\s*[-*•·]\s?/, "").trim()).filter((l) => l.length > 1)
    : [];

  // For code/analysis sessions the steps ARE the execution log (the code the agent ran +
  // each result), so name the tab accordingly — it's how you independently re-check the work.
  const isRun = task.type === "code";
  const TABS: { key: Tab; zh: string; en: string; n?: number }[] = [
    { key: "artifact", zh: "产物", en: "Artifact" },
    { key: "messages", zh: isRun ? "执行" : "消息", en: isRun ? "Execution" : "Messages", n: steps.length || undefined },
    { key: "review", zh: "审查", en: "Review", n: reviewFindings.length || undefined },
    { key: "provenance", zh: "溯源", en: "Provenance" },
  ];

  const [tab, setTab] = useState<Tab>("artifact");
  // reset to the artifact tab whenever a different session is shown
  useEffect(() => setTab("artifact"), [task.id]);

  return (
    <aside className={`ax-artpane ${expanded ? "is-expanded" : ""}`}>
      <div className="ax-artpane__head">
        <span className="ax-artpane__name" title={task.title}>{task.title}</span>
        {versions.length > 1 ? (
          <span className="ax-artpane__vsel">
            <button className="ax-artpane__vbtn" disabled={vIdx <= 0} onClick={() => setVIdx((i) => Math.max(0, i - 1))} title={zh ? "上一版" : "previous version"}>
              <ChevronLeft size={13} aria-hidden />
            </button>
            <span className="ax-artpane__ver" title={zh ? "版本" : "version"}>v{vIdx + 1}/{versions.length}</span>
            <button className="ax-artpane__vbtn" disabled={vIdx >= versions.length - 1} onClick={() => setVIdx((i) => Math.min(versions.length - 1, i + 1))} title={zh ? "下一版" : "next version"}>
              <ChevronRight size={13} aria-hidden />
            </button>
          </span>
        ) : (
          <span className="ax-artpane__ver" title={zh ? "版本" : "version"}>v1</span>
        )}
        <div className="ax-artpane__actions">
          <button className="ax-iconbtn" onClick={copyArtifact} disabled={busy || !ver.synthesis} title={copied ? (zh ? "已复制" : "Copied") : (zh ? "复制产物文本" : "Copy artifact text")} aria-label="copy artifact">
            {copied ? <Check size={15} aria-hidden /> : <Copy size={15} aria-hidden />}
          </button>
          {onSaveReport && (
            <button className="ax-iconbtn" onClick={exportReport} disabled={busy} title={zh ? "导出报告(HTML)" : "Export report (HTML)"} aria-label="export report">
              <FileText size={15} aria-hidden />
            </button>
          )}
          <button className="ax-iconbtn" onClick={onExport} disabled={busy} title={zh ? "导出溯源" : "Export provenance"} aria-label="export provenance">
            <FileDown size={15} aria-hidden />
          </button>
          {onToggleExpand && (
            <button className="ax-iconbtn" onClick={onToggleExpand} title={expanded ? (zh ? "还原" : "Restore") : (zh ? "展开" : "Expand")} aria-label={expanded ? "restore" : "expand"}>
              {expanded ? <Minimize2 size={15} aria-hidden /> : <Maximize2 size={15} aria-hidden />}
            </button>
          )}
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
          <>
            {versions.length > 1 && (
              <p className="ax-hint" style={{ marginBottom: 10 }}>
                {zh ? `第 ${vIdx + 1} 版 · 目标:${ver.goal}` : `Version ${vIdx + 1} · goal: ${ver.goal}`}
              </p>
            )}
            <div ref={artifactRef}>
              {ver.synthesis ? <ArtifactBody text={ver.synthesis} /> : <p className="ax-hint">{zh ? "无产物内容。" : "No artifact content."}</p>}
            </div>
          </>
        )}

        {tab === "messages" && (
          steps.length === 0 ? (
            <p className="ax-hint">{zh ? (isRun ? "此会话没有执行步骤。" : "此会话没有子智能体消息。") : (isRun ? "No execution steps in this session." : "No sub-agent messages in this session.")}</p>
          ) : (
            <div className="ax-artpane__msgs">
              <AgentSteps steps={steps} lang={lang} />
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
