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
import { Download, ListFilter, X, Languages, Settings, Upload, ChevronLeft } from "lucide-react";


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
  const [view, setView] = useState<"board" | "new" | "detail">("board");
  const [activeId, setActiveId] = useState<string | null>(null);
  const [notableOnly, setNotableOnly] = useState(true);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [onboarded, setOnboarded] = useState(localStorage.getItem("axiom3.onboarded") === "1");
  const [settingsOpen, setSettingsOpen] = useState(false);

  useEffect(() => {
    applyTheme((localStorage.getItem("axiom3.theme") as "light" | "dark" | "system") || "system");
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

  return (
    <>
      {!onboarded && <OnboardingWizard lang={lang} onDone={() => setOnboarded(true)} />}
      {settingsOpen && <SettingsModal lang={lang} onLang={setLang} onClose={() => setSettingsOpen(false)} />}
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
              {view === "board" && (
                <>
                  <div className="ax-results__head">
                    <span className="ax-block-label" style={{ margin: 0 }}>{t(lang, "tasks_title")}</span>
                    <button className="ax-btn ax-btn--primary" onClick={() => setView("new")}>
                      {t(lang, "new_task")}
                    </button>
                  </div>
                  {tasks.length === 0 ? (
                    <div className="ax-empty" style={{ marginTop: "6vh" }}>
                      <p className="ax-empty__title">{t(lang, "no_tasks")}</p>
                      <p className="ax-empty__sub">{t(lang, "no_tasks_sub")}</p>
                    </div>
                  ) : (
                    <div className="ax-results__list">
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
                            {t(lang, "tt_check_draft")} · {t(lang, `st_${tk.status}`)}
                            {tk.result ? ` · ${tk.result.total} ${t(lang, "claims")}` : ""}
                          </span>
                        </button>
                      ))}
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
                </>
              )}

              {view === "new" && (
                <>
                  <button className="ax-iconbtn" onClick={() => setView("board")} title={t(lang, "back")} aria-label={t(lang, "back")} style={{ marginBottom: 10 }}>
                    <ChevronLeft size={18} aria-hidden />
                  </button>
                  <p className="ax-sec-label">{t(lang, "task_pick_type")}</p>
                  <div className="ax-type-grid">
                    <button className="ax-type ax-type--active">
                      <span className="ax-type__name">{t(lang, "tt_check_draft")}</span>
                      <span className="ax-type__d">{t(lang, "tt_check_draft_d")}</span>
                    </button>
                    {(["ppt", "code", "research"] as const).map((tt) => (
                      <button key={tt} className="ax-type" disabled>
                        <span className="ax-type__name">
                          {t(lang, `tt_${tt}`)} <span className="ax-hint">({t(lang, "coming_soon")})</span>
                        </span>
                        <span className="ax-type__d">{t(lang, `tt_${tt}_d`)}</span>
                      </button>
                    ))}
                  </div>

                  <p className="ax-block-label" style={{ marginTop: 16 }}>{t(lang, "check_title")}</p>
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
