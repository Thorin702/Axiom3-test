//! Provenance export — the auditable LINEAGE of every verdict.
//!
//! The eval harness MEASURES our quality in aggregate; provenance PROVES each
//! individual verdict's chain of custody: which deterministic checkers ran, which
//! independent cloud families voted and how, what evidence grounded it — all anchored
//! to the project's hash-chained event log so a third party (a journal, a funder, an
//! integrity office) can verify the record was not altered after the fact.
//!
//! Design: this module owns the SCHEMA (neutral, serde) and a deterministic
//! serialization. The shell ASSEMBLES a [`ProvenanceRecord`] from a draft check and
//! writes it AS the `draft.checked` event payload — so the immutable, hash-chained log
//! is the single source of truth, not a separate blob that can drift. Export reads the
//! record back from a committed [`Event`] and attaches the event's [`ChainAnchor`]
//! (seq + hashes), which exists only after the payload is hashed into the chain. The
//! output is plain, documented JSON = zero lock-in; a PROV-O / RO-Crate serialization
//! is a downstream MAPPING of this same record, never a competing source of truth.

use crate::model::Event;
use serde::de::Error as _;
use serde::{Deserialize, Serialize};

/// The schema id of THIS record format (bump on a breaking field change; independent
/// of the app version, which is carried separately in `tool_version`).
pub const PROVENANCE_SCHEMA: &str = "axiom3-provenance/1";

/// A self-contained provenance record for one draft check.
///
/// `anchor` is absent in the STORED payload (it's computed FROM the payload when the
/// event is hashed into the chain) and is filled in by [`ProvenanceRecord::from_event`]
/// at export time. Everything else is exactly what was committed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProvenanceRecord {
    /// Always [`PROVENANCE_SCHEMA`] — lets a reader detect format drift.
    pub schema: String,
    /// The producing tool (`"axiom3"`).
    pub tool: String,
    /// The engine version that produced the record ([`crate::version`]).
    pub tool_version: String,
    /// BLAKE3 of the exact draft text checked — ties the record to its input so a
    /// reader can confirm WHICH text these verdicts describe.
    pub draft_blake3: String,
    /// Total claims extracted (= `claims.len()`, recorded for at-a-glance reading).
    pub total_claims: usize,
    /// How many claims are bearing conclusions (the fleet-eligible ones).
    pub bearing_claims: usize,
    /// How many bearing claims actually got cross-vendor fleet verification.
    pub fleet_verified: usize,
    /// One entry per claim, in document order.
    pub claims: Vec<ClaimProvenance>,
    /// The event-log anchor (filled at export, skipped when absent so the stored
    /// payload — which cannot contain its own hash — round-trips cleanly).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub anchor: Option<ChainAnchor>,
}

/// Ties a record to its position in the project's append-only hash chain. With this a
/// verifier can recompute `entry_hash = BLAKE3(prev_hash ‖ 0x1f ‖ canonical(payload))`
/// and confirm the record is exactly what was committed at `event_seq`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChainAnchor {
    pub event_seq: i64,
    /// Event timestamp (ms since epoch) — the only place a time appears, since the
    /// stored payload is kept time-free to stay deterministic.
    pub event_ts: i64,
    pub prev_hash: String,
    pub entry_hash: String,
    /// Whether the WHOLE project chain re-verified intact when this record was
    /// exported — context the reader cares about (an anchor into a broken chain is
    /// worthless).
    pub chain_intact: bool,
}

/// The lineage of one claim's verdict.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClaimProvenance {
    pub claim_id: String,
    pub text: String,
    pub claim_type: String,
    pub is_bearing: bool,
    /// Final 5-state verdict code.
    pub status: String,
    /// Cross-vendor independence of the final verdict.
    pub independence: String,
    /// Stable machine reason code(s) that shaped the verdict (sorted, deduped).
    pub reason_codes: Vec<String>,
    /// The auto-depth the planner assigned: `"fleet"` or `"deterministic_only"`.
    pub verification_depth: String,
    /// Deterministic checker findings (the offline tier — always advisory).
    pub findings: Vec<FindingItem>,
    /// The verbatim passages the fleet judged against (empty if not grounded).
    pub evidence: Vec<String>,
    /// The cross-vendor breakdown, when a bearing claim reached the fleet.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub fleet: Option<FleetProvenance>,
}

/// One deterministic checker observation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FindingItem {
    pub checker: String,
    /// `"risk"` or `"contradiction"` — the offline tier emits only `risk`.
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub excerpt: Option<String>,
}

/// The fleet's OWN conclusion plus the per-vendor votes that produced it. Recorded
/// pre-downgrade, so a reader sees both what the models concluded and how the honesty
/// gates (e.g. grounded-single-source) capped the final `status`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FleetProvenance {
    pub status: String,
    pub independence: String,
    pub votes: Vec<VendorVote>,
}

/// One model family's vote.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VendorVote {
    pub family: String,
    pub model: String,
    pub status: String,
    pub reasoning: String,
}

impl ProvenanceRecord {
    /// Build the stored (anchor-free) shell of a record. Callers push [`ClaimProvenance`]
    /// onto `claims` and set the counts.
    pub fn new(tool_version: impl Into<String>, draft_blake3: impl Into<String>) -> Self {
        ProvenanceRecord {
            schema: PROVENANCE_SCHEMA.to_string(),
            tool: "axiom3".to_string(),
            tool_version: tool_version.into(),
            draft_blake3: draft_blake3.into(),
            total_claims: 0,
            bearing_claims: 0,
            fleet_verified: 0,
            claims: Vec::new(),
            anchor: None,
        }
    }

    /// Recompute the summary counts from `claims` and a fleet-verified tally.
    pub fn finalize_counts(&mut self, fleet_verified: usize) {
        self.total_claims = self.claims.len();
        self.bearing_claims = self.claims.iter().filter(|c| c.is_bearing).count();
        self.fleet_verified = fleet_verified;
    }

    /// Parse a stored `draft.checked` event payload back into a record and attach the
    /// event's chain anchor. `chain_intact` is supplied by the caller (it must run the
    /// log's `verify_chain` to know it). Errors if the payload isn't a provenance record
    /// (an older ad-hoc payload) OR carries a `schema` this build doesn't recognize (a
    /// FUTURE bump) — so schema drift surfaces as an explicit error instead of being
    /// silently skipped, which would let an exporter return a stale-but-valid record.
    pub fn from_event(ev: &Event, chain_intact: bool) -> serde_json::Result<Self> {
        let mut rec: ProvenanceRecord = serde_json::from_str(&ev.payload_json)?;
        if rec.schema != PROVENANCE_SCHEMA {
            return Err(serde_json::Error::custom(format!(
                "unsupported provenance schema {:?} (this build reads {PROVENANCE_SCHEMA})",
                rec.schema
            )));
        }
        rec.anchor = Some(ChainAnchor {
            event_seq: ev.seq,
            event_ts: ev.ts,
            prev_hash: ev.prev_hash.clone(),
            entry_hash: ev.entry_hash.clone(),
            chain_intact,
        });
        Ok(rec)
    }

    /// Deterministic, human-readable JSON for export. Field order is fixed by the struct
    /// definitions and every collection is in a stable order, so the same record always
    /// serializes to the same bytes. (For signing-grade canonical bytes, hash via the
    /// store's `canonical_json` over the stored payload instead.)
    pub fn to_export_json(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ProvenanceRecord {
        let mut rec = ProvenanceRecord::new("0.0.0", "abc123");
        rec.claims.push(ClaimProvenance {
            claim_id: "c1".into(),
            text: "Accuracy reached 95%.".into(),
            claim_type: "numeric_stat".into(),
            is_bearing: true,
            status: "uncertain".into(),
            independence: "multi_family".into(),
            reason_codes: vec!["evidence_supports".into(), "grounded_single_source".into()],
            verification_depth: "fleet".into(),
            findings: vec![FindingItem { checker: "uncited_quantitative".into(), kind: "risk".into(), excerpt: None }],
            evidence: vec!["the study reports 95% accuracy".into()],
            fleet: Some(FleetProvenance {
                status: "verified".into(),
                independence: "multi_family".into(),
                votes: vec![
                    VendorVote { family: "deepseek".into(), model: "deepseek-v4-flash".into(), status: "verified".into(), reasoning: "matches the source".into() },
                    VendorVote { family: "qwen".into(), model: "qwen-plus".into(), status: "verified".into(), reasoning: "supported".into() },
                ],
            }),
        });
        rec.finalize_counts(1);
        rec
    }

    #[test]
    fn counts_are_finalized_from_claims() {
        let rec = sample();
        assert_eq!(rec.total_claims, 1);
        assert_eq!(rec.bearing_claims, 1);
        assert_eq!(rec.fleet_verified, 1);
    }

    #[test]
    fn export_json_round_trips() {
        let rec = sample();
        let json = rec.to_export_json().unwrap();
        let back: ProvenanceRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(rec, back);
    }

    #[test]
    fn export_is_deterministic() {
        let rec = sample();
        assert_eq!(rec.to_export_json().unwrap(), rec.to_export_json().unwrap());
    }

    #[test]
    fn stored_payload_has_no_anchor_but_export_can_carry_one() {
        // The stored shell omits the anchor entirely (it cannot contain its own hash).
        let rec = sample();
        let stored = serde_json::to_value(&rec).unwrap();
        assert!(stored.get("anchor").is_none(), "stored payload must be anchor-free");

        // from_event re-attaches the anchor from the committed event.
        let ev = Event {
            seq: 7,
            ts: 1_700_000_000_000,
            actor: "engine".into(),
            op: "draft.checked".into(),
            payload_json: serde_json::to_string(&rec).unwrap(),
            prev_hash: "PREV".into(),
            entry_hash: "ENTRY".into(),
        };
        let exported = ProvenanceRecord::from_event(&ev, true).unwrap();
        let anchor = exported.anchor.expect("export carries the anchor");
        assert_eq!(anchor.event_seq, 7);
        assert_eq!(anchor.entry_hash, "ENTRY");
        assert!(anchor.chain_intact);
        // The claims survive the round-trip through the event payload.
        assert_eq!(exported.claims, rec.claims);
    }

    #[test]
    fn an_older_schema_payload_is_a_clean_error_not_a_panic() {
        let ev = Event {
            seq: 1,
            ts: 0,
            actor: "engine".into(),
            op: "draft.checked".into(),
            payload_json: r#"{"claims": 3, "fleet_verified": 1, "verdicts": []}"#.into(),
            prev_hash: "x".into(),
            entry_hash: "y".into(),
        };
        assert!(ProvenanceRecord::from_event(&ev, true).is_err());
    }

    #[test]
    fn a_future_schema_payload_is_rejected_not_silently_accepted() {
        // Structurally a valid record but a schema this build doesn't read — must be a
        // clean error so a caller never mistakes it for an exportable current record.
        let mut rec = sample();
        rec.schema = "axiom3-provenance/2".into();
        let ev = Event {
            seq: 2,
            ts: 0,
            actor: "engine".into(),
            op: "draft.checked".into(),
            payload_json: serde_json::to_string(&rec).unwrap(),
            prev_hash: "x".into(),
            entry_hash: "y".into(),
        };
        assert!(ProvenanceRecord::from_event(&ev, true).is_err());
    }
}
