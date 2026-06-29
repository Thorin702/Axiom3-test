//! Storage traits + the append-only hash-chained event-log primitive (SSOT §一 1.2).
//!
//! `axiom-core` owns the *interfaces* and the *pure* hash-chain logic so they are
//! open and unit-testable; concrete backends (SQLite/SQLCipher, LanceDB, Postgres)
//! live in `axiom-store` and implement these traits.

use crate::model::{Claim, EvidenceSpan, Event, Source, Verdict, Verifier};
use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("backend error: {0}")]
    Backend(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("integrity error: {0}")]
    Integrity(String),
    /// A uniqueness / constraint conflict (e.g. inserting a duplicate immutable
    /// verdict, or an FK/CHECK violation) — distinguishable from a generic backend
    /// error so callers can tell an expected conflict from a real failure.
    #[error("conflict: {0}")]
    Conflict(String),
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// Genesis previous-hash for the first event in a project's chain.
pub const GENESIS_PREV_HASH: &str = "0";

/// Canonical JSON for hashing — object keys EXPLICITLY sorted at every depth.
///
/// We do not rely on `serde_json::Map` being a `BTreeMap`: if any crate in the
/// workspace enabled serde_json's `preserve_order` feature, Cargo feature
/// unification would make `to_string` emit insertion order and silently break the
/// event-hash chain. Sorting here makes the canonical form independent of that.
pub fn canonical_json(payload: &Value) -> Result<String> {
    let mut out = String::new();
    write_canonical(payload, &mut out)?;
    Ok(out)
}

fn write_canonical(v: &Value, out: &mut String) -> Result<()> {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_unstable();
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(k)?); // JSON-escaped key
                out.push(':');
                write_canonical(&map[*k], out)?;
            }
            out.push('}');
        }
        Value::Array(arr) => {
            out.push('[');
            for (i, e) in arr.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(e, out)?;
            }
            out.push(']');
        }
        scalar => out.push_str(&serde_json::to_string(scalar)?),
    }
    Ok(())
}

/// `entry_hash = BLAKE3(prev_hash ‖ 0x1f ‖ canonical(payload))` (SSOT §一 1.2).
///
/// The 0x1f unit separator prevents `("ab","c")` and `("a","bc")` colliding.
/// Tamper-EVIDENT, not tamper-proof: the chain head must be anchored out-of-band.
pub fn entry_hash(prev_hash: &str, canonical_payload: &str) -> String {
    let mut h = blake3::Hasher::new();
    h.update(prev_hash.as_bytes());
    h.update(&[0x1f]);
    h.update(canonical_payload.as_bytes());
    h.finalize().to_hex().to_string()
}

/// Append-only, hash-chained audit/event log. Durable state (verdicts, notes,
/// calibration) is an event-sourced projection of this log.
pub trait EventLog {
    /// Append one event: compute `entry_hash` from the current head and persist.
    /// Returns the stored [`Event`] (with its assigned `seq`/hashes).
    fn append_event(&self, actor: &str, op: &str, payload: &Value) -> Result<Event>;
    /// The most recent `entry_hash`, or `None` for an empty chain.
    fn head_hash(&self) -> Result<Option<String>>;
    /// Replay the full chain in order.
    fn replay(&self) -> Result<Vec<Event>>;
    /// Recompute every `entry_hash` and confirm the chain is intact.
    fn verify_chain(&self) -> Result<bool>;
}

/// The durable, encrypted system-of-record (SSOT §一 — `durable.db`).
/// Verdicts are insert-only (immutable + versioned); never update in place.
pub trait DurableStore {
    fn put_source(&self, s: &Source) -> Result<()>;
    fn get_source(&self, id: &str) -> Result<Option<Source>>;
    fn put_claim(&self, c: &Claim) -> Result<()>;
    fn get_claim(&self, id: &str) -> Result<Option<Claim>>;
    fn put_evidence(&self, e: &EvidenceSpan) -> Result<()>;
    fn get_evidence(&self, id: &str) -> Result<Option<EvidenceSpan>>;
    fn put_verifier(&self, v: &Verifier) -> Result<()>;
    fn get_verifier(&self, id: &str) -> Result<Option<Verifier>>;
    /// Insert an immutable verdict row. A re-judgement is a NEW row linked via
    /// `revises_verdict_id`; this never updates an existing verdict. A duplicate
    /// `verdict_id` (or FK/CHECK violation) surfaces as [`StoreError::Conflict`].
    fn insert_verdict(&self, v: &Verdict) -> Result<()>;
    fn verdicts_for_claim(&self, claim_id: &str) -> Result<Vec<Verdict>>;

    /// Atomically record a verification pass: append the `op` event, insert
    /// `verifier`, and insert each `(claim, verdict)` — ALL in one transaction, so
    /// a partial failure can never leave an audit-anchor event referencing verdicts
    /// that weren't written. Each verdict's `created_event_seq` is assigned from the
    /// event minted here (any value the caller set is overwritten). Returns the event.
    fn record_verification(
        &self,
        actor: &str,
        op: &str,
        payload: &Value,
        verifier: &Verifier,
        items: &[(Claim, Verdict)],
    ) -> Result<Event>;
}

/// Rebuildable vector index (SSOT §一 — LanceDB, cache tier).
/// Concrete impl lands with embeddings; the trait pins the shape now.
pub trait VectorStore {
    fn upsert(&self, id: &str, embedding: &[f32]) -> Result<()>;
    /// Returns `(id, distance)` for the `k` nearest neighbours.
    fn search(&self, query: &[f32], k: usize) -> Result<Vec<(String, f32)>>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn entry_hash_is_deterministic_and_payload_sensitive() {
        let p = canonical_json(&json!({"op": "ingest", "n": 1})).unwrap();
        let a = entry_hash(GENESIS_PREV_HASH, &p);
        let b = entry_hash(GENESIS_PREV_HASH, &p);
        assert_eq!(a, b, "same inputs → same hash");

        let p2 = canonical_json(&json!({"op": "ingest", "n": 2})).unwrap();
        assert_ne!(a, entry_hash(GENESIS_PREV_HASH, &p2), "payload change → hash change");
    }

    #[test]
    fn canonical_json_sorts_object_keys() {
        let a = canonical_json(&json!({"b": 1, "a": 2})).unwrap();
        let b = canonical_json(&json!({"a": 2, "b": 1})).unwrap();
        assert_eq!(a, b, "key order must not affect the canonical form");
        assert_eq!(a, r#"{"a":2,"b":1}"#);
    }

    #[test]
    fn chain_links_each_entry_to_its_predecessor() {
        let p1 = canonical_json(&json!({"e": 1})).unwrap();
        let h1 = entry_hash(GENESIS_PREV_HASH, &p1);
        let p2 = canonical_json(&json!({"e": 2})).unwrap();
        let h2 = entry_hash(&h1, &p2);
        // Re-hashing e2 against a forged predecessor must not reproduce h2.
        assert_ne!(h2, entry_hash("forged", &p2));
    }

    #[test]
    fn separator_prevents_concatenation_collision() {
        // ("ab","c") vs ("a","bc") must differ thanks to the 0x1f separator.
        assert_ne!(entry_hash("ab", "c"), entry_hash("a", "bc"));
    }
}
