import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { Lang, t } from "./i18n";
import "./theme.css";
import type { ProjectStatus, EventInfo, DraftCheck, ExportSummary, Task, Source, RunEvent } from "./lib/types";
import { ClaimDossier } from "./components/ClaimDossier";
import { ArtifactProvenance } from "./components/ArtifactProvenance";
import { ArtifactInspector } from "./components/ArtifactInspector";
import { ReviewerPanel } from "./components/ReviewerPanel";
import { ArtifactBody } from "./components/artifact/ArtifactBody";
import { ArtifactPane } from "./components/ArtifactPane";
import { OnboardingWizard } from "./components/OnboardingWizard";
import { Settings as SettingsModal, applyTheme } from "./components/Settings";
import { ReadingView } from "./components/ReadingView";
import { ProjectsView } from "./components/ProjectsView";
import { SearchView } from "./components/SearchView";
import { FleetMonitor } from "./components/FleetMonitor";
import { MetricsView } from "./components/MetricsView";
import { NotificationsView } from "./components/NotificationsView";
import { CommandPalette, type PaletteItem } from "./components/CommandPalette";
import { ActivityRail } from "./components/ActivityRail";
import { Splash } from "./components/Splash";
import { StatusBar } from "./components/StatusBar";
import { AppBanner } from "./components/AppBanner";
import {
  Download, ListFilter, X, Languages, Settings, Upload, ChevronLeft, Plus, ArrowUp,
  Sparkles, BookOpen, Search, Boxes, GitBranch, Gauge,
  Command, Bell, PanelRight, FolderOpen, RotateCcw, WrapText, Check, type LucideIcon,
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
// Rigor is NOT a set of destinations — it lives on the artifact (see the session's
// inspector tabs: 证据 / 护盾 / 溯源 / 完整性). The sidebar keeps only the places you go.
const NAV: { key: NavKey; zh: string; en: string; icon: LucideIcon; group: NavGroup }[] = [
  { key: "tasks", zh: "会话", en: "Sessions", icon: Sparkles, group: "work" },
  { key: "projects", zh: "项目", en: "Projects", icon: FolderOpen, group: "work" },
  { key: "reading", zh: "材料", en: "Materials", icon: BookOpen, group: "work" },
  { key: "search", zh: "检索", en: "Search", icon: Search, group: "work" },
  { key: "fleet", zh: "舰队", en: "Fleet", icon: Boxes, group: "overview" },
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
  // Mirror `lang` into a ref so the []-dep drag-drop/menu listeners (registered once) read
  // the CURRENT language instead of the value captured at first render.
  const langRef = useRef(lang);
  langRef.current = lang;

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
  const [references, setReferences] = useState(() => localStorage.getItem("axiom3.references") ?? "");
  const [codeWs, setCodeWs] = useState<string | null>(null);
  const [codeWritable, setCodeWritable] = useState(false);
  const [sources, setSources] = useState<Source[]>(() => {
    try {
      return JSON.parse(localStorage.getItem("axiom3.sources") || "[]");
    } catch {
      return [];
    }
  });
  const [taskModel, setTaskModel] = useState(() => localStorage.getItem("axiom3.taskModel") ?? ""); // "" = auto
  const [wfAgents, setWfAgents] = useState(() => {
    const n = parseInt(localStorage.getItem("axiom3.wfAgents") || "", 10);
    return Number.isFinite(n) && n >= 0 ? n : 0; // 0 = auto
  });
  // Deep-search mode: run the workflow as Multi-LLM AB-MCTS (adaptive wider/deeper search
  // across the connected models) instead of a one-shot fan-out.
  const [deepSearch, setDeepSearch] = useState(() => localStorage.getItem("axiom3.deepSearch") === "1");
  // When a session is forked, the next session created inherits this lineage (then clears).
  const [forkOrigin, setForkOrigin] = useState<string | null>(null);
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
  const [fleetAgg, setFleetAgg] = useState({ runs: 0, spawned: 0, finished: 0 });
  const [lastSeenNotif, setLastSeenNotif] = useState(() => {
    const v = localStorage.getItem("axiom3.notifSeen");
    return v ? parseInt(v, 10) || 0 : Date.now(); // first run: don't flag pre-existing tasks
  });

  useEffect(() => {
    // The REAL Claude Science app is light (the dark I'd matched was only its marketing
    // page). Match the real reference: light default, both themes tuned, user can switch.
    // v2 migration corrects an earlier forced-dark, then respects the user's choice.
    if (localStorage.getItem("axiom3.themeMigrated") !== "2") {
      localStorage.setItem("axiom3.themeMigrated", "2");
      applyTheme("light");
      return;
    }
    applyTheme((localStorage.getItem("axiom3.theme") as "light" | "dark" | "system") || "light");
  }, []);

  // Persist the task history (most recent first, ≤50). A task's result carries all its
  // evidence text, so a big check can exceed the localStorage quota; on failure, keep as
  // many RECENT tasks as fit rather than silently dropping the entire history.
  useEffect(() => {
    for (let n = Math.min(tasks.length, 50); n >= 0; n--) {
      try {
        localStorage.setItem("axiom3.tasks", JSON.stringify(tasks.slice(0, n)));
        break;
      } catch {
        /* over quota — keep fewer and retry */
      }
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

  // Persist the reference corpus + workflow prefs so a reload / new task doesn't silently
  // drop the evidence base or reset the agent-count & model choices.
  useEffect(() => {
    try {
      localStorage.setItem("axiom3.references", references);
    } catch {
      /* quota — ignore */
    }
  }, [references]);
  useEffect(() => {
    localStorage.setItem("axiom3.taskModel", taskModel);
    localStorage.setItem("axiom3.wfAgents", String(wfAgents));
    localStorage.setItem("axiom3.deepSearch", deepSearch ? "1" : "0");
  }, [taskModel, wfAgents, deepSearch]);

  // Persist the source library (most recent first, ≤12). A full document's text can be
  // megabytes, so capping to 12 ENTRIES doesn't bound the payload — bound it by BYTES: if
  // the write exceeds quota, drop the TEXT of the OLDEST sources (keeping their metadata)
  // and retry, so the library is never silently lost wholesale. Recent sources keep their
  // text (still openable); older ones degrade to name/size only.
  useEffect(() => {
    const capped = sources.slice(0, 12);
    for (let keepText = capped.length; keepText >= 0; keepText--) {
      const payload = capped.map((s, i) => (i < keepText ? s : { ...s, text: "" }));
      try {
        localStorage.setItem("axiom3.sources", JSON.stringify(payload));
        return;
      } catch {
        /* too big — drop one more full text (oldest first) and retry */
      }
    }
    try {
      localStorage.setItem("axiom3.sources", "[]");
    } catch {
      /* give up — nothing more we can do */
    }
  }, [sources]);

  // Garbage-collect the reviewed map: drop claim ids that belong to no retained task, so it
  // can't grow without bound as tasks are deleted or age out of the persisted history.
  useEffect(() => {
    const live = new Set<string>();
    for (const tk of tasks) for (const e of tk.result?.entries ?? []) live.add(e.claim_id);
    setReviewed((r) => {
      const keys = Object.keys(r);
      if (keys.every((k) => live.has(k))) return r; // nothing stale → no re-render
      const pruned: Record<string, boolean> = {};
      for (const k of keys) if (live.has(k)) pruned[k] = r[k];
      try {
        localStorage.setItem("axiom3.reviewed", JSON.stringify(pruned));
      } catch {
        /* quota */
      }
      return pruned;
    });
  }, [tasks]);

  // Clear any error banner whenever the user navigates (a run sets its error after the
  // view change, so a fresh failure still shows; only stale errors get wiped).
  useEffect(() => {
    setError("");
  }, [nav, view, activeId]);

  // Opening Notifications marks everything currently there as seen (clears the unread badge).
  useEffect(() => {
    if (nav === "notifications") {
      const now = Date.now();
      setLastSeenNotif(now);
      localStorage.setItem("axiom3.notifSeen", String(now));
    }
  }, [nav]);
  const unreadNotif = tasks.filter((tk) => tk.ts > lastSeenNotif).length;

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

  // Fleet aggregate lives at App level (always mounted) so it accrues across navigation —
  // the Fleet view is a pure display of it. A view-local counter would reset on every
  // unmount and miss events emitted while the user was on another surface.
  useEffect(() => {
    const un = listen<RunEvent>("run_event", (e) => {
      const ev = e.payload;
      setFleetAgg((a) =>
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
  // Fork a session: reload its goal + settings into the composer as a NEW session, so you
  // can vary one thing (deep search, model, agents) and compare — the original is untouched.
  function forkSession(tk: Task) {
    if (!tk.goal) return;
    if (!confirmReplaceDraft()) return;
    setForkOrigin(tk.id);
    setNav("tasks");
    setView("home");
    setActiveId(null);
    setSlash("");
    setDraft(tk.goal);
    setCmd(tk.type === "workflow" || tk.type === "code" ? tk.type : "check_draft");
    if (tk.type === "workflow") setDeepSearch(!!tk.deep);
    note(lang === "zh" ? `已分叉:「${tk.title}」— 改设置后运行即可对比` : `Forked: "${tk.title}" — tweak a setting and run to compare`);
  }

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
    // Keep `references`: it's the user's evidence corpus, not per-task scratch. Clearing it
    // silently ungrounded follow-up checks (grounded verdicts flip when the base vanishes).
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
      const name = path.split(/[/\\]/).pop() || "document";
      recordSource(name, text); // always keep it in the library
      const lg = langRef.current; // fresh language, not the drop-listener's captured one
      note(`${t(lg, "imported")}: ${name} · ${text.length.toLocaleString()} ${t(lg, "chars")}`);
      // Only load it into the editor (replacing the draft) if that won't lose unsaved prose.
      if (confirmReplaceDraft()) {
        setNav("tasks");
        setActiveId(null);
        setView("home");
        setCmd("check_draft");
        setDraft(text);
      }
    }
  }

  // ── source library (阅读/摄入) ──
  // Guard the draft against a silent overwrite when loading a source: only ask if there's
  // unsaved prose that would be lost.
  function confirmReplaceDraft(): boolean {
    if (!draft.trim()) return true;
    return window.confirm(
      lang === "zh"
        ? "这会替换编辑器中的当前草稿,继续?"
        : "This will replace the current draft in the editor. Continue?"
    );
  }
  function recordSource(name: string, text: string) {
    const src: Source = { id: `s${Date.now()}`, name, chars: text.length, ts: Date.now(), text };
    // Signal when the 12-entry cap silently drops the oldest source.
    setSources((ss) => {
      const next = [src, ...ss.filter((x) => x.name !== name)];
      if (next.length > 12) {
        note(lang === "zh" ? "来源库已满(12),已移除最早的一条。" : "Source library full (12) — dropped the oldest.");
      }
      return next.slice(0, 12);
    });
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
    if (!confirmReplaceDraft()) return;
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
    setTasks((ts) => [{ id, type: "check_draft", title, status: "running", ts: Date.now(), goal: useDraft, forkedFrom: forkOrigin ?? undefined }, ...ts]);
    setForkOrigin(null);
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
  // `appendTo` = iterate an existing session's artifact (a follow-up → v2, v3 …). Without
  // it, a fresh session is created. Forking (separate thread) is handled elsewhere.
  async function runWorkflow(goal: string, appendTo?: string) {
    const g = goal.trim();
    if (!g) return;
    const proj = await ensureProject();
    if (!proj) return;
    const runid = `run-${crypto.randomUUID()}`;
    const sessionId = appendTo ?? runid;
    const title = g.slice(0, 42) + (g.length > 42 ? "…" : "");
    if (appendTo) {
      setTasks((ts) => ts.map((x) => (x.id === appendTo ? { ...x, status: "running" } : x)));
      setActiveId(appendTo);
    } else {
      setTasks((ts) => [{ id: runid, type: "workflow", title, status: "running", ts: Date.now(), goal: g, deep: deepSearch, forkedFrom: forkOrigin ?? undefined }, ...ts]);
      setForkOrigin(null);
      setActiveId(runid);
    }
    setNav("tasks");
    setView("thread");
    setCmd(null);
    setSlash("");
    setRailOpen(true);
    localStorage.setItem("axiom3.rail", "1");
    const r = await run<{
      status: string;
      synthesis: string | null;
      review?: string | null;
      steps?: { spec: { role: string; model: string }; status: string; output?: string }[];
    }>(() => invoke("run_workflow", { goal: g, runid, model: taskModel || undefined, agents: wfAgents || undefined, search: deepSearch }));
    const steps = r?.steps?.map((s) => ({ role: s.spec.role, status: s.status, output: s.output ?? "", model: s.spec.model }));
    setTasks((ts) =>
      ts.map((x) => {
        if (x.id !== sessionId) return x;
        if (!r) return { ...x, status: "failed" as const, error: "failed" };
        const status = r.status === "done" ? "done" : r.status === "cancelled" ? "cancelled" : "failed";
        const newVer = { synthesis: r.synthesis ?? undefined, review: r.review ?? undefined, steps, ts: Date.now(), goal: g };
        if (appendTo) {
          // seed v1 from the pre-iteration state, then append this run as the newest version.
          const prior = x.versions ?? [{ synthesis: x.synthesis, review: x.review, steps: x.steps, ts: x.ts, goal: x.goal ?? "" }];
          return { ...x, status, synthesis: newVer.synthesis, review: newVer.review, steps, goal: g, versions: [...prior, newVer] };
        }
        return { ...x, status, synthesis: newVer.synthesis, review: newVer.review, steps };
      })
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
    const runid = `run-${crypto.randomUUID()}`;
    const title = g.slice(0, 42) + (g.length > 42 ? "…" : "");
    setTasks((ts) => [{ id: runid, type: "code", title, status: "running", ts: Date.now(), goal: g, forkedFrom: forkOrigin ?? undefined }, ...ts]);
    setForkOrigin(null);
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
  // A finished workflow/coding session shows its result in the right-side ARTIFACT pane
  // (Claude-Science-style). Draft-check sessions keep their claim flow in the thread.
  const showArtifactPane =
    view === "thread" &&
    !!activeTask &&
    (activeTask.type === "workflow" || activeTask.type === "code") &&
    activeTask.status !== "running" &&
    (!!activeTask.synthesis || (activeTask.steps?.length ?? 0) > 0);
  // Auto-open the right column when an artifact becomes available.
  useEffect(() => {
    if (showArtifactPane && !railOpen) {
      setRailOpen(true);
      localStorage.setItem("axiom3.rail", "1");
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [showArtifactPane, activeId]);

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

  // Plain prose typed in the composer runs the action that MATCHES the open thread: a
  // follow-up in a workflow thread starts another workflow, in a coding thread another
  // coding step; on the home screen (or a check thread) it's a draft check. Previously it
  // always ran a draft check, silently doing the wrong operation inside a workflow/coding
  // thread. Returns true if it consumed the submit (so the textarea skips the newline).
  function submitFreeText(): boolean {
    const text = slash.trim();
    if (!text || text.startsWith("/")) return false;
    const at = tasks.find((x) => x.id === activeId);
    if (at?.type === "workflow") runWorkflow(text, at.id); // follow-up → iterate the artifact (v2…)
    else if (at?.type === "code" && codeWs) runCoding(text);
    else if (deepSearch) runWorkflow(text); // deep-search mode: run the task as AB-MCTS
    else runCheckDraft(text);
    return true;
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
            {/* Deep-search mode: sits below the send button so it's discoverable. ON → the
                typed task runs as a Multi-LLM AB-MCTS search (adaptive explore/refine across
                models); OFF → the usual check/answer. */}
            <div className="ax-toolrow" style={{ justifyContent: "flex-start", marginTop: 2 }}>
              <button
                className={`ax-toggle ${deepSearch ? "on" : ""}`}
                onClick={() => setDeepSearch(!deepSearch)}
                title={
                  deepSearch
                    ? (lang === "zh" ? "深度搜索:把任务交给多智能体自适应搜索(AB-MCTS,跨模型扩展/精修择优)" : "Deep search: run the task as an adaptive multi-agent search (AB-MCTS across models)")
                    : (lang === "zh" ? "标准模式:直接校对/回答。开启深度搜索让多智能体自适应择优。" : "Standard mode. Turn on deep search for an adaptive multi-agent search.")
                }
              >
                <Boxes size={13} aria-hidden /> {deepSearch ? (lang === "zh" ? "深度搜索 · 开" : "deep search · on") : (lang === "zh" ? "深度搜索" : "deep search")}
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
                  <button
                    className={`ax-toggle ${deepSearch ? "on" : ""}`}
                    onClick={() => setDeepSearch(!deepSearch)}
                    title={
                      deepSearch
                        ? (lang === "zh" ? "深度搜索:自适应地扩展/精修候选,跨多个模型择优(AB-MCTS)" : "Deep search: adaptively explore/refine candidates across models (AB-MCTS)")
                        : (lang === "zh" ? "标准:一次性并行扇出" : "Standard: one-shot parallel fan-out")
                    }
                  >
                    {deepSearch ? (lang === "zh" ? "深度搜索" : "deep search") : (lang === "zh" ? "标准" : "standard")}
                  </button>
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
          <Plus size={16} aria-hidden /> {lang === "zh" ? "新建研究会话" : "New session"}
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
              {lang === "zh" ? "还没有研究会话。在中间的输入框开始第一个。" : "No sessions yet. Start your first in the box in the center."}
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
                          {tk.forkedFrom && <span className="ax-fork-mark" title={lang === "zh" ? "由其他会话分叉" : "forked from another session"}>↳</span>}
                          <b>{tk.title}</b>
                          {tk.deep && <span className="ax-recent__deep" title={lang === "zh" ? "深度搜索(AB-MCTS)" : "deep search (AB-MCTS)"}>深</span>}
                        </span>
                      </button>
                      {tk.goal && (
                        <button
                          className="ax-recent__del"
                          title={lang === "zh" ? "分叉此会话(对比另一种做法)" : "Fork this session (compare another approach)"}
                          aria-label={lang === "zh" ? "分叉会话" : "Fork session"}
                          onClick={() => forkSession(tk)}
                        >
                          <GitBranch size={13} aria-hidden />
                        </button>
                      )}
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
              className={`ax-iconbtn ax-bell ${nav === "notifications" ? "is-active" : ""}`}
              onClick={() => setNav("notifications")}
              title={lang === "zh" ? "通知" : "Notifications"}
              aria-label={
                unreadNotif > 0
                  ? `${lang === "zh" ? "通知" : "Notifications"} · ${unreadNotif} ${lang === "zh" ? "条未读" : "unread"}`
                  : lang === "zh" ? "通知" : "Notifications"
              }
            >
              <Bell size={16} aria-hidden />
              {unreadNotif > 0 && <span className="ax-bell__dot">{unreadNotif > 9 ? "9+" : unreadNotif}</span>}
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
              {/* 证据 / 护盾 / 溯源 / 完整性 are no longer destinations — they are the
                  artifact's own inspector tabs (see ArtifactInspector in the session view). */}
              {nav === "fleet" && <FleetMonitor lang={lang} agg={fleetAgg} />}
              {nav === "metrics" && <MetricsView lang={lang} tasks={tasks} />}
              {nav === "notifications" && (
                <NotificationsView
                  lang={lang}
                  tasks={tasks}
                  onOpen={(id) => {
                    setNav("tasks");
                    setActiveId(id);
                    setView("thread");
                  }}
                />
              )}
            </div>
          ) : view === "home" ? (
            <div className="ax-home">
              <h1 className="ax-home__greeting">{greeting()}</h1>
              <p className="ax-home__sub">
                {lang === "zh"
                  ? "输入一个任务,或从下面选择一种能力。开启「深度搜索」让多智能体自适应择优。"
                  : "Type a task, or pick a capability below. Turn on deep search for an adaptive multi-agent run."}
              </p>
              {composerBlock()}
              <div className="ax-chips">
                {["check_draft", "verify", "research", "write", "ppt", "code"].map((k) => {
                  const c = COMMANDS.find((x) => x.key === k);
                  if (!c) return null;
                  return (
                    <button key={k} className={`ax-chip ${c.active ? "" : "is-soon"}`} onClick={() => { setCmd(c.key); setSlash(""); }}>
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

                      {/* When the artifact pane is open, the result + agents + provenance live
                          THERE; the thread keeps the conversation (progress + reviewer). */}
                      {showArtifactPane && (
                        <button className="ax-artref" onClick={() => setRailOpen(true)}>
                          {lang === "zh" ? "产物已生成 —— 见右侧「产物」面板 →" : "Artifact ready — see the Artifact panel on the right →"}
                        </button>
                      )}

                      {!showArtifactPane && tk.synthesis && (
                        <div className="ax-synth">
                          <p className="ax-block-label">{lang === "zh" ? "汇总结果" : "Synthesized result"}</p>
                          {/* rendered natively: markdown + molecules (SMILES) + diagrams (mermaid) + math */}
                          <ArtifactBody text={tk.synthesis} />
                        </div>
                      )}

                      {/* the reviewer agent, inline — like Claude Science's Reviewer panel */}
                      {tk.review && <ReviewerPanel lang={lang} review={tk.review} />}

                      {!showArtifactPane && tk.steps && tk.steps.length > 0 && (
                        <details className="ax-agents" style={{ marginTop: 12 }}>
                          <summary className="ax-block-label" style={{ cursor: "pointer" }}>
                            {lang === "zh" ? `各智能体输出 · ${tk.steps.length}` : `Per-agent output · ${tk.steps.length}`}
                          </summary>
                          <div style={{ marginTop: 8, display: "flex", flexDirection: "column", gap: 10 }}>
                            {tk.steps.map((st, i) => (
                              <div className="ax-card ax-card--pad" key={i}>
                                <div className="ax-row" style={{ border: "none", padding: 0, marginBottom: 6 }}>
                                  <span className={`ax-lamp ${st.status === "ok" ? "is-green" : st.status === "failed" ? "is-red" : "is-neutral"}`} />
                                  <div className="ax-row__title">{st.role}</div>
                                </div>
                                <div className="ax-synth__body" style={{ fontSize: 13 }}>
                                  {st.output || (lang === "zh" ? "(无输出)" : "(no output)")}
                                </div>
                              </div>
                            ))}
                          </div>
                        </details>
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

                      {/* Provenance + inspector live in the artifact pane when it's open; for
                          draft-check sessions (no pane) they stay here, on the artifact. */}
                      {!showArtifactPane && tk.status !== "running" && (tk.result || tk.synthesis) && (
                        <>
                          <ArtifactProvenance
                            lang={lang}
                            task={tk}
                            forkedFromTitle={tasks.find((x) => x.id === tk.forkedFrom)?.title}
                            onExport={exportProvenance}
                            busy={busy}
                          />
                          {/* rigor folded onto the artifact, not parked in sidebar destinations */}
                          <ArtifactInspector lang={lang} task={tk} />
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
      {railOpen &&
        (showArtifactPane && activeTask ? (
          <ArtifactPane
            lang={lang}
            task={activeTask}
            forkedFromTitle={tasks.find((x) => x.id === activeTask.forkedFrom)?.title}
            onExport={exportProvenance}
            onClose={() => {
              setRailOpen(false);
              localStorage.setItem("axiom3.rail", "0");
            }}
            busy={busy}
          />
        ) : (
          <ActivityRail
            lang={lang}
            onClose={() => {
              setRailOpen(false);
              localStorage.setItem("axiom3.rail", "0");
            }}
          />
        ))}
      </div>
    </>
  );
}

export default App;
