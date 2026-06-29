//! Axiom3 open verification core.
//!
//! This crate is the OPEN-SOURCE (MIT) verification engine — the claim/evidence
//! domain model, the storage traits the engine reads/writes through, ingest-gate
//! rules, entailment/citation verification, deterministic checkers, and calibration.
//!
//! The proprietary shell (orchestration, UX, tool ecosystem) and the concrete
//! storage backends (`axiom-store`) depend on this crate; never the reverse.
//!
//! Conventions are fixed by the SSOT build spec (`Axiom3-BuildSpec.md` §〇):
//! - IDs are ULIDs (sortable). See [`model::new_id`].
//! - `verdict.status` is a strict 5-state enum ([`model::VerdictStatus`]);
//!   `evidence_redacted` (RTBF) is a separate boolean, never a 6th status.

pub mod model;
pub mod store;

pub use model::VerdictStatus;

/// Open-core crate version — surfaced to the shell to prove the boundary wiring.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
