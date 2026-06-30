import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { Lang, t } from "./i18n";
import "./theme.css";
import type { ProjectStatus, EventInfo, DraftCheck, ExportSummary } from "./lib/types";
import { ClaimDossier } from "./components/ClaimDossier";
import { OnboardingWizard } from "./components/OnboardingWizard";
import { Download, ListFilter, X, Languages, Settings, Upload } from "lucide-react";


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
  const [check, setCheck] = useState<DraftCheck | null>(null);
  const [notableOnly, setNotableOnly] = useState(true);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [onboarded, setOnboarded] = useState(localStorage.getItem("axiom3.onboarded") === "1");

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
      setCheck(null);
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

  async function checkDraft() {
    setProgress(null);
    const c = await run(() => invoke<DraftCheck>("check_draft", { draft, references }));
    setProgress(null);
    if (c) {
      setCheck(c);
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
    setCheck(null);
    note(t(lang, "closed_project"));
  }

  // A clean claim (no deterministic flag, no fleet verdict) is "unchecked" — shown
  // neutral, NOT as an alarming "uncertain". Only flagged / verdicted claims stand out.

  return (
    <>
      {!onboarded && <OnboardingWizard lang={lang} onDone={() => setOnboarded(true)} />}
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

        <div className="ax-side-spacer" />

        <div className="ax-side-foot">
          <button
            className="ax-iconbtn"
            onClick={() => setLang(lang === "zh" ? "en" : "zh")}
            title={lang === "zh" ? "English" : "中文"}
            aria-label="language"
          >
            <Languages size={16} aria-hidden />
            <span style={{ fontSize: 12 }}>{lang === "zh" ? "EN" : "中"}</span>
          </button>
          <button className="ax-iconbtn" disabled title={t(lang, "settings_soon")} aria-label="settings">
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
        </header>

        <div className="ax-content">
          {error && <p className="ax-error">⚠ {error}</p>}

          {!status ? (
            <div className="ax-empty">
              <p className="ax-empty__title">{t(lang, "empty_title")}</p>
              <p className="ax-empty__sub">{t(lang, "empty_sub")}</p>
            </div>
          ) : (
            <div className="ax-canvas">
              <p className="ax-block-label">{t(lang, "check_title")}</p>
              <textarea
                className="ax-textarea"
                value={draft}
                onChange={(e) => setDraft(e.currentTarget.value)}
                rows={10}
                placeholder={t(lang, "check_placeholder")}
              />
              <div className="ax-toolrow">
                <button className="ax-btn ax-btn--primary" onClick={checkDraft} disabled={busy || !draft.trim()}>
                  {t(lang, "check_btn")}
                </button>
                <button
                  className="ax-iconbtn"
                  onClick={() => importInto(setDraft)}
                  disabled={busy}
                  title={t(lang, "import_btn")}
                  aria-label={t(lang, "import_btn")}
                >
                  <Upload size={16} aria-hidden />
                </button>
                <span className="ax-hint">
                  {draft.length.toLocaleString()} {t(lang, "chars")}
                </span>
              </div>

              <details style={{ marginTop: 14 }}>
                <summary className="ax-block-label" style={{ cursor: "pointer" }}>
                  {t(lang, "references_title")}
                </summary>
                <textarea
                  className="ax-textarea"
                  style={{ marginTop: 6 }}
                  value={references}
                  onChange={(e) => setReferences(e.currentTarget.value)}
                  rows={6}
                  placeholder={t(lang, "references_placeholder")}
                />
                <div className="ax-toolrow">
                  <button
                    className="ax-iconbtn"
                    onClick={() => importInto(setReferences)}
                    disabled={busy}
                    title={t(lang, "import_refs")}
                    aria-label={t(lang, "import_refs")}
                  >
                    <Upload size={16} aria-hidden />
                  </button>
                  <span className="ax-hint">
                    {references.length.toLocaleString()} {t(lang, "chars")}
                  </span>
                </div>
              </details>

              {busy && (
                <div className="ax-progress">
                  <span>
                    {t(lang, "checking")}
                    {progress ? ` ${progress.done}/${progress.total}` : ""}
                  </span>
                  {progress && progress.total > 0 && (
                    <span className="ax-progress__bar">
                      <span
                        className="ax-progress__fill"
                        style={{ width: `${Math.round((progress.done / progress.total) * 100)}%` }}
                      />
                    </span>
                  )}
                  <button
                    className="ax-iconbtn"
                    onClick={cancelCheck}
                    title={t(lang, "cancel")}
                    aria-label={t(lang, "cancel")}
                  >
                    <X size={14} aria-hidden />
                  </button>
                </div>
              )}

              {check && (
                <div style={{ marginTop: 18 }}>
                  <div className="ax-results__head">
                    <span className="ax-results__meta">
                      {check.total} {t(lang, "claims")} · {check.flagged} {t(lang, "flagged")} ·{" "}
                      {check.fleet_verified} {t(lang, "cross_vendor_n")}
                    </span>
                    <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
                      <button
                        className="ax-iconbtn"
                        onClick={exportProvenance}
                        disabled={busy}
                        title={t(lang, "export_provenance")}
                        aria-label={t(lang, "export_provenance")}
                      >
                        <Download size={16} aria-hidden />
                      </button>
                      <button
                        className="ax-iconbtn"
                        onClick={() => setNotableOnly(!notableOnly)}
                        title={notableOnly ? t(lang, "show_all") : t(lang, "only_notable")}
                        aria-label={notableOnly ? t(lang, "show_all") : t(lang, "only_notable")}
                      >
                        <ListFilter size={16} aria-hidden />
                      </button>
                    </div>
                  </div>
                  <p className="ax-philosophy">{t(lang, "results_philosophy")}</p>
                  <div className="ax-results__list" style={{ marginTop: 10 }}>
                    {check.entries
                      .filter(
                        (e) =>
                          !notableOnly ||
                          e.findings.length > 0 ||
                          e.fleet ||
                          e.status === "injection_neutralized" ||
                          e.reason?.includes("fleet_cap_exceeded")
                      )
                      .map((e) => (
                        <ClaimDossier key={e.claim_id} entry={e} lang={lang} />
                      ))}
                  </div>
                </div>
              )}

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
            </div>
          )}
        </div>
      </main>
      </div>
    </>
  );
}

export default App;
