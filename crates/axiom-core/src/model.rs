//! Core domain model (SSOT §一 / §〇).
//!
//! Enums that persist to the durable store expose a stable, language-invariant
//! `code()` / `from_code()` (TEXT machine codes) — the DB and the i18n verdict
//! namespace key off these, never off `Display` or serde variant names.

use serde::{Deserialize, Serialize};

/// All entity IDs are ULIDs rendered as their 26-char Crockford string
/// (SSOT §〇.1 — sortable, unlike UUIDv4).
pub type Id = String;

/// Mint a new ULID id.
pub fn new_id() -> Id {
    ulid::Ulid::new().to_string()
}

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Macro: define a TEXT-coded enum with `code()` / `from_code()`.
macro_rules! coded_enum {
    ($(#[$m:meta])* $name:ident { $($variant:ident => $code:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        pub enum $name { $($variant),+ }

        impl $name {
            /// Stable, language-invariant machine code (the value stored in SQLite).
            pub fn code(self) -> &'static str {
                match self { $($name::$variant => $code),+ }
            }
            /// Parse from the stored machine code.
            pub fn from_code(s: &str) -> Option<Self> {
                match s { $($code => Some($name::$variant),)+ _ => None }
            }
        }
    };
}

coded_enum! {
    /// The strict 5-state verdict vocabulary (SSOT §〇.2).
    ///
    /// RTBF redaction is the separate `Verdict::evidence_redacted` boolean — NOT a
    /// sixth status. `PENDING` is a claim *processing* state, not a verdict status.
    VerdictStatus {
        Verified => "verified",
        Refuted => "refuted",
        Uncertain => "uncertain",
        Quarantined => "quarantined",
        InjectionNeutralized => "injection_neutralized",
    }
}

coded_enum! {
    /// Atomic-claim class (SSOT §〇.3 authoritative union). Routes a claim to the
    /// right grounding/checker path; `is_bearing` separately decides the LLM fleet.
    ClaimType {
        Citation => "citation",
        NumericStat => "numeric_stat",
        Causal => "causal",
        Methodological => "methodological",
        Definitional => "definitional",
        Descriptive => "descriptive",
        Synthesized => "synthesized",
        NonRetrievable => "non_retrievable",
    }
}

coded_enum! {
    /// Cross-vendor independence of a verdict (SSOT §〇.4).
    Independence {
        MultiFamily => "multi_family",
        SingleFamily => "single_family",
        LocalOnly => "local_only",
    }
}

coded_enum! {
    /// Content-addressed source-byte state. `CryptoShredded` keeps the BLAKE3 hash
    /// so the hash chain stays valid after RTBF (SSOT §一 / §3 deletion design).
    CasState {
        Present => "present",
        CryptoShredded => "crypto_shredded",
    }
}

coded_enum! {
    /// Whether an evidence span's verbatim text is still present (RTBF).
    RedactionState {
        Present => "present",
        Redacted => "redacted",
    }
}

coded_enum! {
    /// Retraction status of a source; a change re-opens dependent claims.
    RetractedStatus {
        Clean => "clean",
        Concern => "concern",
        Retracted => "retracted",
    }
}

coded_enum! {
    /// Whether a verifier is a human or a model.
    VerifierKind {
        Human => "human",
        Model => "model",
    }
}

/// A content-addressed source (the reading pipeline's manifest row, SSOT §一 1.2.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub id: Id,
    pub project_id: Id,
    /// CAS addressing key; retained even after crypto-shred so the chain stays valid.
    pub blake3_hash: String,
    pub uri: Option<String>,
    pub mime: Option<String>,
    /// Acquisition channel (legal pivot — SSOT §一 #7): publisher-api / oa-cc /
    /// subscription / crossref-polite / unpaywall / byo-key / web-crawl / user-upload.
    pub acquisition_channel: Option<String>,
    /// SPDX-ish license / TDM basis (DSM Art.3/Art.4 hard gate).
    pub license: Option<String>,
    /// i18n axis 3 — auto-detected, feeds quarantine.
    pub source_locale: Option<String>,
    /// Per-project data class (undeclared defaults to most-sensitive).
    pub data_class: Option<String>,
    pub retracted_status: RetractedStatus,
    pub cas_state: CasState,
    pub created_at: i64,
}

/// An atomic claim — the core object (SSOT §一 1.2.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claim {
    pub id: Id,
    pub project_id: Id,
    pub text: String,
    pub claim_type: ClaimType,
    /// Bearing/conclusion claim → triggers the LLM adversarial fleet (tiered verification).
    pub is_bearing: bool,
    /// i18n axis 2 — per-project, injected into the agent prompt.
    pub output_locale: Option<String>,
    pub created_at: i64,
}

/// An evidence span pinned to immutable source bytes + offsets (SSOT §一 1.2.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceSpan {
    pub id: Id,
    pub source_id: Id,
    pub blake3_hash: String,
    pub start_offset: Option<i64>,
    pub end_offset: Option<i64>,
    /// Verbatim text — nulled on RTBF (see `redaction_state`), hash kept.
    pub verbatim_text: Option<String>,
    pub parse_trust: Option<f64>,
    pub redaction_state: RedactionState,
}

/// A verifier identity (human or a specific model+version+prompt).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verifier {
    pub id: Id,
    pub kind: VerifierKind,
    pub model_id: Option<String>,
    /// Used for cross-vendor independence judgement.
    pub model_family: Option<String>,
    pub version: Option<String>,
    pub prompt_hash: Option<String>,
    /// reading / reasoning / routine_verify / adversarial / adjudicate / modality.
    pub role: Option<String>,
}

/// An immutable, versioned verdict on a claim (SSOT §一 1.2.2; §〇.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verdict {
    pub id: Id,
    pub claim_id: Id,
    pub verifier_id: Id,
    pub status: VerdictStatus,
    /// Derived boolean (NOT a 6th status): set when supporting evidence is redacted.
    pub evidence_redacted: bool,
    /// Confidence derived from verification outcomes (never model self-report).
    pub calibrated_confidence: Option<f64>,
    /// e.g. `single_family_independence_unmet`, `evidence_redacted`, offline-fail-closed.
    pub uncertain_reason: Option<String>,
    pub independence: Option<Independence>,
    /// PROV revision link — a re-judgement points at the prior verdict, never updates it.
    pub revises_verdict_id: Option<Id>,
    pub created_event_seq: i64,
}

/// One entry in the append-only, hash-chained audit/event log (SSOT §一 1.2 / event sourcing).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub seq: i64,
    pub ts: i64,
    pub actor: String,
    pub op: String,
    pub payload_json: String,
    pub prev_hash: String,
    pub entry_hash: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coded_enums_roundtrip() {
        assert_eq!(VerdictStatus::InjectionNeutralized.code(), "injection_neutralized");
        assert_eq!(VerdictStatus::from_code("uncertain"), Some(VerdictStatus::Uncertain));
        assert_eq!(VerdictStatus::from_code("nope"), None);
        assert_eq!(ClaimType::from_code("synthesized"), Some(ClaimType::Synthesized));
        assert_eq!(ClaimType::from_code("non_retrievable"), Some(ClaimType::NonRetrievable));
        assert_eq!(Independence::SingleFamily.code(), "single_family");
    }

    #[test]
    fn new_id_is_a_26char_ulid() {
        let id = new_id();
        assert_eq!(id.len(), 26, "ULID Crockford string is 26 chars");
    }
}
