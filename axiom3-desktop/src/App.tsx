import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { Lang, t } from "./i18n";
import "./theme.css";
import type { ProjectStatus, EventInfo, DraftCheck, ExportSummary, Task } from "./lib/types";
import { ClaimDossier } from "./components/ClaimDossier";
import { OnboardingWizard } from "./components/OnboardingWizard";
import { Settings as SettingsModal, applyTheme } from "./components/Settings";
import { ReadingView } from "./components/ReadingView";
import { SearchView } from "./components/SearchView";
import { FleetMonitor } from "./components/FleetMonitor";
import { ShieldView } from "./components/ShieldView";
import { IntegrityView } from "./components/IntegrityView";
import { ProvenanceView } from "./components/ProvenanceView";
import { MetricsView } from "./components/MetricsView";
import { NotificationsView } from "./components/NotificationsView";
import { CommandPalette, type PaletteItem } from "./components/CommandPalette";
import {
  Download, ListFilter, X, Languages, Settings, Upload, ChevronLeft,
  Sparkles, BookOpen, Search, Boxes, ShieldCheck, FileCheck, GitBranch, Gauge,
  Command, Bell, type LucideIcon,
} from "lucide-react";


// The platform's task capabilities — typed after "/" in the composer (like Claude).
// Researched against 2026 AI work platforms and mapped to the four core capabilities:
// rigor/checking (the differentiator), research, multimodal generation, coding — plus a
// dynamic-workflow catch-all. Only check_draft runs today; the rest are visible as 即将,
// so the platform's shape is complete. Every output is meant to be held to the rigor bar.
type SlashCmd = { key: string; cat: string; zh: string; en: string; dzh: string; den: string; active: boolean };
const COMMANDS: SlashCmd[] = [
  // 检查与核验 (rigor — the moat)
  { key: "check_draft", cat: "检查", zh: "校对", en: "check", dzh: "拆成断言,确定性检查 + 证据", den: "claims → deterministic checks + evidence", active: true },
  { key: "verify", cat: "检查", zh: "核查", en: "verify", dzh: "核查单条主张:文献 + 跨模型 + 溯源", den: "verify one claim: literature + cross-model + provenance", active: false },
  { key: "cite", cat: "检查", zh: "查引", en: "cite", dzh: "引文 / DOI / 撤稿核验", den: "citation / DOI / retraction check", active: false },
  // 研究 (research)
  { key: "research", cat: "研究", zh: "调研", en: "research", dzh: "多智能体检索 + 综述,带证据", den: "multi-agent search + synthesis, with evidence", active: false },
  { key: "ask", cat: "研究", zh: "答疑", en: "ask", dzh: "带来源的问答", den: "Q&A grounded in sources", active: false },
  // 写作 (writing)
  { key: "write", cat: "写作", zh: "写作", en: "write", dzh: "起草文稿 / 邮件 / 报告", den: "draft prose / email / report", active: false },
  { key: "polish", cat: "写作", zh: "润色", en: "polish", dzh: "改写 / 精炼 / 语法", den: "rewrite / tighten / grammar", active: false },
  { key: "translate", cat: "写作", zh: "翻译", en: "translate", dzh: "翻译并保术语一致", den: "translate, keep terminology consistent", active: false },
  { key: "summarize", cat: "写作", zh: "摘要", en: "summarize", dzh: "提炼要点", den: "distil key points", active: false },
  // 多模态生成 (multimodal generation)
  { key: "ppt", cat: "生成", zh: "PPT", en: "slides", dzh: "多模态:文本 + 图片 agent 协同", den: "multimodal: text + image agents", active: false },
  { key: "doc", cat: "生成", zh: "文档", en: "doc", dzh: "生成结构化文档 / 报告", den: "structured document / report", active: false },
  { key: "chart", cat: "生成", zh: "图表", en: "chart", dzh: "数据 → 图表", den: "data → chart", active: false },
  { key: "image", cat: "生成", zh: "图片", en: "image", dzh: "文生图", den: "text-to-image", active: false },
  // 数据与多模态处理 (data / multimodal processing)
  { key: "analyze", cat: "数据", zh: "分析", en: "analyze", dzh: "数据分析:统计 + 可视化", den: "data analysis: stats + viz", active: false },
  { key: "extract", cat: "数据", zh: "解析", en: "extract", dzh: "文档解析:图片 / 表格 / 公式提取", den: "parse a doc: images / tables / formulas", active: false },
  { key: "transcribe", cat: "数据", zh: "转写", en: "transcribe", dzh: "音频转写", den: "audio transcription", active: false },
  // 编程 (coding)
  { key: "code", cat: "编程", zh: "编程", en: "code", dzh: "写 + 编译 + 测试(自带 in-loop 校验)", den: "write + compile + test (in-loop checks)", active: false },
  { key: "debug", cat: "编程", zh: "调试", en: "debug", dzh: "定位 + 修复", den: "locate + fix", active: false },
  { key: "review", cat: "编程", zh: "审查", en: "review", dzh: "代码审查", den: "code review", active: false },
  // 编排 (orchestration)
  { key: "workflow", cat: "编排", zh: "工作流", en: "workflow", dzh: "自定义动态多智能体工作流(任意任务)", den: "custom dynamic multi-agent workflow (any task)", active: false },
];

// Sidebar navigation — the platform's surfaces, grouped 工作 / 信任·证据 / 概览. Each routes
// to a self-contained surface component. Notifications is reached from the topbar bell.
type NavKey =
  | "tasks" | "reading" | "search" | "fleet"
  | "shield" | "integrity" | "provenance"
  | "metrics" | "notifications";
type NavGroup = "work" | "trust" | "overview";
const NAV: { key: NavKey; zh: string; en: string; icon: LucideIcon; group: NavGroup }[] = [
  { key: "tasks", zh: "任务", en: "Tasks", icon: Sparkles, group: "work" },
  { key: "reading", zh: "阅读", en: "Reading", icon: BookOpen, group: "work" },
  { key: "search", zh: "检索", en: "Search", icon: Search, group: "work" },
  { key: "fleet", zh: "舰队", en: "Fleet", icon: Boxes, group: "work" },
  { key: "shield", zh: "护盾", en: "Shield", icon: ShieldCheck, group: "trust" },
  { key: "integrity", zh: "完整性", en: "Integrity", icon: FileCheck, group: "trust" },
  { key: "provenance", zh: "溯源", en: "Provenance", icon: GitBranch, group: "trust" },
  { key: "metrics", zh: "概览", en: "Overview", icon: Gauge, group: "overview" },
];
const NAV_GROUPS: { key: NavGroup; zh: string; en: string }[] = [
  { key: "work", zh: "工作", en: "Work" },
  { key: "trust", zh: "信任 · 证据", en: "Trust · Evidence" },
  { key: "overview", zh: "概览", en: "Overview" },
];

function App() {
  const [lang, setLangState] = useState<Lang>(
    (localStorage.getItem("axiom3.lang") as Lang) || "zh"
  );
  function setLang(l: Lang) {
    setLangState(l);
    localStorage.setItem("axiom3.lang", l);
  }

  const [name, setName] = useState("");
  const [status, setStatus] = useState<ProjectStatus | null>(null);
  const [actor, setActor] = useState("researcher");
  const [op, setOp] = useState("note");
  const [payload, setPayload] = useState('{"text":"hello"}');
  const [log, setLog] = useState<string[]>([]);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [draft, setDraft] = useState(
    "Water boils at 100 degrees Celsius at sea level. Therefore the treatment causes recovery."
  );
  const [references, setReferences] = useState("");
  const [tasks, setTasks] = useState<Task[]>([]);
  const [view, setView] = useState<"board" | "detail">("board");
  const [activeId, setActiveId] = useState<string | null>(null);
  const [cmd, setCmd] = useState<string | null>(null);
  const [slash, setSlash] = useState("");
  const [notableOnly, setNotableOnly] = useState(true);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [onboarded, setOnboarded] = useState(localStorage.getItem("axiom3.onboarded") === "1");
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [nav, setNav] = useState<NavKey>("tasks");
  const [paletteOpen, setPaletteOpen] = useState(false);

  useEffect(() => {
    applyTheme((localStorage.getItem("axiom3.theme") as "light" | "dark" | "system") || "system");
  }, []);

  // ⌘K / Ctrl-K opens the command palette anywhere.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && (e.key === "k" || e.key === "K")) {
        e.preventDefault();
        setPaletteOpen((o) => !o);
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  useEffect(() => {
    const un = listen<{ done: number; total: number }>("check_progress", (e) => setProgress(e.payload));
    return () => {
      un.then((f) => f());
    };
  }, []);

  function note(line: string) {
    setLog((l) => [line, ...l].slice(0, 30));
  }

  async function run<T>(fn: () => Promise<T>): Promise<T | undefined> {
    setBusy(true);
    setError("");
    try {
      return await fn();
    } catch (e) {
      setError(String(e));
      return undefined;
    } finally {
      setBusy(false);
    }
  }

  async function openProject() {
    const s = await run(() => invoke<ProjectStatus>("open_project", { name }));
    if (s) {
      setStatus(s);
      setTasks([]);
      setView("board");
      setActiveId(null);
      note(`${t(lang, "opened_project")} "${s.project_id}" · ${s.event_count} ${t(lang, "events")}`);
    }
  }

  async function appendEvent() {
    const e = await run(() => invoke<EventInfo>("append_event", { actor, op, payload }));
    if (e) {
      note(`#${e.seq} ${e.actor}/${e.op} → ${e.entry_hash.slice(0, 12)}…`);
      const s = await run(() => invoke<ProjectStatus>("project_status"));
      if (s) setStatus(s);
    }
  }

  async function verifyChain() {
    const ok = await run(() => invoke<boolean>("verify_chain"));
    if (ok !== undefined)
      note(`${t(lang, "verify_chain")} → ${ok ? t(lang, "chain_intact") : t(lang, "chain_broken")}`);
  }

  async function vectorSelfTest() {
    const r = await run(() => invoke<string>("vector_self_test"));
    if (r) note(`${t(lang, "vector_self_test")} → ${r}`);
  }

  async function importInto(setter: (s: string) => void) {
    const path = await open({
      multiple: false,
      filters: [
        { name: "Documents", extensions: ["pdf", "docx", "txt", "md", "markdown"] },
      ],
    });
    if (typeof path === "string") {
      const text = await run(() => invoke<string>("import_document", { path }));
      if (text !== undefined) {
        setter(text);
        note(
          `${t(lang, "imported")}: ${path.split(/[/\\]/).pop()} · ${text.length.toLocaleString()} ${t(lang, "chars")}`
        );
      }
    }
  }

  async function runCheckDraft() {
    const id = `t${Date.now()}`;
    const trimmed = draft.trim();
    const title = (trimmed.slice(0, 42) || t(lang, "tt_check_draft")) + (trimmed.length > 42 ? "…" : "");
    setTasks((ts) => [{ id, type: "check_draft", title, status: "running", ts: Date.now() }, ...ts]);
    setActiveId(id);
    setView("detail");
    setCmd(null);
    setSlash("");
    setProgress(null);
    const litsearch = localStorage.getItem("axiom3.lit") !== "off";
    const fmRaw = parseInt(localStorage.getItem("axiom3.fleetmax") || "", 10);
    const fleetmax = Number.isFinite(fmRaw) && fmRaw > 0 ? fmRaw : null;
    const c = await run(() => invoke<DraftCheck>("check_draft", { draft, references, litsearch, fleetmax }));
    setProgress(null);
    setTasks((ts) =>
      ts.map((x) =>
        x.id === id
          ? c
            ? { ...x, status: "done", result: c }
            : { ...x, status: "failed", error: "failed" }
          : x
      )
    );
    if (c) {
      note(
        `${t(lang, "checked_draft")} → ${c.total} ${t(lang, "claims")} · ${c.flagged} ${t(lang, "flagged")} · ${c.fleet_verified} ${t(lang, "cross_vendor_n")}`
      );
      const s = await run(() => invoke<ProjectStatus>("project_status"));
      if (s) setStatus(s);
    }
  }

  async function cancelCheck() {
    await invoke("cancel_check");
  }

  async function exportProvenance() {
    const path = await save({
      defaultPath: `${name.trim() || "axiom3"}-provenance.json`,
      filters: [{ name: "JSON", extensions: ["json"] }],
    });
    if (typeof path === "string") {
      const r = await run(() => invoke<ExportSummary>("export_provenance", { path }));
      if (r) {
        note(
          `${t(lang, "exported")}: ${r.path.split(/[/\\]/).pop()} · ${r.claims} ${t(lang, "claims")} · ${t(lang, r.chain_intact ? "chain_intact" : "chain_broken")}`
        );
      }
    }
  }

  async function closeProject() {
    await run(() => invoke("close_project"));
    setStatus(null);
    setTasks([]);
    setView("board");
    setActiveId(null);
    note(t(lang, "closed_project"));
  }

  // A clean claim (no deterministic flag, no fleet verdict) is "unchecked" — shown
  // neutral, NOT as an alarming "uncertain". Only flagged / verdicted claims stand out.

  const paletteItems: PaletteItem[] = [
    ...NAV.map((n) => ({ key: n.key, label: lang === "zh" ? n.zh : n.en, kind: "nav" as const })),
    ...COMMANDS.map((c) => ({
      key: c.key,
      label: `/${lang === "zh" ? c.zh : c.en}`,
      sub: lang === "zh" ? c.dzh : c.den,
      kind: "cmd" as const,
      disabled: !c.active,
    })),
  ];

  function onPalettePick(it: PaletteItem) {
    setPaletteOpen(false);
    if (it.kind === "nav") {
      setNav(it.key as NavKey);
    } else {
      setNav("tasks");
      setView("board");
      setCmd(it.key);
    }
  }

  return (
    <>
      {!onboarded && <OnboardingWizard lang={lang} onDone={() => setOnboarded(true)} />}
      {settingsOpen && <SettingsModal lang={lang} onLang={setLang} onClose={() => setSettingsOpen(false)} />}
      {paletteOpen && (
        <CommandPalette lang={lang} items={paletteItems} onPick={onPalettePick} onClose={() => setPaletteOpen(false)} />
      )}
      <div className="ax-app">
      <aside className="ax-sidebar">
        <div className="ax-brand">
          Axiom3
          <small>{t(lang, "app_subtitle_short")}</small>
        </div>

        <div>
          <p className="ax-side-label">{t(lang, "project")}</p>
          <input
            className="ax-input"
            value={name}
            onChange={(e) => setName(e.currentTarget.value)}
            placeholder={t(lang, "project_placeholder")}
          />
          <div className="ax-toolrow">
            <button className="ax-btn" onClick={openProject} disabled={busy || !name.trim()}>
              {t(lang, "open_create")}
            </button>
            {status && (
              <button
                className="ax-iconbtn"
                onClick={closeProject}
                disabled={busy}
                title={t(lang, "close")}
                aria-label={t(lang, "close")}
              >
                <X size={16} aria-hidden />
              </button>
            )}
          </div>
          {status && (
            <div className="ax-proj">
              <span className={`ax-lamp ${status.chain_ok ? "is-green" : "is-amber"}`} />
              <span className="ax-proj__name">{status.project_id}</span>
            </div>
          )}
        </div>

        <nav className="ax-nav">
          {NAV_GROUPS.map((g) => (
            <div key={g.key}>
              <p className="ax-nav__label">{lang === "zh" ? g.zh : g.en}</p>
              {NAV.filter((n) => n.group === g.key).map((n) => (
                <button
                  key={n.key}
                  className={`ax-nav__item ${nav === n.key ? "is-active" : ""}`}
                  onClick={() => setNav(n.key)}
                >
                  <n.icon size={16} aria-hidden />
                  {lang === "zh" ? n.zh : n.en}
                  {n.key === "tasks" && tasks.length > 0 && <span className="ax-nav__badge">{tasks.length}</span>}
                </button>
              ))}
            </div>
          ))}
        </nav>

        <div className="ax-side-spacer" />

        <div className="ax-side-foot">
          <button
            className="ax-iconbtn"
            onClick={() => setLang(lang === "zh" ? "en" : "zh")}
            title={lang === "zh" ? "English" : "中文"}
            aria-label={t(lang, "switch_language")}
          >
            <Languages size={16} aria-hidden />
            <span style={{ fontSize: 12 }}>{lang === "zh" ? "EN" : "中"}</span>
          </button>
          <button className="ax-iconbtn" onClick={() => setSettingsOpen(true)} title={t(lang, "settings")} aria-label={t(lang, "settings")}>
            <Settings size={16} aria-hidden />
          </button>
        </div>
      </aside>

      <main className="ax-main">
        <header className="ax-topbar">
          <div className="ax-crumb">
            {status ? (
              <>
                <span className={`ax-lamp ${status.chain_ok ? "is-green" : "is-amber"}`} />
                {status.project_id}
                <span className="ax-hint">
                  · {t(lang, "events")} {status.event_count}
                </span>
              </>
            ) : (
              <span className="ax-hint">{t(lang, "app_title")}</span>
            )}
          </div>
          <div className="ax-topbar__actions">
            <button
              className="ax-iconbtn"
              onClick={() => setPaletteOpen(true)}
              title={lang === "zh" ? "命令面板" : "Command palette"}
              aria-label={lang === "zh" ? "命令面板" : "Command palette"}
            >
              <Command size={15} aria-hidden />
              <span className="ax-kbd">⌘K</span>
            </button>
            <button
              className={`ax-iconbtn ${nav === "notifications" ? "is-active" : ""}`}
              onClick={() => setNav("notifications")}
              title={lang === "zh" ? "通知" : "Notifications"}
              aria-label={lang === "zh" ? "通知" : "Notifications"}
            >
              <Bell size={16} aria-hidden />
            </button>
          </div>
        </header>

        <div className="ax-content">
          {error && <p className="ax-error">⚠ {error}</p>}

          {nav !== "tasks" ? (
            <div className="ax-canvas">
              {nav === "reading" && <ReadingView lang={lang} />}
              {nav === "search" && <SearchView lang={lang} />}
              {nav === "fleet" && <FleetMonitor lang={lang} />}
              {nav === "shield" && <ShieldView lang={lang} />}
              {nav === "integrity" && <IntegrityView lang={lang} />}
              {nav === "provenance" && <ProvenanceView lang={lang} />}
              {nav === "metrics" && <MetricsView lang={lang} />}
              {nav === "notifications" && <NotificationsView lang={lang} />}
            </div>
          ) : !status ? (
            <div className="ax-empty">
              <p className="ax-empty__title">{t(lang, "empty_title")}</p>
              <p className="ax-empty__sub">{t(lang, "empty_sub")}</p>
            </div>
          ) : (
            <div className="ax-canvas">
              {view === "board" && (
                <>
                  {tasks.length > 0 && (
                    <div className="ax-results__list" style={{ marginBottom: 18 }}>
                      {tasks.map((tk) => (
                        <button
                          key={tk.id}
                          className="ax-task"
                          onClick={() => {
                            setActiveId(tk.id);
                            setView("detail");
                          }}
                        >
                          <span
                            className={`ax-lamp ${
                              tk.status === "done" ? "is-green" : tk.status === "running" ? "is-amber" : "is-neutral"
                            }`}
                          />
                          <span className="ax-task__title">{tk.title}</span>
                          <span className="ax-task__meta">
                            /{t(lang, "cmd_check")} · {t(lang, `st_${tk.status}`)}
                            {tk.result ? ` · ${tk.result.total} ${t(lang, "claims")}` : ""}
                          </span>
                        </button>
                      ))}
                    </div>
                  )}

                  <div className="ax-composer">
                    {cmd === null ? (
                      <>
                        <textarea
                          className="ax-textarea"
                          value={slash}
                          onChange={(e) => setSlash(e.currentTarget.value)}
                          rows={3}
                          placeholder={t(lang, "composer_ph")}
                        />
                        {slash.trimStart().startsWith("/") && (
                          <div className="ax-slash-menu">
                            {COMMANDS.filter((c) => {
                              const q = slash.trimStart().slice(1).toLowerCase();
                              return q === "" || (lang === "zh" ? c.zh : c.en).toLowerCase().includes(q) || c.cat.includes(q);
                            }).map((c) => (
                              <button
                                key={c.key}
                                className="ax-slash-item"
                                disabled={!c.active}
                                onClick={() => {
                                  setCmd(c.key);
                                  setSlash("");
                                }}
                              >
                                <span className="ax-slash-cmd">/{lang === "zh" ? c.zh : c.en}</span>
                                <span className="ax-slash-desc">
                                  {lang === "zh" ? c.dzh : c.den}
                                  {!c.active ? ` · ${t(lang, "coming_soon")}` : ""}
                                </span>
                              </button>
                            ))}
                          </div>
                        )}
                      </>
                    ) : (
                      <>
                        <div className="ax-composer__chip">
                          /{(() => { const c = COMMANDS.find((x) => x.key === cmd); return c ? (lang === "zh" ? c.zh : c.en) : ""; })()}
                          <button className="ax-iconbtn" onClick={() => setCmd(null)} title={t(lang, "close")} aria-label={t(lang, "close")}>
                            <X size={13} aria-hidden />
                          </button>
                        </div>
                        {cmd === "check_draft" ? (
                          <>
                            <textarea className="ax-textarea" value={draft} onChange={(e) => setDraft(e.currentTarget.value)} rows={9} placeholder={t(lang, "check_placeholder")} />
                            <div className="ax-toolrow">
                              <button className="ax-btn ax-btn--primary" onClick={runCheckDraft} disabled={busy || !draft.trim()}>
                                {t(lang, "task_run")}
                              </button>
                              <button className="ax-iconbtn" onClick={() => importInto(setDraft)} disabled={busy} title={t(lang, "import_btn")} aria-label={t(lang, "import_btn")}>
                                <Upload size={16} aria-hidden />
                              </button>
                              <span className="ax-hint">{draft.length.toLocaleString()} {t(lang, "chars")}</span>
                            </div>
                            <details style={{ marginTop: 12 }}>
                              <summary className="ax-block-label" style={{ cursor: "pointer" }}>{t(lang, "references_title")}</summary>
                              <textarea className="ax-textarea" style={{ marginTop: 6 }} value={references} onChange={(e) => setReferences(e.currentTarget.value)} rows={6} placeholder={t(lang, "references_placeholder")} />
                              <div className="ax-toolrow">
                                <button className="ax-iconbtn" onClick={() => importInto(setReferences)} disabled={busy} title={t(lang, "import_refs")} aria-label={t(lang, "import_refs")}>
                                  <Upload size={16} aria-hidden />
                                </button>
                                <span className="ax-hint">{references.length.toLocaleString()} {t(lang, "chars")}</span>
                              </div>
                            </details>
                          </>
                        ) : (
                          <p className="ax-hint">{t(lang, "soon_note")}</p>
                        )}
                      </>
                    )}
                  </div>

                  <details className="ax-dev">
                    <summary>{t(lang, "dev_tools")}</summary>
                    <div style={{ marginTop: 10, display: "grid", gap: 6 }}>
                      <div className="ax-toolrow">
                        <input className="ax-input" style={{ width: 120 }} value={actor} onChange={(e) => setActor(e.currentTarget.value)} placeholder={t(lang, "actor")} />
                        <input className="ax-input" style={{ width: 120 }} value={op} onChange={(e) => setOp(e.currentTarget.value)} placeholder={t(lang, "op")} />
                        <input className="ax-input" style={{ flex: 1, minWidth: 160 }} value={payload} onChange={(e) => setPayload(e.currentTarget.value)} placeholder={t(lang, "payload")} />
                      </div>
                      <div className="ax-toolrow">
                        <button className="ax-btn" onClick={appendEvent} disabled={busy}>{t(lang, "append_event")}</button>
                        <button className="ax-btn" onClick={verifyChain} disabled={busy}>{t(lang, "verify_chain")}</button>
                        <button className="ax-btn" onClick={vectorSelfTest} disabled={busy}>{t(lang, "vector_self_test")}</button>
                      </div>
                      {status && <div className="ax-hint">{status.root}</div>}
                      {log.length > 0 && (
                        <ul className="ax-log">
                          {log.map((l, i) => (
                            <li key={i}>{l}</li>
                          ))}
                        </ul>
                      )}
                    </div>
                  </details>
                </>
              )}

              {view === "detail" &&
                (() => {
                  const tk = tasks.find((x) => x.id === activeId);
                  if (!tk) return null;
                  return (
                    <>
                      <div className="ax-results__head">
                        <span style={{ display: "flex", alignItems: "center", gap: 8 }}>
                          <button className="ax-iconbtn" onClick={() => setView("board")} title={t(lang, "back")} aria-label={t(lang, "back")}>
                            <ChevronLeft size={18} aria-hidden />
                          </button>
                          <span className="ax-block-label" style={{ margin: 0 }}>{tk.title}</span>
                        </span>
                        {tk.result && (
                          <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
                            <button className="ax-iconbtn" onClick={exportProvenance} disabled={busy} title={t(lang, "export_provenance")} aria-label={t(lang, "export_provenance")}>
                              <Download size={16} aria-hidden />
                            </button>
                            <button className="ax-iconbtn" onClick={() => setNotableOnly(!notableOnly)} title={notableOnly ? t(lang, "show_all") : t(lang, "only_notable")} aria-label={notableOnly ? t(lang, "show_all") : t(lang, "only_notable")}>
                              <ListFilter size={16} aria-hidden />
                            </button>
                          </div>
                        )}
                      </div>

                      {busy && tk.status === "running" && (
                        <div className="ax-progress">
                          <span>
                            {t(lang, "checking")}
                            {progress ? ` ${progress.done}/${progress.total}` : ""}
                          </span>
                          {progress && progress.total > 0 && (
                            <span className="ax-progress__bar">
                              <span className="ax-progress__fill" style={{ width: `${Math.round((progress.done / progress.total) * 100)}%` }} />
                            </span>
                          )}
                          <button className="ax-iconbtn" onClick={cancelCheck} title={t(lang, "cancel")} aria-label={t(lang, "cancel")}>
                            <X size={14} aria-hidden />
                          </button>
                        </div>
                      )}

                      {tk.result && (
                        <>
                          <span className="ax-results__meta">
                            {tk.result.total} {t(lang, "claims")} · {tk.result.flagged} {t(lang, "flagged")} ·{" "}
                            {tk.result.fleet_verified} {t(lang, "cross_vendor_n")}
                          </span>
                          <p className="ax-philosophy">{t(lang, "results_philosophy")}</p>
                          <div className="ax-results__list" style={{ marginTop: 10 }}>
                            {tk.result.entries
                              .filter(
                                (e) =>
                                  !notableOnly ||
                                  e.findings.length > 0 ||
                                  e.fleet ||
                                  e.status === "injection_neutralized" ||
                                  e.reason?.includes("fleet_cap_exceeded") ||
                                  e.reason?.includes("cross_vendor_unavailable")
                              )
                              .map((e) => (
                                <ClaimDossier key={e.claim_id} entry={e} lang={lang} />
                              ))}
                          </div>
                        </>
                      )}
                    </>
                  );
                })()}
            </div>
          )}
        </div>
      </main>
      </div>
    </>
  );
}

export default App;
