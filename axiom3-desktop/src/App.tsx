import { useState, useEffect, useLayoutEffect, useRef, useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getVersion } from "@tauri-apps/api/app";
import { open, save } from "@tauri-apps/plugin-dialog";
import { Lang, t } from "./i18n";
import "./theme.css";
import type { ProjectStatus, DraftCheck, ExportSummary, Task, Source, AgentStepView, CiteSource } from "./lib/types";
import { ClaimDossier } from "./components/ClaimDossier";
import { ArtifactProvenance } from "./components/ArtifactProvenance";
import { ArtifactInspector } from "./components/ArtifactInspector";
import { ReviewerPanel } from "./components/ReviewerPanel";
import { AgentSteps } from "./components/AgentSteps";
import { ThreadActivity } from "./components/ThreadActivity";
import { RunStatusPill } from "./components/RunStatusPill";
import { ContextWindow } from "./components/ContextWindow";
import { ResizeHandle } from "./components/ResizeHandle";
import { SessionSettings, DEFAULT_CFG, type SessionCfg } from "./components/SessionSettings";
import { runStore } from "./lib/runStore";
import { ArtifactBody } from "./components/artifact/ArtifactBody";
import { ArtifactPane } from "./components/ArtifactPane";
import { OnboardingWizard } from "./components/OnboardingWizard";
import { Settings as SettingsModal, applyTheme } from "./components/Settings";
import { ReadingView } from "./components/ReadingView";
import { ProjectsView } from "./components/ProjectsView";
import { MetricsView } from "./components/MetricsView";
import { NotificationsView } from "./components/NotificationsView";
import { CommandPalette, type PaletteItem } from "./components/CommandPalette";
import { ActivityRail } from "./components/ActivityRail";
import { Splash } from "./components/Splash";
import { AppBanner } from "./components/AppBanner";
import {
  Download, ListFilter, X, Languages, Settings, Upload, ChevronLeft, Plus, ArrowUp,
  Sparkles, BookOpen, Search, GitBranch, Gauge, TerminalSquare, Paperclip,
  Bell, PanelRight, PanelLeft, FolderOpen, RotateCcw, WrapText, Check,
  MoreHorizontal, Pencil, Mic, Info, Pause, Play, type LucideIcon,
} from "lucide-react";

// Composer modes (the icon-button popover, Claude-style). Each maps to backend behaviour:
// 自动 infers intent + thinking depth; 研究 runs literature/multi-agent research; 分析 spins up
// the Python kernel. The backend picks thinking vs non-thinking per mode.
const MODES = [
  { k: "auto", zh: "自动", en: "Auto", Icon: Sparkles, tzh: "读懂请求,自己选研究/分析/校验与思考深度", ten: "infer intent + thinking depth" },
  { k: "research", zh: "研究", en: "Research", Icon: Search, tzh: "深度研究:无预算 · 联网/文献优先 · 带引用高价值报告", ten: "deep research: no budget · web/literature-first · cited report" },
  { k: "analysis", zh: "分析", en: "Analysis", Icon: TerminalSquare, tzh: "分析你的本地数据:Python 内核真跑 → 计算图 + 统计(有预算)", ten: "analyze your local data: Python kernel → figures + stats" },
] as const;
type ModeKey = (typeof MODES)[number]["k"];

// Set during the tasks initializer (first render): how many persisted tasks were left
// "running" when the app last closed — i.e. interrupted by an abnormal exit (#9).
let bootInterrupted = 0;


// The platform's task capabilities — typed after "/" in the composer (like Claude).
// Researched against 2026 AI work platforms and mapped to the four core capabilities:
// rigor/checking (the differentiator), research, multimodal generation, coding — plus a
// dynamic-workflow catch-all. Only check_draft runs today; the rest are visible as 即将,
// so the platform's shape is complete. Every output is meant to be held to the rigor bar.
type SlashCmd = { key: string; cat: string; zh: string; en: string; dzh: string; den: string; active: boolean };
// Claude-style `/` commands — genuine actions/verbs, NOT task categories (write/ppt/… are
// inferred from what you type). `/技能` picks a specific domain skill to run.
const COMMANDS: SlashCmd[] = [
  { key: "skills", cat: "命令", zh: "技能", en: "skills", dzh: "选一个领域技能来执行(单细胞 / 系统发育 / 差异表达 …)", den: "run a specific domain skill", active: true },
  { key: "check_draft", cat: "命令", zh: "校对", en: "check", dzh: "拆成断言,确定性检查 + 证据(校验为中心)", den: "claims → deterministic checks + evidence", active: true },
  { key: "code", cat: "命令", zh: "编程", en: "code", dzh: "读取 / 搜索 / 精确编辑 / 跑测试", den: "read / search / precise edit / run tests", active: true },
  { key: "workflow", cat: "命令", zh: "工作流", en: "workflow", dzh: "自定义动态多智能体工作流(任意任务)", den: "custom dynamic multi-agent workflow", active: true },
];

// Domain skills the `/技能` command offers — mirrors the engine's skill registry. Picking one
// prefills a directive + switches to 分析 mode; the analysis agent then loads that skill.
const SKILLS = [
  { key: "single-cell-rnaseq", zh: "单细胞 RNA-seq", en: "Single-cell RNA-seq", zhh: "QC→归一化→聚类→UMAP→注释", enh: "QC → cluster → UMAP → annotate" },
  { key: "phylogenetics", zh: "系统发育 / 祖先重建", en: "Phylogenetics", zhh: "比对→建树→追踪残基→祖先重建", enh: "align → tree → ancestral states" },
  { key: "differential-expression", zh: "差异表达", en: "Differential expression", zhh: "效应量 + BH/FDR → 火山图", enh: "effect size + BH/FDR → volcano" },
  { key: "crispr-screen", zh: "CRISPR 敲除筛选", en: "CRISPR screen", zhh: "文库/向导/脱靶/构建/测序", enh: "library / guides / off-target" },
  { key: "protein-engineering", zh: "蛋白工程", en: "Protein engineering", zhh: "PLM/结构打分 → 共识排序", enh: "PLM/structure scoring → rank" },
  { key: "gwas", zh: "GWAS 关联分析", en: "GWAS", zhh: "关联检验 + 校正 → 曼哈顿图", enh: "assoc test + correction → manhattan" },
];

// Sidebar navigation — the platform's surfaces, grouped 工作 / 信任·证据 / 概览. Each routes
// to a self-contained surface component. Notifications is reached from the topbar bell.
type NavKey =
  | "tasks" | "projects" | "reading" | "search" | "notebook" | "fleet"
  | "shield" | "integrity" | "provenance" | "evidence"
  | "metrics" | "notifications";
type NavGroup = "work" | "trust" | "overview";
// Rigor is NOT a set of destinations — it lives on the artifact (see the session's
// inspector tabs: 证据 / 护盾 / 溯源 / 完整性). The sidebar keeps only the places you go.
// Aggressively trimmed to the four surfaces you actually navigate to. 检索 folds into 材料
// (search over your materials), 笔记本 lives inside a session, 舰队 into 概览.
const NAV: { key: NavKey; zh: string; en: string; icon: LucideIcon; group: NavGroup }[] = [
  { key: "tasks", zh: "会话", en: "Sessions", icon: Sparkles, group: "work" },
  { key: "projects", zh: "项目", en: "Projects", icon: FolderOpen, group: "work" },
  { key: "reading", zh: "材料", en: "Materials", icon: BookOpen, group: "work" },
  { key: "metrics", zh: "概览", en: "Overview", icon: Gauge, group: "overview" },
];
// Model choices offered in the task control bar (the connected provider is DeepSeek).
// "" = auto (the app's default per provider). deepseek-reasoner emits a real chain-of-thought.
const MODELS: { id: string; label: string }[] = [
  { id: "", label: "自动 · Auto (GLM)" },
  { id: "glm-5.2", label: "GLM-5.2 · 深度推理" },
  { id: "glm-5-turbo", label: "GLM Turbo · 快" },
  { id: "glm-4.6", label: "GLM-4.6" },
  { id: "deepseek-v4-flash", label: "DeepSeek V4 Flash · 备选" },
  { id: "deepseek-reasoner", label: "DeepSeek Reasoner · 备选" },
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

// For LIVE step persistence: strip the heavy base64 `pyfig` blocks from a step body (they'd blow
// the localStorage quota if snapshotted every 1.5s). The code + text output survive; the final
// `r.trace` restores the real figures when the run completes.
function stripLiveFigure(s: AgentStepView): AgentStepView {
  if (!s.output || !s.output.includes("```pyfig")) return s;
  return { ...s, output: s.output.replace(/\n*```pyfig\n[\s\S]*?```/g, "") };
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
  const [error, setError] = useState("");
  const [toast, setToast] = useState(""); // transient confirmation (export done, project opened, …)
  const noteTimer = useRef(0);
  const [busy, setBusy] = useState(false);
  const [draft, setDraft] = useState(
    () => localStorage.getItem("axiom3.draft") ?? "Water boils at 100 degrees Celsius at sea level. Therefore the treatment causes recovery."
  );
  const [references, setReferences] = useState(() => localStorage.getItem("axiom3.references") ?? "");
  // The session's project directory (working folder on disk). A new session must pick one.
  const [projectDir, setProjectDir] = useState<string | null>(() => localStorage.getItem("axiom3.projectDir"));
  // The agent's working folder. Defaults to the persisted project dir so a restart anchors
  // analysis/coding to the SAME directory the UI shows (else analyze fell back to a hidden
  // scratch dir while the composer still displayed the project path).
  const [codeWs, setCodeWs] = useState<string | null>(() => localStorage.getItem("axiom3.projectDir"));
  const [newSessionOpen, setNewSessionOpen] = useState(false); // the "pick a project directory" dialog
  const [pickingDir, setPickingDir] = useState<string | null>(null); // dir chosen inside that dialog
  const projectName = (dir: string | null) => (dir ? dir.split(/[/\\]/).filter(Boolean).pop() || dir : "");
  const [codeWritable, setCodeWritable] = useState(false);
  const [codeAnalyze, setCodeAnalyze] = useState(false); // give the agent a live Python kernel
  const [codeAttachments, setCodeAttachments] = useState<string[]>([]); // data files for analysis
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
  // Composer mode (next to Send): 自动 infers intent + thinking depth in the backend; 研究 runs a
  // literature/multi-agent research pass; 分析 spins up the Python kernel. Replaces the old
  // deep-search toggle + model dropdown — the backend picks thinking/non-thinking per mode.
  const [mode, setMode] = useState<ModeKey>(() => (localStorage.getItem("axiom3.mode") as ModeKey) || "auto");
  const [modeMenuOpen, setModeMenuOpen] = useState(false);
  const [appVer, setAppVer] = useState(""); // app version from tauri.conf (single source), shown in the sidebar
  useEffect(() => { getVersion().then(setAppVer).catch(() => {}); }, []);
  const [skillMenu, setSkillMenu] = useState(false); // /技能 picker open
  const [pendingSkill, setPendingSkill] = useState<string | null>(null); // skill key to inject into the next run
  // Persistent run settings (the composer's settings popover). `specialist` maps to the skill
  // registry and is applied to EVERY run until cleared (surfaced as a removable chip so it's
  // never a silent override; an explicit /技能 pick wins over it). The other toggles persist as
  // preferences — only `specialist` is wired into the run today.
  const [sessionCfg, setSessionCfg] = useState<SessionCfg>(() => {
    try { return { ...DEFAULT_CFG, ...JSON.parse(localStorage.getItem("axiom3.sessionCfg") || "{}") }; } catch { return DEFAULT_CFG; }
  });
  useEffect(() => { localStorage.setItem("axiom3.sessionCfg", JSON.stringify(sessionCfg)); }, [sessionCfg]);
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
  const slashRef = useRef<HTMLTextAreaElement>(null);
  // Composer auto-grows from a single line. Keyed on `slash` so it also SHRINKS back after a
  // programmatic clear (submit / new-session / fork all call setSlash("")), which a pure
  // onChange handler would miss. First set height:auto so scrollHeight reflects content.
  useLayoutEffect(() => {
    const el = slashRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = Math.min(el.scrollHeight, 200) + "px";
  }, [slash]);
  // A run the user tried to start before choosing a project directory: stashed as a plain
  // descriptor (NOT a closure — a closure would capture projectDir=null and re-trigger the gate
  // forever). Once the new-session dialog writes projectDir, the effect below re-dispatches with
  // the preserved text, and the FRESH run function (projectDir now set) sails past its own gate.
  const [pendingRun, setPendingRun] = useState<
    null | { run: "free" | "check" | "workflow" | "coding"; text: string; forceAnalyze?: boolean; appendTo?: string; research?: boolean }
  >(null);
  useEffect(() => {
    if (!projectDir || !pendingRun) return;
    const pr = pendingRun;
    setPendingRun(null);
    if (pr.run === "free") submitFreeText(pr.text);
    else if (pr.run === "check") runCheckDraft(pr.text);
    else if (pr.run === "workflow") runWorkflow(pr.text, pr.appendTo);
    else if (pr.run === "coding") runCoding(pr.text, pr.forceAnalyze, pr.appendTo, pr.research);
  }, [projectDir, pendingRun]);
  // Live orchestration runs (module-level store) — feeds the in-thread thinking chain.
  const liveRuns = useSyncExternalStore(runStore.subscribe, runStore.snapshot);
  // Merged send/pause button: while the ACTIVE session's run is in flight, an EMPTY composer's
  // button pauses it (coding runs only — the only path the backend can pause), and a NON-empty
  // one QUEUES the text. queued/paused are keyed to a specific session/run (never global) so a
  // mid-run session switch can't mis-target them.
  const [queued, setQueued] = useState<string | null>(null);
  const [queuedFor, setQueuedFor] = useState<string | null>(null); // the session the queued text belongs to
  const [pausedRids, setPausedRids] = useState<Set<string>>(() => new Set());
  // Auto-send the queued follow-up once ITS session's run finishes — routed to that session (not
  // whichever is open now), keyed on the target task's status (not the imprecise global busy edge).
  useEffect(() => {
    if (queued == null || queuedFor == null) return;
    const target = tasks.find((t) => t.id === queuedFor);
    if (!target) { setQueued(null); setQueuedFor(null); return; }
    if (target.status === "running") return; // still running — keep waiting
    const text = queued;
    setQueued(null); setQueuedFor(null);
    if (target.type === "workflow") runWorkflow(text, target.id);
    else runCoding(text, true);
  }, [queued, queuedFor, tasks]);
  // Live step snapshot: while a run is in flight, mirror its steps into the task (throttled ~1.5s)
  // so a HARD interruption — process killed, window closed, crash — doesn't lose the progress
  // already made. The history then shows how far it got instead of a blank cancelled record.
  // Figures are stripped for the live snapshot (quota); the final r.trace restores them on finish.
  const liveSyncRef = useRef(0);
  useEffect(() => {
    if (!liveRuns.some((r) => r.status === "running")) return;
    const now = Date.now();
    if (now - liveSyncRef.current < 1500) return;
    liveSyncRef.current = now;
    setTasks((ts) => {
      let changed = false;
      const next = ts.map((tk) => {
        if (tk.status !== "running") return tk;
        const run = liveRuns.find((r) => r.id === (tk.liveRunId ?? tk.id));
        if (!run || run.status !== "running") return tk;
        const rich = run.agents.flatMap((a) => a.rich ?? []);
        if (rich.length === 0) return tk;
        changed = true;
        return { ...tk, steps: rich.map(stripLiveFigure) };
      });
      return changed ? next : ts;
    });
  }, [liveRuns]);
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
  const [settingsCat, setSettingsCat] = useState<string | undefined>(undefined);
  const [nav, setNav] = useState<NavKey>("tasks");
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [railOpen, setRailOpen] = useState(localStorage.getItem("axiom3.rail") === "1");
  // Left sidebar can collapse to a slim icon rail (like Claude). Persisted.
  const [sidebarCollapsed, setSidebarCollapsed] = useState(localStorage.getItem("axiom3.sidebarCollapsed") === "1");
  // Draggable panel widths (persisted). Double-click a handle resets to the default.
  const SIDEBAR_DEFAULT = 258, RAIL_DEFAULT = 360;
  const [sidebarW, setSidebarW] = useState(() => Number(localStorage.getItem("axiom3.sidebarW")) || SIDEBAR_DEFAULT);
  const [railW, setRailW] = useState(() => Number(localStorage.getItem("axiom3.railW")) || RAIL_DEFAULT);
  useEffect(() => { localStorage.setItem("axiom3.sidebarW", String(sidebarW)); }, [sidebarW]);
  useEffect(() => { localStorage.setItem("axiom3.railW", String(railW)); }, [railW]);
  function toggleSidebar() {
    setSidebarCollapsed((c) => {
      const n = !c;
      localStorage.setItem("axiom3.sidebarCollapsed", n ? "1" : "0");
      return n;
    });
  }
  // Multiple sessions can be open at once as tabs (an "Open tabs" bar shows when ≥2).
  const [openTabs, setOpenTabs] = useState<string[]>([]);
  // Per-session actions menu (⋯) + inline rename state.
  const [menuFor, setMenuFor] = useState<string | null>(null);
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState("");
  // Artifact pane can expand to fill the window; topbar account menu.
  const [artifactExpanded, setArtifactExpanded] = useState(false);
  const [accountMenu, setAccountMenu] = useState(false);
  // Voice dictation into the composer (Web Speech API, when the webview supports it).
  const [listening, setListening] = useState(false);
  const recognitionRef = useRef<any>(null);
  const [booting, setBooting] = useState(true);
  const [dropping, setDropping] = useState(false);
  const [lastSeenNotif, setLastSeenNotif] = useState(() => {
    const v = localStorage.getItem("axiom3.notifSeen");
    return v ? parseInt(v, 10) || 0 : Date.now(); // first run: don't flag pre-existing tasks
  });

  useEffect(() => {
    // Align with Claude Science, which is a light workbench (warm cream field, white cards,
    // a dark code cell that pops against it). Default to light; the user can still switch in
    // Settings. The v5 migration flips the earlier dark default to light once, then respects
    // the user's choice.
    if (localStorage.getItem("axiom3.themeMigrated") !== "5") {
      localStorage.setItem("axiom3.themeMigrated", "5");
      localStorage.setItem("axiom3.theme", "light");
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
    localStorage.setItem("axiom3.mode", mode);
  }, [taskModel, wfAgents, deepSearch, mode]);

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
    setArtifactExpanded(false);
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


  useEffect(() => {
    const un = listen<{ done: number; total: number }>("check_progress", (e) => setProgress(e.payload));
    return () => {
      un.then((f) => f());
    };
  }, []);

  function note(line: string) {
    setToast(line);
    window.clearTimeout(noteTimer.current);
    noteTimer.current = window.setTimeout(() => setToast(""), 2800);
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
    dropTab(id);
  }

  // ── open-tabs bar: several sessions open at once, like Claude's tab strip ──
  const TAB_CAP = 8;
  function pushTab(id: string) {
    setOpenTabs((prev) => (prev.includes(id) ? prev : [...prev, id].slice(-TAB_CAP)));
  }
  // Open a session: surface it as a tab and show its thread.
  function openTask(id: string) {
    pushTab(id);
    setNav("tasks");
    setActiveId(id);
    setView("thread");
  }
  // Remove a tab (and, if it was active, fall back to a neighbour or home).
  function dropTab(id: string) {
    setOpenTabs((prev) => {
      const next = prev.filter((x) => x !== id);
      if (activeId === id) {
        const fallback = next[next.length - 1] ?? null;
        setActiveId(fallback);
        setView(fallback ? "thread" : "home");
      }
      return next;
    });
  }

  // ── rename a session (a title override that persists in the task history) ──
  function startRename(tk: Task) {
    setRenamingId(tk.id);
    setRenameValue(tk.title);
    setMenuFor(null);
  }
  function commitRename() {
    const id = renamingId;
    if (!id) return;
    const title = renameValue.trim();
    if (title) setTasks((ts) => ts.map((x) => (x.id === id ? { ...x, title } : x)));
    setRenamingId(null);
  }

  // ── voice dictation into the composer (webkitSpeechRecognition when available) ──
  const speechSupported = typeof window !== "undefined" &&
    !!((window as any).webkitSpeechRecognition || (window as any).SpeechRecognition);
  function toggleDictation() {
    if (!speechSupported) return;
    if (listening) {
      recognitionRef.current?.stop();
      return;
    }
    const Ctor = (window as any).webkitSpeechRecognition || (window as any).SpeechRecognition;
    const rec = new Ctor();
    rec.lang = lang === "zh" ? "zh-CN" : "en-US";
    rec.interimResults = false;
    rec.continuous = false;
    rec.onresult = (ev: any) => {
      const text = Array.from(ev.results).map((r: any) => r[0].transcript).join(" ").trim();
      if (text) setSlash((s) => (s ? `${s} ${text}` : text));
    };
    rec.onend = () => setListening(false);
    rec.onerror = () => setListening(false);
    recognitionRef.current = rec;
    setListening(true);
    try {
      rec.start();
    } catch {
      setListening(false);
    }
  }

  // Start a fresh task: clear the composer and show the home greeting.
  // A new session must first pick a project directory — open that dialog (default to the last dir).
  function newTask() {
    setPickingDir(projectDir);
    setNewSessionOpen(true);
  }
  async function pickProjectDir() {
    const dir = await open({ directory: true, multiple: false });
    if (typeof dir === "string") setPickingDir(dir);
  }
  // Abandon the new-session dialog. Crucially clears any pendingRun stashed by the project
  // gate — otherwise a cancelled "type → gate → cancel" would silently fire the next time a
  // project dir gets set. (confirmNewSession deliberately does NOT clear it: it re-fires it.)
  function closeNewSession() {
    setNewSessionOpen(false);
    setPendingRun(null);
  }
  // Confirm the project directory, then start a fresh session anchored to it.
  function confirmNewSession() {
    const dir = pickingDir;
    if (!dir) return;
    setProjectDir(dir);
    localStorage.setItem("axiom3.projectDir", dir);
    setCodeWs(dir); // the agent works in the project directory
    invoke<ProjectStatus>("open_project", { name: projectName(dir) }).then(setStatus).catch(() => {});
    setNewSessionOpen(false);
    setNav("tasks");
    setView("home");
    setActiveId(null);
    setCmd(null);
    setSlash("");
    setDraft("");
    // Keep `references`: it's the user's evidence corpus, not per-task scratch.
  }

  // Time-of-day greeting for the home screen.
  function greeting(): string {
    const h = new Date().getHours();
    if (lang === "zh") return h < 5 ? "夜深了" : h < 11 ? "早上好" : h < 13 ? "中午好" : h < 18 ? "下午好" : "晚上好";
    return h < 5 ? "Still up?" : h < 12 ? "Good morning" : h < 18 ? "Good afternoon" : "Good evening";
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
    if (!projectDir) { setPendingRun({ run: "check", text: useDraft }); newTask(); return; }
    const proj = await ensureProject();
    if (!proj) return;
    const id = `t${Date.now()}`;
    const title = useDraft.slice(0, 42) + (useDraft.length > 42 ? "…" : "");
    setTasks((ts) => [{ id, type: "check_draft", title, status: "running", ts: Date.now(), goal: useDraft, forkedFrom: forkOrigin ?? undefined }, ...ts]);
    setForkOrigin(null);
    setActiveId(id);
    pushTab(id);
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
    if (!projectDir) { setPendingRun({ run: "workflow", text: g, appendTo }); newTask(); return; }
    const proj = await ensureProject();
    if (!proj) return;
    const runid = `run-${crypto.randomUUID()}`;
    const sessionId = appendTo ?? runid;
    const title = g.slice(0, 42) + (g.length > 42 ? "…" : "");
    const priorSynth = appendTo ? tasks.find((x) => x.id === appendTo)?.synthesis : undefined; // continue prior research
    if (appendTo) {
      // Point the live chain at THIS run (a fresh runid ≠ the session id) and drop the prior
      // version's steps so the stale v1 log isn't shown as if it were the live v2 stream.
      setTasks((ts) => ts.map((x) => (x.id === appendTo ? { ...x, status: "running", liveRunId: runid, steps: undefined } : x)));
      setActiveId(appendTo);
      pushTab(appendTo);
    } else {
      setTasks((ts) => [{ id: runid, type: "workflow", title, status: "running", ts: Date.now(), goal: g, deep: mode === "research", forkedFrom: forkOrigin ?? undefined, liveRunId: runid }, ...ts]);
      setForkOrigin(null);
      setActiveId(runid);
      pushTab(runid);
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
    }>(() => invoke("run_workflow", { goal: g, runid, model: taskModel || undefined, agents: wfAgents || undefined, search: mode === "research", analyze: mode === "analysis" || codeAttachments.length > 0, attachments: codeAttachments, mode, skill: pendingSkill || sessionCfg.specialist || undefined, prior: priorSynth || undefined, compute: sessionCfg.compute }));
    if (!appendTo) { setCodeAttachments([]); setPendingSkill(null); }
    const steps = r?.steps?.map((s) => ({ role: s.spec.role, status: s.status, output: s.output ?? "", model: s.spec.model }));
    setTasks((ts) =>
      ts.map((x) => {
        if (x.id !== sessionId) return x;
        if (!r) return { ...x, status: "failed" as const, error: "failed" };
        const status = r.status === "done" ? "done" : r.status === "cancelled" ? "cancelled" : "failed";
        // Keep the live-snapshotted steps if this run returned none (e.g. a cancel with no trace).
        const st = steps && steps.length ? steps : x.steps;
        const newVer = { synthesis: r.synthesis ?? undefined, review: r.review ?? undefined, steps: st, ts: Date.now(), goal: g };
        if (appendTo) {
          // seed v1 from the pre-iteration state, then append this run as the newest version.
          const prior = x.versions ?? [{ synthesis: x.synthesis, review: x.review, steps: x.steps, ts: x.ts, goal: x.goal ?? "" }];
          return { ...x, status, synthesis: newVer.synthesis, review: newVer.review, steps: st, goal: g, versions: [...prior, newVer] };
        }
        return { ...x, status, synthesis: newVer.synthesis, review: newVer.review, steps: st };
      })
    );
    if (r?.synthesis && r.status !== "cancelled") generateRecap(sessionId, g, r.synthesis);
  }

  // After a task produces a report, generate the chat-style wrap-up (what was done + key findings +
  // suggested next steps) shown in the thread — the full report stays in the artifact panel. Runs
  // AFTER the task is already "done" (non-blocking): the recap pops in a few seconds later. Falls back
  // to the report's own opening lines if the model call fails, so the thread always reads conversationally.
  async function generateRecap(sessionId: string, goal: string, report: string) {
    const rep = (report || "").trim();
    if (!rep) return;
    setTasks((ts) => ts.map((x) => (x.id === sessionId ? { ...x, recapPending: true } : x)));
    let recap = "";
    try {
      recap = await invoke<string>("recap_run", { goal, report: rep, model: taskModel || undefined });
    } catch {
      recap = "";
    }
    if (!recap.trim()) {
      // Fallback must NEVER be empty (else the thread shows nothing): strip headers + figure/code
      // blocks for a prose lead-in, but if that leaves nothing (a figure-heavy report), fall back to
      // the raw opening, and finally to a generic line — so a wrap-up always appears.
      const prose = rep.replace(/^#+\s.*$/gm, "").replace(/```[\s\S]*?```/g, "").trim();
      const lead = prose.split(/\n\n/).filter((p) => p.trim().length > 0).slice(0, 2).join("\n\n").slice(0, 600);
      recap = lead || rep.replace(/```[\s\S]*?```/g, "").trim().slice(0, 500)
        || (lang === "zh" ? "本次分析已完成,完整结果(含图表)见右侧「产物」面板。" : "Analysis complete — see the full result and figures in the Artifact panel.");
    }
    setTasks((ts) => ts.map((x) => (x.id === sessionId ? { ...x, recap, recapPending: false } : x)));
  }

  // Read-only coding agent over a chosen workspace folder. Streams into the rail.
  async function pickWorkspace() {
    const dir = await open({ directory: true, multiple: false });
    if (typeof dir === "string") setCodeWs(dir);
  }
  async function pickDataFiles() {
    const sel = await open({
      multiple: true,
      filters: [{ name: lang === "zh" ? "数据 / 文档" : "Data / docs", extensions: ["csv", "tsv", "json", "txt", "md", "xlsx", "xls", "parquet", "npy", "npz", "pdf", "docx"] }],
    });
    const paths = Array.isArray(sel) ? sel : typeof sel === "string" ? [sel] : [];
    if (paths.length) setCodeAttachments((a) => [...new Set([...a, ...paths])]);
  }
  // `appendTo` = a follow-up in the SAME analysis session → continue the research (prior findings
  // fed back in) and append the result as a new version (v2, v3 …), for multi-round deep research.
  async function runCoding(goal: string, forceAnalyze = false, appendTo?: string, research = false) {
    const g = goal.trim();
    if (!g) return;
    if (!projectDir) { setPendingRun({ run: "coding", text: g, forceAnalyze, appendTo, research }); newTask(); return; }
    const doAnalyze = forceAnalyze || codeAnalyze || cmd === "analyze" || research; // research implies a kernel
    if (!codeWs && !doAnalyze) return; // code mode needs a folder; analyze/research can use a scratch dir
    const runid = `run-${crypto.randomUUID()}`;
    const sessionId = appendTo ?? runid;
    const title = g.slice(0, 42) + (g.length > 42 ? "…" : "");
    // Prior round's findings → carried into the run so the agent continues the investigation.
    const prior = appendTo ? tasks.find((x) => x.id === appendTo)?.synthesis : undefined;
    if (appendTo) {
      setTasks((ts) => ts.map((x) => (x.id === appendTo ? { ...x, status: "running", liveRunId: runid, steps: undefined } : x)));
      setActiveId(appendTo);
      pushTab(appendTo);
    } else {
      setTasks((ts) => [{ id: runid, type: research ? "research" : "code", title, status: "running", ts: Date.now(), goal: g, deep: research || undefined, forkedFrom: forkOrigin ?? undefined, liveRunId: runid }, ...ts]);
      setForkOrigin(null);
      setActiveId(runid);
      pushTab(runid);
    }
    setNav("tasks");
    setView("thread");
    setCmd(null);
    setSlash("");
    setRailOpen(true);
    localStorage.setItem("axiom3.rail", "1");
    const r = await run<{ status: string; synthesis: string; review?: string; sources?: CiteSource[]; trace?: AgentStepView[] }>(() =>
      invoke("coding_run", { goal: g, runid, workspace: codeWs || "", writable: codeWritable, analyze: doAnalyze, attachments: codeAttachments, model: taskModel || undefined, skill: pendingSkill || sessionCfg.specialist || undefined, prior: prior || undefined, compute: sessionCfg.compute, research })
    );
    if (!appendTo) { setCodeAttachments([]); }
    setPendingSkill(null);
    setTasks((ts) =>
      ts.map((x) => {
        if (x.id !== sessionId) return x;
        if (!r) return { ...x, status: "failed" as const, error: "failed" };
        const status = r.status === "done" ? "done" : r.status === "cancelled" ? "cancelled" : "failed";
        // Prefer the backend's full trace (it carries the figures); but if it came back empty
        // (e.g. a cancel that returned no trace), keep the steps we snapshotted live so a
        // cancelled run still shows how far it got instead of blanking out.
        const steps = (r.trace && r.trace.length) ? r.trace : (x.steps ?? []);
        if (appendTo) {
          const prior = x.versions ?? [{ synthesis: x.synthesis, review: x.review, steps: x.steps, ts: x.ts, goal: x.goal ?? "" }];
          const newVer = { synthesis: r.synthesis, review: r.review ?? undefined, steps, ts: Date.now(), goal: g };
          return { ...x, status, synthesis: r.synthesis, review: r.review ?? undefined, sources: r.sources, steps, goal: g, versions: [...prior, newVer] };
        }
        return { ...x, status, synthesis: r.synthesis, review: r.review ?? undefined, sources: r.sources, steps };
      })
    );
    if (r?.synthesis && r.status !== "cancelled") generateRecap(sessionId, g, r.synthesis);
  }

  async function cancelCheck() {
    await invoke("cancel_check");
  }

  async function exportProvenance() {
    const at = activeTask;
    // Code/workflow artifacts have no draft-check hash-chain (latest_provenance() would error);
    // export the ARTIFACT's own provenance instead — goal, every step + its verbatim code, the
    // synthesis, the reviewer's findings, and version history — as a self-contained JSON.
    if (at && (at.type === "code" || at.type === "workflow" || at.type === "research")) {
      const base = (at.title || "artifact").slice(0, 40).replace(/[^\p{L}\p{N} _-]/gu, "").trim() || "artifact";
      const path = await save({ defaultPath: `${base}-provenance.json`, filters: [{ name: "JSON", extensions: ["json"] }] });
      if (typeof path !== "string") return;
      const payload = JSON.stringify(
        { title: at.title, type: at.type, goal: at.goal, status: at.status, ts: at.ts, synthesis: at.synthesis, review: at.review, steps: at.steps, versions: at.versions },
        null, 2
      );
      const bytes = Array.from(new TextEncoder().encode(payload));
      const ok = await run(() => invoke("save_png", { path, bytes }));
      if (ok !== undefined) note(`${t(lang, "exported")}: ${path.split(/[/\\]/).pop()} · ${at.steps?.length ?? 0} ${lang === "zh" ? "步" : "steps"}`);
      return;
    }
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
  // Save a self-contained report HTML (built by ArtifactPane from the rendered artifact).
  async function saveReport(html: string, defaultName: string) {
    const path = await save({
      defaultPath: `${defaultName || "report"}.html`,
      filters: [{ name: "HTML", extensions: ["html"] }],
    });
    if (typeof path === "string") {
      const ok = await run(() => invoke("save_report_html", { path, html }));
      if (ok !== undefined) note(`${lang === "zh" ? "已导出报告" : "Report exported"}: ${path.split(/[/\\]/).pop()}`);
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
    (activeTask.type === "workflow" || activeTask.type === "code" || activeTask.type === "research") &&
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
  // Selecting a `/` command. `/技能` opens the skill picker instead of a sub-composer.
  function pickCommand(key: string) {
    if (key === "skills") {
      setSkillMenu(true);
      setSlash("");
      return;
    }
    setCmd(key);
    setSlash("");
  }
  // Picking a skill: prefill a directive and switch to 分析 mode; the analysis agent loads it.
  function pickSkill(s: (typeof SKILLS)[number]) {
    setSkillMenu(false);
    setMode("analysis");
    setPendingSkill(s.key); // the backend injects this skill's card into the run
    const label = lang === "zh" ? s.zh : s.en;
    setSlash(lang === "zh" ? `用「${label}」技能分析:` : `Analyze with the ${label} skill: `);
  }

  function submitFreeText(textArg?: string): boolean {
    const text = (textArg ?? slash).trim();
    if (!text || text.startsWith("/")) return false;
    // A new session must pick a project directory first. Stash this run, open the picker, and
    // re-fire (with the preserved text) once projectDir is set — see the pendingRun effect.
    if (!projectDir) {
      setPendingRun({ run: "free", text });
      newTask();
      return false;
    }
    const at = tasks.find((x) => x.id === activeId);
    // Follow-ups continue in the SAME mode: research→deep tool agent, code/analysis→coding, old
    // multi-agent workflow sessions→workflow.
    if (at?.type === "research") runCoding(text, true, at.id, true); // deep-research follow-up
    else if (at?.type === "workflow") runWorkflow(text, at.id); // legacy multi-agent session (v2…)
    else if (at?.type === "code") runCoding(text, true, at.id); // analysis follow-up (build on prior)
    // TWO MODES (the Opus/Haiku split):
    // 分析 (Haiku tier) = single agent + Python kernel, budget-controlled, YOUR local data — code-first
    //   so every number is computed, not asserted.
    // 研究 (Opus tier) = the SAME rich tool agent but NO budget, literature/web-first (not local data),
    //   steered to a deep, cited, high-value report (backend research=true; multi-agent delegation next).
    else if (mode === "analysis") runCoding(text, true);
    else if (mode === "research") runCoding(text, true, undefined, true);
    else if (codeAttachments.length > 0) runCoding(text, true); // attached data → analyze it locally
    else runCheckDraft(text);
    return true;
  }

  // The slash composer — shared by the home screen and thread follow-ups. Typing "/"
  // surfaces task types; picking one swaps in that task's inputs (only /校对 runs today).
  function composerBlock() {
    return (
      <>
      {/* running indicator sits right above the input box (renders nothing when idle) */}
      <RunStatusPill lang={lang} />
      <div className="ax-composer">
        {cmd === null ? (
          <>
            <textarea
              className="ax-textarea ax-textarea--grow"
              ref={slashRef}
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
                    if (c && c.active) pickCommand(c.key);
                  } else if (e.key === "Escape") {
                    setSlash("");
                  }
                } else if (e.key === "Enter" && !e.shiftKey) {
                  e.preventDefault();
                  submitFreeText();
                }
              }}
              rows={1}
              placeholder={t(lang, "composer_ph")}
            />
            {(codeAttachments.length > 0 || pendingSkill || sessionCfg.specialist || (queued && queuedFor === activeId)) && (
              <div className="ax-toolrow" style={{ flexWrap: "wrap", padding: "2px 6px 0" }}>
                {queued && queuedFor === activeId && (
                  <span className="ax-chip ax-chip--queued" title={lang === "zh" ? "已排队 · 上次运行完成后自动发送" : "queued — auto-sends after the current run"}>
                    ⏳ {queued.slice(0, 32)}{queued.length > 32 ? "…" : ""}
                    <button className="ax-chip__x" onClick={() => { setQueued(null); setQueuedFor(null); }} aria-label="remove queued">×</button>
                  </span>
                )}
                {pendingSkill && (
                  <span className="ax-chip ax-chip--skill" title={lang === "zh" ? "本次运行将使用此技能" : "this skill will guide the run"}>
                    🧭 {(() => { const s = SKILLS.find((x) => x.key === pendingSkill); return s ? (lang === "zh" ? s.zh : s.en) : pendingSkill; })()}
                    <button className="ax-chip__x" onClick={() => setPendingSkill(null)} aria-label="remove skill">×</button>
                  </span>
                )}
                {/* the persistent Specialist from session settings — shown so it's never a silent
                    prompt override; an explicit /技能 pick (pendingSkill) takes priority over it. */}
                {!pendingSkill && sessionCfg.specialist && (
                  <span className="ax-chip ax-chip--skill" title={lang === "zh" ? "会话设置中的专家 · 每次运行都会启用(点 × 关闭)" : "Specialist from settings — applied to every run"}>
                    🧭 {(() => { const s = SKILLS.find((x) => x.key === sessionCfg.specialist); return s ? (lang === "zh" ? s.zh : s.en) : sessionCfg.specialist; })()}
                    <button className="ax-chip__x" onClick={() => setSessionCfg((c) => ({ ...c, specialist: "" }))} aria-label="remove specialist">×</button>
                  </span>
                )}
                {codeAttachments.map((p) => {
                  const name = p.split(/[/\\]/).pop() || p;
                  return (
                    <span key={p} className="ax-chip" title={p}>
                      {name}
                      <button className="ax-chip__x" onClick={() => setCodeAttachments((a) => a.filter((x) => x !== p))} aria-label="remove">×</button>
                    </span>
                  );
                })}
              </div>
            )}
            <div className="ax-toolrow" style={{ justifyContent: "space-between" }}>
              <span className="ax-composer__tools">
                <button className="ax-iconbtn" onClick={pickDataFiles} title={lang === "zh" ? "附加数据 / 文档" : "Attach data / docs"} aria-label="attach">
                  <Paperclip size={15} aria-hidden />
                </button>
                {/* session settings popover — between attach and voice, per the reference design */}
                <SessionSettings cfg={sessionCfg} onChange={setSessionCfg} lang={lang} skills={SKILLS} models={MODELS.map((m) => m.label)} />
                <button
                  className={`ax-iconbtn ${listening ? "is-active" : ""}`}
                  onClick={toggleDictation}
                  disabled={!speechSupported}
                  title={
                    !speechSupported
                      ? (lang === "zh" ? "此环境不支持语音输入" : "Voice input not available here")
                      : listening
                      ? (lang === "zh" ? "停止听写" : "Stop dictation")
                      : (lang === "zh" ? "语音输入" : "Voice input")
                  }
                  aria-label={lang === "zh" ? "语音输入" : "Voice input"}
                >
                  <Mic size={15} aria-hidden />
                </button>
              </span>
              {/* Mode picker: a compact icon button that expands to choose 自动/研究/分析
                  (Claude-style). The backend chooses thinking vs non-thinking per mode. */}
              <span style={{ display: "flex", alignItems: "center", gap: 8 }}>
                <ContextWindow
                  run={liveRuns.find((r) => r.id === (activeTask?.liveRunId ?? activeTask?.id))}
                  specialist={!!sessionCfg.specialist}
                  memory={sessionCfg.memory}
                  lang={lang}
                />
                <div className="ax-modepick">
                  {(() => {
                    const cur = MODES.find((m) => m.k === mode) ?? MODES[0];
                    return (
                      <button
                        className={`ax-modepick__btn is-icon ${modeMenuOpen ? "on" : ""}`}
                        onClick={() => setModeMenuOpen((o) => !o)}
                        aria-haspopup="listbox"
                        aria-expanded={modeMenuOpen}
                        title={`${lang === "zh" ? "模式" : "Mode"} · ${lang === "zh" ? cur.zh : cur.en}`}
                        aria-label={`${lang === "zh" ? "模式" : "Mode"}: ${lang === "zh" ? cur.zh : cur.en}`}
                      >
                        <cur.Icon size={16} aria-hidden />
                      </button>
                    );
                  })()}
                  {modeMenuOpen && (
                    <>
                      <div className="ax-modepick__scrim" onClick={() => setModeMenuOpen(false)} />
                      <div className="ax-modepick__menu" role="listbox" aria-label={lang === "zh" ? "模式" : "Mode"}>
                        {MODES.map((m) => (
                          <button
                            key={m.k}
                            role="option"
                            aria-selected={mode === m.k}
                            className={`ax-modepick__opt ${mode === m.k ? "on" : ""}`}
                            onClick={() => { setMode(m.k); setModeMenuOpen(false); }}
                          >
                            <m.Icon size={15} aria-hidden />
                            <span className="ax-modepick__txt">
                              <b>{lang === "zh" ? m.zh : m.en}</b>
                              <span>{lang === "zh" ? m.tzh : m.ten}</span>
                            </span>
                            {mode === m.k && <Check size={14} aria-hidden style={{ marginLeft: "auto" }} />}
                          </button>
                        ))}
                      </div>
                    </>
                  )}
                </div>
                {/* merged send / pause button: send → (while running) pause; typing + send while
                    running queues the text to auto-send when the current run finishes. */}
                {(() => {
                  const hasText = slash.trim().length > 0;
                  const activeRunning = activeTask?.status === "running";
                  // coding AND workflow runs are pausable — the engine holds each agent at its next
                  // turn while paused (cancel still wins); resume picks up where it left off.
                  const runningCode = activeRunning && (activeTask?.type === "code" || activeTask?.type === "workflow" || activeTask?.type === "research");
                  if (runningCode && !hasText) {
                    const rid = activeTask!.liveRunId ?? activeTask!.id;
                    const isPaused = pausedRids.has(rid);
                    return (
                      <button
                        className={`ax-send ax-send--run ${isPaused ? "is-paused" : ""}`}
                        onClick={() => {
                          if (isPaused) { invoke("resume_run", { runid: rid }).catch(() => {}); setPausedRids((s) => { const n = new Set(s); n.delete(rid); return n; }); }
                          else { invoke("pause_run", { runid: rid }).catch(() => {}); setPausedRids((s) => new Set(s).add(rid)); }
                        }}
                        title={isPaused ? (lang === "zh" ? "继续" : "Resume") : (lang === "zh" ? "暂停" : "Pause")}
                        aria-label={isPaused ? "resume" : "pause"}
                      >
                        {isPaused ? <Play size={15} aria-hidden /> : <Pause size={15} aria-hidden />}
                      </button>
                    );
                  }
                  return (
                    <button
                      className="ax-send"
                      onClick={() => { const txt = slash.trim(); if (!txt) return; if (activeRunning) { setQueued(txt); setQueuedFor(activeTask?.id ?? null); setSlash(""); } else submitFreeText(); }}
                      disabled={!hasText || slashOpen}
                      title={activeRunning ? (lang === "zh" ? "排队 · 上次完成后自动发送" : "Queue — sends after the current run") : t(lang, "task_run")}
                      aria-label={t(lang, "task_run")}
                    >
                      <ArrowUp size={16} aria-hidden />
                    </button>
                  );
                })()}
              </span>
            </div>
            {slashOpen && slashMatches.length > 0 && (
              <div className="ax-slash-menu">
                {slashMatches.map((c, i) => (
                  <button
                    key={c.key}
                    className={`ax-slash-item ${i === Math.min(slashIdx, slashMatches.length - 1) ? "is-active" : ""}`}
                    disabled={!c.active}
                    onMouseEnter={() => setSlashIdx(i)}
                    onClick={() => pickCommand(c.key)}
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
            {skillMenu && (
              <div className="ax-slash-menu">
                <div className="ax-slash-head">{lang === "zh" ? "选择技能" : "Choose a skill"}</div>
                {SKILLS.map((s) => (
                  <button key={s.key} className="ax-slash-item" onClick={() => pickSkill(s)}>
                    <span className="ax-slash-cmd">{lang === "zh" ? s.zh : s.en}</span>
                    <span className="ax-slash-desc">{lang === "zh" ? s.zhh : s.enh}</span>
                  </button>
                ))}
                <button className="ax-slash-item" onClick={() => setSkillMenu(false)}>
                  <span className="ax-slash-desc">{lang === "zh" ? "取消" : "Cancel"}</span>
                </button>
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
                  <select className="ax-select ax-select--sm" value={taskModel} onChange={(e) => setTaskModel(e.currentTarget.value)} title={lang === "zh" ? "模型" : "Model"}>
                    {MODELS.map((m) => (
                      <option key={m.id} value={m.id}>{m.label}</option>
                    ))}
                  </select>
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
                  <button
                    className={`ax-toggle ${codeAnalyze ? "on" : ""}`}
                    onClick={() => setCodeAnalyze(!codeAnalyze)}
                    title={lang === "zh" ? "综合后追加一个计算阶段:分析智能体在常驻内核里真跑代码,把关键图由真实数据算出并入报告" : "After synthesis, add a compute phase: the analysis agent runs code in a live kernel and computes the key figures from real data into the report"}
                  >
                    🔬 {lang === "zh" ? "计算图" : "compute figures"}
                  </button>
                </div>
                {codeAnalyze && (
                  <div className="ax-toolrow" style={{ flexWrap: "wrap", marginTop: 6 }}>
                    <button className="ax-btn" onClick={pickDataFiles}>{lang === "zh" ? "＋ 附加数据" : "＋ Attach data"}</button>
                    {codeAttachments.map((p) => {
                      const name = p.split("/").pop() || p;
                      return (
                        <span key={p} className="ax-chip" title={p}>
                          {name}
                          <button className="ax-chip__x" onClick={() => setCodeAttachments((a) => a.filter((x) => x !== p))} aria-label="remove">×</button>
                        </span>
                      );
                    })}
                  </div>
                )}
              </>
            ) : cmd === "code" || cmd === "analyze" ? (
              <>
                {cmd === "analyze" && (
                  <p className="ax-hint" style={{ marginBottom: 8 }}>
                    {lang === "zh"
                      ? "智能体会写 Python、在常驻内核里真跑,统计与图表由真实数据算出并入报告。附上数据即可,无需选目录。"
                      : "The agent writes Python, runs it in a live kernel, and computes the stats + figures from real data into the report. Just attach data — no folder needed."}
                  </p>
                )}
                <div className="ax-toolrow">
                  <button className="ax-btn" onClick={pickWorkspace}>
                    {codeWs ? (lang === "zh" ? "更换目录" : "Change folder") : cmd === "analyze" ? (lang === "zh" ? "选择工作目录(可选)" : "Choose folder (optional)") : (lang === "zh" ? "选择工作目录" : "Choose workspace")}
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
                  placeholder={codeAnalyze || cmd === "analyze"
                    ? (lang === "zh" ? "要分析什么(附上数据,智能体会真跑代码并算出图)…" : "What to analyze (attach data — the agent runs code and computes the figures)…")
                    : (lang === "zh" ? "要分析的任务(只读:读取 / 搜索 / 提议改动)…" : "What to analyze (read-only: read / search / propose)…")}
                />
                {(codeAnalyze || cmd === "analyze") && (
                  <div className="ax-toolrow" style={{ flexWrap: "wrap" }}>
                    <button className="ax-btn" onClick={pickDataFiles}>
                      {lang === "zh" ? "＋ 附加数据" : "＋ Attach data"}
                    </button>
                    {codeAttachments.map((p) => {
                      const name = p.split("/").pop() || p;
                      return (
                        <span key={p} className="ax-chip" title={p}>
                          {name}
                          <button className="ax-chip__x" onClick={() => setCodeAttachments((a) => a.filter((x) => x !== p))} aria-label="remove">×</button>
                        </span>
                      );
                    })}
                  </div>
                )}
                <div className="ax-toolrow">
                  <button className="ax-btn ax-btn--primary" onClick={() => runCoding(draft)} disabled={busy || !draft.trim() || (!codeWs && !codeAnalyze && cmd !== "analyze")}>
                    {t(lang, "task_run")}
                  </button>
                  {cmd !== "analyze" && (
                  <button
                    className={`ax-toggle ${codeWritable ? "on" : ""}`}
                    onClick={() => setCodeWritable(!codeWritable)}
                    title={codeWritable ? (lang === "zh" ? "可写入文件并运行构建/测试" : "can write files + run build/test") : (lang === "zh" ? "只读分析" : "read-only")}
                  >
                    {codeWritable ? (lang === "zh" ? "写入 · 构建" : "write · build") : (lang === "zh" ? "只读" : "read-only")}
                  </button>
                  )}
                  {cmd !== "analyze" && (
                  <button
                    className={`ax-toggle ${codeAnalyze ? "on" : ""}`}
                    onClick={() => setCodeAnalyze(!codeAnalyze)}
                    title={lang === "zh" ? "给智能体一个常驻 Python 内核:它写代码→内核真跑→图由真实数据算出并入报告" : "give the agent a live Python kernel: it writes code, the kernel runs it, figures are computed from real data into the report"}
                  >
                    {lang === "zh" ? "🔬 分析(运行代码)" : "🔬 analyze (run code)"}
                  </button>
                  )}
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
      {/* project name + path, tight under the composer (home AND thread), so the working
          directory is always visible once a session is anchored to one. */}
      {projectDir && (
        <p className="ax-proj-line" title={projectDir}>
          <FolderOpen size={12} aria-hidden />
          <b>{projectName(projectDir)}</b>
          <span>{projectDir}</span>
        </p>
      )}
      </>
    );
  }

  return (
    <>
      {booting && <Splash lang={lang} onDone={() => setBooting(false)} />}
      {!onboarded && <OnboardingWizard lang={lang} onDone={() => setOnboarded(true)} />}
      {settingsOpen && <SettingsModal lang={lang} onLang={setLang} initialCat={settingsCat} onClose={() => setSettingsOpen(false)} />}
      {paletteOpen && (
        <CommandPalette lang={lang} items={paletteItems} onPick={onPalettePick} onClose={() => setPaletteOpen(false)} />
      )}
      {/* New session must pick a project directory first (Claude-style). */}
      {newSessionOpen && (
        <div className="ax-set-overlay" onClick={closeNewSession}>
          <div className="ax-newsess" onClick={(e) => e.stopPropagation()}>
            <p className="ax-newsess__title">{lang === "zh" ? "新建会话 · 选择项目目录" : "New session · pick a project folder"}</p>
            <p className="ax-newsess__hint">{lang === "zh" ? "会话在这个目录里工作:数据、产物、代码都在此。" : "The session works inside this folder — its data, artifacts, and code live here."}</p>
            <button className="ax-newsess__pick" onClick={pickProjectDir}>
              <FolderOpen size={16} aria-hidden />
              {pickingDir ? (
                <span className="ax-newsess__path">
                  <b>{projectName(pickingDir)}</b>
                  <span>{pickingDir}</span>
                </span>
              ) : (
                <span>{lang === "zh" ? "选择文件夹…" : "Choose a folder…"}</span>
              )}
            </button>
            <div className="ax-newsess__actions">
              <button className="ax-btn" onClick={closeNewSession}>{lang === "zh" ? "取消" : "Cancel"}</button>
              <button className="ax-btn ax-btn--primary" onClick={confirmNewSession} disabled={!pickingDir}>
                {lang === "zh" ? "开始" : "Start"}
              </button>
            </div>
          </div>
        </div>
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
      {toast && <div className="ax-toast" role="status">{toast}</div>}
      <div
        className={`ax-app ${railOpen ? "has-rail" : ""} ${sidebarCollapsed ? "is-collapsed" : ""}`}
        style={{
          gridTemplateColumns: railOpen
            ? `${sidebarCollapsed ? 60 : sidebarW}px 1fr ${railW}px`
            : `${sidebarCollapsed ? 60 : sidebarW}px 1fr`,
        }}
      >
        {/* drag the boundaries to resize the sidebar / right panel (double-click resets) */}
        {!sidebarCollapsed && (
          <ResizeHandle
            style={{ left: sidebarW - 3 }}
            onResize={(x) => setSidebarW(Math.max(190, Math.min(460, x, window.innerWidth - (railOpen ? railW : 0) - 360)))}
            onReset={() => setSidebarW(SIDEBAR_DEFAULT)}
          />
        )}
        {railOpen && !artifactExpanded && (
          <ResizeHandle
            className="ax-resize--rail"
            style={{ right: railW - 3 }}
            onResize={(x) => setRailW(Math.max(300, Math.min(760, window.innerWidth - x, window.innerWidth - (sidebarCollapsed ? 60 : sidebarW) - 360)))}
            onReset={() => setRailW(RAIL_DEFAULT)}
          />
        )}
      <aside className="ax-sidebar">
        <div className="ax-side-top">
          {!sidebarCollapsed && <div className="ax-brand" style={{ fontSize: 19 }}>Axiom3</div>}
          <button
            className="ax-iconbtn ax-side-collapse"
            onClick={toggleSidebar}
            title={sidebarCollapsed ? (lang === "zh" ? "展开侧栏" : "Expand sidebar") : (lang === "zh" ? "折叠侧栏" : "Collapse sidebar")}
            aria-label={sidebarCollapsed ? (lang === "zh" ? "展开侧栏" : "Expand sidebar") : (lang === "zh" ? "折叠侧栏" : "Collapse sidebar")}
          >
            <PanelLeft size={16} aria-hidden />
          </button>
        </div>

        <button className="ax-side-new" onClick={newTask} title={lang === "zh" ? "新建研究会话" : "New session"}>
          <Plus size={16} aria-hidden /> {!sidebarCollapsed && (lang === "zh" ? "新建研究会话" : "New session")}
        </button>

        {!sidebarCollapsed && tasks.length > 4 && (
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
          {sidebarCollapsed ? null : tasks.length === 0 ? (
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
                      {renamingId === tk.id ? (
                        <input
                          className="ax-recent__rename"
                          autoFocus
                          value={renameValue}
                          onChange={(e) => setRenameValue(e.currentTarget.value)}
                          onBlur={commitRename}
                          onKeyDown={(e) => {
                            if (e.key === "Enter") { e.preventDefault(); commitRename(); }
                            else if (e.key === "Escape") { e.preventDefault(); setRenamingId(null); }
                          }}
                        />
                      ) : (
                        <button
                          className="ax-recent__main"
                          title={`${tk.title} · ${t(lang, `st_${tk.status}`)}`}
                          onClick={() => openTask(tk.id)}
                          onDoubleClick={() => startRename(tk)}
                        >
                          <span className="ax-recent__title">
                            <span className={`ax-lamp ${tk.status === "done" ? "is-green" : tk.status === "running" ? "is-amber" : "is-neutral"}`} />
                            {tk.forkedFrom && <span className="ax-fork-mark" title={lang === "zh" ? "由其他会话分叉" : "forked from another session"}>↳</span>}
                            <b>{tk.title}</b>
                            {tk.deep && <span className="ax-recent__deep" title={lang === "zh" ? "深度搜索(AB-MCTS)" : "deep search (AB-MCTS)"}>深</span>}
                          </span>
                        </button>
                      )}
                      <button
                        className="ax-recent__del"
                        title={lang === "zh" ? "会话操作" : "Session actions"}
                        aria-label={lang === "zh" ? "会话操作" : "Session actions"}
                        onClick={() => setMenuFor((m) => (m === tk.id ? null : tk.id))}
                      >
                        <MoreHorizontal size={14} aria-hidden />
                      </button>
                      {menuFor === tk.id && (
                        <>
                          <div className="ax-menu-scrim" onClick={() => setMenuFor(null)} />
                          <div className="ax-menu" role="menu">
                            <button className="ax-menu__item" role="menuitem" onClick={() => startRename(tk)}>
                              <Pencil size={13} aria-hidden /> {lang === "zh" ? "重命名" : "Rename"}
                            </button>
                            {tk.goal && (
                              <button className="ax-menu__item" role="menuitem" onClick={() => { forkSession(tk); setMenuFor(null); }}>
                                <GitBranch size={13} aria-hidden /> {lang === "zh" ? "分叉会话" : "Fork session"}
                              </button>
                            )}
                            <button className="ax-menu__item is-danger" role="menuitem" onClick={() => { removeTask(tk.id); setMenuFor(null); }}>
                              <X size={13} aria-hidden /> {lang === "zh" ? "删除" : "Delete"}
                            </button>
                          </div>
                        </>
                      )}
                    </div>
                  ))}
                </div>
              );
            })
          )}
        </div>

        <div className="ax-side-tools">
          {!sidebarCollapsed && <p className="ax-nav__label">{lang === "zh" ? "工具" : "Tools"}</p>}
          {NAV.filter((n) => n.key !== "tasks").map((n) => (
            <button
              key={n.key}
              className={`ax-nav__item ${nav === n.key ? "is-active" : ""}`}
              onClick={() => setNav(n.key)}
              title={lang === "zh" ? n.zh : n.en}
            >
              <n.icon size={16} aria-hidden />
              {!sidebarCollapsed && (lang === "zh" ? n.zh : n.en)}
            </button>
          ))}
        </div>

        {/* Bottom-left account menu (Claude-style): the single home for Settings / Language /
            Help — no duplicate settings/account buttons elsewhere. Opens upward. */}
        <div className="ax-side-foot">
          <div className="ax-acct ax-acct--foot">
            <button className={`ax-acct__btn ${accountMenu ? "is-active" : ""}`} onClick={() => setAccountMenu((o) => !o)} aria-haspopup="menu" aria-expanded={accountMenu}>
              <span className="ax-acct__avatar">A</span>
              {!sidebarCollapsed && (
                <span className="ax-acct__label">
                  <b>{lang === "zh" ? "本地版" : "Local"}</b>
                  <span>{(lang === "zh" ? "本机数据" : "on-device")}{appVer && ` · v${appVer}`}</span>
                </span>
              )}
            </button>
            {accountMenu && (
              <>
                <div className="ax-menu-scrim" onClick={() => setAccountMenu(false)} />
                <div className="ax-menu ax-menu--acct-foot" role="menu">
                  <button className="ax-menu__item" role="menuitem" onClick={() => { setSettingsCat(undefined); setSettingsOpen(true); setAccountMenu(false); }}>
                    <Settings size={14} aria-hidden /> {t(lang, "settings")}
                  </button>
                  <button className="ax-menu__item" role="menuitem" onClick={() => { setLang(lang === "zh" ? "en" : "zh"); setAccountMenu(false); }}>
                    <Languages size={14} aria-hidden /> {lang === "zh" ? "切换到 English" : "切换到 中文"}
                  </button>
                  <button className="ax-menu__item" role="menuitem" onClick={() => { setSettingsCat("about"); setSettingsOpen(true); setAccountMenu(false); }}>
                    <Info size={14} aria-hidden /> {lang === "zh" ? "帮助与关于" : "Help & about"}
                  </button>
                  <div className="ax-menu__foot">
                    <span className="ax-acct__avatar ax-acct__avatar--sm">A</span>
                    {lang === "zh" ? "本地版 · 本机数据" : "Local · on-device"}
                  </div>
                </div>
              </>
            )}
          </div>
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
            {/* Command palette button removed — still reachable via ⌘K. */}
            <button
              className={`ax-iconbtn ax-bell ${nav === "notifications" ? "is-active" : ""}`}
              onClick={() => setNav(nav === "notifications" ? "tasks" : "notifications")}
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

        {(() => {
          // Open-tabs bar (like Claude): several sessions open at once. Shown only when ≥2
          // live tabs exist, on the Sessions surface — a single session keeps the clean header.
          const liveTabs = openTabs.filter((id) => tasks.some((tk) => tk.id === id));
          if (nav !== "tasks" || liveTabs.length < 2) return null;
          return (
            <div className="ax-tabs" role="tablist">
              {liveTabs.map((id) => {
                const tk = tasks.find((x) => x.id === id)!;
                const on = activeId === id && view === "thread";
                return (
                  <div key={id} className={`ax-tab ${on ? "is-active" : ""}`} role="tab" aria-selected={on}>
                    <button
                      className="ax-tab__main"
                      title={tk.title}
                      onClick={() => { setNav("tasks"); setActiveId(id); setView("thread"); }}
                    >
                      <span className={`ax-lamp ${tk.status === "done" ? "is-green" : tk.status === "running" ? "is-amber" : "is-neutral"}`} />
                      <span className="ax-tab__title">{tk.title}</span>
                    </button>
                    <button className="ax-tab__x" onClick={() => dropTab(id)} title={t(lang, "close")} aria-label={t(lang, "close")}>
                      <X size={12} aria-hidden />
                    </button>
                  </div>
                );
              })}
            </div>
          );
        })()}

        <div className={`ax-content ${nav === "tasks" && view === "thread" ? "ax-content--thread" : ""}`}>
          {nav !== "tasks" ? (
            <div className="ax-canvas">
              {nav === "projects" && <ProjectsView lang={lang} />}
              {nav === "reading" && (
                <ReadingView lang={lang} sources={sources} onImport={importSource} onOpen={openSource} onRemove={removeSource} />
              )}
              {/* 检索 / 笔记本 / 舰队 removed (aggressive simplification); 证据/护盾/溯源/完整性
                  live on the artifact's own inspector tabs, not the sidebar. */}
              {nav === "metrics" && <MetricsView lang={lang} tasks={tasks} />}
              {nav === "notifications" && (
                <NotificationsView
                  lang={lang}
                  tasks={tasks}
                  onOpen={openTask}
                />
              )}
            </div>
          ) : view === "home" ? (
            <div className="ax-home">
              <h1 className="ax-home__greeting">{greeting()}</h1>
              <p className="ax-home__sub">
                {lang === "zh" ? "输入任务开始,或用 / 唤出能力" : "Type a task, or use / for capabilities"}
              </p>
              {composerBlock()}
            </div>
          ) : (
            <div className="ax-thread">
              <div className="ax-thread__scroll">
              {(() => {
                  const tk = tasks.find((x) => x.id === activeId);
                  if (!tk) return null;
                  // Match the live run by the task's active run id (an append iteration mints a
                  // fresh runid that differs from the session id).
                  const liveRun = liveRuns.find((r) => r.id === (tk.liveRunId ?? tk.id));
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

                      {/* the user's own message — what they typed — pinned at the top of the thread */}
                      {tk.goal && (
                        <div className="ax-usermsg">
                          <span className="ax-usermsg__who">{lang === "zh" ? "你" : "You"}</span>
                          <p className="ax-usermsg__text">{tk.goal}</p>
                        </div>
                      )}

                      {/* check-draft progress bar (workflow/code render the live thinking chain below) */}
                      {tk.status === "running" && tk.type !== "workflow" && tk.type !== "code" && (
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

                      {/* live/persisted THINKING CHAIN for workflow/code — right above the composer:
                          phases + each agent (launch animation + name/model) + its steps (verbatim
                          code for coding/analysis), streamed live from runStore; falls back to the
                          persisted tk.steps once the finished live run is cleared. */}
                      {(tk.type === "workflow" || tk.type === "code" || tk.type === "research") && (
                        liveRun ? (
                          <ThreadActivity
                            run={liveRun}
                            lang={lang}
                            onCancel={tk.status === "running" ? () => invoke("cancel_run", { runid: tk.liveRunId ?? tk.id }) : undefined}
                          />
                        ) : tk.steps && tk.steps.length > 0 ? (
                          <div className="ax-agents" style={{ marginTop: 12 }}>
                            <p className="ax-block-label">{lang === "zh" ? `思考过程 · ${tk.steps.length} 步` : `Thinking · ${tk.steps.length} steps`}</p>
                            <AgentSteps steps={tk.steps} lang={lang} />
                          </div>
                        ) : tk.status === "running" ? (
                          <div className="ax-progress"><span>{lang === "zh" ? "智能体启动中…" : "spawning agent…"}</span></div>
                        ) : null
                      )}

                      {/* When the artifact pane is open, the FULL report lives THERE; the thread keeps
                          the conversation — a chat-style wrap-up (what was done + key findings + next
                          steps) then a pointer into the panel. */}
                      {showArtifactPane && (tk.recap || tk.recapPending) && (
                        <div className="ax-recap">
                          {tk.recap ? (
                            <ArtifactBody text={tk.recap} />
                          ) : (
                            <p className="ax-recap__pending">
                              <span className="ax-recap__dot" />
                              {lang === "zh" ? "正在总结要点与下一步…" : "Summarizing key points & next steps…"}
                            </p>
                          )}
                        </div>
                      )}
                      {showArtifactPane && (
                        <button className="ax-artref" onClick={() => setRailOpen(true)}>
                          {lang === "zh" ? "查看完整产物 · 见右侧「产物」面板 →" : "View the full artifact — Artifact panel on the right →"}
                        </button>
                      )}

                      {!showArtifactPane && tk.synthesis && (
                        <div className="ax-synth">
                          <p className="ax-block-label">{lang === "zh" ? "汇总结果" : "Synthesized result"}</p>
                          {/* rendered natively: markdown + molecules (SMILES) + diagrams (mermaid) + math */}
                          <ArtifactBody text={tk.synthesis} sources={tk.sources} />
                        </div>
                      )}

                      {/* the reviewer agent, inline — like Claude Science's Reviewer panel */}
                      {tk.review && <ReviewerPanel lang={lang} review={tk.review} />}

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
              </div>
              {activeId && (
                <div className="ax-thread__followup">{composerBlock()}</div>
              )}
            </div>
          )}
        </div>
      </main>
      {railOpen &&
        (showArtifactPane && activeTask ? (
          <ArtifactPane
            lang={lang}
            task={activeTask}
            forkedFromTitle={tasks.find((x) => x.id === activeTask.forkedFrom)?.title}
            onExport={exportProvenance}
            onSaveReport={saveReport}
            onRedraw={(tool) => {
              const g = (activeTask.goal || "").trim();
              if (!g) return;
              runCoding(
                `${g}\n\n【重绘要求】请改用 ${tool} 重新绘制本次分析的全部图表:保持相同的数据、统计与结论,仅更换绘图库。若 ${tool} 不是 matplotlib,请先用 pip_install 安装它,再把图导出为 PNG 展示。`,
                true
              );
            }}
            onRelayout={() => {
              const g = (activeTask.goal || "").trim();
              if (!g) return;
              runCoding(
                `${g}\n\n【重排要求】把本次分析的最终报告重新组织成一篇学术论文的结构:标题、摘要(Abstract)、引言、方法、结果(含图表与统计)、讨论、(如有)参考文献。保持相同的数据、计算与结论,只调整叙述结构与排版,并复用/重绘必要的图表。`,
                true
              );
            }}
            onRevise={(newSynthesis, note) => {
              // Splice a surgically-revised passage in as a NEW artifact version (v_n+1),
              // leaving the computed data/figures and prior versions intact.
              setTasks((ts) =>
                ts.map((x) => {
                  if (x.id !== activeTask.id) return x;
                  const base = x.versions ?? [{ synthesis: x.synthesis, review: x.review, steps: x.steps, ts: x.ts, goal: x.goal ?? "" }];
                  const last = base[base.length - 1];
                  const nv = { synthesis: newSynthesis, review: last?.review, steps: last?.steps, ts: Date.now(), goal: note ? `批注改写:${note}` : "批注改写" };
                  return { ...x, synthesis: newSynthesis, versions: [...base, nv] };
                })
              );
              note && note.trim() && setToast(lang === "zh" ? "已按批注改写并生成新版本" : "Revised and saved as a new version");
            }}
            expanded={artifactExpanded}
            onToggleExpand={() => setArtifactExpanded((e) => !e)}
            onClose={() => {
              setRailOpen(false);
              setArtifactExpanded(false);
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
