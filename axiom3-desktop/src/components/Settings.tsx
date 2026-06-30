import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { X, Trash2 } from "lucide-react";
import { Lang, t } from "../i18n";

type ProviderState = { family: string; connected: boolean };
const PROVIDERS = ["anthropic", "openai", "deepseek", "gemini", "qwen"];
type Theme = "light" | "dark" | "system";

/** Apply a theme to the document root (and persist). Exported so App can apply on load. */
export function applyTheme(theme: Theme) {
  document.documentElement.dataset.theme = theme;
  localStorage.setItem("axiom3.theme", theme);
}

export function Settings({ lang, onLang, onClose }: { lang: Lang; onLang: (l: Lang) => void; onClose: () => void }) {
  const [theme, setTheme] = useState<Theme>((localStorage.getItem("axiom3.theme") as Theme) || "system");
  const [lit, setLit] = useState(localStorage.getItem("axiom3.lit") !== "off");
  const [fleetMax, setFleetMax] = useState(localStorage.getItem("axiom3.fleetmax") || "");
  const [provs, setProvs] = useState<ProviderState[]>([]);
  const [provSel, setProvSel] = useState("deepseek");
  const [keyInput, setKeyInput] = useState("");
  const [version, setVersion] = useState("");

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

  return (
    <div className="ax-set-overlay" onClick={onClose}>
      <div className="ax-set" onClick={(e) => e.stopPropagation()}>
        <div className="ax-set__head">
          <p className="ax-set__title">{t(lang, "settings")}</p>
          <button className="ax-iconbtn" onClick={onClose} title={t(lang, "close")} aria-label={t(lang, "close")}>
            <X size={18} aria-hidden />
          </button>
        </div>

        <div className="ax-set__body">
          {/* 外观 */}
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

          {/* 语言 */}
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

          {/* 模型与提供方 */}
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
              <input
                className="ax-input"
                type="password"
                value={keyInput}
                onChange={(e) => setKeyInput(e.currentTarget.value)}
                placeholder={t(lang, "set_provider_ph")}
                style={{ flex: 1, minWidth: 180 }}
              />
              <button className="ax-btn" onClick={addKey} disabled={!keyInput.trim()}>
                {t(lang, "prov_save")}
              </button>
            </div>
          </div>

          {/* 校验与检查 */}
          <div className="ax-set-group">
            <p className="ax-set-group__label">{t(lang, "set_checking")}</p>
            <div className="ax-set-row">
              <span>
                {t(lang, "set_litsearch")}
                <br />
                <span className="ax-hint">{t(lang, "set_litsearch_hint")}</span>
              </span>
              <button className={`ax-toggle ${lit ? "on" : ""}`} onClick={() => setLiterature(!lit)}>
                {t(lang, lit ? "on" : "off")}
              </button>
            </div>
            <div className="ax-set-row">
              <span>{t(lang, "set_fleetmax")}</span>
              <input
                className="ax-input"
                value={fleetMax}
                onChange={(e) => saveFleetMax(e.currentTarget.value.replace(/[^0-9]/g, ""))}
                placeholder="24"
                style={{ width: 80 }}
                inputMode="numeric"
              />
            </div>
          </div>

          {/* 关于 */}
          <div className="ax-set-group">
            <p className="ax-set-group__label">{t(lang, "set_about")}</p>
            <div className="ax-set-row">
              <span>{t(lang, "set_version")}</span>
              <span className="ax-hint">axiom-core {version}</span>
            </div>
            <p className="ax-hint" style={{ marginTop: 6, lineHeight: 1.6 }}>{t(lang, "set_disclaimer")}</p>
          </div>
        </div>
      </div>
    </div>
  );
}
