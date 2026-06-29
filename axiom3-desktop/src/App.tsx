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

function App() {
  const [name, setName] = useState("");
  const [status, setStatus] = useState<ProjectStatus | null>(null);
  const [actor, setActor] = useState("researcher");
  const [op, setOp] = useState("note");
  const [payload, setPayload] = useState('{"text":"hello"}');
  const [log, setLog] = useState<string[]>([]);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

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

  async function closeProject() {
    await run(() => invoke("close_project"));
    setStatus(null);
    note("closed project");
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
