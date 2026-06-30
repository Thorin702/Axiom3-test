import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { ChevronLeft, ChevronRight, Check, Download, Square, SquareCheck, ListChecks, Network, Code, Languages as LangIcon } from "lucide-react";
import type { Lang } from "../i18n";

type SystemCheck = { os: string; keychain: boolean; python: string | null; node: string | null; ollama: boolean };
type ProviderState = { family: string; connected: boolean };

// Onboarding copy is local to this self-contained component (kept out of the shared i18n).
const TXT: Record<Lang, Record<string, string>> = {
  zh: {
    welcome: "欢迎", syscheck: "本机检查", providers: "连接模型", deploy: "部署档",
    lit: "文献来源", howto: "使用说明", consent: "声明与同意",
    brand_sub: "面向科研工作者 · 以严谨为目标",
    lead: "把草稿拆成断言,运行检查、摆出证据、留可审计溯源 — 是否成立由你判断,它不替你下对错。",
    b1: "确定性检查 + 证据接地(statcheck、引文撤稿、数值核对…)",
    b2: "多智能体工作流:按需派发数十~数百 agent 协同核验与调研",
    b3: "编程场景:把编译器 / 测试 / linter 这类确定性工具当检查器",
    b4: "中英等多语言",
    sys_lead: "检查运行环境。", sys_os: "操作系统", sys_keychain: "钥匙串访问",
    sys_python: "Python 3", sys_node: "Node.js", sys_ollama: "本地模型(Ollama)",
    optional: "可选", install: "安装", checking: "检测中…",
    prov_lead: "选提供方并填入 API key。连 ≥2 家,跨模型检查更扎实。", prov_key: "粘贴 API key…",
    prov_save: "连接", prov_done: "已连接", prov_n1: "已连接 ", prov_n2a: " 家 · 跨模型检查可用", prov_n2b: " 家 · 再连 1 家更扎实",
    deploy_lead: "决定数据与算力去向。",
    dep_cloud: "家庭 · 云", dep_cloud_d: "数据走云端模型,最省心",
    dep_hybrid: "家庭 · 混合", dep_hybrid_d: "本地优先,敏感内容不出机",
    dep_onprem: "机构 · 本地", dep_onprem_d: "vLLM 舰队,数据完全本地",
    lit_lead: "开启后,Axiom3 从已发表文献检索相关证据(候选,非裁决)。",
    lit_on: "开启文献检索", lit_off: "暂不开启",
    how1: "新建项目,粘贴或导入草稿。", how2: "查看每条断言的检查与证据。", how3: "需要时导出可审计的溯源记录。",
    how_note: "三两分钟即可上手。",
    consent_text: "Axiom3 做检查与证据呈现,不判定真伪、不给确定性结论。研究结论与相应责任归你。",
    consent_agree: "我已阅读并同意", finish: "进入工作台",
    prev: "上一步", next: "下一步",
  },
  en: {
    welcome: "Welcome", syscheck: "System check", providers: "Connect models", deploy: "Deployment",
    lit: "Literature", howto: "How it works", consent: "Disclaimer & consent",
    brand_sub: "for researchers · built for rigor",
    lead: "It breaks a draft into claims, runs checks, surfaces evidence, and leaves an auditable trail — whether it holds is your call; it doesn't decide right or wrong for you.",
    b1: "Deterministic checks + evidence grounding (statcheck, retraction, numbers…)",
    b2: "Multi-agent workflow: dispatch tens–hundreds of agents to verify and research",
    b3: "Coding: compilers / tests / linters used as deterministic checkers",
    b4: "Chinese, English, and more",
    sys_lead: "Checking your environment.", sys_os: "Operating system", sys_keychain: "Keychain access",
    sys_python: "Python 3", sys_node: "Node.js", sys_ollama: "Local model (Ollama)",
    optional: "optional", install: "install", checking: "checking…",
    prov_lead: "Pick a provider and paste its API key. Connect ≥2 for stronger cross-model checking.", prov_key: "Paste API key…",
    prov_save: "Connect", prov_done: "connected", prov_n1: "Connected ", prov_n2a: " · cross-model checking available", prov_n2b: " · one more makes it stronger",
    deploy_lead: "Where data and compute go.",
    dep_cloud: "Home · cloud", dep_cloud_d: "Data goes to cloud models — simplest",
    dep_hybrid: "Home · hybrid", dep_hybrid_d: "Local-first; sensitive content stays on device",
    dep_onprem: "Institution · on-prem", dep_onprem_d: "vLLM fleet; data fully local",
    lit_lead: "When on, Axiom3 retrieves related evidence from the literature (candidate, not a verdict).",
    lit_on: "Enable literature search", lit_off: "Not now",
    how1: "Create a project, paste or import a draft.", how2: "Read the checks and evidence per claim.", how3: "Export an auditable provenance record when needed.",
    how_note: "Up and running in a couple of minutes.",
    consent_text: "Axiom3 runs checks and surfaces evidence. It does not judge truth or give definitive conclusions. The research conclusion and responsibility are yours.",
    consent_agree: "I have read and agree", finish: "Enter the workbench",
    prev: "Back", next: "Next",
  },
};

const STEPS = ["welcome", "syscheck", "providers", "deploy", "lit", "howto", "consent"] as const;
const PROVIDERS = ["anthropic", "openai", "deepseek", "gemini", "qwen"];

function Lamp({ ok }: { ok: boolean }) {
  return <span className={`ax-lamp ${ok ? "is-green" : "is-amber"}`} />;
}

export function OnboardingWizard({ lang, onDone }: { lang: Lang; onDone: () => void }) {
  const tx = (k: string) => TXT[lang][k] ?? TXT.zh[k] ?? k;
  const [step, setStep] = useState(0);
  const [sys, setSys] = useState<SystemCheck | null>(null);
  const [provs, setProvs] = useState<ProviderState[]>([]);
  const [provSel, setProvSel] = useState("deepseek");
  const [keyInput, setKeyInput] = useState("");
  const [deploy, setDeploy] = useState(localStorage.getItem("axiom3.deploy") || "");
  const [lit, setLit] = useState(localStorage.getItem("axiom3.lit") !== "off");
  const [consent, setConsent] = useState(false);
  const [saving, setSaving] = useState(false);

  const cur = STEPS[step];
  const last = step === STEPS.length - 1;
  const connected = provs.filter((p) => p.connected).length;

  const refreshProvs = useCallback(() => {
    invoke<ProviderState[]>("provider_status").then(setProvs).catch(() => {});
  }, []);

  useEffect(() => {
    if (cur === "syscheck" && !sys) invoke<SystemCheck>("system_check").then(setSys).catch(() => {});
    if (cur === "providers") refreshProvs();
  }, [cur, sys, refreshProvs]);

  const finish = useCallback(() => {
    if (!consent) return;
    localStorage.setItem("axiom3.onboarded", "1");
    localStorage.setItem("axiom3.deploy", deploy);
    localStorage.setItem("axiom3.lit", lit ? "on" : "off");
    onDone();
  }, [consent, deploy, lit, onDone]);

  const next = useCallback(() => {
    if (!last) setStep((s) => s + 1);
    else finish();
  }, [last, finish]);

  useEffect(() => {
    const h = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement)?.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA") return;
      if (e.key === "ArrowRight") next();
      if (e.key === "ArrowLeft" && step > 0) setStep((s) => s - 1);
    };
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, [next, step]);

  async function saveKey() {
    if (!keyInput.trim()) return;
    setSaving(true);
    try {
      await invoke("set_provider_key", { provider: provSel, key: keyInput });
      setKeyInput("");
      refreshProvs();
    } catch {
      /* surfaced via lamp staying off */
    } finally {
      setSaving(false);
    }
  }

  function installRuntime(target: "python" | "node") {
    const url = target === "python" ? "https://www.python.org/downloads/" : "https://nodejs.org/en/download";
    openUrl(url).catch(() => {});
  }

  function body() {
    switch (cur) {
      case "welcome":
        return (
          <>
            <p className="ax-wiz-brand">
              Axiom3<small>{tx("brand_sub")}</small>
            </p>
            <p style={{ margin: "8px 0 14px" }}>{tx("lead")}</p>
            {[["b1", ListChecks], ["b2", Network], ["b3", Code], ["b4", LangIcon]].map(([k, Icon]: any) => (
              <div className="ax-wiz-bullet" key={k}>
                <Icon size={18} color="var(--ax-accent)" style={{ marginTop: 1, flex: "none" }} aria-hidden />
                <span>{tx(k)}</span>
              </div>
            ))}
          </>
        );
      case "syscheck": {
        const rows: [string, boolean, ("python" | "node") | null, boolean][] = [
          [tx("sys_os") + (sys ? ` · ${sys.os}` : ""), !!sys, null, false],
          [tx("sys_keychain"), !!sys?.keychain, null, false],
          [tx("sys_python") + (sys?.python ? ` · ${sys.python}` : ""), !!sys?.python, "python", false],
          [tx("sys_node") + (sys?.node ? ` · ${sys.node}` : ""), !!sys?.node, "node", false],
          [tx("sys_ollama"), !!sys?.ollama, null, true],
        ];
        return (
          <>
            <p style={{ margin: "0 0 10px" }}>{tx("sys_lead")}</p>
            {!sys && <p className="ax-hint">{tx("checking")}</p>}
            {sys &&
              rows.map(([label, ok, inst, opt], i) => (
                <div className="ax-wiz-row" key={i}>
                  <span>
                    {label} {opt && <span className="ax-hint">({tx("optional")})</span>}
                  </span>
                  <span style={{ display: "flex", alignItems: "center", gap: 8 }}>
                    <Lamp ok={ok} />
                    {!ok && inst && (
                      <button className="ax-iconbtn" onClick={() => installRuntime(inst)} title={tx("install")} aria-label={tx("install")}>
                        <Download size={15} aria-hidden />
                      </button>
                    )}
                  </span>
                </div>
              ))}
          </>
        );
      }
      case "providers":
        return (
          <>
            <p style={{ margin: "0 0 10px" }}>{tx("prov_lead")}</p>
            {PROVIDERS.map((fam) => {
              const on = provs.find((p) => p.family === fam)?.connected;
              return (
                <button key={fam} className={`ax-wiz-opt ${provSel === fam ? "sel" : ""}`} onClick={() => setProvSel(fam)}>
                  <span style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
                    <span>{fam}</span>
                    {on && (
                      <span style={{ display: "inline-flex", alignItems: "center", gap: 5, color: "var(--ax-accent)", fontSize: 12 }}>
                        <Lamp ok /> {tx("prov_done")}
                      </span>
                    )}
                  </span>
                </button>
              );
            })}
            <div className="ax-toolrow">
              <input
                className="ax-input"
                type="password"
                value={keyInput}
                onChange={(e) => setKeyInput(e.currentTarget.value)}
                placeholder={`${provSel} · ${tx("prov_key")}`}
                style={{ flex: 1, minWidth: 180 }}
              />
              <button className="ax-btn" onClick={saveKey} disabled={saving || !keyInput.trim()}>
                {tx("prov_save")}
              </button>
            </div>
            <p className="ax-hint" style={{ marginTop: 8 }}>
              {tx("prov_n1")}{connected}{connected >= 2 ? tx("prov_n2a") : tx("prov_n2b")}
            </p>
          </>
        );
      case "deploy": {
        const opts: [string, string, string][] = [
          ["cloud", tx("dep_cloud"), tx("dep_cloud_d")],
          ["hybrid", tx("dep_hybrid"), tx("dep_hybrid_d")],
          ["onprem", tx("dep_onprem"), tx("dep_onprem_d")],
        ];
        return (
          <>
            <p style={{ margin: "0 0 10px" }}>{tx("deploy_lead")}</p>
            {opts.map(([k, n, d]) => (
              <button key={k} className={`ax-wiz-opt ${deploy === k ? "sel" : ""}`} onClick={() => setDeploy(k)}>
                <span style={{ fontWeight: 500, color: "var(--ax-ink)" }}>{n}</span>
                <br />
                <span className="ax-hint">{d}</span>
              </button>
            ))}
          </>
        );
      }
      case "lit":
        return (
          <>
            <p style={{ margin: "0 0 10px" }}>{tx("lit_lead")}</p>
            <button className={`ax-wiz-opt ${lit ? "sel" : ""}`} onClick={() => setLit(true)}>
              {tx("lit_on")}
            </button>
            <button className={`ax-wiz-opt ${!lit ? "sel" : ""}`} onClick={() => setLit(false)}>
              {tx("lit_off")}
            </button>
          </>
        );
      case "howto":
        return (
          <>
            <ol style={{ margin: 0, paddingLeft: 18, lineHeight: 1.9 }}>
              <li>{tx("how1")}</li>
              <li>{tx("how2")}</li>
              <li>{tx("how3")}</li>
            </ol>
            <p className="ax-hint" style={{ marginTop: 10 }}>{tx("how_note")}</p>
          </>
        );
      case "consent":
        return (
          <div className="ax-wiz-consent">
            <p style={{ margin: "0 0 12px", color: "var(--ax-ink)" }}>{tx("consent_text")}</p>
            <button className="ax-wiz-opt" style={{ marginBottom: 0 }} onClick={() => setConsent((c) => !c)}>
              <span style={{ display: "inline-flex", alignItems: "center", gap: 8 }}>
                {consent ? <SquareCheck size={16} aria-hidden /> : <Square size={16} aria-hidden />}
                {tx("consent_agree")}
              </span>
            </button>
          </div>
        );
    }
  }

  return (
    <div className="ax-wiz-overlay">
      <div className="ax-wiz">
        <div className="ax-wiz__head">
          <p className="ax-wiz__title">{tx(cur)}</p>
          <span className="ax-wiz__count">{step + 1} / {STEPS.length}</span>
        </div>
        <div className="ax-wiz__body">{body()}</div>
        <div className="ax-wiz__foot">
          <button
            className="ax-iconbtn"
            onClick={() => setStep((s) => Math.max(0, s - 1))}
            style={{ visibility: step === 0 ? "hidden" : "visible" }}
            title={tx("prev")}
            aria-label={tx("prev")}
          >
            <ChevronLeft size={18} aria-hidden />
          </button>
          <div className="ax-wiz__dots">
            {STEPS.map((_, i) => (
              <button
                key={i}
                className={`ax-wiz__dot ${i === step ? "on" : ""}`}
                onClick={() => setStep(i)}
                aria-label={`step ${i + 1}`}
              />
            ))}
          </div>
          <button
            className="ax-iconbtn"
            onClick={next}
            disabled={last && !consent}
            title={last ? tx("finish") : tx("next")}
            aria-label={last ? tx("finish") : tx("next")}
          >
            {last ? <Check size={18} aria-hidden /> : <ChevronRight size={18} aria-hidden />}
          </button>
        </div>
      </div>
    </div>
  );
}
