//! Encrypted SQLite (SQLCipher) durable store + hash-chained event log.
//!
//! Implements the [`axiom_core::store`] traits. This is the `durable.db` backend
//! from the SSOT (§一): STRICT tables, AES-256 at rest via SQLCipher, verdicts
//! insert-only, and an append-only event log whose `entry_hash` chain is computed
//! by the pure helpers in `axiom-core`.
//!
//! `cache.db` (rebuildable) and the LanceDB vector index reuse the same crate and
//! land in later P0.2 increments.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};

use axiom_core::model::{
    now_ms, CasState, Claim, ClaimType, EvidenceSpan, Event, Independence, RedactionState,
    RetractedStatus, Source, Verdict, VerdictStatus, Verifier, VerifierKind,
};
use axiom_core::store::{
    canonical_json, entry_hash, DurableStore, EventLog, Result, StoreError, GENESIS_PREV_HASH,
};

/// Current durable-schema version (`PRAGMA user_version`).
const SCHEMA_VERSION: i64 = 1;

/// DDL for schema v1 (subset of SSOT §一: core entities + event log).
/// Edge/junction tables (verdict_uses_evidence, claim_relation, …) land next.
const MIGRATION_V1: &str = r#"
CREATE TABLE IF NOT EXISTS event (
  seq           INTEGER PRIMARY KEY,
  ts            INTEGER NOT NULL,
  actor         TEXT NOT NULL,
  op            TEXT NOT NULL,
  payload_json  TEXT NOT NULL,
  prev_hash     TEXT NOT NULL,
  entry_hash    TEXT NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS source (
  source_id            TEXT PRIMARY KEY,
  project_id           TEXT NOT NULL,
  blake3_hash          TEXT NOT NULL,
  uri                  TEXT,
  mime                 TEXT,
  acquisition_channel  TEXT,
  license              TEXT,
  source_locale        TEXT,
  data_class           TEXT,
  retracted_status     TEXT NOT NULL,
  cas_state            TEXT NOT NULL,
  created_at           INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS claim (
  claim_id       TEXT PRIMARY KEY,
  project_id     TEXT NOT NULL,
  text           TEXT NOT NULL,
  claim_type     TEXT NOT NULL,
  is_bearing     INTEGER NOT NULL,
  output_locale  TEXT,
  created_at     INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS evidence_span (
  evidence_id     TEXT PRIMARY KEY,
  source_id       TEXT NOT NULL REFERENCES source(source_id),
  blake3_hash     TEXT NOT NULL,
  start_offset    INTEGER,
  end_offset      INTEGER,
  verbatim_text   TEXT,
  parse_trust     REAL,
  redaction_state TEXT NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS verifier (
  verifier_id   TEXT PRIMARY KEY,
  kind          TEXT NOT NULL,
  model_id      TEXT,
  model_family  TEXT,
  version       TEXT,
  prompt_hash   TEXT,
  role          TEXT
) STRICT;

-- Verdicts are insert-only (immutable + versioned). The CHECK enforces the
-- strict 5-state vocabulary at the DB layer (SSOT §〇.2).
CREATE TABLE IF NOT EXISTS verdict (
  verdict_id            TEXT PRIMARY KEY,
  claim_id              TEXT NOT NULL REFERENCES claim(claim_id),
  verifier_id           TEXT NOT NULL REFERENCES verifier(verifier_id),
  status                TEXT NOT NULL CHECK (status IN
                          ('verified','refuted','uncertain','quarantined','injection_neutralized')),
  evidence_redacted     INTEGER NOT NULL DEFAULT 0,
  calibrated_confidence REAL,
  uncertain_reason      TEXT,
  independence          TEXT,
  revises_verdict_id    TEXT REFERENCES verdict(verdict_id),
  created_event_seq     INTEGER NOT NULL
) STRICT;
CREATE INDEX IF NOT EXISTS idx_verdict_claim ON verdict(claim_id);

CREATE TABLE IF NOT EXISTS quarantine_event (
  id                TEXT PRIMARY KEY,
  source_id         TEXT REFERENCES source(source_id),
  trigger_rule      TEXT NOT NULL,
  detail            TEXT,
  created_event_seq INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS usage_record (
  id                       TEXT PRIMARY KEY,
  task_id                  TEXT,
  modality                 TEXT NOT NULL,
  provider                 TEXT NOT NULL,
  model                    TEXT NOT NULL,
  input_units              INTEGER NOT NULL,
  output_units             INTEGER NOT NULL,
  cache_read_input_tokens  INTEGER NOT NULL DEFAULT 0,
  normalized_usd           REAL NOT NULL DEFAULT 0,
  created_at               INTEGER NOT NULL
) STRICT;
"#;

fn backend<E: std::fmt::Display>(e: E) -> StoreError {
    StoreError::Backend(e.to_string())
}

/// Classify a rusqlite error: constraint violations (PRIMARY KEY / UNIQUE / FK /
/// CHECK) become [`StoreError::Conflict`] so callers can distinguish an expected
/// conflict (e.g. a duplicate immutable verdict) from a real backend failure.
fn map_sqlite(e: rusqlite::Error) -> StoreError {
    if let rusqlite::Error::SqliteFailure(err, msg) = &e {
        if err.code == rusqlite::ErrorCode::ConstraintViolation {
            return StoreError::Conflict(msg.clone().unwrap_or_else(|| e.to_string()));
        }
    }
    StoreError::Backend(e.to_string())
}

fn decode<T>(field: &str, v: Option<T>) -> Result<T> {
    v.ok_or_else(|| StoreError::Integrity(format!("invalid coded value in column `{field}`")))
}

/// Encrypted SQLite durable store. `Connection` is `Send` but not `Sync`, so the
/// (synchronous) crate guards it with a `Mutex`; the async shell wraps calls in a
/// blocking task / `tokio::task::spawn_blocking`.
pub struct SqliteStore {
    conn: Mutex<Connection>,
}

impl SqliteStore {
    /// Open (or create) an encrypted durable store at `path` with `key` as the
    /// SQLCipher passphrase, then run migrations. An empty key opens unencrypted
    /// (tests only) — production keys come from the OS keychain in P0.3.
    pub fn open(path: impl AsRef<Path>, key: &str) -> Result<Self> {
        let conn = Connection::open(path).map_err(backend)?;
        Self::init(conn, key)
    }

    /// In-memory store (tests).
    pub fn open_in_memory(key: &str) -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(backend)?;
        Self::init(conn, key)
    }

    fn init(conn: Connection, key: &str) -> Result<Self> {
        if !key.is_empty() {
            // Set the SQLCipher key via a bound parameter (no string interpolation).
            conn.pragma_update(None, "key", key).map_err(backend)?;
        }
        // Fail fast if the key is wrong / db is not actually decryptable.
        conn.pragma_update(None, "foreign_keys", true)
            .map_err(|e| StoreError::Backend(format!("open/key failed: {e}")))?;
        conn.execute_batch(MIGRATION_V1).map_err(backend)?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(backend)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        // A poisoned lock means a prior writer panicked mid-transaction; recover
        // the guard rather than cascading the panic (SQLite tx already rolled back).
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }
}

impl EventLog for SqliteStore {
    fn append_event(&self, actor: &str, op: &str, payload: &serde_json::Value) -> Result<Event> {
        let canonical = canonical_json(payload)?;
        let mut guard = self.lock();
        let tx = guard.transaction().map_err(backend)?;

        let head: Option<(i64, String)> = tx
            .query_row(
                "SELECT seq, entry_hash FROM event ORDER BY seq DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(backend)?;

        let (seq, prev_hash) = match head {
            Some((s, h)) => (s + 1, h),
            None => (1, GENESIS_PREV_HASH.to_string()),
        };
        let ts = now_ms();
        let entry = entry_hash(&prev_hash, &canonical);

        tx.execute(
            "INSERT INTO event (seq, ts, actor, op, payload_json, prev_hash, entry_hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![seq, ts, actor, op, canonical, prev_hash, entry],
        )
        .map_err(backend)?;
        tx.commit().map_err(backend)?;

        Ok(Event {
            seq,
            ts,
            actor: actor.to_string(),
            op: op.to_string(),
            payload_json: canonical,
            prev_hash,
            entry_hash: entry,
        })
    }

    fn head_hash(&self) -> Result<Option<String>> {
        self.lock()
            .query_row("SELECT entry_hash FROM event ORDER BY seq DESC LIMIT 1", [], |r| r.get(0))
            .optional()
            .map_err(backend)
    }

    fn replay(&self) -> Result<Vec<Event>> {
        let guard = self.lock();
        let mut stmt = guard
            .prepare("SELECT seq, ts, actor, op, payload_json, prev_hash, entry_hash FROM event ORDER BY seq ASC")
            .map_err(backend)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(Event {
                    seq: r.get(0)?,
                    ts: r.get(1)?,
                    actor: r.get(2)?,
                    op: r.get(3)?,
                    payload_json: r.get(4)?,
                    prev_hash: r.get(5)?,
                    entry_hash: r.get(6)?,
                })
            })
            .map_err(backend)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(backend)
    }

    fn verify_chain(&self) -> Result<bool> {
        let mut prev = GENESIS_PREV_HASH.to_string();
        for (i, e) in self.replay()?.into_iter().enumerate() {
            // seq must be a dense 1..N run — catches deleted/reordered rows that
            // hash linkage alone would miss.
            let expected_seq = i as i64 + 1;
            if e.seq != expected_seq {
                return Ok(false);
            }
            if e.prev_hash != prev {
                return Ok(false);
            }
            // payload_json is stored canonical, so recompute directly.
            if entry_hash(&prev, &e.payload_json) != e.entry_hash {
                return Ok(false);
            }
            prev = e.entry_hash;
        }
        Ok(true)
    }
}

impl DurableStore for SqliteStore {
    fn put_source(&self, s: &Source) -> Result<()> {
        self.lock()
            .execute(
                "INSERT OR REPLACE INTO source
                 (source_id, project_id, blake3_hash, uri, mime, acquisition_channel, license,
                  source_locale, data_class, retracted_status, cas_state, created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                params![
                    s.id, s.project_id, s.blake3_hash, s.uri, s.mime, s.acquisition_channel,
                    s.license, s.source_locale, s.data_class,
                    s.retracted_status.code(), s.cas_state.code(), s.created_at
                ],
            )
            .map_err(backend)?;
        Ok(())
    }

    fn get_source(&self, id: &str) -> Result<Option<Source>> {
        self.lock()
            .query_row(
                "SELECT source_id, project_id, blake3_hash, uri, mime, acquisition_channel, license,
                        source_locale, data_class, retracted_status, cas_state, created_at
                 FROM source WHERE source_id = ?1",
                params![id],
                |r| {
                    Ok((
                        Source {
                            id: r.get(0)?,
                            project_id: r.get(1)?,
                            blake3_hash: r.get(2)?,
                            uri: r.get(3)?,
                            mime: r.get(4)?,
                            acquisition_channel: r.get(5)?,
                            license: r.get(6)?,
                            source_locale: r.get(7)?,
                            data_class: r.get(8)?,
                            retracted_status: RetractedStatus::Clean, // replaced below
                            cas_state: CasState::Present,             // replaced below
                            created_at: r.get(11)?,
                        },
                        r.get::<_, String>(9)?,
                        r.get::<_, String>(10)?,
                    ))
                },
            )
            .optional()
            .map_err(backend)?
            .map(|(mut s, rs, cs)| {
                s.retracted_status = decode("retracted_status", RetractedStatus::from_code(&rs))?;
                s.cas_state = decode("cas_state", CasState::from_code(&cs))?;
                Ok(s)
            })
            .transpose()
    }

    fn put_claim(&self, c: &Claim) -> Result<()> {
        self.lock()
            .execute(
                "INSERT OR REPLACE INTO claim
                 (claim_id, project_id, text, claim_type, is_bearing, output_locale, created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![
                    c.id, c.project_id, c.text, c.claim_type.code(),
                    c.is_bearing as i64, c.output_locale, c.created_at
                ],
            )
            .map_err(backend)?;
        Ok(())
    }

    fn get_claim(&self, id: &str) -> Result<Option<Claim>> {
        self.lock()
            .query_row(
                "SELECT claim_id, project_id, text, claim_type, is_bearing, output_locale, created_at
                 FROM claim WHERE claim_id = ?1",
                params![id],
                |r| Ok((
                    Claim {
                        id: r.get(0)?,
                        project_id: r.get(1)?,
                        text: r.get(2)?,
                        claim_type: ClaimType::Citation, // replaced below
                        is_bearing: r.get::<_, i64>(4)? != 0,
                        output_locale: r.get(5)?,
                        created_at: r.get(6)?,
                    },
                    r.get::<_, String>(3)?,
                )),
            )
            .optional()
            .map_err(backend)?
            .map(|(mut c, ct)| {
                c.claim_type = decode("claim_type", ClaimType::from_code(&ct))?;
                Ok(c)
            })
            .transpose()
    }

    fn put_evidence(&self, e: &EvidenceSpan) -> Result<()> {
        self.lock()
            .execute(
                "INSERT OR REPLACE INTO evidence_span
                 (evidence_id, source_id, blake3_hash, start_offset, end_offset, verbatim_text,
                  parse_trust, redaction_state)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    e.id, e.source_id, e.blake3_hash, e.start_offset, e.end_offset,
                    e.verbatim_text, e.parse_trust, e.redaction_state.code()
                ],
            )
            .map_err(map_sqlite)?; // a bad source_id (FK) surfaces as Conflict
        Ok(())
    }

    fn get_evidence(&self, id: &str) -> Result<Option<EvidenceSpan>> {
        self.lock()
            .query_row(
                "SELECT evidence_id, source_id, blake3_hash, start_offset, end_offset,
                        verbatim_text, parse_trust, redaction_state
                 FROM evidence_span WHERE evidence_id = ?1",
                params![id],
                |r| {
                    Ok((
                        EvidenceSpan {
                            id: r.get(0)?,
                            source_id: r.get(1)?,
                            blake3_hash: r.get(2)?,
                            start_offset: r.get(3)?,
                            end_offset: r.get(4)?,
                            verbatim_text: r.get(5)?,
                            parse_trust: r.get(6)?,
                            redaction_state: RedactionState::Present, // replaced below
                        },
                        r.get::<_, String>(7)?,
                    ))
                },
            )
            .optional()
            .map_err(backend)?
            .map(|(mut e, rs)| {
                e.redaction_state = decode("redaction_state", RedactionState::from_code(&rs))?;
                Ok(e)
            })
            .transpose()
    }

    fn put_verifier(&self, v: &Verifier) -> Result<()> {
        self.lock()
            .execute(
                "INSERT OR REPLACE INTO verifier
                 (verifier_id, kind, model_id, model_family, version, prompt_hash, role)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![v.id, v.kind.code(), v.model_id, v.model_family, v.version, v.prompt_hash, v.role],
            )
            .map_err(backend)?;
        Ok(())
    }

    fn get_verifier(&self, id: &str) -> Result<Option<Verifier>> {
        self.lock()
            .query_row(
                "SELECT verifier_id, kind, model_id, model_family, version, prompt_hash, role
                 FROM verifier WHERE verifier_id = ?1",
                params![id],
                |r| {
                    Ok((
                        Verifier {
                            id: r.get(0)?,
                            kind: VerifierKind::Model, // replaced below
                            model_id: r.get(2)?,
                            model_family: r.get(3)?,
                            version: r.get(4)?,
                            prompt_hash: r.get(5)?,
                            role: r.get(6)?,
                        },
                        r.get::<_, String>(1)?,
                    ))
                },
            )
            .optional()
            .map_err(backend)?
            .map(|(mut v, k)| {
                v.kind = decode("kind", VerifierKind::from_code(&k))?;
                Ok(v)
            })
            .transpose()
    }

    fn insert_verdict(&self, v: &Verdict) -> Result<()> {
        // Plain INSERT (not REPLACE): verdicts are immutable; a re-judgement is a
        // new row linked via revises_verdict_id.
        self.lock()
            .execute(
                "INSERT INTO verdict
                 (verdict_id, claim_id, verifier_id, status, evidence_redacted,
                  calibrated_confidence, uncertain_reason, independence, revises_verdict_id,
                  created_event_seq)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![
                    v.id, v.claim_id, v.verifier_id, v.status.code(), v.evidence_redacted as i64,
                    v.calibrated_confidence, v.uncertain_reason,
                    v.independence.map(|i| i.code()), v.revises_verdict_id, v.created_event_seq
                ],
            )
            .map_err(map_sqlite)?; // duplicate verdict_id / FK / CHECK -> Conflict
        Ok(())
    }

    fn verdicts_for_claim(&self, claim_id: &str) -> Result<Vec<Verdict>> {
        let guard = self.lock();
        let mut stmt = guard
            .prepare(
                "SELECT verdict_id, claim_id, verifier_id, status, evidence_redacted,
                        calibrated_confidence, uncertain_reason, independence, revises_verdict_id,
                        created_event_seq
                 FROM verdict WHERE claim_id = ?1 ORDER BY created_event_seq ASC",
            )
            .map_err(backend)?;
        let rows = stmt
            .query_map(params![claim_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?, // status code
                    r.get::<_, i64>(4)?,
                    r.get::<_, Option<f64>>(5)?,
                    r.get::<_, Option<String>>(6)?,
                    r.get::<_, Option<String>>(7)?, // independence code
                    r.get::<_, Option<String>>(8)?,
                    r.get::<_, i64>(9)?,
                ))
            })
            .map_err(backend)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(backend)?;

        rows.into_iter()
            .map(|(id, claim_id, verifier_id, status, redacted, conf, reason, indep, revises, seq)| {
                Ok(Verdict {
                    id,
                    claim_id,
                    verifier_id,
                    status: decode("status", VerdictStatus::from_code(&status))?,
                    evidence_redacted: redacted != 0,
                    calibrated_confidence: conf,
                    uncertain_reason: reason,
                    independence: match indep {
                        Some(code) => Some(decode("independence", Independence::from_code(&code))?),
                        None => None,
                    },
                    revises_verdict_id: revises,
                    created_event_seq: seq,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axiom_core::model::new_id;
    use serde_json::json;

    fn mkstore() -> SqliteStore {
        SqliteStore::open_in_memory("test-key").expect("open")
    }

    #[test]
    fn event_chain_appends_and_verifies() {
        let s = mkstore();
        assert_eq!(s.head_hash().unwrap(), None);
        let e1 = s.append_event("system", "ingest", &json!({"src": "a"})).unwrap();
        let e2 = s.append_event("system", "verify", &json!({"claim": "c1"})).unwrap();
        assert_eq!(e1.seq, 1);
        assert_eq!(e2.seq, 2);
        assert_eq!(e2.prev_hash, e1.entry_hash, "chain links to predecessor");
        assert_eq!(s.head_hash().unwrap().as_deref(), Some(e2.entry_hash.as_str()));
        assert!(s.verify_chain().unwrap(), "intact chain verifies");
        assert_eq!(s.replay().unwrap().len(), 2);
    }

    #[test]
    fn verify_chain_detects_tampering() {
        let s = mkstore();
        s.append_event("system", "a", &json!({"n": 1})).unwrap();
        s.append_event("system", "b", &json!({"n": 2})).unwrap();
        // Tamper with a stored payload without recomputing the hash.
        s.lock()
            .execute("UPDATE event SET payload_json = ?1 WHERE seq = 1", params![r#"{"n":999}"#])
            .unwrap();
        assert!(!s.verify_chain().unwrap(), "tampered payload breaks the chain");
    }

    #[test]
    fn claim_and_verdict_roundtrip() {
        let s = mkstore();
        let claim = Claim {
            id: new_id(),
            project_id: "proj1".into(),
            text: "The effect size was d=0.8".into(),
            claim_type: ClaimType::NumericStat,
            is_bearing: true,
            output_locale: Some("en".into()),
            created_at: now_ms(),
        };
        s.put_claim(&claim).unwrap();
        let got = s.get_claim(&claim.id).unwrap().expect("claim exists");
        assert_eq!(got.claim_type, ClaimType::NumericStat);
        assert!(got.is_bearing);

        let verifier = Verifier {
            id: new_id(),
            kind: VerifierKind::Model,
            model_id: Some("opus-4-8".into()),
            model_family: Some("anthropic".into()),
            version: Some("4.8".into()),
            prompt_hash: None,
            role: Some("adjudicate".into()),
        };
        s.put_verifier(&verifier).unwrap();

        let seq = s.append_event("system", "verdict", &json!({"claim": claim.id})).unwrap().seq;
        let verdict = Verdict {
            id: new_id(),
            claim_id: claim.id.clone(),
            verifier_id: verifier.id.clone(),
            status: VerdictStatus::Uncertain,
            evidence_redacted: false,
            calibrated_confidence: Some(0.42),
            uncertain_reason: Some("single_family_independence_unmet".into()),
            independence: Some(Independence::SingleFamily),
            revises_verdict_id: None,
            created_event_seq: seq,
        };
        s.insert_verdict(&verdict).unwrap();

        let verdicts = s.verdicts_for_claim(&claim.id).unwrap();
        assert_eq!(verdicts.len(), 1);
        assert_eq!(verdicts[0].status, VerdictStatus::Uncertain);
        assert_eq!(verdicts[0].independence, Some(Independence::SingleFamily));
    }

    #[test]
    fn verdict_status_check_constraint_rejects_bad_value() {
        // The DB CHECK must reject any status outside the 5-state vocabulary.
        let s = mkstore();
        let err = s.lock().execute(
            "INSERT INTO claim (claim_id, project_id, text, claim_type, is_bearing, created_at)
             VALUES ('c','p','t','citation',0,0)",
            [],
        );
        assert!(err.is_ok());
        s.lock()
            .execute(
                "INSERT INTO verifier (verifier_id, kind) VALUES ('v','model')",
                [],
            )
            .unwrap();
        let bad = s.lock().execute(
            "INSERT INTO verdict (verdict_id, claim_id, verifier_id, status, created_event_seq)
             VALUES ('vd','c','v','bogus_status',1)",
            [],
        );
        assert!(bad.is_err(), "CHECK constraint must reject non-vocabulary status");
    }

    #[test]
    fn wrong_key_cannot_open_encrypted_db() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("durable.db");
        {
            let s = SqliteStore::open(&path, "correct-horse").unwrap();
            s.append_event("system", "seed", &json!({"x": 1})).unwrap();
        }
        // Right key reopens fine.
        assert!(SqliteStore::open(&path, "correct-horse").is_ok());
        // Wrong key: SQLCipher cannot decrypt the page header → migration/read fails.
        assert!(
            SqliteStore::open(&path, "wrong-key").is_err(),
            "wrong SQLCipher key must fail to open (proves at-rest encryption)"
        );
    }

    #[test]
    fn verify_chain_detects_seq_gap() {
        let s = mkstore();
        s.append_event("system", "a", &json!({"n": 1})).unwrap();
        s.append_event("system", "b", &json!({"n": 2})).unwrap();
        s.append_event("system", "c", &json!({"n": 3})).unwrap();
        // Delete the middle event → replay yields seq [1, 3], a gap.
        s.lock().execute("DELETE FROM event WHERE seq = 2", []).unwrap();
        assert!(!s.verify_chain().unwrap(), "a seq gap must fail verification");
    }

    #[test]
    fn evidence_and_verifier_roundtrip() {
        let s = mkstore();
        let src = Source {
            id: new_id(),
            project_id: "p".into(),
            blake3_hash: "h".into(),
            uri: None,
            mime: None,
            acquisition_channel: Some("oa-cc".into()),
            license: None,
            source_locale: Some("en".into()),
            data_class: None,
            retracted_status: RetractedStatus::Clean,
            cas_state: CasState::Present,
            created_at: now_ms(),
        };
        s.put_source(&src).unwrap();
        let ev = EvidenceSpan {
            id: new_id(),
            source_id: src.id.clone(),
            blake3_hash: "h".into(),
            start_offset: Some(0),
            end_offset: Some(12),
            verbatim_text: Some("hello world".into()),
            parse_trust: Some(0.9),
            redaction_state: RedactionState::Present,
        };
        s.put_evidence(&ev).unwrap();
        let got = s.get_evidence(&ev.id).unwrap().expect("evidence exists");
        assert_eq!(got.source_id, src.id);
        assert_eq!(got.verbatim_text.as_deref(), Some("hello world"));
        assert!(s.get_evidence("nope").unwrap().is_none());

        let v = Verifier {
            id: new_id(),
            kind: VerifierKind::Human,
            model_id: None,
            model_family: None,
            version: None,
            prompt_hash: None,
            role: Some("reviewer".into()),
        };
        s.put_verifier(&v).unwrap();
        let gv = s.get_verifier(&v.id).unwrap().expect("verifier exists");
        assert_eq!(gv.kind, VerifierKind::Human);
        assert_eq!(gv.role.as_deref(), Some("reviewer"));
    }

    #[test]
    fn orphan_evidence_violates_fk() {
        let s = mkstore();
        let ev = EvidenceSpan {
            id: new_id(),
            source_id: "does-not-exist".into(),
            blake3_hash: "h".into(),
            start_offset: None,
            end_offset: None,
            verbatim_text: None,
            parse_trust: None,
            redaction_state: RedactionState::Present,
        };
        let err = s.put_evidence(&ev).unwrap_err();
        assert!(
            matches!(err, StoreError::Conflict(_)),
            "orphan evidence (bad source FK) must be a Conflict, got {err:?}"
        );
    }

    #[test]
    fn duplicate_verdict_is_conflict() {
        let s = mkstore();
        s.lock()
            .execute(
                "INSERT INTO claim (claim_id, project_id, text, claim_type, is_bearing, created_at)
                 VALUES ('c','p','t','citation',1,0)",
                [],
            )
            .unwrap();
        s.lock()
            .execute("INSERT INTO verifier (verifier_id, kind) VALUES ('v','model')", [])
            .unwrap();
        let mk = |id: &str| Verdict {
            id: id.into(),
            claim_id: "c".into(),
            verifier_id: "v".into(),
            status: VerdictStatus::Verified,
            evidence_redacted: false,
            calibrated_confidence: None,
            uncertain_reason: None,
            independence: None,
            revises_verdict_id: None,
            created_event_seq: 1,
        };
        s.insert_verdict(&mk("vd1")).unwrap();
        let err = s.insert_verdict(&mk("vd1")).unwrap_err();
        assert!(
            matches!(err, StoreError::Conflict(_)),
            "re-inserting an immutable verdict_id must be a Conflict, got {err:?}"
        );
    }
}
