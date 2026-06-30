import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { Lang, t, verdictLabel, indepLabel, findingDesc, reasonLabel } from "./i18n";
import "./App.css";

type ProjectStatus = {
  project_id: string;
  root: string;
  event_count: number;
  chain_head: string | null;
  chain_ok: boolean;
};

type EventInfo = {
  seq: number;
  ts: number;
  actor: string;
  op: string;
  entry_hash: string;
};

type FindingView = { checker: string; message: string; excerpt: string | null };
type VendorView = { family: string; model: string; status: string; reasoning: string };
type FleetView = {
  status: string;
  independence: string;
  reason: string;
  vendors: VendorView[];
};
type ClaimEntry = {
  claim_id: string;
  text: string;
  claim_type: string;
  is_bearing: boolean;
  status: string;
  reason: string | null;
  findings: FindingView[];
  fleet: FleetView | null;
  evidence: string[];
};
type DraftCheck = {
  event_seq: number;
  total: number;
  flagged: number;
  fleet_verified: number;
  entries: ClaimEntry[];
};

type ExportSummary = {
  path: string;
  bytes: number;
  claims: number;
  event_seq: number;
  chain_intact: boolean;
};

function statusColor(s: string): string {
  if (s === "verified") return "var(--text-success, green)";
  if (s === "refuted") return "var(--text-danger, crimson)";
  if (s === "uncertain") return "#b8860b";
  if (s === "injection_neutralized" || s === "quarantined") return "#7b2fbe";
  return "gray";
}

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
  function entryStatus(e: ClaimEntry): { label: string; color: string } {
    if (e.fleet) return { label: verdictLabel(lang, e.status), color: statusColor(e.status) };
    // A neutralized injection is a terminal verdict in its own right (no fleet ran).
    if (e.status === "injection_neutralized")
      return { label: verdictLabel(lang, e.status), color: statusColor(e.status) };
    if (e.findings.length > 0)
      return { label: verdictLabel(lang, "uncertain"), color: statusColor("uncertain") };
    return { label: t(lang, "unchecked"), color: "gray" };
  }

  return (
    <main className="container" style={{ maxWidth: 720, textAlign: "left" }}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
        <h1 style={{ margin: 0 }}>{t(lang, "app_title")}</h1>
        <button
          onClick={() => setLang(lang === "zh" ? "en" : "zh")}
          style={{ fontSize: 13, padding: "2px 10px" }}
        >
          {lang === "zh" ? "EN" : "中文"}
        </button>
      </div>
      <p style={{ opacity: 0.7 }}>{t(lang, "app_subtitle")}</p>

      <section style={{ display: "flex", gap: 8, alignItems: "center" }}>
        <input
          value={name}
          onChange={(e) => setName(e.currentTarget.value)}
          placeholder={t(lang, "project_placeholder")}
        />
        <button onClick={openProject} disabled={busy || !name.trim()}>
          {t(lang, "open_create")}
        </button>
        {status && (
          <button onClick={closeProject} disabled={busy}>
            {t(lang, "close")}
          </button>
        )}
      </section>

      {error && <p style={{ color: "crimson", whiteSpace: "pre-wrap" }}>⚠ {error}</p>}

      {status && (
        <>
          <section
            style={{ marginTop: 16, padding: 12, border: "1px solid #8884", borderRadius: 8 }}
          >
            <div>
              <b>{status.project_id}</b>{" "}
              <span style={{ color: status.chain_ok ? "green" : "crimson" }}>
                {status.chain_ok ? t(lang, "chain_intact") : t(lang, "chain_broken")}
              </span>
            </div>
            <div style={{ fontSize: 12, opacity: 0.7 }}>{status.root}</div>
            <div style={{ fontSize: 12 }}>
              {t(lang, "events")}: {status.event_count} · {t(lang, "head")}:{" "}
              <code>{status.chain_head?.slice(0, 16) ?? "—"}</code>
            </div>
          </section>

          <section style={{ marginTop: 16, display: "grid", gap: 6 }}>
            <div style={{ display: "flex", gap: 8 }}>
              <input value={actor} onChange={(e) => setActor(e.currentTarget.value)} placeholder={t(lang, "actor")} style={{ width: 120 }} />
              <input value={op} onChange={(e) => setOp(e.currentTarget.value)} placeholder={t(lang, "op")} style={{ width: 120 }} />
              <input value={payload} onChange={(e) => setPayload(e.currentTarget.value)} placeholder={t(lang, "payload")} style={{ flex: 1 }} />
            </div>
            <div style={{ display: "flex", gap: 8 }}>
              <button onClick={appendEvent} disabled={busy}>{t(lang, "append_event")}</button>
              <button onClick={verifyChain} disabled={busy}>{t(lang, "verify_chain")}</button>
              <button onClick={vectorSelfTest} disabled={busy}>{t(lang, "vector_self_test")}</button>
            </div>
          </section>

          <section style={{ marginTop: 20 }}>
            <div style={{ fontSize: 13, fontWeight: 500, marginBottom: 6 }}>{t(lang, "check_title")}</div>
            <textarea
              value={draft}
              onChange={(e) => setDraft(e.currentTarget.value)}
              rows={10}
              placeholder={t(lang, "check_placeholder")}
              style={{ width: "100%", fontFamily: "inherit", fontSize: 13, padding: 8 }}
            />
            <div style={{ display: "flex", gap: 8, marginTop: 6, alignItems: "center" }}>
              <button onClick={() => importInto(setDraft)} disabled={busy}>
                {t(lang, "import_btn")}
              </button>
              <button onClick={checkDraft} disabled={busy || !draft.trim()}>
                {t(lang, "check_btn")}
              </button>
              <span style={{ fontSize: 12, opacity: 0.6 }}>
                {draft.length.toLocaleString()} {t(lang, "chars")}
              </span>
            </div>

            <details style={{ marginTop: 12 }}>
              <summary style={{ fontSize: 13, fontWeight: 500, cursor: "pointer" }}>
                {t(lang, "references_title")}
              </summary>
              <textarea
                value={references}
                onChange={(e) => setReferences(e.currentTarget.value)}
                rows={6}
                placeholder={t(lang, "references_placeholder")}
                style={{
                  width: "100%",
                  fontFamily: "inherit",
                  fontSize: 13,
                  padding: 8,
                  marginTop: 6,
                }}
              />
              <div style={{ display: "flex", gap: 8, marginTop: 6, alignItems: "center" }}>
                <button onClick={() => importInto(setReferences)} disabled={busy}>
                  {t(lang, "import_refs")}
                </button>
                <span style={{ fontSize: 12, opacity: 0.6 }}>
                  {references.length.toLocaleString()} {t(lang, "chars")}
                </span>
              </div>
            </details>
            {busy && (
              <div
                style={{
                  fontSize: 12,
                  color: "#185fa5",
                  marginTop: 6,
                  display: "flex",
                  gap: 8,
                  alignItems: "center",
                }}
              >
                <span>
                  ⏳ {t(lang, "checking")}
                  {progress ? ` ${progress.done}/${progress.total}` : ""}
                </span>
                {progress && progress.total > 0 && (
                  <span
                    style={{
                      flex: 1,
                      maxWidth: 200,
                      height: 4,
                      background: "#8883",
                      borderRadius: 2,
                      overflow: "hidden",
                    }}
                  >
                    <span
                      style={{
                        display: "block",
                        height: "100%",
                        width: `${Math.round((progress.done / progress.total) * 100)}%`,
                        background: "#185fa5",
                      }}
                    />
                  </span>
                )}
                <button onClick={cancelCheck} style={{ fontSize: 11, padding: "1px 8px" }}>
                  {t(lang, "cancel")}
                </button>
              </div>
            )}

            {check && (
              <div style={{ marginTop: 12 }}>
                <div
                  style={{
                    fontSize: 13,
                    marginBottom: 8,
                    display: "flex",
                    justifyContent: "space-between",
                    alignItems: "center",
                  }}
                >
                  <span>
                    {check.total} {t(lang, "claims")} · <b>{check.flagged} {t(lang, "flagged")}</b> ·{" "}
                    {check.fleet_verified} {t(lang, "cross_vendor_n")} · {t(lang, "event_no")} #
                    {check.event_seq}
                  </span>
                  <div style={{ display: "flex", gap: 8 }}>
                    <button
                      onClick={exportProvenance}
                      disabled={busy}
                      title={t(lang, "export_provenance_hint")}
                      style={{ fontSize: 12, padding: "2px 8px" }}
                    >
                      {t(lang, "export_provenance")}
                    </button>
                    <button
                      onClick={() => setNotableOnly(!notableOnly)}
                      style={{ fontSize: 12, padding: "2px 8px" }}
                    >
                      {notableOnly ? t(lang, "show_all") : t(lang, "only_notable")}
                    </button>
                  </div>
                </div>
                <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
                  {check.entries
                    .filter(
                      (e) =>
                        !notableOnly ||
                        e.findings.length > 0 ||
                        e.fleet ||
                        e.status === "injection_neutralized" ||
                        // A bearing conclusion that lost the fleet budget is notable —
                        // an unverified conclusion the user should see (won't flood: only
                        // the over-cap overflow, not the no-providers case).
                        e.reason?.includes("fleet_cap_exceeded")
                    )
                    .map((e) => {
                      const st = entryStatus(e);
                      return (
                        <div
                          key={e.claim_id}
                          style={{
                            border: "0.5px solid #8884",
                            borderLeft: `3px solid ${st.color}`,
                            borderRadius: 6,
                            padding: "8px 10px",
                          }}
                        >
                          <div style={{ display: "flex", gap: 8, alignItems: "baseline" }}>
                            <span style={{ fontSize: 11, color: st.color, minWidth: 56 }}>{st.label}</span>
                            <span style={{ fontSize: 14 }}>{e.text}</span>
                          </div>
                          {/* Why the verdict landed where it did — the grounded direction
                              for fleet-judged claims, and for a BEARING conclusion the
                              honest reason it was (or wasn't) cross-checked (budget /
                              unavailable). Not shown for clean non-bearing claims. */}
                          {e.reason &&
                            (e.fleet || e.is_bearing || e.status === "injection_neutralized") &&
                            e.reason !== "clean_pending" && (
                              <div style={{ margin: "4px 0 0 64px", fontSize: 12, opacity: 0.85, color: st.color }}>
                                {reasonLabel(lang, e.reason)}
                              </div>
                            )}
                          {e.findings.length > 0 && (
                            <ul style={{ margin: "6px 0 0 64px", fontSize: 12, color: "#a05a00" }}>
                              {e.findings.map((f, i) => (
                                <li key={i}>
                                  {findingDesc(lang, f.checker)}
                                  {f.excerpt ? ` 「${f.excerpt}」` : ""}
                                </li>
                              ))}
                            </ul>
                          )}
                          {e.fleet && (
                            <div style={{ margin: "6px 0 0 64px", fontSize: 12 }}>
                              <div style={{ opacity: 0.7, marginBottom: 2 }}>
                                {t(lang, "cross_vendor")}({indepLabel(lang, e.fleet.independence)})
                              </div>
                              {e.fleet.vendors.map((v, i) => (
                                <div key={i} style={{ marginBottom: 2 }}>
                                  {v.family}/{v.model}{" "}
                                  <b style={{ color: statusColor(v.status) }}>
                                    {verdictLabel(lang, v.status)}
                                  </b>
                                  {v.reasoning ? (
                                    <span style={{ opacity: 0.7 }}> — {v.reasoning}</span>
                                  ) : null}
                                </div>
                              ))}
                            </div>
                          )}
                          {e.evidence.length > 0 &&
                            (e.fleet || e.status === "injection_neutralized") && (
                            <div style={{ margin: "6px 0 0 64px", fontSize: 12 }}>
                              <div style={{ opacity: 0.7, marginBottom: 2 }}>
                                {t(
                                  lang,
                                  e.status === "injection_neutralized"
                                    ? "injection_evidence_label"
                                    : "evidence_label"
                                )}
                              </div>
                              <ol style={{ margin: 0, paddingLeft: 18, opacity: 0.85 }}>
                                {e.evidence.map((p, i) => (
                                  <li key={i} style={{ marginBottom: 2 }}>
                                    {p}
                                  </li>
                                ))}
                              </ol>
                            </div>
                          )}
                        </div>
                      );
                    })}
                </div>
              </div>
            )}
          </section>
        </>
      )}

      {log.length > 0 && (
        <section style={{ marginTop: 16 }}>
          <div style={{ fontSize: 12, opacity: 0.6 }}>{t(lang, "activity")}</div>
          <ul style={{ fontSize: 13, fontFamily: "monospace", lineHeight: 1.6 }}>
            {log.map((l, i) => (
              <li key={i}>{l}</li>
            ))}
          </ul>
        </section>
      )}
    </main>
  );
}

export default App;
