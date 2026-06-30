//! Auto-depth verification planning — the supervisor's per-claim runtime-expansion
//! decision (build-spec §3C C.2 / R1·R2).
//!
//! This is NOT a user-facing "rigor dial" (a trust product never lets the user dial
//! rigor down). There is ONE verification standard; the supervisor only scales the
//! COST/DEPTH it spends per claim, by importance and budget. The deterministic tier
//! always runs on every claim (elsewhere); this module decides which claims ALSO earn
//! the expensive cross-vendor fleet when the budget can't cover them all.

use axiom_core::verify::ClaimReport;

/// How deeply to verify one claim. More tiers (e.g. an extra red-team pass for the
/// highest-stakes claims) slot in here later without changing callers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationDepth {
    /// Deterministic tier only — non-bearing claims, or bearing claims the budget
    /// can't cover this run. Stays `Uncertain` (honest: never cross-checked).
    DeterministicOnly,
    /// Deterministic tier + the cross-vendor fleet — bearing claims worth the cost.
    Fleet,
}

/// The budget the planner allocates within.
#[derive(Debug, Clone, Copy)]
pub struct PlanBudget {
    /// Maximum claims that may receive the (paid) cross-vendor fleet this run.
    pub max_fleet_claims: usize,
}

/// Assign each report a [`VerificationDepth`], returned 1:1 with `reports` by index.
///
/// Bearing claims earn the fleet, but within `max_fleet_claims` and PRIORITIZED so the
/// highest-value claims are never crowded out by a long tail: a bearing claim that ALSO
/// carries a deterministic risk flag (over-claim, uncited statistic, numeric conflict)
/// is exactly what a reviewer most wants independently cross-checked, so those win the
/// budget first; any remaining budget covers other bearing claims in document order.
/// Everything else is deterministic-only.
pub fn plan_verification(reports: &[ClaimReport], budget: &PlanBudget) -> Vec<VerificationDepth> {
    // Bearing-claim indices, prioritized: flagged before clean, ties keep doc order.
    let mut bearing: Vec<usize> = reports
        .iter()
        .enumerate()
        .filter(|(_, r)| r.claim.is_bearing)
        .map(|(i, _)| i)
        .collect();
    // `sort_by_key` is stable, so equal keys preserve document order. `false` (flagged)
    // sorts before `true` (clean).
    bearing.sort_by_key(|&i| reports[i].findings.is_empty());

    let fleet: std::collections::HashSet<usize> =
        bearing.into_iter().take(budget.max_fleet_claims).collect();

    (0..reports.len())
        .map(|i| {
            if fleet.contains(&i) {
                VerificationDepth::Fleet
            } else {
                VerificationDepth::DeterministicOnly
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axiom_core::verify::verify_draft;

    fn depths(draft: &str, max: usize) -> Vec<VerificationDepth> {
        let reports = verify_draft("p", draft);
        plan_verification(&reports, &PlanBudget { max_fleet_claims: max })
    }

    #[test]
    fn non_bearing_claims_stay_deterministic_only() {
        // "The sky is blue." is descriptive, not bearing → no fleet.
        let reports = verify_draft("p", "The sky is blue today.");
        let d = plan_verification(&reports, &PlanBudget { max_fleet_claims: 8 });
        assert!(d.iter().all(|x| *x == VerificationDepth::DeterministicOnly));
    }

    #[test]
    fn bearing_claims_within_budget_get_the_fleet() {
        // "Therefore the treatment causes recovery." is causal → bearing.
        let d = depths("Therefore the treatment causes recovery.", 8);
        assert!(d.contains(&VerificationDepth::Fleet), "a bearing claim is fleet-verified");
    }

    #[test]
    fn flagged_bearing_claims_win_a_tight_budget() {
        // Two bearing claims, budget for ONE. The first sentence is a clean causal
        // claim; the second is a bearing over-claim (carries a deterministic flag) and
        // must be the one that gets the fleet despite coming later in the document.
        let draft = "Therefore the treatment causes recovery. \
                     We demonstrate the first-ever room-temperature superconductor.";
        let reports = verify_draft("p", draft);
        // Sanity: exactly two bearing claims, and only the 2nd carries a flag.
        let bearing: Vec<usize> = reports.iter().enumerate()
            .filter(|(_, r)| r.claim.is_bearing).map(|(i, _)| i).collect();
        assert_eq!(bearing.len(), 2, "two bearing claims: {reports:#?}");
        let flagged = *bearing.iter().find(|&&i| !reports[i].findings.is_empty()).unwrap();
        let clean = *bearing.iter().find(|&&i| reports[i].findings.is_empty()).unwrap();

        let d = plan_verification(&reports, &PlanBudget { max_fleet_claims: 1 });
        assert_eq!(d[flagged], VerificationDepth::Fleet, "the flagged bearing claim wins the budget");
        assert_eq!(d[clean], VerificationDepth::DeterministicOnly, "the clean one waits");
    }

    #[test]
    fn zero_budget_yields_no_fleet() {
        let d = depths("Therefore the treatment causes recovery.", 0);
        assert!(d.iter().all(|x| *x == VerificationDepth::DeterministicOnly));
    }

    #[test]
    fn depths_align_one_to_one_with_reports() {
        let reports = verify_draft("p", "A causes B. The sky is blue. Therefore X causes Y.");
        let d = plan_verification(&reports, &PlanBudget { max_fleet_claims: 8 });
        assert_eq!(d.len(), reports.len());
    }
}
