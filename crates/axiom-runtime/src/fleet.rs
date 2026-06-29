//! The model-verification tier: a bearing claim is judged by an adversarial fleet
//! of models, then adjudicated through the cross-vendor INDEPENDENCE gate.
//!
//! Why a fleet, not one model: a single model's opinion is exactly the untrusted
//! "proposal" this product is built to distrust. Corroboration only counts when it
//! comes from ≥2 INDEPENDENT model families (SSOT §〇.4). So:
//! - ≥2 independent families REFUTE → [`VerdictStatus::Refuted`] (`multi_family`)
//! - ≥2 independent families SUPPORT and none refute → [`VerdictStatus::Verified`]
//! - otherwise → [`VerdictStatus::Uncertain`] (single family, disagreement, or too
//!   little corroboration). Conservative by construction.
//!
//! This module is the ORCHESTRATION + adjudication, fully unit-tested against a
//! mock backend (no API key). The real backend ([`SingleProviderRigBackend`]) is
//! key-gated; true cross-vendor dispatch waits on multi-provider construction in
//! the Rig adapter (today it hits one provider, so it cannot meet the gate alone).

use std::collections::BTreeSet;
use std::future::Future;

use axiom_core::model::{Independence, VerdictStatus};
use axiom_providers::message::{ContentBlock, Message, Request};
use axiom_providers::routing::independence_of_families;

/// One vendor's judgement on a claim.
#[derive(Debug, Clone, PartialEq)]
pub struct VendorJudgement {
    /// Model family (e.g. `anthropic`, `openai`) — the unit of independence.
    pub family: String,
    pub model: String,
    /// This vendor's call: `verified` | `refuted` | `uncertain`.
    pub status: VerdictStatus,
    pub reasoning: String,
}

/// A member of the verification fleet to dispatch to.
#[derive(Debug, Clone)]
pub struct FleetMember {
    pub family: String,
    pub provider: String,
    pub model: String,
}

/// The fleet's adjudicated verdict on a bearing claim.
#[derive(Debug, Clone, PartialEq)]
pub struct FleetVerdict {
    pub status: VerdictStatus,
    pub independence: Independence,
    /// Stable machine reason code (never prose), e.g. `multi_family_supported`.
    pub reason: String,
    pub judgements: Vec<VendorJudgement>,
}

/// A backend that asks ONE model to judge ONE claim. Implemented by the real Rig
/// adapter and by a mock in tests. The returned future is `Send` so the
/// orchestrator can run on any runtime.
pub trait FleetBackend {
    fn judge(
        &self,
        claim: &str,
        member: &FleetMember,
    ) -> impl Future<Output = anyhow::Result<VendorJudgement>> + Send;
}

/// Dispatch a bearing claim across the fleet and adjudicate with the independence
/// gate. A backend error for one member is FAIL-CLOSED: that member simply
/// contributes no judgement — a failed verifier can never manufacture corroboration.
pub async fn verify_bearing_claim<B: FleetBackend>(
    claim: &str,
    fleet: &[FleetMember],
    backend: &B,
) -> FleetVerdict {
    let mut judgements = Vec::new();
    for m in fleet {
        if let Ok(j) = backend.judge(claim, m).await {
            judgements.push(j);
        }
    }
    adjudicate_fleet(judgements)
}

/// Aggregate per-vendor judgements through the cross-vendor independence gate.
pub fn adjudicate_fleet(judgements: Vec<VendorJudgement>) -> FleetVerdict {
    let families_with = |status: VerdictStatus| -> Vec<String> {
        judgements
            .iter()
            .filter(|j| j.status == status)
            .map(|j| j.family.clone())
            .collect()
    };
    let refuting = families_with(VerdictStatus::Refuted);
    let verifying = families_with(VerdictStatus::Verified);
    let all_families: Vec<String> = judgements
        .iter()
        .map(|j| j.family.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    let refuting_indep = independence_of_families(&refuting);
    let verifying_indep = independence_of_families(&verifying);

    let (status, reason, independence) = if !refuting.is_empty() && !verifying.is_empty() {
        // ANY cross-vendor disagreement → Uncertain. Verified and Refuted are
        // symmetric: neither fires while the other camp has a single dissenter.
        (
            VerdictStatus::Uncertain,
            "cross_vendor_disagreement",
            independence_of_families(&all_families),
        )
    } else if refuting_indep == Independence::MultiFamily {
        // Independent cross-vendor refutation, with no dissent.
        (VerdictStatus::Refuted, "multi_family_refuted", refuting_indep)
    } else if verifying_indep == Independence::MultiFamily {
        (VerdictStatus::Verified, "multi_family_supported", verifying_indep)
    } else if all_families.len() <= 1 {
        (
            VerdictStatus::Uncertain,
            "single_family_independence_unmet",
            independence_of_families(&all_families),
        )
    } else {
        (
            VerdictStatus::Uncertain,
            "insufficient_corroboration",
            independence_of_families(&all_families),
        )
    };

    FleetVerdict { status, independence, reason: reason.to_string(), judgements }
}

// ---------------------------------------------------------------------------
// Real backend (Rig) — key-gated, single-provider for now.
// ---------------------------------------------------------------------------

const VERIFY_SYSTEM: &str = "You are an adversarial scientific claim verifier. Judge the claim ONLY \
on whether it is well-established. Try to REFUTE it. Respond on the first line exactly \
`VERDICT: verified` (well-established), `VERDICT: refuted` (contradicted by established knowledge), \
or `VERDICT: uncertain` (you cannot confidently determine). Default to uncertain when unsure. Then \
a line `REASON: <one sentence>`.";

/// Build the canonical adversarial-verification request for one claim.
pub fn build_verify_request(claim: &str, model: &str) -> Request {
    Request {
        system: Some(VERIFY_SYSTEM.to_string()),
        messages: vec![Message::user(vec![ContentBlock::text(format!(
            "Claim:\n{claim}\n\nRespond with VERDICT then REASON."
        ))])],
        model: model.to_string(),
        max_tokens: None,
        reasoning_effort: None,
        tools: vec![],
    }
}

/// Parse a model reply into a verdict. Conservative: anything unrecognised → Uncertain
/// (a verifier can never accidentally up-vote to Verified/Refuted on garbled output).
pub fn parse_verdict(reply: &str) -> VerdictStatus {
    let low = reply.to_lowercase();
    if let Some(idx) = low.find("verdict:") {
        let tail = low[idx + "verdict:".len()..].trim_start();
        if word_at_start(tail, "refuted") {
            return VerdictStatus::Refuted;
        }
        if word_at_start(tail, "verified") {
            return VerdictStatus::Verified;
        }
    }
    VerdictStatus::Uncertain
}

/// `s` begins with `word` AND `word` ends at a boundary — so a run-on token like
/// `verifieddespite` does NOT count as the verdict `verified` (a verifier must
/// never up-vote on garbled output). `word` is ASCII, so byte-slicing is safe.
fn word_at_start(s: &str, word: &str) -> bool {
    s.starts_with(word)
        && s[word.len()..]
            .chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric() && c != '_')
}

/// Real backend: dispatch each fleet member to its ACTUAL cloud provider via the
/// Rig adapter. Each provider's key is read from its env var; a member whose key is
/// unset (or provider unknown) errors and is fail-closed by the orchestrator — it
/// cannot manufacture corroboration. The judgement's `family` is the provider it
/// actually reached (never a caller-supplied label), so independence stays honest.
pub struct MultiProviderRigBackend;

impl FleetBackend for MultiProviderRigBackend {
    fn judge(
        &self,
        claim: &str,
        member: &FleetMember,
    ) -> impl Future<Output = anyhow::Result<VendorJudgement>> + Send {
        let claim = claim.to_string();
        let member = member.clone();
        async move {
            let provider = crate::Provider::parse(&member.provider)
                .ok_or_else(|| anyhow::anyhow!("unknown provider: {}", member.provider))?;
            let req = build_verify_request(&claim, &member.model);
            let reply = crate::complete_with(provider, &req).await?;
            Ok(VendorJudgement {
                family: provider.family().to_string(),
                model: member.model,
                status: parse_verdict(&reply),
                reasoning: reply,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Mock: maps a MODEL to its canned judgement, or to an error (fail-closed).
    /// Keyed by model (not family) so two models of one family can disagree.
    struct Mock {
        by_model: HashMap<String, Result<VerdictStatus, ()>>,
    }

    impl FleetBackend for Mock {
        fn judge(
            &self,
            _claim: &str,
            member: &FleetMember,
        ) -> impl Future<Output = anyhow::Result<VendorJudgement>> + Send {
            // Resolve synchronously so the future owns its data (Send, 'static).
            let resolved = self.by_model.get(&member.model).cloned();
            let member = member.clone();
            async move {
                match resolved {
                    Some(Ok(status)) => Ok(VendorJudgement {
                        family: member.family,
                        model: member.model,
                        status,
                        reasoning: "mock".into(),
                    }),
                    _ => anyhow::bail!("mock backend error for {}", member.model),
                }
            }
        }
    }

    fn member(family: &str, model: &str) -> FleetMember {
        FleetMember { family: family.into(), provider: family.into(), model: model.into() }
    }

    async fn run(fleet: &[FleetMember], map: &[(&str, Result<VerdictStatus, ()>)]) -> FleetVerdict {
        let backend = Mock {
            by_model: map.iter().map(|(m, r)| (m.to_string(), *r)).collect(),
        };
        verify_bearing_claim("a bearing claim", fleet, &backend).await
    }

    #[tokio::test]
    async fn two_independent_families_supporting_verifies() {
        let v = run(
            &[member("anthropic", "opus"), member("openai", "gpt")],
            &[("opus", Ok(VerdictStatus::Verified)), ("gpt", Ok(VerdictStatus::Verified))],
        )
        .await;
        assert_eq!(v.status, VerdictStatus::Verified);
        assert_eq!(v.independence, Independence::MultiFamily);
        assert_eq!(v.reason, "multi_family_supported");
    }

    #[tokio::test]
    async fn two_independent_families_refuting_refutes() {
        let v = run(
            &[member("anthropic", "opus"), member("openai", "gpt")],
            &[("opus", Ok(VerdictStatus::Refuted)), ("gpt", Ok(VerdictStatus::Refuted))],
        )
        .await;
        assert_eq!(v.status, VerdictStatus::Refuted);
        assert_eq!(v.reason, "multi_family_refuted");
    }

    #[tokio::test]
    async fn disagreement_is_uncertain() {
        let v = run(
            &[member("anthropic", "opus"), member("openai", "gpt")],
            &[("opus", Ok(VerdictStatus::Verified)), ("gpt", Ok(VerdictStatus::Refuted))],
        )
        .await;
        assert_eq!(v.status, VerdictStatus::Uncertain);
        assert_eq!(v.reason, "cross_vendor_disagreement");
    }

    #[tokio::test]
    async fn single_family_cannot_verify() {
        // Two models, but same family → independence gate unmet.
        let v = run(
            &[member("anthropic", "opus"), member("anthropic", "haiku")],
            &[("opus", Ok(VerdictStatus::Verified)), ("haiku", Ok(VerdictStatus::Verified))],
        )
        .await;
        assert_eq!(v.status, VerdictStatus::Uncertain);
        assert_eq!(v.reason, "single_family_independence_unmet");
        assert_eq!(v.independence, Independence::SingleFamily);
    }

    #[tokio::test]
    async fn failed_verifier_is_fail_closed() {
        // openai errors → only anthropic weighs in → can't meet the gate.
        let v = run(
            &[member("anthropic", "opus"), member("openai", "gpt")],
            &[("opus", Ok(VerdictStatus::Verified)), ("gpt", Err(()))],
        )
        .await;
        assert_eq!(v.status, VerdictStatus::Uncertain);
        assert_eq!(v.judgements.len(), 1, "the failed member contributes nothing");
    }

    #[tokio::test]
    async fn one_dissenter_blocks_a_multi_family_refutation() {
        // Two families refute, one independent family verifies → disagreement,
        // NOT Refuted. A trust tool does not assert under cross-vendor conflict.
        let v = run(
            &[member("anthropic", "opus"), member("openai", "gpt"), member("google", "gemini")],
            &[
                ("opus", Ok(VerdictStatus::Refuted)),
                ("gpt", Ok(VerdictStatus::Refuted)),
                ("gemini", Ok(VerdictStatus::Verified)),
            ],
        )
        .await;
        assert_eq!(v.status, VerdictStatus::Uncertain);
        assert_eq!(v.reason, "cross_vendor_disagreement");
    }

    #[tokio::test]
    async fn all_uncertain_is_insufficient_corroboration() {
        let v = run(
            &[member("anthropic", "opus"), member("openai", "gpt")],
            &[("opus", Ok(VerdictStatus::Uncertain)), ("gpt", Ok(VerdictStatus::Uncertain))],
        )
        .await;
        assert_eq!(v.status, VerdictStatus::Uncertain);
        assert_eq!(v.reason, "insufficient_corroboration");
    }

    #[tokio::test]
    async fn same_family_models_disagreeing_is_uncertain() {
        // Two models of ONE family disagree → still a disagreement → Uncertain.
        let v = run(
            &[member("anthropic", "opus"), member("anthropic", "haiku")],
            &[("opus", Ok(VerdictStatus::Verified)), ("haiku", Ok(VerdictStatus::Refuted))],
        )
        .await;
        assert_eq!(v.status, VerdictStatus::Uncertain);
        assert_eq!(v.reason, "cross_vendor_disagreement");
    }

    #[test]
    fn parse_verdict_is_conservative() {
        assert_eq!(parse_verdict("VERDICT: verified\nREASON: well established"), VerdictStatus::Verified);
        assert_eq!(parse_verdict("verdict: refuted — contradicts X"), VerdictStatus::Refuted);
        assert_eq!(parse_verdict("VERDICT: uncertain"), VerdictStatus::Uncertain);
        assert_eq!(parse_verdict("I think this is probably true"), VerdictStatus::Uncertain);
        assert_eq!(parse_verdict(""), VerdictStatus::Uncertain);
        // Run-on / garbled tokens must NOT up-vote (word-boundary guard).
        assert_eq!(parse_verdict("VERDICT: verifieddespitealluncertainty"), VerdictStatus::Uncertain);
        assert_eq!(parse_verdict("VERDICT: refutedness is unclear"), VerdictStatus::Uncertain);
        // A boundary char after the keyword is fine.
        assert_eq!(parse_verdict("VERDICT: refuted."), VerdictStatus::Refuted);
    }
}
