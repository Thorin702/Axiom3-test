import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { Lang, t } from "./i18n";
import "./theme.css";
import type { ProjectStatus, EventInfo, DraftCheck, ExportSummary, Task, Source } from "./lib/types";
import { ClaimDossier } from "./components/ClaimDossier";
import { OnboardingWizard } from "./components/OnboardingWizard";
import { Settings as SettingsModal, applyTheme } from "./components/Settings";
import { ReadingView } from "./components/ReadingView";
import { ProjectsView } from "./components/ProjectsView";
import { EvidenceView } from "./components/EvidenceView";
import { SearchView } from "./components/SearchView";
import { FleetMonitor } from "./components/FleetMonitor";
import { ShieldView } from "./components/ShieldView";
import { IntegrityView } from "./components/IntegrityView";
import { ProvenanceView } from "./components/ProvenanceView";
import { MetricsView } from "./components/MetricsView";
import { NotificationsView } from "./components/NotificationsView";
import { CommandPalette, type PaletteItem } from "./components/CommandPalette";
import { ActivityRail } from "./components/ActivityRail";
import { Splash } from "./components/Splash";
import { StatusBar } from "./components/StatusBar";
import { AppBanner } from "./components/AppBanner";
import {
  Download, ListFilter, X, Languages, Settings, Upload, ChevronLeft, Plus, ArrowUp,
  Sparkles, BookOpen, Search, Boxes, ShieldCheck, FileCheck, GitBranch, Gauge,
  Command, Bell, PanelRight, FolderOpen, Quote, RotateCcw, WrapText, Check, type LucideIcon,
} from "lucide-react";

// Set during the tasks initializer (first render): how many persisted tasks were left
// "running" when the app last closed — i.e. interrupted by an abnormal exit (#9).
let bootInterrupted = 0;


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
  { key: "code", cat: "编程", zh: "编程", en: "code", dzh: "只读分析:读取 / 搜索 / 提议(写入即将)", den: "read-only: read / search / propose (writes soon)", active: true },
  { key: "debug", cat: "编程", zh: "调试", en: "debug", dzh: "定位 + 修复", den: "locate + fix", active: false },
  { key: "review", cat: "编程", zh: "审查", en: "review", dzh: "代码审查", den: "code review", active: false },
  // 编排 (orchestration)
  { key: "workflow", cat: "编排", zh: "工作流", en: "workflow", dzh: "自定义动态多智能体工作流(任意任务)", den: "custom dynamic multi-agent workflow (any task)", active: true },
];

// Sidebar navigation — the platform's surfaces, grouped 工作 / 信任·证据 / 概览. Each routes
// to a self-contained surface component. Notifications is reached from the topbar bell.
type NavKey =
  | "tasks" | "projects" | "reading" | "search" | "fleet"
  | "shield" | "integrity" | "provenance" | "evidence"
  | "metrics" | "notifications";
type NavGroup = "work" | "trust" | "overview";
const NAV: { key: NavKey; zh: string; en: string; icon: LucideIcon; group: NavGroup }[] = [
  { key: "tasks", zh: "任务", en: "Tasks", icon: Sparkles, group: "work" },
  { key: "projects", zh: "项目", en: "Projects", icon: FolderOpen, group: "work" },
  { key: "reading", zh: "阅读", en: "Reading", icon: BookOpen, group: "work" },
  { key: "search", zh: "检索", en: "Search", icon: Search, group: "work" },
  { key: "fleet", zh: "舰队", en: "Fleet", icon: Boxes, group: "work" },
  { key: "shield", zh: "护盾", en: "Shield", icon: ShieldCheck, group: "trust" },
  { key: "integrity", zh: "完整性", en: "Integrity", icon: FileCheck, group: "trust" },
  { key: "provenance", zh: "溯源", en: "Provenance", icon: GitBranch, group: "trust" },
  { key: "evidence", zh: "证据", en: "Evidence", icon: Quote, group: "trust" },
  { key: "metrics", zh: "概览", en: "Overview", icon: Gauge, group: "overview" },
];
// Model choices offered in the task control bar (the connected provider is DeepSeek).
// "" = auto (the app's default per provider). deepseek-reasoner emits a real chain-of-thought.
const MODELS: { id: string; label: string }[] = [
  { id: "", label: "自动 · Auto" },
  { id: "deepseek-chat", label: "DeepSeek Chat" },
  { id: "deepseek-reasoner", label: "DeepSeek Reasoner" },
  { id: "deepseek-v4-flash", label: "DeepSeek V4 Flash" },
];
// Target agent count for /工作流 (0 = auto). >8 triggers deep two-level orchestration
// (top × sub), so tens of agents queue through the pool.
const AGENT_CHOICES = [0, 2, 4, 8, 12, 16];

// Tidy pasted/imported text (#113): de-hyphenate line-broken words and rejoin hard
// single line breaks (PDF copy artifacts), while keeping paragraph breaks. CJK joins
// without a space; Latin joins with one.
function tidyDraft(s: string): string {
  let out = s.replace(/([A-Za-zÀ-ɏ])-\n([A-Za-z])/g, "$1$2");
  out = out.replace(/([^\n])\n(?!\n)([^\n])/g, (_m, a: string, b: string) => {
    const cjk = /[一-鿿　-〿＀-￯]/;
    return cjk.test(a) && cjk.test(b) ? a + b : `${a} ${b}`;
  });
  return out.replace(/[ \t]{2,}/g, " ").replace(/\n{3,}/g, "\n\n").trim();
}

function App() {
  const [lang, setLangState] = useState<Lang>(
    (localStorage.getItem("axiom3.lang") as Lang) || "zh"
  );
  function setLang(l: Lang) {
    setLangState(l);
    localStorage.setItem("axiom3.lang", l);
  }

  const [status, setStatus] = useState<ProjectStatus | null>(null);
  const [actor, setActor] = useState("researcher");
  const [op, setOp] = useState("note");
  const [payload, setPayload] = useState('{"text":"hello"}');
  const [log, setLog] = useState<string[]>([]);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [draft, setDraft] = useState(
    () => localStorage.getItem("axiom3.draft") ?? "Water boils at 100 degrees Celsius at sea level. Therefore the treatment causes recovery."
  );
  const [references, setReferences] = useState("");
  const [codeWs, setCodeWs] = useState<string | null>(null);
  const [codeWritable, setCodeWritable] = useState(false);
  const [sources, setSources] = useState<Source[]>(() => {
    try {
      return JSON.parse(localStorage.getItem("axiom3.sources") || "[]");
    } catch {
      return [];
    }
  });
  const [taskModel, setTaskModel] = useState(""); // "" = auto
  const [wfAgents, setWfAgents] = useState(0); // 0 = auto
  const [tasks, setTasks] = useState<Task[]>(() => {
    // Task history persists like Claude's chat list. A task left "running" when the app
    // closed can't still be running, so normalize it to cancelled on load.
    try {
      const raw: Task[] = JSON.parse(localStorage.getItem("axiom3.tasks") || "[]");
      bootInterrupted = raw.filter((tk) => tk.status === "running").length;
      return raw.map((tk) => (tk.status === "running" ? { ...tk, status: "cancelled" } : tk));
    } catch {
      return [];
    }
  });
  const [interrupted, setInterrupted] = useState(bootInterrupted);
  const [view, setView] = useState<"home" | "thread">("home");
  const [activeId, setActiveId] = useState<string | null>(null);
  const [taskFilter, setTaskFilter] = useState<"all" | "check_draft" | "workflow" | "code">("all");
  const [cmd, setCmd] = useState<string | null>(null);
  const [slash, setSlash] = useState("");
  const [slashIdx, setSlashIdx] = useState(0);
  const [notableOnly, setNotableOnly] = useState(true);
  const [reviewed, setReviewed] = useState<Record<string, boolean>>(() => {
    try {
      return JSON.parse(localStorage.getItem("axiom3.reviewed") || "{}");
    } catch {
      return {};
    }
  });
  const [hideReviewed, setHideReviewed] = useState(false);
  function toggleReviewed(id: string) {
    setReviewed((r) => {
      const n = { ...r, [id]: !r[id] };
      try {
        localStorage.setItem("axiom3.reviewed", JSON.stringify(n));
      } catch {
        /* quota */
      }
      return n;
    });
  }
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [onboarded, setOnboarded] = useState(localStorage.getItem("axiom3.onboarded") === "1");
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [nav, setNav] = useState<NavKey>("tasks");
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [railOpen, setRailOpen] = useState(localStorage.getItem("axiom3.rail") === "1");
  const [booting, setBooting] = useState(true);
  const [dropping, setDropping] = useState(false);

  useEffect(() => {
    applyTheme((localStorage.getItem("axiom3.theme") as "light" | "dark" | "system") || "system");
  }, []);

  // Persist the task history (cap to the most recent 50).
  useEffect(() => {
    try {
      localStorage.setItem("axiom3.tasks", JSON.stringify(tasks.slice(0, 50)));
    } catch {
      /* quota — ignore */
    }
  }, [tasks]);

  // Auto-save the draft so a paste/edit survives reloads and crashes (#109).
  useEffect(() => {
    try {
      localStorage.setItem("axiom3.draft", draft);
    } catch {
      /* quota — ignore */
    }
  }, [draft]);

  // Persist the source library (cap to the most recent 12 to bound localStorage).
  useEffect(() => {
    try {
      localStorage.setItem("axiom3.sources", JSON.stringify(sources.slice(0, 12)));
    } catch {
      /* quota — ignore */
    }
  }, [sources]);

  // Clear any error banner whenever the user navigates (a run sets its error after the
  // view change, so a fresh failure still shows; only stale errors get wiped).
  useEffect(() => {
    setError("");
  }, [nav, view, activeId]);

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

  // Drag a document anywhere onto the window → import it into the composer (#6).
  useEffect(() => {
    let un: (() => void) | undefined;
    let alive = true;
    import("@tauri-apps/api/webview").then(({ getCurrentWebview }) => {
      getCurrentWebview()
        .onDragDropEvent((e) => {
          const ty = e.payload.type;
          if (ty === "enter" || ty === "over") setDropping(true);
          else if (ty === "leave") setDropping(false);
          else if (ty === "drop") {
            setDropping(false);
            const p = (e.payload as { paths?: string[] }).paths?.[0];
            if (p) importDropped(p);
          }
        })
        .then((f) => (alive ? (un = f) : f()));
    });
    return () => {
      alive = false;
      if (un) un();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Native macOS menu → app actions (#42).
  useEffect(() => {
    const un = listen<string>("menu", (e) => {
      const a = e.payload;
      if (a === "new-task") newTask();
      else if (a === "new-workflow") {
        newTask();
        setCmd("workflow");
      } else if (a === "settings") setSettingsOpen(true);
      else if (a === "toggle-rail")
        setRailOpen((o) => {
          const n = !o;
          localStorage.setItem("axiom3.rail", n ? "1" : "0");
          return n;
        });
    });
    return () => {
      un.then((f) => f());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
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

  // Auto-open a default workspace so the user can just start a task (no "open a project"
  // step, like Claude). A named project is still possible later (a Projects feature).
  async function ensureProject(): Promise<ProjectStatus | null> {
    if (status) return status;
    try {
      const s = await invoke<ProjectStatus>("open_project", { name: lang === "zh" ? "工作区" : "Workspace" });
      setStatus(s);
      note(`${t(lang, "opened_project")} "${s.project_id}" · ${s.event_count} ${t(lang, "events")}`);
      return s;
    } catch (e) {
      setError(String(e));
      return null;
    }
  }

  // Remove a task from the history (localStorage persists via the effect).
  function removeTask(id: string) {
    setTasks((ts) => ts.filter((tk) => tk.id !== id));
    if (activeId === id) {
      setActiveId(null);
      setView("home");
    }
  }

  // Start a fresh task: clear the composer and show the home greeting.
  function newTask() {
    setNav("tasks");
    setView("home");
    setActiveId(null);
    setCmd(null);
    setSlash("");
    setDraft("");
    setReferences("");
  }

  // Time-of-day greeting for the home screen.
  function greeting(): string {
    const h = new Date().getHours();
    if (lang === "zh") return h < 5 ? "夜深了" : h < 11 ? "早上好" : h < 13 ? "中午好" : h < 18 ? "下午好" : "晚上好";
    return h < 5 ? "Still up?" : h < 12 ? "Good morning" : h < 18 ? "Good afternoon" : "Good evening";
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
        recordSource(path.split(/[/\\]/).pop() || "document", text);
        note(
          `${t(lang, "imported")}: ${path.split(/[/\\]/).pop()} · ${text.length.toLocaleString()} ${t(lang, "chars")}`
        );
      }
    }
  }

  // Import a file dropped onto the window: extract text, record it, open /校对 with it.
  async function importDropped(path: string) {
    const text = await run(() => invoke<string>("import_document", { path }));
    if (text !== undefined) {
      recordSource(path.split(/[/\\]/).pop() || "document", text);
      setNav("tasks");
      setActiveId(null);
      setView("home");
      setCmd("check_draft");
      setDraft(text);
      note(`${t(lang, "imported")}: ${path.split(/[/\\]/).pop()} · ${text.length.toLocaleString()} ${t(lang, "chars")}`);
    }
  }

  // ── source library (阅读/摄入) ──
  function recordSource(name: string, text: string) {
    const src: Source = { id: `s${Date.now()}`, name, chars: text.length, ts: Date.now(), text };
    setSources((ss) => [src, ...ss.filter((x) => x.name !== name)].slice(0, 12));
  }
  async function importSource() {
    const path = await open({
      multiple: false,
      filters: [{ name: "Documents", extensions: ["pdf", "docx", "txt", "md", "markdown"] }],
    });
    if (typeof path === "string") {
      const text = await run(() => invoke<string>("import_document", { path }));
      if (text !== undefined) {
        const name = path.split(/[/\\]/).pop() || "document";
        recordSource(name, text);
        note(`${t(lang, "imported")}: ${name} · ${text.length.toLocaleString()} ${t(lang, "chars")}`);
      }
    }
  }
  function openSource(src: Source) {
    setNav("tasks");
    setActiveId(null);
    setView("home");
    setCmd("check_draft");
    setDraft(src.text);
  }
  function removeSource(id: string) {
    setSources((ss) => ss.filter((x) => x.id !== id));
  }

  async function runCheckDraft(draftText?: string) {
    const useDraft = (draftText ?? draft).trim();
    if (!useDraft) return;
    const proj = await ensureProject();
    if (!proj) return;
    const id = `t${Date.now()}`;
    const title = useDraft.slice(0, 42) + (useDraft.length > 42 ? "…" : "");
    setTasks((ts) => [{ id, type: "check_draft", title, status: "running", ts: Date.now() }, ...ts]);
    setActiveId(id);
    setNav("tasks");
    setView("thread");
    setCmd(null);
    setSlash("");
    setProgress(null);
    const litsearch = localStorage.getItem("axiom3.lit") !== "off";
    const fmRaw = parseInt(localStorage.getItem("axiom3.fleetmax") || "", 10);
    const fleetmax = Number.isFinite(fmRaw) && fmRaw > 0 ? fmRaw : null;
    const c = await run(() => invoke<DraftCheck>("check_draft", { draft: useDraft, references, litsearch, fleetmax }));
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

  // Launch a dynamic multi-agent WORKFLOW run. The supervisor decomposes the goal,
  // fans out sub-agents, and streams their lifecycle into the ActivityRail; we open the
  // rail and record a task so its synthesized result is readable in the thread.
  async function runWorkflow(goal: string) {
    const g = goal.trim();
    if (!g) return;
    const proj = await ensureProject();
    if (!proj) return;
    const runid = `run-${Date.now()}`;
    const title = g.slice(0, 42) + (g.length > 42 ? "…" : "");
    setTasks((ts) => [{ id: runid, type: "workflow", title, status: "running", ts: Date.now() }, ...ts]);
    setActiveId(runid);
    setNav("tasks");
    setView("thread");
    setCmd(null);
    setSlash("");
    setRailOpen(true);
    localStorage.setItem("axiom3.rail", "1");
    const r = await run<{ status: string; synthesis: string | null }>(() =>
      invoke("run_workflow", { goal: g, runid, model: taskModel || undefined, agents: wfAgents || undefined })
    );
    setTasks((ts) =>
      ts.map((x) =>
        x.id === runid
          ? r
            ? { ...x, status: r.status === "done" ? "done" : r.status === "cancelled" ? "cancelled" : "failed", synthesis: r.synthesis ?? undefined }
            : { ...x, status: "failed", error: "failed" }
          : x
      )
    );
  }

  // Read-only coding agent over a chosen workspace folder. Streams into the rail.
  async function pickWorkspace() {
    const dir = await open({ directory: true, multiple: false });
    if (typeof dir === "string") setCodeWs(dir);
  }
  async function runCoding(goal: string) {
    const g = goal.trim();
    if (!g || !codeWs) return;
    const runid = `run-${Date.now()}`;
    const title = g.slice(0, 42) + (g.length > 42 ? "…" : "");
    setTasks((ts) => [{ id: runid, type: "code", title, status: "running", ts: Date.now() }, ...ts]);
    setActiveId(runid);
    setNav("tasks");
    setView("thread");
    setCmd(null);
    setSlash("");
    setRailOpen(true);
    localStorage.setItem("axiom3.rail", "1");
    const r = await run<{ status: string; synthesis: string }>(() =>
      invoke("coding_run", { goal: g, runid, workspace: codeWs, writable: codeWritable, model: taskModel || undefined })
    );
    setTasks((ts) =>
      ts.map((x) =>
        x.id === runid
          ? r
            ? { ...x, status: r.status === "done" ? "done" : r.status === "cancelled" ? "cancelled" : "failed", synthesis: r.synthesis }
            : { ...x, status: "failed", error: "failed" }
          : x
      )
    );
  }

  async function cancelCheck() {
    await invoke("cancel_check");
  }

  async function exportProvenance() {
    const path = await save({
      defaultPath: "axiom3-provenance.json",
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

  // A clean claim (no deterministic flag, no fleet verdict) is "unchecked" — shown
  // neutral, NOT as an alarming "uncertain". Only flagged / verdicted claims stand out.

  const activeTask = tasks.find((x) => x.id === activeId) ?? null;

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
      if (it.key === "tasks") newTask();
      else setNav(it.key as NavKey);
    } else {
      setNav("tasks");
      setActiveId(null);
      setView("home");
      setCmd(it.key);
    }
  }

  // Bucket the task history by recency, like Claude's chat list.
  function groupOf(ts: number): "today" | "yesterday" | "earlier" {
    const now = new Date();
    const startToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
    if (ts >= startToday) return "today";
    if (ts >= startToday - 86400000) return "yesterday";
    return "earlier";
  }
  const RECENT_GROUPS: { key: "today" | "yesterday" | "earlier"; zh: string; en: string }[] = [
    { key: "today", zh: "今天", en: "Today" },
    { key: "yesterday", zh: "昨天", en: "Yesterday" },
    { key: "earlier", zh: "更早", en: "Earlier" },
  ];

  // Slash-menu matches for the current composer text (shared by render + keyboard nav).
  const slashOpen = slash.trimStart().startsWith("/");
  const slashMatches = slashOpen
    ? COMMANDS.filter((c) => {
        const q = slash.trimStart().slice(1).toLowerCase();
        return q === "" || (lang === "zh" ? c.zh : c.en).toLowerCase().includes(q) || c.cat.includes(q);
      })
    : [];

  // Plain prose typed in the composer == "check this text" (the one live task). Returns
  // true if it consumed the submit (so the textarea doesn't also insert a newline).
  function submitFreeText(): boolean {
    const text = slash.trim();
    if (text && !text.startsWith("/")) {
      runCheckDraft(text);
      return true;
    }
    return false;
  }

  // The slash composer — shared by the home screen and thread follow-ups. Typing "/"
  // surfaces task types; picking one swaps in that task's inputs (only /校对 runs today).
  function composerBlock() {
    return (
      <div className="ax-composer">
        {cmd === null ? (
          <>
            <textarea
              className="ax-textarea"
              value={slash}
              onChange={(e) => {
                setSlash(e.currentTarget.value);
                setSlashIdx(0);
              }}
              onKeyDown={(e) => {
                if (slashOpen && slashMatches.length) {
                  if (e.key === "ArrowDown") {
                    e.preventDefault();
                    setSlashIdx((i) => Math.min(i + 1, slashMatches.length - 1));
                  } else if (e.key === "ArrowUp") {
                    e.preventDefault();
                    setSlashIdx((i) => Math.max(i - 1, 0));
                  } else if (e.key === "Enter") {
                    e.preventDefault();
                    const c = slashMatches[Math.min(slashIdx, slashMatches.length - 1)];
                    if (c && c.active) {
                      setCmd(c.key);
                      setSlash("");
                    }
                  } else if (e.key === "Escape") {
                    setSlash("");
                  }
                } else if (e.key === "Enter" && !e.shiftKey) {
                  e.preventDefault();
                  submitFreeText();
                }
              }}
              rows={3}
              placeholder={t(lang, "composer_ph")}
            />
            <div className="ax-toolrow" style={{ justifyContent: "flex-end" }}>
              <button
                className="ax-send"
                onClick={() => submitFreeText()}
                disabled={busy || !slash.trim() || slashOpen}
                title={t(lang, "task_run")}
                aria-label={t(lang, "task_run")}
              >
                <ArrowUp size={16} aria-hidden />
              </button>
            </div>
            {slashOpen && slashMatches.length > 0 && (
              <div className="ax-slash-menu">
                {slashMatches.map((c, i) => (
                  <button
                    key={c.key}
                    className={`ax-slash-item ${i === Math.min(slashIdx, slashMatches.length - 1) ? "is-active" : ""}`}
                    disabled={!c.active}
                    onMouseEnter={() => setSlashIdx(i)}
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
                  <button className="ax-btn ax-btn--primary" onClick={() => runCheckDraft()} disabled={busy || !draft.trim()}>
                    {t(lang, "task_run")}
                  </button>
                  <button className="ax-iconbtn" onClick={() => importInto(setDraft)} disabled={busy} title={t(lang, "import_btn")} aria-label={t(lang, "import_btn")}>
                    <Upload size={16} aria-hidden />
                  </button>
                  <button className="ax-iconbtn" onClick={() => setDraft(tidyDraft(draft))} disabled={busy || !draft.trim()} title={lang === "zh" ? "整理:去连字符、重接被打断的换行" : "Tidy: de-hyphenate + rejoin broken line breaks"} aria-label="tidy">
                    <WrapText size={16} aria-hidden />
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
            ) : cmd === "workflow" ? (
              <>
                <textarea
                  className="ax-textarea"
                  value={draft}
                  onChange={(e) => setDraft(e.currentTarget.value)}
                  rows={5}
                  placeholder={lang === "zh" ? "描述目标,一支智能体队伍会被组建去完成它…" : "Describe a goal — a fleet of agents will be assembled to do it…"}
                />
                <div className="ax-toolrow">
                  <button className="ax-btn ax-btn--primary" onClick={() => runWorkflow(draft)} disabled={busy || !draft.trim()}>
                    {t(lang, "task_run")}
                  </button>
                  <select className="ax-select" value={wfAgents} onChange={(e) => setWfAgents(Number(e.currentTarget.value))} title={lang === "zh" ? "并行智能体数量" : "parallel agents"}>
                    {AGENT_CHOICES.map((n) => (
                      <option key={n} value={n}>{n === 0 ? (lang === "zh" ? "智能体:自动" : "agents: auto") : `${n} ${lang === "zh" ? "智能体" : "agents"}`}</option>
                    ))}
                  </select>
                  <select className="ax-select" value={taskModel} onChange={(e) => setTaskModel(e.currentTarget.value)} title={lang === "zh" ? "模型" : "model"}>
                    {MODELS.map((m) => (
                      <option key={m.id} value={m.id}>{m.label}</option>
                    ))}
                  </select>
                </div>
              </>
            ) : cmd === "code" ? (
              <>
                <div className="ax-toolrow">
                  <button className="ax-btn" onClick={pickWorkspace}>
                    {codeWs ? (lang === "zh" ? "更换目录" : "Change folder") : (lang === "zh" ? "选择工作目录" : "Choose workspace")}
                  </button>
                  {codeWs && (
                    <span className="ax-hint" style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", maxWidth: 300 }}>
                      {codeWs}
                    </span>
                  )}
                </div>
                <textarea
                  className="ax-textarea"
                  value={draft}
                  onChange={(e) => setDraft(e.currentTarget.value)}
                  rows={4}
                  placeholder={lang === "zh" ? "要分析的任务(只读:读取 / 搜索 / 提议改动)…" : "What to analyze (read-only: read / search / propose)…"}
                />
                <div className="ax-toolrow">
                  <button className="ax-btn ax-btn--primary" onClick={() => runCoding(draft)} disabled={busy || !draft.trim() || !codeWs}>
                    {t(lang, "task_run")}
                  </button>
                  <button
                    className={`ax-toggle ${codeWritable ? "on" : ""}`}
                    onClick={() => setCodeWritable(!codeWritable)}
                    title={codeWritable ? (lang === "zh" ? "可写入文件并运行构建/测试" : "can write files + run build/test") : (lang === "zh" ? "只读分析" : "read-only")}
                  >
                    {codeWritable ? (lang === "zh" ? "写入 · 构建" : "write · build") : (lang === "zh" ? "只读" : "read-only")}
                  </button>
                  <select className="ax-select" value={taskModel} onChange={(e) => setTaskModel(e.currentTarget.value)} title={lang === "zh" ? "模型" : "model"}>
                    {MODELS.map((m) => (
                      <option key={m.id} value={m.id}>{m.label}</option>
                    ))}
                  </select>
                </div>
              </>
            ) : (
              <p className="ax-hint">{t(lang, "soon_note")}</p>
            )}
          </>
        )}
      </div>
    );
  }

  return (
    <>
      {booting && <Splash lang={lang} onDone={() => setBooting(false)} />}
      {!onboarded && <OnboardingWizard lang={lang} onDone={() => setOnboarded(true)} />}
      {settingsOpen && <SettingsModal lang={lang} onLang={setLang} onClose={() => setSettingsOpen(false)} />}
      {paletteOpen && (
        <CommandPalette lang={lang} items={paletteItems} onPick={onPalettePick} onClose={() => setPaletteOpen(false)} />
      )}
      {dropping && (
        <div className="ax-dropzone">
          <div className="ax-dropzone__box">
            <Upload size={28} aria-hidden />
            <p className="ax-dropzone__t">{lang === "zh" ? "拖放以导入文档" : "Drop to import a document"}</p>
            <p className="ax-hint">PDF · DOCX · TXT · Markdown</p>
          </div>
        </div>
      )}
      <div className={`ax-app ${railOpen ? "has-rail" : ""}`}>
      <aside className="ax-sidebar">
        <div className="ax-brand" style={{ fontSize: 19 }}>Axiom3</div>

        <button className="ax-side-new" onClick={newTask}>
          <Plus size={16} aria-hidden /> {lang === "zh" ? "新建任务" : "New task"}
        </button>

        {tasks.length > 4 && (
          <select
            className="ax-select"
            style={{ width: "100%", fontSize: 12 }}
            value={taskFilter}
            onChange={(e) => setTaskFilter(e.currentTarget.value as typeof taskFilter)}
          >
            <option value="all">{lang === "zh" ? "全部任务" : "All tasks"}</option>
            <option value="check_draft">{lang === "zh" ? "校对" : "Check"}</option>
            <option value="workflow">{lang === "zh" ? "工作流" : "Workflow"}</option>
            <option value="code">{lang === "zh" ? "编程" : "Code"}</option>
          </select>
        )}

        <div className="ax-side-scroll">
          {tasks.length === 0 ? (
            <p className="ax-hint" style={{ padding: "6px 9px" }}>
              {lang === "zh" ? "还没有任务。在中间的输入框开始第一个。" : "No tasks yet. Start your first in the box in the center."}
            </p>
          ) : (
            RECENT_GROUPS.map((g) => {
              const items = tasks.filter((tk) => (taskFilter === "all" || tk.type === taskFilter) && groupOf(tk.ts) === g.key);
              if (!items.length) return null;
              return (
                <div key={g.key}>
                  <p className="ax-nav__label">{lang === "zh" ? g.zh : g.en}</p>
                  {items.map((tk) => (
                    <div
                      key={tk.id}
                      className={`ax-recent ${nav === "tasks" && activeId === tk.id ? "is-active" : ""}`}
                    >
                      <button
                        className="ax-recent__main"
                        title={`${tk.title} · ${t(lang, `st_${tk.status}`)}`}
                        onClick={() => {
                          setNav("tasks");
                          setActiveId(tk.id);
                          setView("thread");
                        }}
                      >
                        <span className="ax-recent__title">
                          <span className={`ax-lamp ${tk.status === "done" ? "is-green" : tk.status === "running" ? "is-amber" : "is-neutral"}`} />
                          <b>{tk.title}</b>
                        </span>
                      </button>
                      <button
                        className="ax-recent__del"
                        title={lang === "zh" ? "删除任务" : "Delete task"}
                        aria-label={lang === "zh" ? "删除任务" : "Delete task"}
                        onClick={() => removeTask(tk.id)}
                      >
                        <X size={13} aria-hidden />
                      </button>
                    </div>
                  ))}
                </div>
              );
            })
          )}
        </div>

        <div className="ax-side-tools">
          <p className="ax-nav__label">{lang === "zh" ? "工具" : "Tools"}</p>
          {NAV.filter((n) => n.key !== "tasks").map((n) => (
            <button
              key={n.key}
              className={`ax-nav__item ${nav === n.key ? "is-active" : ""}`}
              onClick={() => setNav(n.key)}
            >
              <n.icon size={16} aria-hidden />
              {lang === "zh" ? n.zh : n.en}
            </button>
          ))}
        </div>

        <div className="ax-side-foot">
          {status && (
            <span className="ax-proj" style={{ marginTop: 0, flex: 1, minWidth: 0 }}>
              <span className={`ax-lamp ${status.chain_ok ? "is-green" : "is-amber"}`} />
              <span className="ax-proj__name">{status.project_id}</span>
            </span>
          )}
          <button
            className="ax-iconbtn"
            onClick={() => setLang(lang === "zh" ? "en" : "zh")}
            title={lang === "zh" ? "English" : "中文"}
            aria-label={t(lang, "switch_language")}
            style={{ marginLeft: status ? 0 : "auto" }}
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
        <AppBanner lang={lang} error={error} onDismiss={() => setError("")} />
        {interrupted > 0 && (
          <div className="ax-banners">
            <div className="ax-banner ax-banner--warn">
              <RotateCcw size={14} aria-hidden />
              <span className="ax-banner__msg">
                {lang === "zh"
                  ? `上次有 ${interrupted} 个任务被中断,已保留在任务历史中(标记为已取消)。`
                  : `${interrupted} task(s) were interrupted last session — kept in your history (marked cancelled).`}
              </span>
              <button className="ax-iconbtn" onClick={() => setInterrupted(0)} title={lang === "zh" ? "知道了" : "Dismiss"} aria-label="dismiss">
                <X size={13} aria-hidden />
              </button>
            </div>
          </div>
        )}
        <header className="ax-topbar">
          <div className="ax-crumb">
            {nav === "tasks" && view === "thread" && activeTask ? (
              <>
                <span className={`ax-lamp ${activeTask.status === "done" ? "is-green" : activeTask.status === "running" ? "is-amber" : "is-neutral"}`} />
                <span className="ax-proj__name" style={{ maxWidth: 380 }}>{activeTask.title}</span>
              </>
            ) : nav !== "tasks" ? (
              <span>
                {(() => {
                  const n = NAV.find((x) => x.key === nav);
                  return n ? (lang === "zh" ? n.zh : n.en) : lang === "zh" ? "通知" : "Notifications";
                })()}
              </span>
            ) : status ? (
              <>
                <span className={`ax-lamp ${status.chain_ok ? "is-green" : "is-amber"}`} />
                {status.project_id}
                <span className="ax-hint">· {t(lang, "events")} {status.event_count}</span>
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
            <button
              className={`ax-iconbtn ${railOpen ? "is-active" : ""}`}
              onClick={() => {
                const next = !railOpen;
                setRailOpen(next);
                localStorage.setItem("axiom3.rail", next ? "1" : "0");
              }}
              title={lang === "zh" ? "活动栏" : "Activity"}
              aria-label={lang === "zh" ? "活动栏" : "Activity"}
            >
              <PanelRight size={16} aria-hidden />
            </button>
          </div>
        </header>

        <div className="ax-content">


          {nav !== "tasks" ? (
            <div className="ax-canvas">
              {nav === "projects" && <ProjectsView lang={lang} />}
              {nav === "reading" && (
                <ReadingView lang={lang} sources={sources} onImport={importSource} onOpen={openSource} onRemove={removeSource} />
              )}
              {nav === "search" && (
                <SearchView
                  lang={lang}
                  tasks={tasks}
                  onOpenTask={(id) => {
                    setNav("tasks");
                    setActiveId(id);
                    setView("thread");
                  }}
                />
              )}
              {nav === "evidence" && <EvidenceView lang={lang} tasks={tasks} />}
              {nav === "fleet" && <FleetMonitor lang={lang} />}
              {nav === "shield" && <ShieldView lang={lang} />}
              {nav === "integrity" && <IntegrityView lang={lang} />}
              {nav === "provenance" && <ProvenanceView lang={lang} />}
              {nav === "metrics" && <MetricsView lang={lang} />}
              {nav === "notifications" && <NotificationsView lang={lang} />}
            </div>
          ) : view === "home" ? (
            <div className="ax-home">
              <h1 className="ax-home__greeting">{greeting()}</h1>
              {composerBlock()}
              <div className="ax-chips">
                {["check_draft", "verify", "research", "write", "ppt", "code"].map((k) => {
                  const c = COMMANDS.find((x) => x.key === k);
                  if (!c) return null;
                  return (
                    <button key={k} className="ax-chip" onClick={() => { setCmd(c.key); setSlash(""); }}>
                      /{lang === "zh" ? c.zh : c.en}
                      {!c.active && <span className="ax-chip__d"> · {t(lang, "coming_soon")}</span>}
                    </button>
                  );
                })}
              </div>

              <details className="ax-dev" style={{ maxWidth: 700, margin: "26px auto 0" }}>
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
            </div>
          ) : (
            <div className="ax-thread">
              {(() => {
                  const tk = tasks.find((x) => x.id === activeId);
                  if (!tk) return null;
                  return (
                    <>
                      <div className="ax-results__head">
                        <span style={{ display: "flex", alignItems: "center", gap: 8 }}>
                          <button className="ax-iconbtn" onClick={() => { setView("home"); setActiveId(null); }} title={t(lang, "back")} aria-label={t(lang, "back")}>
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
                            <button className={`ax-iconbtn ${hideReviewed ? "is-active" : ""}`} onClick={() => setHideReviewed(!hideReviewed)} title={lang === "zh" ? (hideReviewed ? "显示已复核" : "隐藏已复核") : hideReviewed ? "show reviewed" : "hide reviewed"} aria-label="toggle reviewed">
                              <Check size={16} aria-hidden />
                            </button>
                          </div>
                        )}
                      </div>

                      {tk.status === "running" &&
                        (tk.type === "workflow" || tk.type === "code" ? (
                          <div className="ax-progress">
                            <span>{lang === "zh" ? "智能体运行中 · 见右侧活动栏 →" : "agents running · see the activity rail →"}</span>
                            <button className="ax-iconbtn" onClick={() => invoke("cancel_run", { runid: tk.id })} title={t(lang, "cancel")} aria-label={t(lang, "cancel")}>
                              <X size={14} aria-hidden />
                            </button>
                          </div>
                        ) : (
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
                        ))}

                      {tk.synthesis && (
                        <div className="ax-synth">
                          <p className="ax-block-label">{lang === "zh" ? "汇总结果" : "Synthesized result"}</p>
                          <div className="ax-synth__body">{tk.synthesis}</div>
                        </div>
                      )}

                      {!tk.result && !tk.synthesis && tk.status !== "running" && (
                        <p className="ax-hint" style={{ marginTop: 14 }}>
                          {tk.status === "cancelled"
                            ? lang === "zh" ? "任务已取消。" : "Task was cancelled."
                            : lang === "zh" ? "任务未完成。" : "Task did not complete."}
                        </p>
                      )}

                      {tk.result && (
                        <>
                          <span className="ax-results__meta">
                            {tk.result.total} {t(lang, "claims")} · {tk.result.flagged} {t(lang, "flagged")} ·{" "}
                            {tk.result.fleet_verified} {t(lang, "cross_vendor_n")}
                            {(() => {
                              const rc = tk.result.entries.filter((e) => reviewed[e.claim_id]).length;
                              return rc > 0 ? ` · ${rc} ${lang === "zh" ? "已复核" : "reviewed"}` : "";
                            })()}
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
                              .filter((e) => !hideReviewed || !reviewed[e.claim_id])
                              .map((e) => (
                                <ClaimDossier
                                  key={e.claim_id}
                                  entry={e}
                                  lang={lang}
                                  reviewed={!!reviewed[e.claim_id]}
                                  onReview={() => toggleReviewed(e.claim_id)}
                                />
                              ))}
                          </div>
                        </>
                      )}
                    </>
                  );
                })()}
              {activeId && (
                <div className="ax-thread__followup">{composerBlock()}</div>
              )}
            </div>
          )}
        </div>
        <StatusBar lang={lang} status={status} />
      </main>
      {railOpen && (
        <ActivityRail
          lang={lang}
          onClose={() => {
            setRailOpen(false);
            localStorage.setItem("axiom3.rail", "0");
          }}
        />
      )}
      </div>
    </>
  );
}

export default App;
