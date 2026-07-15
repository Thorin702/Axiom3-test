import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ChevronLeft, ChevronRight, Check, Square, SquareCheck, Network, Code } from "lucide-react";
import type { Lang } from "../i18n";

type ProviderState = { family: string; connected: boolean };
type ProvItem = { key: string; ready: boolean; detail: string; cli: string };

// Onboarding copy is local to this self-contained component (kept out of the shared i18n).
const TXT: Record<Lang, Record<string, string>> = {
  zh: {
    welcome: "欢迎", syscheck: "本机检查", providers: "连接模型", deploy: "部署档",
    lit: "文献来源", howto: "使用说明", consent: "声明与同意",
    brand_sub: "会做真科研的 AI · 校验为中心",
    lead: "把一个科研课题交给它 —— 它像研究员一样查文献、跑真实计算、画出版级图,产出一份带引用、可回溯的深度报告。",
    modes_h: "两种工作模式",
    mode_a_t: "分析 · Loc-SAgent",
    mode_a_d: "给它你的数据,单个智能体在本地 Python 内核跑统计、作图,快速得结论。",
    mode_r_t: "研究 · Glob-DAgent",
    mode_r_d: "给它一个课题,联网查文献 + 多智能体协同深挖 + 出版级绘图,产出深度研究报告。",
    soul: "校验为中心:每步留可审计证据、审查员标记无据主张 —— 结论是否成立由你判断,它从不替你下对错。",
    sys_lead: "检查运行环境。", sys_os: "操作系统", sys_keychain: "钥匙串访问",
    sys_python: "Python 3", sys_node: "Node.js", sys_ollama: "本地模型(Ollama)",
    optional: "可选", install: "安装", checking: "检测中…",
    setup_title: "环境预装",
    setup_lead: "一键装齐运行所需组件,全部就绪后方可继续 —— 首次安装可能需要几分钟(PyMOL 稍久),请耐心等待。",
    install_all: "一键装齐",
    prov_installing: "安装中…",
    prov_retry: "重试",
    prov_cli: "自动安装失败?可在终端手动运行:",
    prov_allok: "全部就绪,可以继续",
    prov_needall: "仍有组件未就绪 —— 装齐后才能下一步",
    prov_apikey_hint: "在下一步填入你的 GLM-5.2 Key 即可点亮",
    prov_lead: "填入你的 GLM-5.2 API Key(自备)。智谱或超算互联网 SCNet 任一即可。",
    prov_zhipu: "智谱 GLM-5.2", prov_scnet: "SCNet 超算互联网",
    prov_key: "粘贴 API Key…",
    prov_save: "连接", prov_done: "已连接",
    prov_ok: "GLM-5.2 已就绪", prov_need: "填入任一 GLM-5.2 Key 即可继续",
    deploy_lead: "算力与数据去向。前期仅开放云端,本地/混合将在后续版本开放。",
    dep_cloud: "云端算力", dep_cloud_d: "调用云端 GLM-5.2,开箱即用",
    dep_hybrid: "本地 · 混合", dep_hybrid_d: "本地优先,敏感内容不出机(即将开放)",
    dep_onprem: "机构 · 本地", dep_onprem_d: "vLLM 舰队,数据完全本地(即将开放)",
    dep_soon: "即将开放",
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
    brand_sub: "AI that does real science · verification-first",
    lead: "Hand it a research question — it reads the literature, runs real computation, draws publication-grade figures, and produces a cited, auditable deep report, like a researcher would.",
    modes_h: "Two modes",
    mode_a_t: "Analysis · Loc-SAgent",
    mode_a_d: "Give it your data; a single agent runs stats and figures in a local Python kernel for a fast answer.",
    mode_r_t: "Research · Glob-DAgent",
    mode_r_d: "Give it a topic; it searches the literature online, coordinates many agents, and draws publication-grade figures into a deep report.",
    soul: "Verification-first: an auditable trail per step + a reviewer that flags unsupported claims — whether a conclusion holds is your call; it never decides right or wrong for you.",
    sys_lead: "Checking your environment.", sys_os: "Operating system", sys_keychain: "Keychain access",
    sys_python: "Python 3", sys_node: "Node.js", sys_ollama: "Local model (Ollama)",
    optional: "optional", install: "install", checking: "checking…",
    setup_title: "Setup",
    setup_lead: "Install everything the workbench needs. You can continue once all are ready — the first install may take a few minutes (PyMOL a bit longer).",
    install_all: "Install all",
    prov_installing: "installing…",
    prov_retry: "retry",
    prov_cli: "Auto-install failed? Run this in a terminal:",
    prov_allok: "All ready — you can continue",
    prov_needall: "Some components aren't ready — install them to continue",
    prov_apikey_hint: "Lights up once you paste your GLM-5.2 key on the next step",
    prov_lead: "Paste your own GLM-5.2 API key. Either Zhipu or SCNet works.",
    prov_zhipu: "Zhipu GLM-5.2", prov_scnet: "SCNet",
    prov_key: "Paste API key…",
    prov_save: "Connect", prov_done: "connected",
    prov_ok: "GLM-5.2 ready", prov_need: "Paste either GLM-5.2 key to continue",
    deploy_lead: "Where compute and data go. Cloud only for now; local/hybrid coming later.",
    dep_cloud: "Cloud", dep_cloud_d: "Calls cloud GLM-5.2 — works out of the box",
    dep_hybrid: "Local · hybrid", dep_hybrid_d: "Local-first; sensitive content stays on device (coming soon)",
    dep_onprem: "Institution · on-prem", dep_onprem_d: "vLLM fleet; data fully local (coming soon)",
    dep_soon: "coming soon",
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
// BYO GLM-5.2: either endpoint that serves it satisfies onboarding. resolve_model routes both to
// glm-5.2 (SCNet gateway → GLM-5.2; Zhipu thinking tier → glm-5.2).
const PROVIDERS = ["zhipu", "scnet"];
const PROV_LABEL: Record<string, string> = { zhipu: "prov_zhipu", scnet: "prov_scnet" };

function Lamp({ ok }: { ok: boolean }) {
  return <span className={`ax-lamp ${ok ? "is-green" : "is-amber"}`} />;
}

export function OnboardingWizard({ lang, onDone }: { lang: Lang; onDone: () => void }) {
  const tx = (k: string) => TXT[lang][k] ?? TXT.zh[k] ?? k;
  const [step, setStep] = useState(0);
  const [provs, setProvs] = useState<ProviderState[]>([]);
  const [provSel, setProvSel] = useState("zhipu");
  const [keyInput, setKeyInput] = useState("");
  const [deploy, setDeploy] = useState(localStorage.getItem("axiom3.deploy") || "cloud");
  const [lit, setLit] = useState(localStorage.getItem("axiom3.lit") !== "off");
  const [consent, setConsent] = useState(false);
  const [saving, setSaving] = useState(false);
  // Provisioning gate state
  const [prov, setProv] = useState<ProvItem[] | null>(null);
  const [provBusy, setProvBusy] = useState<Set<string>>(new Set());
  const [provErr, setProvErr] = useState<Record<string, string>>({});

  const cur = STEPS[step];
  const last = step === STEPS.length - 1;
  const connected = provs.filter((p) => p.connected && PROVIDERS.includes(p.family)).length;
  // env prerequisites (everything except the API key, which the providers step handles)
  const envItems = (prov ?? []).filter((p) => p.key !== "apikey");
  const envAllReady = envItems.length > 0 && envItems.every((p) => p.ready);

  const refreshProv = useCallback(() => {
    invoke<ProvItem[]>("provision_status").then(setProv).catch(() => setProv([]));
  }, []);

  async function installOne(key: string) {
    setProvBusy((s) => new Set(s).add(key));
    setProvErr((e) => { const n = { ...e }; delete n[key]; return n; });
    try {
      await invoke("provision_install", { key });
    } catch (err) {
      setProvErr((e) => ({ ...e, [key]: String(err) }));
    } finally {
      setProvBusy((s) => { const n = new Set(s); n.delete(key); return n; });
      refreshProv();
    }
  }
  async function installAll() {
    for (const it of envItems) {
      if (!it.ready) await installOne(it.key); // sequential — conda solves shouldn't race
    }
  }

  const refreshProvs = useCallback(() => {
    invoke<ProviderState[]>("provider_status").then(setProvs).catch(() => {});
  }, []);

  useEffect(() => {
    if (cur === "syscheck" && !prov) refreshProv();
    if (cur === "providers") refreshProvs();
  }, [cur, prov, refreshProv, refreshProvs]);

  const finish = useCallback(() => {
    if (!consent) return;
    localStorage.setItem("axiom3.onboarded", "1");
    localStorage.setItem("axiom3.deploy", deploy);
    localStorage.setItem("axiom3.lit", lit ? "on" : "off");
    onDone();
  }, [consent, deploy, lit, onDone]);

  // Gates: the setup step needs every env prerequisite green; the providers step needs a GLM key.
  const gated = (cur === "syscheck" && !envAllReady) || (cur === "providers" && connected < 1);
  const next = useCallback(() => {
    if (gated) return;
    if (!last) setStep((s) => s + 1);
    else finish();
  }, [last, finish, gated]);

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


  function body() {
    switch (cur) {
      case "welcome":
        return (
          <>
            <p className="ax-wiz-brand">
              Axiom3<small>{tx("brand_sub")}</small>
            </p>
            <p style={{ margin: "8px 0 16px", lineHeight: 1.6 }}>{tx("lead")}</p>
            <div className="ax-wiz-mode">
              <Code size={18} color="var(--ax-accent)" style={{ marginTop: 2, flex: "none" }} aria-hidden />
              <div>
                <b>{tx("mode_a_t")}</b>
                <span>{tx("mode_a_d")}</span>
              </div>
            </div>
            <div className="ax-wiz-mode">
              <Network size={18} color="var(--ax-accent)" style={{ marginTop: 2, flex: "none" }} aria-hidden />
              <div>
                <b>{tx("mode_r_t")}</b>
                <span>{tx("mode_r_d")}</span>
              </div>
            </div>
            <p className="ax-wiz-soul">{tx("soul")}</p>
          </>
        );
      case "syscheck":
        return (
          <>
            <p style={{ margin: "0 0 10px" }}>{tx("setup_lead")}</p>
            <button
              className="ax-btn ax-btn--primary"
              onClick={installAll}
              disabled={provBusy.size > 0 || envAllReady}
              style={{ marginBottom: 12 }}
            >
              {provBusy.size > 0 ? tx("prov_installing") : tx("install_all")}
            </button>
            {prov === null && <p className="ax-hint">{tx("checking")}</p>}
            {(prov ?? []).map((it) => {
              const busy = provBusy.has(it.key);
              const err = provErr[it.key];
              const isKey = it.key === "apikey";
              return (
                <div className="ax-wiz-prov" key={it.key}>
                  <div className="ax-wiz-prov__row">
                    <span style={{ display: "inline-flex", alignItems: "center", gap: 8 }}>
                      <Lamp ok={it.ready} />
                      <span>{it.detail}</span>
                    </span>
                    {!it.ready && !isKey && (
                      <button className="ax-btn" onClick={() => installOne(it.key)} disabled={busy}>
                        {busy ? tx("prov_installing") : err ? tx("prov_retry") : tx("install")}
                      </button>
                    )}
                    {isKey && !it.ready && <span className="ax-hint" style={{ fontSize: 11 }}>{tx("prov_apikey_hint")}</span>}
                  </div>
                  {err && (
                    <div className="ax-wiz-prov__cli">
                      <span className="ax-hint">{tx("prov_cli")}</span>
                      <code>{it.cli}</code>
                    </div>
                  )}
                </div>
              );
            })}
            <p className="ax-hint" style={{ marginTop: 10, display: "inline-flex", alignItems: "center", gap: 6 }}>
              <Lamp ok={envAllReady} /> {envAllReady ? tx("prov_allok") : tx("prov_needall")}
            </p>
          </>
        );
      case "providers":
        return (
          <>
            <p style={{ margin: "0 0 10px" }}>{tx("prov_lead")}</p>
            {PROVIDERS.map((fam) => {
              const on = provs.find((p) => p.family === fam)?.connected;
              return (
                <button key={fam} className={`ax-wiz-opt ${provSel === fam ? "sel" : ""}`} onClick={() => setProvSel(fam)}>
                  <span style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
                    <span>{tx(PROV_LABEL[fam] ?? fam)}</span>
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
                placeholder={`${tx(PROV_LABEL[provSel] ?? provSel)} · ${tx("prov_key")}`}
                style={{ flex: 1, minWidth: 180 }}
              />
              <button className="ax-btn" onClick={saveKey} disabled={saving || !keyInput.trim()}>
                {tx("prov_save")}
              </button>
            </div>
            <p className="ax-hint" style={{ marginTop: 8, display: "inline-flex", alignItems: "center", gap: 6 }}>
              <Lamp ok={connected >= 1} /> {connected >= 1 ? tx("prov_ok") : tx("prov_need")}
            </p>
          </>
        );
      case "deploy": {
        // Cloud only for now; local/hybrid are shown but DISABLED (coming soon) so the roadmap is
        // visible without being selectable.
        const opts: [string, string, string, boolean][] = [
          ["cloud", tx("dep_cloud"), tx("dep_cloud_d"), true],
          ["hybrid", tx("dep_hybrid"), tx("dep_hybrid_d"), false],
          ["onprem", tx("dep_onprem"), tx("dep_onprem_d"), false],
        ];
        return (
          <>
            <p style={{ margin: "0 0 10px" }}>{tx("deploy_lead")}</p>
            {opts.map(([k, n, d, enabled]) => (
              <button
                key={k}
                className={`ax-wiz-opt ${deploy === k ? "sel" : ""} ${enabled ? "" : "is-disabled"}`}
                onClick={() => enabled && setDeploy(k)}
                disabled={!enabled}
              >
                <span style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
                  <span style={{ fontWeight: 500, color: "var(--ax-ink)" }}>{n}</span>
                  {!enabled && <span className="ax-hint" style={{ fontSize: 11 }}>{tx("dep_soon")}</span>}
                </span>
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

  // Hard gate: dots can't jump PAST an unsatisfied gate. Setup (env all green) then providers (a key).
  const setupIdx = STEPS.indexOf("syscheck");
  const provIdx = STEPS.indexOf("providers");
  const maxStep = !envAllReady ? setupIdx : connected < 1 ? provIdx : STEPS.length - 1;
  return (
    <div className="ax-wiz-overlay">
      <div className="ax-wiz">
        <div className="ax-wiz__head">
          <p className="ax-wiz__title">{cur === "syscheck" ? tx("setup_title") : tx(cur)}</p>
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
                onClick={() => i <= maxStep && setStep(i)}
                disabled={i > maxStep}
                aria-label={`step ${i + 1}`}
              />
            ))}
          </div>
          <button
            className="ax-iconbtn"
            onClick={next}
            disabled={(last && !consent) || gated}
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
