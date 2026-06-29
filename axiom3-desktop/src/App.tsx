import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
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

type ClaimEntry = {
  claim_id: string;
  text: string;
  claim_type: string;
  is_bearing: boolean;
  status: string;
  reason: string | null;
  findings: FindingView[];
};

type DraftCheck = {
  event_seq: number;
  total: number;
  flagged: number;
  entries: ClaimEntry[];
};

function App() {
  const [name, setName] = useState("");
  const [status, setStatus] = useState<ProjectStatus | null>(null);
  const [actor, setActor] = useState("researcher");
  const [op, setOp] = useState("note");
  const [payload, setPayload] = useState('{"text":"hello"}');
  const [log, setLog] = useState<string[]>([]);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [draft, setDraft] = useState(
    "This is the first-ever room-temperature synthesis. The yield was 94%. The yield was 80%."
  );
  const [check, setCheck] = useState<DraftCheck | null>(null);

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
      setCheck(null); // clear any ledger from a previously-open project
      note(`opened project "${s.project_id}" (${s.event_count} events)`);
    }
  }

  async function appendEvent() {
    const e = await run(() =>
      invoke<EventInfo>("append_event", { actor, op, payload })
    );
    if (e) {
      note(`#${e.seq} ${e.actor}/${e.op} → ${e.entry_hash.slice(0, 12)}…`);
      const s = await run(() => invoke<ProjectStatus>("project_status"));
      if (s) setStatus(s);
    }
  }

  async function verifyChain() {
    const ok = await run(() => invoke<boolean>("verify_chain"));
    if (ok !== undefined) note(`verify_chain → ${ok ? "intact ✓" : "BROKEN ✗"}`);
  }

  async function vectorSelfTest() {
    const r = await run(() => invoke<string>("vector_self_test"));
    if (r) note(`vector self-test → ${r}`);
  }

  async function checkDraft() {
    const c = await run(() => invoke<DraftCheck>("check_draft", { draft }));
    if (c) {
      setCheck(c);
      note(`checked draft → ${c.total} claims, ${c.flagged} flagged (event #${c.event_seq})`);
      const s = await run(() => invoke<ProjectStatus>("project_status"));
      if (s) setStatus(s);
    }
  }

  async function closeProject() {
    await run(() => invoke("close_project"));
    setStatus(null);
    setCheck(null);
    note("closed project");
  }

  function statusColor(s: string): string {
    if (s === "verified") return "var(--text-success, green)";
    if (s === "refuted") return "var(--text-danger, crimson)";
    if (s === "uncertain") return "#b8860b";
    return "gray";
  }

  return (
    <main className="container" style={{ maxWidth: 720, textAlign: "left" }}>
      <h1>Axiom3 — workbench</h1>
      <p style={{ opacity: 0.7 }}>
        Per-project encrypted system-of-record (SQLCipher + hash-chained event
        log) and the LanceDB vector tier, wired through the shell.
      </p>

      <section style={{ display: "flex", gap: 8, alignItems: "center" }}>
        <input
          value={name}
          onChange={(e) => setName(e.currentTarget.value)}
          placeholder="project name…"
        />
        <button onClick={openProject} disabled={busy || !name.trim()}>
          Open / create
        </button>
        {status && (
          <button onClick={closeProject} disabled={busy}>
            Close
          </button>
        )}
      </section>

      {error && (
        <p style={{ color: "crimson", whiteSpace: "pre-wrap" }}>⚠ {error}</p>
      )}

      {status && (
        <>
          <section
            style={{
              marginTop: 16,
              padding: 12,
              border: "1px solid #8884",
              borderRadius: 8,
            }}
          >
            <div>
              <b>{status.project_id}</b>{" "}
              <span style={{ color: status.chain_ok ? "green" : "crimson" }}>
                {status.chain_ok ? "chain intact ✓" : "chain BROKEN ✗"}
              </span>
            </div>
            <div style={{ fontSize: 12, opacity: 0.7 }}>{status.root}</div>
            <div style={{ fontSize: 12 }}>
              events: {status.event_count} · head:{" "}
              <code>{status.chain_head?.slice(0, 16) ?? "—"}</code>
            </div>
          </section>

          <section style={{ marginTop: 16, display: "grid", gap: 6 }}>
            <div style={{ display: "flex", gap: 8 }}>
              <input
                value={actor}
                onChange={(e) => setActor(e.currentTarget.value)}
                placeholder="actor"
                style={{ width: 120 }}
              />
              <input
                value={op}
                onChange={(e) => setOp(e.currentTarget.value)}
                placeholder="op"
                style={{ width: 120 }}
              />
              <input
                value={payload}
                onChange={(e) => setPayload(e.currentTarget.value)}
                placeholder="payload (JSON)"
                style={{ flex: 1 }}
              />
            </div>
            <div style={{ display: "flex", gap: 8 }}>
              <button onClick={appendEvent} disabled={busy}>
                Append event
              </button>
              <button onClick={verifyChain} disabled={busy}>
                Verify chain
              </button>
              <button onClick={vectorSelfTest} disabled={busy}>
                Vector self-test
              </button>
            </div>
          </section>

          <section style={{ marginTop: 20 }}>
            <div style={{ fontSize: 13, fontWeight: 500, marginBottom: 6 }}>
              Check a draft (deterministic tier)
            </div>
            <textarea
              value={draft}
              onChange={(e) => setDraft(e.currentTarget.value)}
              rows={4}
              placeholder="Paste a draft… each sentence becomes a claim and is risk-checked."
              style={{ width: "100%", fontFamily: "inherit", fontSize: 13, padding: 8 }}
            />
            <button onClick={checkDraft} disabled={busy || !draft.trim()} style={{ marginTop: 6 }}>
              Check draft
            </button>

            {check && (
              <div style={{ marginTop: 12 }}>
                <div style={{ fontSize: 13, marginBottom: 8 }}>
                  {check.total} claims · <b>{check.flagged} flagged</b> · recorded as event #
                  {check.event_seq}
                </div>
                <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
                  {check.entries.map((e) => (
                    <div
                      key={e.claim_id}
                      style={{
                        border: "0.5px solid #8884",
                        borderLeft: `3px solid ${statusColor(e.status)}`,
                        borderRadius: 6,
                        padding: "8px 10px",
                      }}
                    >
                      <div style={{ display: "flex", gap: 8, alignItems: "baseline" }}>
                        <span
                          style={{
                            fontSize: 11,
                            color: statusColor(e.status),
                            textTransform: "uppercase",
                            minWidth: 70,
                          }}
                        >
                          {e.status}
                        </span>
                        <span style={{ fontSize: 14 }}>{e.text}</span>
                      </div>
                      {e.findings.length > 0 && (
                        <ul style={{ margin: "6px 0 0 78px", fontSize: 12, color: "#a05a00" }}>
                          {e.findings.map((f, i) => (
                            <li key={i}>
                              <code>{f.checker}</code>: {f.message}
                            </li>
                          ))}
                        </ul>
                      )}
                    </div>
                  ))}
                </div>
              </div>
            )}
          </section>
        </>
      )}

      {log.length > 0 && (
        <section style={{ marginTop: 16 }}>
          <div style={{ fontSize: 12, opacity: 0.6 }}>activity</div>
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
