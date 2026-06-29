//! Axiom3 open verification core.
//!
//! This crate is the OPEN-SOURCE (MIT) verification engine — claim model,
//! ingest-gate rules, entailment/citation verification, deterministic checkers,
//! and calibration. The proprietary shell (orchestration, UX, tool ecosystem)
//! lives in `axiom3-desktop/` and depends on this crate; never the reverse.
//!
//! P0.1 seed: only the load-bearing `VerdictStatus` type + a `version()` probe,
//! enough to prove the open-core boundary is wired. Real components land in P0.2+.

/// The 5-state verdict vocabulary (SSOT §〇.2 / §一).
///
/// NOTE: `evidence_redacted` (RTBF) is a separate *derived boolean* on the
/// persisted verdict row, NOT a sixth status value. Do not add it here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerdictStatus {
    Verified,
    Refuted,
    Uncertain,
    Quarantined,
    InjectionNeutralized,
}

impl VerdictStatus {
    /// Stable, language-invariant machine code (SSOT §〇.2: verdict vocabulary
    /// is a locked namespace keyed by a machine code, localized labels layer on top).
    pub fn code(self) -> &'static str {
        match self {
            VerdictStatus::Verified => "verified",
            VerdictStatus::Refuted => "refuted",
            VerdictStatus::Uncertain => "uncertain",
            VerdictStatus::Quarantined => "quarantined",
            VerdictStatus::InjectionNeutralized => "injection_neutralized",
        }
    }
}

/// Open-core crate version — surfaced to the shell to prove the wiring.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdict_codes_are_stable() {
        assert_eq!(VerdictStatus::Verified.code(), "verified");
        assert_eq!(VerdictStatus::InjectionNeutralized.code(), "injection_neutralized");
    }

    #[test]
    fn version_is_nonempty() {
        assert!(!version().is_empty());
    }
}
