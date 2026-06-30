import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  X, Trash2, Palette, KeyRound, ListChecks, Scale, Boxes, Shield, Lock,
  Database, FileCheck, UserRound, RefreshCw, Keyboard, Accessibility, Wrench, Info,
  type LucideIcon,
} from "lucide-react";
import { Lang, t } from "../i18n";

type ProviderState = { family: string; connected: boolean };
const PROVIDERS = ["anthropic", "openai", "deepseek", "gemini", "qwen"];
type Theme = "light" | "dark" | "system";

/** Apply a theme to the document root (and persist). Exported so App can apply on load. */
export function applyTheme(theme: Theme) {
  document.documentElement.dataset.theme = theme;
  localStorage.setItem("axiom3.theme", theme);
}

// Settings categories. general / providers / checking / about are fully wired; the rest are
// honest shells (an "未接入 / wired next phase" note up top) so the settings surface is complete
// per the feature list without pretending unbuilt switches do anything.
type Cat =
  | "general" | "providers" | "checking" | "redline" | "fleet" | "security"
  | "privacy" | "storage" | "integrity" | "account" | "updates"
  | "shortcuts" | "accessibility" | "advanced" | "about";
const CATS: { key: Cat; icon: LucideIcon; zh: string; en: string }[] = [
  { key: "general", icon: Palette, zh: "通用", en: "General" },
  { key: "providers", icon: KeyRound, zh: "模型与提供方", en: "Models & providers" },
  { key: "checking", icon: ListChecks, zh: "校验与检查", en: "Checking" },
  { key: "redline", icon: Scale, zh: "红线与责任", en: "Red line & liability" },
  { key: "fleet", icon: Boxes, zh: "多智能体 · 编排", en: "Fleet · orchestration" },
  { key: "security", icon: Shield, zh: "安全 · 抗投毒", en: "Security · shield" },
  { key: "privacy", icon: Lock, zh: "隐私与数据", en: "Privacy & data" },
  { key: "storage", icon: Database, zh: "存储与加密", en: "Storage & encryption" },
  { key: "integrity", icon: FileCheck, zh: "完整性与审计", en: "Integrity & audit" },
  { key: "account", icon: UserRound, zh: "账户", en: "Account" },
  { key: "updates", icon: RefreshCw, zh: "更新", en: "Updates" },
  { key: "shortcuts", icon: Keyboard, zh: "快捷键", en: "Shortcuts" },
  { key: "accessibility", icon: Accessibility, zh: "无障碍", en: "Accessibility" },
  { key: "advanced", icon: Wrench, zh: "开发者", en: "Developer" },
  { key: "about", icon: Info, zh: "关于", en: "About" },
];

export function Settings({ lang, onLang, onClose }: { lang: Lang; onLang: (l: Lang) => void; onClose: () => void }) {
  const [cat, setCat] = useState<Cat>("general");
  const [theme, setTheme] = useState<Theme>((localStorage.getItem("axiom3.theme") as Theme) || "system");
  const [lit, setLit] = useState(localStorage.getItem("axiom3.lit") !== "off");
  const [fleetMax, setFleetMax] = useState(localStorage.getItem("axiom3.fleetmax") || "");
  const [provs, setProvs] = useState<ProviderState[]>([]);
  const [provSel, setProvSel] = useState("deepseek");
  const [keyInput, setKeyInput] = useState("");
  const [version, setVersion] = useState("");
  const zh = lang === "zh";

  const refreshProvs = useCallback(() => {
    invoke<ProviderState[]>("provider_status").then(setProvs).catch(() => {});
  }, []);
  useEffect(() => {
    refreshProvs();
    invoke<string>("axiom_core_version").then(setVersion).catch(() => {});
  }, [refreshProvs]);

  function chooseTheme(th: Theme) {
    setTheme(th);
    applyTheme(th);
  }
  function setLiterature(on: boolean) {
    setLit(on);
    localStorage.setItem("axiom3.lit", on ? "on" : "off");
  }
  function saveFleetMax(v: string) {
    setFleetMax(v);
    const n = parseInt(v, 10);
    if (Number.isFinite(n) && n > 0) localStorage.setItem("axiom3.fleetmax", String(n));
    else localStorage.removeItem("axiom3.fleetmax");
  }
  async function addKey() {
    if (!keyInput.trim()) return;
    await invoke("set_provider_key", { provider: provSel, key: keyInput }).catch(() => {});
    setKeyInput("");
    refreshProvs();
  }
  async function removeKey(family: string) {
    await invoke("delete_provider_key", { provider: family }).catch(() => {});
    refreshProvs();
  }
  function resetOnboarding() {
    localStorage.removeItem("axiom3.onboarded");
    location.reload();
  }

  // honest shell note shown atop not-yet-wired panels
  const shellNote = (
    <p className="ax-set-note">{zh ? "此面板为界面预览,设置项将在功能完善阶段接入并持久化。" : "This panel is a UI preview — these settings get wired and persisted in the next phase."}</p>
  );
  // a static, non-functional row used in shell panels (label + a neutral state badge)
  const row = (label: string, value: string, sub?: string) => (
    <div className="ax-set-row">
      <span>
        {label}
        {sub && (<><br /><span className="ax-hint">{sub}</span></>)}
      </span>
      <span className="ax-badge">{value}</span>
    </div>
  );

  return (
    <div className="ax-set-overlay" onClick={onClose}>
      <div className="ax-set ax-set--wide" onClick={(e) => e.stopPropagation()}>
        <div className="ax-set__head">
          <p className="ax-set__title">{t(lang, "settings")}</p>
          <button className="ax-iconbtn" onClick={onClose} title={t(lang, "close")} aria-label={t(lang, "close")}>
            <X size={18} aria-hidden />
          </button>
        </div>

        <div className="ax-set__main">
          <nav className="ax-set-nav">
            {CATS.map((c) => (
              <button key={c.key} className={`ax-set-nav__item ${cat === c.key ? "is-active" : ""}`} onClick={() => setCat(c.key)}>
                <c.icon size={15} aria-hidden />
                {zh ? c.zh : c.en}
              </button>
            ))}
          </nav>

          <div className="ax-set__panel">
            {/* ── 通用 (wired) ── */}
            {cat === "general" && (
              <>
                <div className="ax-set-group">
                  <p className="ax-set-group__label">{t(lang, "set_appearance")}</p>
                  <div className="ax-set-row">
                    <span>{t(lang, "set_theme")}</span>
                    <span className="ax-seg">
                      {(["light", "dark", "system"] as Theme[]).map((th) => (
                        <button key={th} className={theme === th ? "on" : ""} onClick={() => chooseTheme(th)}>
                          {t(lang, th === "light" ? "theme_light" : th === "dark" ? "theme_dark" : "theme_system")}
                        </button>
                      ))}
                    </span>
                  </div>
                </div>
                <div className="ax-set-group">
                  <p className="ax-set-group__label">{t(lang, "set_language")}</p>
                  <div className="ax-set-row">
                    <span>{t(lang, "set_language")}</span>
                    <span className="ax-seg">
                      <button className={lang === "zh" ? "on" : ""} onClick={() => onLang("zh")}>中文</button>
                      <button className={lang === "en" ? "on" : ""} onClick={() => onLang("en")}>EN</button>
                    </span>
                  </div>
                </div>
              </>
            )}

            {/* ── 模型与提供方 (wired) ── */}
            {cat === "providers" && (
              <div className="ax-set-group">
                <p className="ax-set-group__label">{t(lang, "set_providers")}</p>
                {PROVIDERS.map((fam) => {
                  const on = provs.find((p) => p.family === fam)?.connected;
                  return (
                    <div className="ax-prov-row" key={fam}>
                      <span style={{ display: "inline-flex", alignItems: "center", gap: 8 }}>
                        <span className={`ax-lamp ${on ? "is-green" : "is-neutral"}`} />
                        {fam}
                      </span>
                      {on && (
                        <button className="ax-iconbtn" onClick={() => removeKey(fam)} title={t(lang, "prov_remove")} aria-label={t(lang, "prov_remove")}>
                          <Trash2 size={15} aria-hidden />
                        </button>
                      )}
                    </div>
                  );
                })}
                <div className="ax-toolrow">
                  <span className="ax-seg">
                    {PROVIDERS.map((fam) => (
                      <button key={fam} className={provSel === fam ? "on" : ""} onClick={() => setProvSel(fam)} style={{ fontSize: 11, padding: "4px 8px" }}>
                        {fam}
                      </button>
                    ))}
                  </span>
                </div>
                <div className="ax-toolrow">
                  <input className="ax-input" type="password" value={keyInput} onChange={(e) => setKeyInput(e.currentTarget.value)} placeholder={t(lang, "set_provider_ph")} style={{ flex: 1, minWidth: 180 }} />
                  <button className="ax-btn" onClick={addKey} disabled={!keyInput.trim()}>{t(lang, "prov_save")}</button>
                </div>
                <p className="ax-hint" style={{ marginTop: 8 }}>{zh ? "密钥保存在系统钥匙串,绝不写入磁盘或日志。" : "Keys are stored in the OS keychain — never written to disk or logs."}</p>
              </div>
            )}

            {/* ── 校验与检查 (wired) ── */}
            {cat === "checking" && (
              <div className="ax-set-group">
                <p className="ax-set-group__label">{t(lang, "set_checking")}</p>
                <div className="ax-set-row">
                  <span>{t(lang, "set_litsearch")}<br /><span className="ax-hint">{t(lang, "set_litsearch_hint")}</span></span>
                  <button className={`ax-toggle ${lit ? "on" : ""}`} onClick={() => setLiterature(!lit)}>{t(lang, lit ? "on" : "off")}</button>
                </div>
                <div className="ax-set-row">
                  <span>{t(lang, "set_fleetmax")}</span>
                  <input className="ax-input" value={fleetMax} onChange={(e) => saveFleetMax(e.currentTarget.value.replace(/[^0-9]/g, ""))} placeholder="24" style={{ width: 80 }} inputMode="numeric" />
                </div>
              </div>
            )}

            {/* ── 红线与责任 (informational, by design) ── */}
            {cat === "redline" && (
              <div className="ax-set-group">
                <p className="ax-set-group__label">{zh ? "红线与责任" : "Red line & liability"}</p>
                <p className="ax-set-note" style={{ background: "var(--ax-accent-soft)", borderColor: "transparent" }}>
                  {zh
                    ? "本产品不判定真假、不给出确定性结论 —— 只做检查并呈现证据。判断权始终在你。这是一条不可逾越的红线。"
                    : "This product never judges truth or issues definitive conclusions — it only checks and surfaces evidence. The judgement is always yours. This is a hard red line."}
                </p>
                {row(zh ? "保守上限" : "Conservative cap", zh ? "已锁定" : "Locked", zh ? "有依据的结论最多标到「存疑」,永不标「成立」" : "Grounded findings cap at “Uncertain”, never “Confirmed”")}
                {row(zh ? "跨模型核查" : "Cross-model checks", zh ? "建议性" : "Advisory", zh ? "多模型只产生倾向信号,不充当裁判" : "Multiple models produce leanings, never a verdict")}
                {row(zh ? "免责声明" : "Disclaimer", zh ? "随导出附带" : "Attached on export")}
              </div>
            )}

            {/* ── 多智能体 · 编排 (shell) ── */}
            {cat === "fleet" && (
              <div className="ax-set-group">
                <p className="ax-set-group__label">{zh ? "多智能体 · 编排" : "Fleet · orchestration"}</p>
                {shellNote}
                {row(zh ? "最大并发智能体" : "Max concurrent agents", "16")}
                {row(zh ? "每供应商配额" : "Per-vendor quota", zh ? "自动" : "Auto")}
                {row(zh ? "本地 Ollama 优先" : "Prefer local Ollama", zh ? "开" : "On")}
                {row(zh ? "智能体超时" : "Agent timeout", "90s")}
              </div>
            )}

            {/* ── 安全 · 抗投毒 (shell) ── */}
            {cat === "security" && (
              <div className="ax-set-group">
                <p className="ax-set-group__label">{zh ? "安全 · 抗投毒" : "Security · shield"}</p>
                {shellNote}
                {row(zh ? "指令注入过滤" : "Injection filtering", zh ? "开" : "On", zh ? "导入材料经模型读取前先中和注入指令" : "Neutralize injected instructions before any model reads imported text")}
                {row(zh ? "跨文档隔离" : "Cross-document isolation", zh ? "开" : "On")}
                {row(zh ? "可疑段落隔离" : "Quarantine suspicious passages", zh ? "默认" : "Default")}
              </div>
            )}

            {/* ── 隐私与数据 (shell) ── */}
            {cat === "privacy" && (
              <div className="ax-set-group">
                <p className="ax-set-group__label">{zh ? "隐私与数据" : "Privacy & data"}</p>
                {shellNote}
                {row(zh ? "本地优先" : "Local-first", zh ? "开" : "On", zh ? "默认全部在本机处理" : "Everything runs on-device by default")}
                {row(zh ? "外发范围" : "What leaves the device", zh ? "仅调用外部模型时" : "Only on external model calls")}
                {row(zh ? "使用情况遥测" : "Usage telemetry", zh ? "关" : "Off")}
              </div>
            )}

            {/* ── 存储与加密 (shell) ── */}
            {cat === "storage" && (
              <div className="ax-set-group">
                <p className="ax-set-group__label">{zh ? "存储与加密" : "Storage & encryption"}</p>
                {shellNote}
                {row(zh ? "数据库加密" : "Database encryption", "SQLCipher")}
                {row(zh ? "向量索引" : "Vector index", "LanceDB")}
                {row(zh ? "数据位置" : "Data location", zh ? "本机应用目录" : "Local app dir")}
                <div className="ax-set-row">
                  <span>{zh ? "清理缓存" : "Clear cache"}</span>
                  <button className="ax-btn" disabled>{zh ? "清理" : "Clear"}</button>
                </div>
              </div>
            )}

            {/* ── 完整性与审计 (shell) ── */}
            {cat === "integrity" && (
              <div className="ax-set-group">
                <p className="ax-set-group__label">{zh ? "完整性与审计" : "Integrity & audit"}</p>
                {shellNote}
                {row(zh ? "哈希链审计" : "Hash-chain audit", zh ? "始终开启" : "Always on")}
                {row(zh ? "事件签名" : "Event signing", zh ? "开" : "On")}
                <div className="ax-set-row">
                  <span>{zh ? "导出审计记录" : "Export audit log"}</span>
                  <button className="ax-btn" disabled>{zh ? "导出" : "Export"}</button>
                </div>
                <p className="ax-hint" style={{ marginTop: 6 }}>{zh ? "完整性证明的是记录未被篡改,而非内容为真。" : "Integrity proves the record wasn't altered — not that the content is true."}</p>
              </div>
            )}

            {/* ── 账户 (shell, no phone verification by design) ── */}
            {cat === "account" && (
              <div className="ax-set-group">
                <p className="ax-set-group__label">{zh ? "账户" : "Account"}</p>
                {shellNote}
                <div className="ax-set-row">
                  <span>{zh ? "邮箱登录" : "Email sign-in"}</span>
                  <button className="ax-btn" disabled>{zh ? "登录(即将)" : "Sign in (soon)"}</button>
                </div>
                {row(zh ? "套餐" : "Plan", zh ? "本地版" : "Local")}
                {row(zh ? "许可" : "License", "MIT core")}
              </div>
            )}

            {/* ── 更新 (shell) ── */}
            {cat === "updates" && (
              <div className="ax-set-group">
                <p className="ax-set-group__label">{zh ? "更新" : "Updates"}</p>
                {shellNote}
                <div className="ax-set-row">
                  <span>{zh ? "检查更新" : "Check for updates"}</span>
                  <button className="ax-btn" disabled>{zh ? "检查" : "Check"}</button>
                </div>
                <div className="ax-set-row">
                  <span>{zh ? "更新通道" : "Channel"}</span>
                  <span className="ax-seg"><button className="on">{zh ? "稳定" : "Stable"}</button><button>Beta</button></span>
                </div>
              </div>
            )}

            {/* ── 快捷键 (read-only) ── */}
            {cat === "shortcuts" && (
              <div className="ax-set-group">
                <p className="ax-set-group__label">{zh ? "快捷键" : "Shortcuts"}</p>
                {row(zh ? "命令面板" : "Command palette", "⌘K")}
                {row(zh ? "新任务" : "New task", "⌘N")}
                {row(zh ? "全局检索" : "Global search", "⌘F")}
                {row(zh ? "设置" : "Settings", "⌘,")}
                <p className="ax-hint" style={{ marginTop: 6 }}>{zh ? "⌘K 已可用;其余将随对应功能接入。" : "⌘K is live; the rest activate with their features."}</p>
              </div>
            )}

            {/* ── 无障碍 (shell) ── */}
            {cat === "accessibility" && (
              <div className="ax-set-group">
                <p className="ax-set-group__label">{zh ? "无障碍" : "Accessibility"}</p>
                {shellNote}
                {row(zh ? "减少动效" : "Reduce motion", zh ? "跟随系统" : "Match system")}
                {row(zh ? "字号" : "Font size", zh ? "标准" : "Standard")}
                {row(zh ? "高对比" : "High contrast", zh ? "关" : "Off")}
              </div>
            )}

            {/* ── 开发者 (reset onboarding is wired) ── */}
            {cat === "advanced" && (
              <div className="ax-set-group">
                <p className="ax-set-group__label">{zh ? "开发者" : "Developer"}</p>
                <div className="ax-set-row">
                  <span>{zh ? "重看引导" : "Replay onboarding"}<br /><span className="ax-hint">{zh ? "重置欢迎向导并重新加载" : "Reset the welcome wizard and reload"}</span></span>
                  <button className="ax-btn" onClick={resetOnboarding}>{zh ? "重置" : "Reset"}</button>
                </div>
                <div className="ax-set-row">
                  <span>{zh ? "诊断日志" : "Diagnostics log"}</span>
                  <button className="ax-btn" disabled>{zh ? "打开" : "Open"}</button>
                </div>
                {row(zh ? "引擎版本" : "Engine", `axiom-core ${version || "—"}`)}
              </div>
            )}

            {/* ── 关于 (wired) ── */}
            {cat === "about" && (
              <div className="ax-set-group">
                <p className="ax-set-group__label">{t(lang, "set_about")}</p>
                <div className="ax-set-row">
                  <span>{t(lang, "set_version")}</span>
                  <span className="ax-hint">axiom-core {version}</span>
                </div>
                <p className="ax-hint" style={{ marginTop: 6, lineHeight: 1.6 }}>{t(lang, "set_disclaimer")}</p>
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
