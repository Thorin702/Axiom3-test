//! Eval / calibration harness — the moat's *measured track record*.
//!
//! The deterministic checkers are the open floor: anyone can rebuild lexical and
//! numeric heuristics. What compounds into a moat is a labeled CORPUS we can score on
//! every commit, so "we never over-flag, never under-flag, and never assert a verdict
//! we can't back" stops being prose and becomes a number that MOVES when a checker
//! changes.
//!
//! This module runs a golden corpus of labeled claim cases through [`verify_draft`]
//! and reports:
//!   * per-checker **precision / recall** (did each checker fire on exactly the cases
//!     it should — the over-suppression / false-positive regressions we keep catching
//!     in review become a CI failure instead of a manual review pass), and
//!   * the headline **honesty invariant H1**: the deterministic tier emits ONLY
//!     [`VerdictStatus::Uncertain`] (and the injection/quarantine states) — never
//!     [`VerdictStatus::Verified`] or [`VerdictStatus::Refuted`], which require
//!     grounding the offline tier does not have.
//!
//! It is OFFLINE, FREE, and DETERMINISTIC, so it runs in `cargo test` as a regression
//! guard. The network calibration piece (how often a cloud *Verified* is actually
//! true — needs keys + a labeled benchmark) is a separate, paid increment.

use crate::model::VerdictStatus;
use crate::verify::verify_draft;
use std::collections::{BTreeMap, BTreeSet};

/// One labeled case in the golden corpus.
///
/// A case is scored by the SET of checker codes that fire anywhere in its draft, so a
/// positive case should list EVERY code it legitimately triggers (a numeric conflict
/// sentence also lacks a citation, so it fires both `internal_conflict` AND
/// `uncited_quantitative` — both belong in `expect`). A case with an empty `expect` is
/// a NEGATIVE case: the deterministic tier must stay completely silent on it.
#[derive(Debug, Clone)]
pub struct EvalCase {
    /// Text fed to `verify_draft` — usually one sentence; multi-sentence only where the
    /// checker is inherently cross-claim (e.g. `internal_conflict`).
    pub draft: &'static str,
    /// The checker codes that SHOULD fire. Empty ⇒ a clean case (must stay silent).
    pub expect: &'static [&'static str],
    /// The checker this case primarily exercises — groups the scorecard.
    pub targets: &'static str,
    /// Why the case is labeled this way (documentation, not used in scoring).
    pub note: &'static str,
}

/// Per-checker tally over the whole corpus. `tp`/`fp`/`fn_` count CASES, not findings:
/// a checker that fires twice in one draft still counts once toward that case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckerScore {
    pub code: String,
    /// Expected and fired.
    pub tp: u32,
    /// Fired but NOT expected — the false-positive / over-flagging axis.
    pub fp: u32,
    /// Expected but did NOT fire — the missed-detection axis.
    pub fn_: u32,
}

impl CheckerScore {
    /// Precision = tp / (tp + fp); `None` when the checker never fired (undefined).
    pub fn precision(&self) -> Option<f64> {
        let denom = self.tp + self.fp;
        (denom > 0).then(|| self.tp as f64 / denom as f64)
    }
    /// Recall = tp / (tp + fn); `None` when the checker has no positive cases.
    pub fn recall(&self) -> Option<f64> {
        let denom = self.tp + self.fn_;
        (denom > 0).then(|| self.tp as f64 / denom as f64)
    }
}

/// A case where the deterministic tier asserted a verdict it must never assert (H1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HonestyViolation {
    pub draft: &'static str,
    pub status: VerdictStatus,
    pub reason: Option<String>,
}

/// The full scorecard for a corpus run.
#[derive(Debug, Clone)]
pub struct Scorecard {
    /// One entry per checker code seen as expected-or-fired, sorted by code.
    pub per_checker: Vec<CheckerScore>,
    /// Any case where a report was `Verified`/`Refuted` — must be empty.
    pub honesty_violations: Vec<HonestyViolation>,
    pub total_cases: usize,
}

impl Scorecard {
    /// True iff the corpus is perfectly clean: no honesty violation, no false positive,
    /// no missed detection. This is the property CI asserts.
    pub fn is_clean(&self) -> bool {
        self.honesty_violations.is_empty()
            && self.per_checker.iter().all(|s| s.fp == 0 && s.fn_ == 0)
    }

    /// A human-readable scorecard table (for the `eval` example binary).
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "Axiom3 deterministic eval — {} cases\n",
            self.total_cases
        ));
        out.push_str("─────────────────────────────────────────────────────\n");
        out.push_str("checker                 tp  fp  fn   prec   recall\n");
        for s in &self.per_checker {
            let p = s.precision().map(|v| format!("{v:.2}")).unwrap_or_else(|| "  — ".into());
            let r = s.recall().map(|v| format!("{v:.2}")).unwrap_or_else(|| "  — ".into());
            out.push_str(&format!(
                "{:<22} {:>3} {:>3} {:>3}   {:>4}   {:>4}\n",
                s.code, s.tp, s.fp, s.fn_, p, r
            ));
        }
        out.push_str("─────────────────────────────────────────────────────\n");
        out.push_str(&format!(
            "H1 honesty invariant (0 Verified/Refuted): {}\n",
            if self.honesty_violations.is_empty() { "PASS" } else { "FAIL" }
        ));
        for v in &self.honesty_violations {
            out.push_str(&format!(
                "  ✗ {:?} ({}) on: {}\n",
                v.status,
                v.reason.as_deref().unwrap_or(""),
                v.draft
            ));
        }
        out.push_str(&format!(
            "corpus clean (0 fp, 0 fn, 0 H1): {}\n",
            if self.is_clean() { "YES" } else { "NO" }
        ));
        out
    }
}

/// Run every case through the deterministic engine and tally the scorecard.
pub fn score(corpus: &[EvalCase]) -> Scorecard {
    let mut tp: BTreeMap<&str, u32> = BTreeMap::new();
    let mut fp: BTreeMap<&str, u32> = BTreeMap::new();
    let mut fn_: BTreeMap<&str, u32> = BTreeMap::new();
    let mut honesty_violations = Vec::new();

    for case in corpus {
        let reports = verify_draft("eval", case.draft);

        // The set of checker codes that fired anywhere in this draft.
        let mut fired: Vec<&str> = reports
            .iter()
            .flat_map(|r| r.findings.iter().map(|f| f.checker))
            .collect();
        fired.sort_unstable();
        fired.dedup();

        // H1: the deterministic tier must never assert a verdict.
        for r in &reports {
            if matches!(r.status, VerdictStatus::Verified | VerdictStatus::Refuted) {
                honesty_violations.push(HonestyViolation {
                    draft: case.draft,
                    status: r.status,
                    reason: r.reason.clone(),
                });
            }
        }

        // Per-checker confusion over the union of expected and fired codes.
        let codes: BTreeSet<&str> =
            case.expect.iter().copied().chain(fired.iter().copied()).collect();
        for code in codes {
            let expected = case.expect.contains(&code);
            let did_fire = fired.contains(&code);
            match (expected, did_fire) {
                (true, true) => *tp.entry(code).or_default() += 1,
                (false, true) => *fp.entry(code).or_default() += 1,
                (true, false) => *fn_.entry(code).or_default() += 1,
                (false, false) => {}
            }
        }
    }

    let mut all_codes: BTreeSet<&str> = BTreeSet::new();
    all_codes.extend(tp.keys().copied());
    all_codes.extend(fp.keys().copied());
    all_codes.extend(fn_.keys().copied());
    let per_checker = all_codes
        .into_iter()
        .map(|code| CheckerScore {
            code: code.to_string(),
            tp: *tp.get(code).unwrap_or(&0),
            fp: *fp.get(code).unwrap_or(&0),
            fn_: *fn_.get(code).unwrap_or(&0),
        })
        .collect();

    Scorecard {
        per_checker,
        honesty_violations,
        total_cases: corpus.len(),
    }
}

/// The golden corpus. Each case is labeled against the ACTUAL behaviour of
/// `verify_draft` (verified empirically — a mislabeled case would make the harness
/// lie). Grouped by the checker it primarily exercises; every group carries both
/// positive (should-fire) and negative (must-stay-silent) cases.
pub fn corpus() -> Vec<EvalCase> {
    vec![
        // ── overclaim ────────────────────────────────────────────────────────
        EvalCase {
            draft: "我们首次提出了这一方法。",
            expect: &["overclaim"],
            targets: "overclaim",
            note: "首次 is a novelty claim needing external support",
        },
        EvalCase {
            draft: "This is a novel approach to summarization.",
            expect: &["overclaim"],
            targets: "overclaim",
            note: "\"novel approach\" is an overclaim term",
        },
        EvalCase {
            draft: "我们改进了一种已有的方法。",
            expect: &[],
            targets: "overclaim",
            note: "improving an existing method makes no novelty claim",
        },
        EvalCase {
            draft: "We used an existing approach without modification.",
            expect: &[],
            targets: "overclaim",
            note: "\"approach\" alone is not a term; only \"novel approach\" is",
        },
        // ── absolute_quantifier ──────────────────────────────────────────────
        EvalCase {
            draft: "该方法在所有情况下都有效。",
            expect: &["absolute_quantifier"],
            targets: "absolute_quantifier",
            note: "所有 is an absolute quantifier rarely literally true",
        },
        EvalCase {
            draft: "The method always converges.",
            expect: &["absolute_quantifier"],
            targets: "absolute_quantifier",
            note: "\"always\" is an absolute quantifier",
        },
        EvalCase {
            draft: "该方法在多数情况下有效。",
            expect: &[],
            targets: "absolute_quantifier",
            note: "多数 (most) is appropriately hedged",
        },
        EvalCase {
            draft: "The method usually converges.",
            expect: &[],
            targets: "absolute_quantifier",
            note: "\"usually\" is hedged, not absolute",
        },
        // ── uncited_quantitative ─────────────────────────────────────────────
        EvalCase {
            draft: "踝关节扭伤影响了 25% 的运动员。",
            expect: &["uncited_quantitative"],
            targets: "uncited_quantitative",
            note: "borrowed background statistic with no citation",
        },
        EvalCase {
            draft: "Ankle sprains affect 25% of athletes.",
            expect: &["uncited_quantitative"],
            targets: "uncited_quantitative",
            note: "borrowed statistic stated as fact, uncited",
        },
        EvalCase {
            draft: "踝关节扭伤影响了 25% 的运动员 [1]。",
            expect: &[],
            targets: "uncited_quantitative",
            note: "[1] is a citation marker — suppressed",
        },
        EvalCase {
            draft: "我们测得准确率为 95% ± 2%。",
            expect: &[],
            targets: "uncited_quantitative",
            note: "± marks the study reporting its OWN data — suppressed",
        },
        // ── internal_conflict (inherently multi-claim) ───────────────────────
        EvalCase {
            draft: "Accuracy reached 95%. Accuracy reached 80%.",
            expect: &["internal_conflict", "uncited_quantitative"],
            targets: "internal_conflict",
            note: "same topic+unit, 95≠80; both sentences are also uncited numbers",
        },
        // ── statcheck (p-value recompute) ────────────────────────────────────
        EvalCase {
            draft: "A t-test was significant, t(28) = 2.50, p < .001.",
            expect: &["statcheck"],
            targets: "statcheck",
            note: "t(28)=2.50 ⇒ two-tailed p≈.019, one-tailed≈.009; p<.001 fits neither",
        },
        EvalCase {
            draft: "The effect was reliable, t(28) = 2.50, p = .019.",
            expect: &[],
            targets: "statcheck",
            note: "two-tailed p for t(28)=2.50 rounds to .019 — consistent",
        },
        EvalCase {
            draft: "A one-tailed test gave t(28) = 2.50, p = .009.",
            expect: &[],
            targets: "statcheck",
            note: "one-tailed p≈.009 is consistent — must NOT flag (the review regression)",
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// H1, MEASURED: across the whole corpus the deterministic tier never asserts a
    /// verdict. A single Verified/Refuted here is a moat-breaking honesty bug.
    #[test]
    fn deterministic_tier_never_asserts_a_verdict() {
        let card = score(&corpus());
        assert!(
            card.honesty_violations.is_empty(),
            "H1 VIOLATED: {:#?}",
            card.honesty_violations
        );
    }

    /// No checker fires on a case where it isn't expected — the over-flagging axis.
    #[test]
    fn no_false_positives() {
        let card = score(&corpus());
        let fps: Vec<_> = card.per_checker.iter().filter(|s| s.fp > 0).collect();
        assert!(fps.is_empty(), "false positives: {fps:#?}");
    }

    /// Every expected detection actually fires — the missed-detection axis.
    #[test]
    fn full_recall_on_positive_cases() {
        let card = score(&corpus());
        let misses: Vec<_> = card.per_checker.iter().filter(|s| s.fn_ > 0).collect();
        assert!(misses.is_empty(), "missed detections: {misses:#?}");
    }

    /// The corpus as a whole is clean (the single property CI guards).
    #[test]
    fn corpus_is_clean() {
        assert!(score(&corpus()).is_clean(), "{}", score(&corpus()).render());
    }
}
