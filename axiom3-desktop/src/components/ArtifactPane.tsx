// The right-side ARTIFACT pane — Claude Science's center of gravity. A finished session's
// artifact lives here with its own tabs (产物 · 消息 · 审查 · 溯源), a version chip, and
// export — so the conversation stays in the center and the made thing sits beside it,
// carrying its own rigor. Scoped to workflow / coding sessions (the generated artifacts);
// draft-check sessions keep their claim-by-claim flow in the thread.
import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { X, FileDown, FileText, ScanSearch, ChevronLeft, ChevronRight, ChevronDown, Copy, Check, Maximize2, Minimize2, PenLine, CornerDownLeft, Loader2 } from "lucide-react";
import type { Lang } from "../i18n";
import type { Task, RunView } from "../lib/types";
import { ArtifactBody } from "./artifact/ArtifactBody";
import { AgentSteps } from "./AgentSteps";
import { ArtifactProvenance } from "./ArtifactProvenance";
import { ArtifactInspector } from "./ArtifactInspector";
import { buildReportHtml } from "../lib/exportReport";
import { toFindings, renderInlineMd } from "../lib/reviewText";

type Tab = "artifact" | "messages" | "review" | "provenance";

// A friendly name for a live agent's status lamp (research → 研究员, analysis → 分析师, etc.).
function agentLabel(role: string, id: string, zh: boolean): string {
  const r = (role || id).trim();
  const map: Record<string, string> = {
    reframe: zh ? "理解课题" : "Framing",
    coder: zh ? "智能体" : "Agent",
    reviewer: zh ? "审查员" : "Reviewer",
    analyst: zh ? "分析师" : "Analyst",
  };
  return map[r] ?? r;
}

export function ArtifactPane({
  lang,
  task,
  forkedFromTitle,
  liveRun,
  onExport,
  onSaveReport,
  onClose,
  onRevise,
  expanded,
  onToggleExpand,
  busy,
}: {
  lang: Lang;
  task: Task;
  forkedFromTitle?: string;
  liveRun?: RunView; // the in-flight run for this session — drives the live agent-status strip
  onExport: () => void;
  onSaveReport?: (html: string, defaultName: string) => void;
  onClose: () => void;
  onRevise?: (newSynthesis: string, note: string) => void; // splice a surgically-revised passage in as a new version
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
  const [verMenuOpen, setVerMenuOpen] = useState(false);
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
  const reviewFindings = ver.review ? toFindings(ver.review) : [];

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

  // ── Select-to-revise: highlight a passage of the report → give a plain instruction →
  // the model rewrites ONLY that passage and it's spliced back in as a new version. A
  // surgical text edit, not an analysis re-run; it never touches the computed data/figures.
  const [sel, setSel] = useState<{ text: string; x: number; y: number } | null>(null);
  const [annOpen, setAnnOpen] = useState(false);
  const [instr, setInstr] = useState("");
  const [revising, setRevising] = useState(false);
  const [reviseErr, setReviseErr] = useState<string | null>(null);
  const reviseGen = useRef(0); // bumped on dismiss/switch so an in-flight revision can't apply late
  // Only the LATEST version is editable — splicing into an older version and saving it as the new
  // latest would silently discard everything changed in the versions after it.
  const isLatest = vIdx === versions.length - 1;

  function clearSel() {
    reviseGen.current++; // invalidate any in-flight revision (dismiss = cancel)
    setSel(null);
    setAnnOpen(false);
    setInstr("");
    setReviseErr(null);
  }

  // reset tab + any open revise popover whenever a different session is shown
  useEffect(() => { setTab("artifact"); clearSel(); }, [task.id]); // eslint-disable-line react-hooks/exhaustive-deps

  // Locate the selected prose in the RAW markdown source, refusing anything unsafe to splice:
  // returns the byte offset ONLY when the text occurs exactly once OUTSIDE any fenced block
  // (``` figure/code specs). 0 matches → formatting mismatch (rendered ≠ source); >1 → ambiguous.
  // The selection comes from the rendered DOM, so we cannot map it to a source offset directly;
  // requiring a unique out-of-fence match is what keeps us from corrupting a figure spec or
  // editing the wrong occurrence. Returns -1 (none) or -2 (ambiguous) on refusal.
  function uniqueProsePos(full: string, passage: string): number {
    const fences: [number, number][] = [];
    const fre = /```[\s\S]*?```/g;
    let fm: RegExpExecArray | null;
    while ((fm = fre.exec(full))) fences.push([fm.index, fre.lastIndex]);
    const inFence = (i: number) => fences.some(([a, b]) => i >= a && i < b);
    const hits: number[] = [];
    for (let from = 0; ; ) {
      const at = full.indexOf(passage, from);
      if (at < 0) break;
      if (!inFence(at)) hits.push(at);
      from = at + 1;
      if (hits.length > 1) break; // ambiguous — no need to keep scanning
    }
    if (hits.length === 0) return -1;
    if (hits.length > 1) return -2;
    return hits[0];
  }

  function onArtifactMouseUp() {
    if (!onRevise || busy || annOpen || !isLatest) return; // typing / older version → ignore stray mouseups
    // Defer so the browser finalizes the selection first.
    setTimeout(() => {
      const s = window.getSelection();
      const text = s?.toString().trim() ?? "";
      if (!s || s.rangeCount === 0 || text.length < 2) { setSel(null); return; } // a click, not a drag → dismiss
      const range = s.getRangeAt(0);
      // Only react to selections inside the artifact prose.
      if (!artifactRef.current || !artifactRef.current.contains(range.commonAncestorContainer)) return;
      const r = range.getBoundingClientRect();
      setSel({ text, x: Math.round(r.left + r.width / 2), y: Math.round(r.bottom) });
      setReviseErr(null);
    }, 0);
  }

  async function runRevise() {
    if (!sel || !onRevise || !instr.trim()) return;
    const full = ver.synthesis ?? "";
    const passage = sel.text;
    // Resolve the exact splice site up front — fail fast (no model call) if it's unsafe.
    const at = uniqueProsePos(full, passage);
    if (at === -1) {
      setReviseErr(zh ? "无法在原文中定位这段(可能跨越了加粗/标题/表格/图表等格式)。请选一段纯文字。" : "Can't locate this in the source (it likely crosses bold/heading/table/figure markup). Select plain prose.");
      return;
    }
    if (at === -2) {
      setReviseErr(zh ? "这段文字在报告里出现多次,无法确定改哪一处。请多选一些上下文让它唯一。" : "This text appears more than once — select more context so it's unique.");
      return;
    }
    const gen = ++reviseGen.current;
    setRevising(true);
    setReviseErr(null);
    try {
      const revised = await invoke<string>("rewrite_passage", { full, passage, instruction: instr.trim() });
      if (reviseGen.current !== gen) return; // dismissed / session switched while in flight → drop it
      if (revised && revised !== passage) {
        const next = full.slice(0, at) + revised + full.slice(at + passage.length);
        onRevise(next, instr.trim());
        clearSel();
        window.getSelection()?.removeAllRanges();
      } else {
        // model returned no change — keep the popover and tell the user
        setReviseErr(zh ? "模型未作改动。" : "The model made no change.");
      }
    } catch (e) {
      if (reviseGen.current === gen) setReviseErr(String(e));
    } finally {
      if (reviseGen.current === gen) setRevising(false);
    }
  }

  return (
    <aside className={`ax-artpane ${expanded ? "is-expanded" : ""}`}>
      <div className="ax-artpane__head">
        <span className="ax-artpane__name" title={task.title}>{task.title}</span>
        {versions.length > 1 ? (
          <span className="ax-artpane__vsel">
            <button className="ax-artpane__vbtn" disabled={vIdx <= 0} onClick={() => setVIdx((i) => Math.max(0, i - 1))} title={zh ? "上一版" : "previous version"}>
              <ChevronLeft size={13} aria-hidden />
            </button>
            {/* click the chip → a collapsed list of every round's artifact, by index + its topic */}
            <button className="ax-artpane__ver ax-artpane__ver--btn" onClick={() => setVerMenuOpen((o) => !o)} title={zh ? "选择产物版本" : "select version"}>
              v{vIdx + 1}/{versions.length}
              <ChevronDown size={11} aria-hidden />
            </button>
            <button className="ax-artpane__vbtn" disabled={vIdx >= versions.length - 1} onClick={() => setVIdx((i) => Math.min(versions.length - 1, i + 1))} title={zh ? "下一版" : "next version"}>
              <ChevronRight size={13} aria-hidden />
            </button>
            {verMenuOpen && (
              <>
                <div className="ax-artpane__vscrim" onClick={() => setVerMenuOpen(false)} />
                <div className="ax-artpane__vmenu" role="menu">
                  {versions.map((v, i) => (
                    <button
                      key={i}
                      className={`ax-artpane__vitem ${i === vIdx ? "is-active" : ""}`}
                      onClick={() => { setVIdx(i); setVerMenuOpen(false); }}
                    >
                      <span className="ax-artpane__vnum">v{i + 1}</span>
                      <span className="ax-artpane__vgoal">{(v.goal || "").trim() || (zh ? `第 ${i + 1} 版` : `Version ${i + 1}`)}</span>
                    </button>
                  ))}
                </div>
              </>
            )}
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

      {/* live agent-status strip — while the run works, each agent/expert/reviewer as a status lamp
          (Claude-desktop style: green=done, amber pulse=running, red=error). */}
      {liveRun && liveRun.status === "running" && (
        <div className="ax-agentbar">
          <span className="ax-agentbar__phase">{zh ? "执行中" : "Running"}</span>
          {liveRun.agents.map((a) => (
            <span className="ax-agentbar__agent" key={a.id} title={a.model ? `${a.role || a.id} · ${a.model}` : a.role || a.id}>
              <span className={`ax-lamp ${a.status === "ok" ? "is-green" : a.status === "err" ? "is-red" : a.status === "running" ? "is-amber is-running" : "is-neutral"}`} />
              <span className="ax-agentbar__name">{agentLabel(a.role, a.id, zh)}</span>
            </span>
          ))}
        </div>
      )}

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
            <div ref={artifactRef} onMouseUp={onArtifactMouseUp}>
              {ver.synthesis ? <ArtifactBody text={ver.synthesis} sources={task.sources} /> : <p className="ax-hint">{zh ? "无产物内容。" : "No artifact content."}</p>}
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
                <div className="ax-reviewer__finding" key={i} dangerouslySetInnerHTML={{ __html: renderInlineMd(f) }} />
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

      {/* Select-to-revise floating control — anchored to the highlighted passage. The dismiss
          scrim is only mounted once the instruction box is open (so the chip phase still lets
          you freely re-drag a new selection). */}
      {onRevise && sel && tab === "artifact" && isLatest && (
        <>
          {annOpen && <div className="ax-annscrim" onMouseDown={clearSel} />}
          <div
            className="ax-annpop"
            style={{ left: Math.max(12, Math.min(sel.x, window.innerWidth - 190)), top: sel.y + 8 }}
            onMouseDown={(e) => e.stopPropagation()}
          >
            {!annOpen ? (
              <button className="ax-annpop__chip" onMouseDown={(e) => { e.preventDefault(); setAnnOpen(true); }}>
                <PenLine size={13} aria-hidden /> {zh ? "改写这段" : "Revise"}
              </button>
            ) : (
              <div className="ax-annpop__box">
                <div className="ax-annpop__quote">“{sel.text.length > 90 ? sel.text.slice(0, 90) + "…" : sel.text}”</div>
                <div className="ax-annpop__row">
                  <input
                    className="ax-annpop__input"
                    autoFocus
                    placeholder={zh ? "如何改?例如「更简洁」「弱化这个结论」「补一句局限」" : "How? e.g. 'shorten', 'hedge this claim', 'add a caveat'"}
                    value={instr}
                    disabled={revising}
                    onChange={(e) => setInstr(e.target.value)}
                    onKeyDown={(e) => { if (e.key === "Enter" && instr.trim()) runRevise(); if (e.key === "Escape") clearSel(); }}
                  />
                  <button className="ax-annpop__go" disabled={revising || !instr.trim()} onMouseDown={(e) => { e.preventDefault(); runRevise(); }} title={zh ? "改写" : "Revise"}>
                    {revising ? <Loader2 size={14} className="ax-spin" aria-hidden /> : <CornerDownLeft size={14} aria-hidden />}
                  </button>
                </div>
                {reviseErr && <div className="ax-annpop__err">{reviseErr}</div>}
                <div className="ax-annpop__hint">{zh ? "只改写选中的文字,数据与图表不动,生成新版本。" : "Rewrites the selected prose only; data and figures are untouched. Creates a new version."}</div>
              </div>
            )}
          </div>
        </>
      )}
    </aside>
  );
}
