import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  X, Trash2, Palette, KeyRound, ListChecks, Scale, Info, Star, Plug, type LucideIcon,
} from "lucide-react";
import { Lang, t } from "../i18n";

type Conn = { label: string; base_url: string; model: string; primary: boolean; has_key: boolean };
type Theme = "light" | "dark" | "system";

// Same GLM-5.2 providers the onboarding wizard offers — paste a key, no manual URL/model needed.
const GLM_PROVIDERS: { fam: string; zh: string; en: string }[] = [
  { fam: "zhipu", zh: "智谱 GLM-5.2", en: "Zhipu GLM-5.2" },
  { fam: "scnet", zh: "SCNet 超算互联网", en: "SCNet" },
];

/** Apply a theme to the document root (and persist). Exported so App can apply on load. */
export function applyTheme(theme: Theme) {
  document.documentElement.dataset.theme = theme;
  localStorage.setItem("axiom3.theme", theme);
}

// Settings categories. general / providers / checking / about are fully wired; the rest are
// honest shells (an "未接入 / wired next phase" note up top) so the settings surface is complete
// per the feature list without pretending unbuilt switches do anything.
type Cat = "general" | "providers" | "checking" | "redline" | "about";
// Trimmed to the surfaces that actually do something (aggressive simplification): 通用 / 模型 /
// 校验 / 红线 / 关于. The former shell panels (fleet/security/privacy/storage/…) are gone.
const CATS: { key: Cat; icon: LucideIcon; zh: string; en: string }[] = [
  { key: "general", icon: Palette, zh: "通用", en: "General" },
  { key: "providers", icon: KeyRound, zh: "模型连接", en: "Model connections" },
  { key: "checking", icon: ListChecks, zh: "校验与检查", en: "Checking" },
  { key: "redline", icon: Scale, zh: "红线与责任", en: "Red line & liability" },
  { key: "about", icon: Info, zh: "关于", en: "About" },
];

export function Settings({ lang, onLang, initialCat, onClose }: { lang: Lang; onLang: (l: Lang) => void; initialCat?: string; onClose: () => void }) {
  const [cat, setCat] = useState<Cat>((CATS.some((c) => c.key === initialCat) ? initialCat : "general") as Cat);
  const [theme, setTheme] = useState<Theme>((localStorage.getItem("axiom3.theme") as Theme) || "system");
  const [lit, setLit] = useState(localStorage.getItem("axiom3.lit") !== "off");
  const [telem, setTelem] = useState(localStorage.getItem("axiom3.telemetry") !== "off");
  const [fleetMax, setFleetMax] = useState(localStorage.getItem("axiom3.fleetmax") || "");
  const [version, setVersion] = useState("");
  // Model connections (#4): base URL + API key + model per row — reaches any OpenAI-compatible API.
  const [conns, setConns] = useState<Conn[]>([]);
  const [form, setForm] = useState({ label: "", base_url: "", api_key: "", model: "" });
  const [connNote, setConnNote] = useState("");
  const [connBusy, setConnBusy] = useState(false);
  // Onboarding-style GLM-5.2 key flow (same as OnboardingWizard's providers step).
  const [provs, setProvs] = useState<{ family: string; connected: boolean }[]>([]);
  const [provSel, setProvSel] = useState("zhipu");
  const [keyInput, setKeyInput] = useState("");
  const [keyBusy, setKeyBusy] = useState(false);
  const [keyNote, setKeyNote] = useState("");
  const zh = lang === "zh";

  const refreshConns = useCallback(() => {
    invoke<Conn[]>("list_connections").then(setConns).catch(() => {});
  }, []);
  const refreshProvs = useCallback(() => {
    invoke<{ family: string; connected: boolean }[]>("provider_status").then(setProvs).catch(() => {});
  }, []);
  useEffect(() => {
    refreshConns();
    refreshProvs();
    invoke<string>("axiom_core_version").then(setVersion).catch(() => {});
  }, [refreshConns, refreshProvs]);

  async function saveKey() {
    const key = keyInput.trim();
    if (!key) return;
    setKeyBusy(true);
    setKeyNote("");
    try {
      await invoke("set_provider_key", { provider: provSel, key });
      setKeyInput("");
      setKeyNote(zh ? "已连接 ✓" : "Connected ✓");
      refreshProvs();
    } catch (e) {
      setKeyNote(zh ? `连接失败:${e}` : `Failed: ${e}`);
    } finally {
      setKeyBusy(false);
    }
  }

  function chooseTheme(th: Theme) {
    setTheme(th);
    applyTheme(th);
  }
  function setTelemetry(on: boolean) {
    setTelem(on);
    localStorage.setItem("axiom3.telemetry", on ? "on" : "off");
    invoke("set_telemetry", { on }).catch(() => {});
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
  async function testConn() {
    if (!form.base_url.trim() || !form.api_key.trim() || !form.model.trim()) {
      setConnNote(zh ? "调用地址 / APIKey / 型号 都要填" : "fill base URL / API key / model");
      return;
    }
    setConnBusy(true);
    setConnNote(zh ? "测试中…" : "Testing…");
    try {
      const r = await invoke<string>("test_connection", { baseUrl: form.base_url, key: form.api_key, model: form.model });
      setConnNote(r);
    } catch (e) {
      setConnNote(String(e));
    }
    setConnBusy(false);
  }
  async function saveConn(primary = false) {
    if (!form.label.trim() || !form.base_url.trim() || !form.model.trim()) {
      setConnNote(zh ? "连接名 / 调用地址 / 型号 必填" : "name / base URL / model required");
      return;
    }
    try {
      await invoke("save_connection", { label: form.label, baseUrl: form.base_url, model: form.model, key: form.api_key || null, primary });
      setForm({ label: "", base_url: "", api_key: "", model: "" });
      setConnNote(zh ? "已保存" : "Saved");
      refreshConns();
    } catch (e) {
      setConnNote(String(e));
    }
  }
  async function delConn(label: string) {
    try {
      await invoke("delete_connection", { label });
      refreshConns();
    } catch { /* ignore */ }
  }
  async function makePrimary(c: Conn) {
    try {
      await invoke("save_connection", { label: c.label, baseUrl: c.base_url, model: c.model, key: null, primary: true });
      refreshConns();
    } catch { /* ignore */ }
  }
  function resetOnboarding() {
    localStorage.removeItem("axiom3.onboarded");
    location.reload();
  }

  // a small static row (label + neutral state badge), used in the red-line panel
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

            {/* ── 模型连接:和引导界面一致的 GLM-5.2 provider + 粘贴Key 流程 ── */}
            {cat === "providers" && (
              <div className="ax-set-group">
                <p className="ax-set-group__label">{zh ? "模型连接" : "Model connections"}</p>
                <p className="ax-hint" style={{ marginTop: -4, marginBottom: 10 }}>
                  {zh ? "填入你的 GLM-5.2 API Key(自备)。智谱或超算互联网 SCNet 任一即可。" : "Paste your own GLM-5.2 API key. Either Zhipu or SCNet works."}
                </p>
                {GLM_PROVIDERS.map((p) => {
                  const on = provs.find((x) => x.family === p.fam)?.connected;
                  return (
                    <button key={p.fam} className={`ax-wiz-opt ${provSel === p.fam ? "sel" : ""}`} onClick={() => setProvSel(p.fam)}>
                      <span style={{ display: "flex", justifyContent: "space-between", alignItems: "center", width: "100%" }}>
                        <span>{zh ? p.zh : p.en}</span>
                        {on && (
                          <span style={{ display: "inline-flex", alignItems: "center", gap: 5, color: "var(--ax-accent)", fontSize: 12 }}>
                            <span className="ax-lamp is-green" /> {zh ? "已连接" : "connected"}
                          </span>
                        )}
                      </span>
                    </button>
                  );
                })}
                <div className="ax-toolrow" style={{ marginTop: 8, gap: 8 }}>
                  <input
                    className="ax-input"
                    type="password"
                    value={keyInput}
                    onChange={(e) => setKeyInput(e.currentTarget.value)}
                    onKeyDown={(e) => { if (e.key === "Enter") saveKey(); }}
                    placeholder={`${zh ? (provSel === "scnet" ? "SCNet 超算互联网" : "智谱 GLM-5.2") : (provSel === "scnet" ? "SCNet" : "Zhipu GLM-5.2")} · ${zh ? "粘贴 API Key…" : "Paste API key…"}`}
                    style={{ flex: 1, minWidth: 180 }}
                  />
                  <button className="ax-btn" onClick={saveKey} disabled={keyBusy || !keyInput.trim()}>
                    {keyBusy ? (zh ? "连接中…" : "Connecting…") : (zh ? "连接" : "Connect")}
                  </button>
                </div>
                <p className="ax-hint" style={{ marginTop: 8, display: "inline-flex", alignItems: "center", gap: 6 }}>
                  <span className={`ax-lamp ${provs.some((x) => x.connected) ? "is-green" : "is-neutral"}`} />
                  {keyNote || (provs.some((x) => x.connected) ? (zh ? "GLM-5.2 已就绪" : "GLM-5.2 ready") : (zh ? "填入任一 GLM-5.2 Key 即可继续" : "Paste either GLM-5.2 key to continue"))}
                </p>

                {/* 高级:自定义 OpenAI-兼容连接(保留原能力,默认折叠)*/}
                <details className="ax-set-adv" style={{ marginTop: 16 }}>
                  <summary style={{ cursor: "pointer", fontSize: 13, opacity: 0.75, userSelect: "none" }}>
                    {zh ? "高级:自定义 OpenAI-兼容连接(DeepSeek / Qwen / OpenRouter / 本地 vLLM·Ollama…)" : "Advanced: custom OpenAI-compatible connection"}
                  </summary>
                  <div style={{ marginTop: 10 }}>
                    <p className="ax-hint" style={{ marginTop: -2, marginBottom: 8 }}>
                      {zh ? "填「调用地址 / APIKey / 型号」即可接任意 OpenAI-兼容模型,供跨模型核查。" : "Add any OpenAI-compatible model with base URL / API key / model name, for cross-model checks."}
                    </p>
                    {conns.length === 0 && <p className="ax-hint" style={{ marginBottom: 8 }}>{zh ? "还没有自定义连接。" : "No custom connections yet."}</p>}
                    {conns.map((c) => (
                      <div className="ax-prov-row" key={c.label}>
                        <span style={{ display: "inline-flex", alignItems: "center", gap: 8, minWidth: 0 }}>
                          <Plug size={14} aria-hidden style={{ flex: "none", opacity: 0.7 }} />
                          <span style={{ minWidth: 0 }}>
                            <b>{c.label}</b> {c.primary && <span className="ax-badge" style={{ marginLeft: 4 }}>{zh ? "主" : "primary"}</span>}
                            {!c.has_key && <span className="ax-badge" style={{ marginLeft: 4 }}>{zh ? "缺密钥" : "no key"}</span>}
                            <br /><span className="ax-hint">{c.model} · {c.base_url}</span>
                          </span>
                        </span>
                        <span style={{ display: "inline-flex", gap: 4, flex: "none" }}>
                          {!c.primary && (
                            <button className="ax-iconbtn" onClick={() => makePrimary(c)} title={zh ? "设为主连接" : "Make primary"} aria-label="primary">
                              <Star size={15} aria-hidden />
                            </button>
                          )}
                          <button className="ax-iconbtn" onClick={() => delConn(c.label)} title={zh ? "删除" : "Delete"} aria-label="delete">
                            <Trash2 size={15} aria-hidden />
                          </button>
                        </span>
                      </div>
                    ))}
                    <div className="ax-set-row" style={{ display: "block", marginTop: 10 }}>
                      <p className="ax-set-group__label" style={{ marginBottom: 6 }}>{zh ? "添加连接" : "Add a connection"}</p>
                      <div style={{ display: "grid", gap: 6 }}>
                        <input className="ax-input" value={form.label} onChange={(e) => setForm((f) => ({ ...f, label: e.currentTarget.value }))} placeholder={zh ? "连接名(如 deepseek-主)" : "name (e.g. deepseek-main)"} />
                        <input className="ax-input" value={form.base_url} onChange={(e) => setForm((f) => ({ ...f, base_url: e.currentTarget.value }))} placeholder={zh ? "调用地址 https://api.deepseek.com/v1" : "base URL https://api.deepseek.com/v1"} />
                        <input className="ax-input" type="password" value={form.api_key} onChange={(e) => setForm((f) => ({ ...f, api_key: e.currentTarget.value }))} placeholder="API Key" />
                        <input className="ax-input" value={form.model} onChange={(e) => setForm((f) => ({ ...f, model: e.currentTarget.value }))} placeholder={zh ? "模型型号(如 deepseek-chat)" : "model (e.g. deepseek-chat)"} />
                      </div>
                      <div className="ax-toolrow" style={{ marginTop: 8, gap: 8 }}>
                        <button className="ax-btn" onClick={testConn} disabled={connBusy}>{connBusy ? (zh ? "测试中…" : "Testing…") : (zh ? "测试连接" : "Test")}</button>
                        <button className="ax-btn" onClick={() => saveConn(false)}>{zh ? "保存" : "Save"}</button>
                        <button className="ax-btn" onClick={() => saveConn(true)}>{zh ? "保存并设为主" : "Save as primary"}</button>
                      </div>
                      {connNote && <p className="ax-hint" style={{ marginTop: 6 }}>{connNote}</p>}
                    </div>
                  </div>
                </details>
                <p className="ax-hint" style={{ marginTop: 10 }}>{zh ? "密钥存于系统凭据库或本地仅属主可读的密钥文件;绝不写入日志、产物或代码库。" : "Keys are held in your OS credential store or a local owner-only file; never in logs, artifacts, or the repo."}</p>
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
                <div className="ax-set-row">
                  <span>{zh ? "匿名使用遥测" : "Anonymous usage telemetry"}<br /><span className="ax-hint">{zh ? "只回传交互元数据(模式/状态/耗时/是否出图等),帮我们发现卡顿与失败。绝不上传课题、数据或报告内容。" : "Sends only interaction metadata (mode/status/duration/…) so we can spot failures. Never your topics, data, or reports."}</span></span>
                  <button className={`ax-toggle ${telem ? "on" : ""}`} onClick={() => setTelemetry(!telem)}>{t(lang, telem ? "on" : "off")}</button>
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


            {/* ── 关于 (wired) ── */}
            {cat === "about" && (
              <div className="ax-set-group">
                <p className="ax-set-group__label">{t(lang, "set_about")}</p>
                <div className="ax-set-row">
                  <span>{t(lang, "set_version")}</span>
                  <span className="ax-hint">axiom-core {version}</span>
                </div>
                <div className="ax-set-row">
                  <span>{zh ? "重看引导" : "Replay onboarding"}<br /><span className="ax-hint">{zh ? "重置欢迎向导并重新加载" : "Reset the welcome wizard and reload"}</span></span>
                  <button className="ax-btn" onClick={resetOnboarding}>{zh ? "重置" : "Reset"}</button>
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
