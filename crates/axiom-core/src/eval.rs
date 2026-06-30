//! Eval / calibration harness — the moat's *measured track record*.
//!
//! The deterministic checkers are the open floor: anyone can rebuild lexical and
//! numeric heuristics. What compounds into a moat is a labeled CORPUS we can score on
//! every commit, so "we never over-flag and never under-flag" stops being prose and
//! becomes a number that MOVES when a checker changes.
//!
//! This module runs a golden corpus of labeled claim cases through [`verify_draft`]
//! and reports two distinct things — keep them distinct, because they carry different
//! kinds of assurance:
//!   * **per-checker precision / recall** — the DATA-DEPENDENT signal. Each number is a
//!     function of the corpus, so it actually moves when a checker over- or
//!     under-fires; the regressions we keep catching by hand in review become a CI
//!     failure. This is the real measured track record.
//!   * the **honesty invariant H1** — that the deterministic tier never asserts a
//!     verdict it can't ground ([`VerdictStatus::Verified`] / [`VerdictStatus::Refuted`]
//!     require cross-vendor grounding the offline tier does not have). H1 is guarded at
//!     TWO levels. (a) Structural: `adjudicate` only returns `Refuted` on a
//!     `Contradiction`-kind finding and never returns `Verified`, and no default
//!     checker emits `Contradiction` today — so over `verify_draft` H1 holds *by
//!     construction*, and the status-level scan is a cheap belt against a future
//!     `adjudicate` regression, NOT an empirical measurement (do not read its "PASS" as
//!     a moving number). (b) Non-vacuous: the harness inspects the actual MECHANISM —
//!     every finding the deterministic checkers emit over the corpus must be advisory
//!     (`FindingKind::Risk`). THAT check fails the moment a default checker starts
//!     emitting a verdict-bearing kind, which is exactly the regression that would let
//!     a false `Refuted` through. It is the load-bearing half of H1.
//!
//! It is OFFLINE, FREE, and DETERMINISTIC, so it runs in `cargo test` as a regression
//! guard. The corpus is hand-curated and small — its size is the coverage bound, not a
//! claim of completeness. The network calibration piece (how often a cloud *Verified*
//! is actually true — needs keys + a labeled benchmark) is a separate, paid increment.

use crate::model::VerdictStatus;
use crate::verify::{verify_draft, FindingKind};
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

/// A case where the deterministic tier asserted a verdict it must never assert (H1,
/// status level) — the cheap belt against an `adjudicate` regression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HonestyViolation {
    pub draft: &'static str,
    pub status: VerdictStatus,
    pub reason: Option<String>,
}

/// A deterministic checker that emitted a NON-advisory finding (kind != `Risk`) — the
/// load-bearing half of H1. The offline tier must stay advisory: a `Contradiction`
/// here is exactly what drives a false `Refuted`. Unlike the status scan, this inspects
/// the actual mechanism, so it fails the moment a default checker stops being advisory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NonAdvisory {
    pub draft: &'static str,
    pub checker: &'static str,
}

/// The full scorecard for a corpus run.
#[derive(Debug, Clone)]
pub struct Scorecard {
    /// One entry per checker code seen as expected-or-fired, sorted by code.
    pub per_checker: Vec<CheckerScore>,
    /// Any case where a report reached `Verified`/`Refuted` — must be empty (H1, belt).
    pub honesty_violations: Vec<HonestyViolation>,
    /// Any deterministic finding that wasn't advisory (`Risk`) — must be empty (H1, the
    /// load-bearing mechanism check).
    pub non_advisory: Vec<NonAdvisory>,
    pub total_cases: usize,
}

impl Scorecard {
    /// True iff the corpus is perfectly clean: it actually scored something (a non-empty
    /// corpus with at least one true positive — so a zero-case run can't pass
    /// vacuously), no honesty violation, no non-advisory finding, no false positive, no
    /// missed detection. This is the single property CI asserts.
    pub fn is_clean(&self) -> bool {
        self.total_cases > 0
            && self.per_checker.iter().any(|s| s.tp > 0)
            && self.honesty_violations.is_empty()
            && self.non_advisory.is_empty()
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
            "H1a checkers stay advisory (Risk-only, mechanism): {}\n",
            if self.non_advisory.is_empty() { "PASS" } else { "FAIL" }
        ));
        for v in &self.non_advisory {
            out.push_str(&format!("  ✗ checker {} emitted a non-Risk finding on: {}\n", v.checker, v.draft));
        }
        out.push_str(&format!(
            "H1b no verdict asserted (0 Verified/Refuted, structural belt): {}\n",
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
            "corpus clean ({} cases, 0 fp, 0 fn, 0 H1): {}\n",
            self.total_cases,
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
    let mut non_advisory = Vec::new();

    for case in corpus {
        let reports = verify_draft("eval", case.draft);

        // The set of checker codes that fired anywhere in this draft.
        let mut fired: Vec<&str> = reports
            .iter()
            .flat_map(|r| r.findings.iter().map(|f| f.checker))
            .collect();
        fired.sort_unstable();
        fired.dedup();

        // H1a (mechanism): every deterministic finding must be advisory (Risk). A
        // non-Risk finding is the actual cause of a false verdict — this is the check
        // that moves when a checker stops being advisory.
        for r in &reports {
            for f in &r.findings {
                if f.kind != FindingKind::Risk {
                    non_advisory.push(NonAdvisory { draft: case.draft, checker: f.checker });
                }
            }
            // H1b (belt): the tier must never reach Verified/Refuted. Holds by
            // construction today; catches a future adjudicate() regression.
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
        non_advisory,
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
        // Two SINGLE-gate negatives (split from one double-gated case): each pins a
        // distinct own-results suppression path, so a regression in either path flips
        // exactly one case red instead of being masked by the other gate.
        EvalCase {
            draft: "准确率为 95% ± 2%。",
            expect: &[],
            targets: "uncited_quantitative",
            note: "± alone (no CJK marker) marks own-data reporting — suppressed",
        },
        EvalCase {
            draft: "本研究测得准确率为 95%。",
            expect: &[],
            targets: "uncited_quantitative",
            note: "测得 alone (no ±) is a CJK own-results marker — suppressed",
        },
        // ── internal_conflict (inherently multi-claim) ───────────────────────
        EvalCase {
            draft: "Accuracy reached 95%. Accuracy reached 80%.",
            expect: &["internal_conflict", "uncited_quantitative"],
            targets: "internal_conflict",
            note: "same topic+unit, 95≠80; both sentences are also uncited numbers",
        },
        EvalCase {
            draft: "反应温度为 80℃。反应温度为 60℃。",
            expect: &["internal_conflict", "uncited_quantitative"],
            targets: "internal_conflict",
            note: "CJK: 温度 shared, ℃ unit, 80≠60 — exercises CJK unit comparison",
        },
        // ── multi-finding within a SINGLE claim ──────────────────────────────
        EvalCase {
            draft: "Our novel approach always achieves 99% accuracy.",
            expect: &["absolute_quantifier", "overclaim", "uncited_quantitative"],
            targets: "multi",
            note: "three checkers co-fire on one claim — guards intra-claim suppression",
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
        EvalCase {
            draft: "The effect was reliable, t(28) = 2.05, p = .50.",
            expect: &["statcheck"],
            targets: "statcheck",
            note: "p=.05-class recompute reported as .50 — the significance decision FLIPS branch",
        },
        EvalCase {
            draft: "An ANOVA, F(2, 27) = 3.35, p = .80.",
            expect: &["statcheck"],
            targets: "statcheck",
            note: "F(2,27)=3.35 ⇒ p≈.05; reported .80 is inconsistent (F upper-tail path)",
        },
        EvalCase {
            draft: "The ANOVA gave F(2, 27) = 3.35, p = .05.",
            expect: &[],
            targets: "statcheck",
            note: "F(2,27)=3.35 ⇒ p≈.05 — consistent",
        },
        EvalCase {
            draft: "A weak correlation, r(48) = .10, p < .01.",
            expect: &["statcheck"],
            targets: "statcheck",
            note: "r(48)=.10 ⇒ p≈.50; reported p<.01 is inconsistent (r→t path)",
        },
        EvalCase {
            draft: "The test gave chi-square(2) = 5.99, p = .80.",
            expect: &["statcheck"],
            targets: "statcheck",
            note: "χ²(2)=5.99 ⇒ p≈.05; reported .80 is inconsistent (chi-square path)",
        },
        EvalCase {
            draft: "卡方检验显著,chi-square(1) = 3.84, p = .05。",
            expect: &[],
            targets: "statcheck",
            note: "CJK-framed: χ²(1)=3.84 ⇒ p≈.05 consistent — ASCII parser tolerates CJK",
        },
        EvalCase {
            draft: "t检验显著,t(28) = 2.50, p < .001。",
            expect: &["statcheck"],
            targets: "statcheck",
            note: "CJK-framed inconsistent t-test — guards the ASCII parser against CJK fragility",
        },
        // ── non-assertive (proposal / plan / goal) — must stay SILENT ────────
        // Research writing isn't only papers: a grant proposal is mostly aspirational
        // prose. Such sentences are NOT checkable assertions; the engine surfaces
        // nothing (no flags, not bearing → no fleet/literature) rather than emitting a
        // verdict on a goal.
        EvalCase {
            draft: "本项目拟开发新型催化剂,预期将反应效率提高 30%。",
            expect: &[],
            targets: "non_assertive",
            note: "proposal goal with a target number — not an uncited established statistic",
        },
        EvalCase {
            draft: "本项目将首次实现室温超导,争取在三年内突破。",
            expect: &[],
            targets: "non_assertive",
            note: "a future novelty PITCH is not an overclaim to flag",
        },
        EvalCase {
            draft: "We aim to be the first to achieve room-temperature superconductivity.",
            expect: &[],
            targets: "non_assertive",
            note: "aspirational English — not a claim asserted as fact",
        },
        EvalCase {
            draft: "据报道,该病在运动员中的发病率约为 25%。",
            expect: &["uncited_quantitative"],
            targets: "non_assertive",
            note: "control: an ASSERTED background statistic is still checked (uncited)",
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// H1a (load-bearing, mechanism): every deterministic finding stays advisory
    /// (`Risk`). This is the NON-vacuous half — it fails the moment a default checker
    /// emits a verdict-bearing kind, which is the regression that would let a false
    /// `Refuted` through.
    #[test]
    fn deterministic_checkers_stay_advisory() {
        let card = score(&corpus());
        assert!(
            card.non_advisory.is_empty(),
            "a deterministic checker emitted a non-Risk finding: {:#?}",
            card.non_advisory
        );
    }

    /// H1b (structural belt): across the corpus the tier never reaches Verified/Refuted.
    /// Holds by construction today; this catches a future `adjudicate` regression.
    #[test]
    fn deterministic_tier_never_asserts_a_verdict() {
        let card = score(&corpus());
        assert!(
            card.honesty_violations.is_empty(),
            "H1 VIOLATED: {:#?}",
            card.honesty_violations
        );
    }

    /// A zero-case corpus must NOT report clean — `is_clean()` requires real coverage,
    /// so an accidentally-empty corpus fails loudly instead of passing vacuously.
    #[test]
    fn empty_corpus_is_not_clean() {
        assert!(!score(&[]).is_clean(), "an empty corpus must never be reported clean");
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
