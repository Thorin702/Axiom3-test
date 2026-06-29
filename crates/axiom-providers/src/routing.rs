//! Role-based routing + the cross-vendor independence gate (SSOT §〇.4 / §点4).
//!
//! Connection is per-provider; the router maps an AGENT ROLE to the best usable
//! model from the capability matrix (quality-first defaults; an explicit override
//! can pin a model to a role — that lives in the shell). A bearing claim's
//! adversarial verification requires ≥2 INDEPENDENT model families; otherwise its
//! verdict must be downgraded to `uncertain` rather than emitting `verified`.

use axiom_core::model::Independence;

use crate::capability::{is_local_family, CapabilityMatrix, Modality, ModelCapability};

/// The agent roles the router selects a model for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentRole {
    ReadingParse,
    Reasoning,
    RoutineVerify,
    Adversarial,
    Adjudicate,
    AudioSlot,
    VideoSlot,
}

/// What a role needs from a model.
#[derive(Debug, Clone, Default)]
pub struct RouteRequest {
    pub required_input: Vec<Modality>,
    pub required_output: Vec<Modality>,
    pub required_features: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RouteDecision {
    pub model: Option<ModelCapability>,
    pub note: Option<String>,
}

/// Pick a usable model satisfying every required modality and feature.
///
/// Among matches, selection is deterministic via a stable `(provider, model_id)`
/// tie-break, so routing is reproducible regardless of `/models` discovery order.
/// Cost/latency/quality ranking and the user's per-role pin layer on later.
/// Returns `model: None` with a note when nothing qualifies.
pub fn route(matrix: &CapabilityMatrix, req: &RouteRequest, now_ms: i64) -> RouteDecision {
    let mut matches: Vec<&ModelCapability> = matrix
        .usable(now_ms)
        .filter(|m| {
            req.required_input.iter().all(|&x| m.supports_input(x))
                && req.required_output.iter().all(|&x| m.supports_output(x))
                && req.required_features.iter().all(|f| m.has_feature(f))
        })
        .collect();
    // Stable tie-break — independent of insertion/discovery order.
    matches.sort_by(|a, b| {
        (a.provider.as_str(), a.model_id.as_str()).cmp(&(b.provider.as_str(), b.model_id.as_str()))
    });

    match matches.first() {
        Some(m) => RouteDecision { model: Some((*m).clone()), note: None },
        None => RouteDecision {
            model: None,
            note: Some("no usable model satisfies the required modalities/features".into()),
        },
    }
}

/// The independence currently achievable across connected providers.
///
/// NOTE: 0 usable families also maps to `SingleFamily` (conservative — the bearing
/// gate blocks either way). To distinguish "nothing connected" from "one vendor"
/// when setting an `uncertain_reason`, callers check [`CapabilityMatrix::has_usable`].
pub fn independence(matrix: &CapabilityMatrix, now_ms: i64) -> Independence {
    independence_of_families(&matrix.usable_families(now_ms))
}

/// Classify the cross-vendor independence of a SET of model families (deduped).
/// Independence comes only from distinct CLOUD vendors — a local/offline model is
/// the user's own and provides no third-party corroboration. So: ≥2 distinct CLOUD
/// families → `MultiFamily`; exactly 1 cloud family → `SingleFamily`; 0 cloud but a
/// local family present → `LocalOnly`; nothing → `SingleFamily` (conservative).
/// Shared by [`independence`] (over the connected matrix) and the verification
/// fleet (over the families that actually weighed in on a claim).
pub fn independence_of_families(families: &[String]) -> Independence {
    let mut uniq: Vec<&str> = families.iter().map(|s| s.as_str()).collect();
    uniq.sort_unstable();
    uniq.dedup();
    let cloud = uniq.iter().filter(|f| !is_local_family(f)).count();
    let has_local = uniq.iter().any(|f| is_local_family(f));
    match (cloud, has_local) {
        (c, _) if c >= 2 => Independence::MultiFamily,
        (1, _) => Independence::SingleFamily,
        (0, true) => Independence::LocalOnly,
        _ => Independence::SingleFamily, // nothing usable → conservative
    }
}

/// May a BEARING claim be `verified` by the adversarial fleet, or must it stay
/// `uncertain` for want of cross-vendor independence? (SSOT §〇.4 gate.)
pub fn bearing_independence_met(matrix: &CapabilityMatrix, now_ms: i64) -> bool {
    independence(matrix, now_ms) == Independence::MultiFamily
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::ModelStatus;

    fn cap(model: &str, family: &str, status: ModelStatus, feats: &[&str], out: &[Modality]) -> ModelCapability {
        ModelCapability {
            provider: family.into(),
            model_id: model.into(),
            family: family.into(),
            input_modalities: vec![Modality::Text, Modality::Image],
            output_modalities: out.to_vec(),
            features: feats.iter().map(|s| s.to_string()).collect(),
            status,
            sunset_date: None,
            cost_input_per_mtok: None,
            cost_output_per_mtok: None,
        }
    }

    #[test]
    fn route_picks_a_usable_model_meeting_requirements() {
        let m = CapabilityMatrix::new(vec![
            cap("haiku", "anthropic", ModelStatus::Available, &["reasoning"], &[Modality::Text]),
        ]);
        let req = RouteRequest {
            required_input: vec![Modality::Text],
            required_output: vec![Modality::Text],
            required_features: vec!["reasoning".into()],
        };
        let d = route(&m, &req, 0);
        assert_eq!(d.model.unwrap().model_id, "haiku");
    }

    #[test]
    fn route_is_deterministic_regardless_of_order() {
        let req = RouteRequest {
            required_input: vec![Modality::Text],
            required_output: vec![Modality::Text],
            required_features: vec![],
        };
        let opus = cap("opus", "anthropic", ModelStatus::Available, &[], &[Modality::Text]);
        let haiku = cap("haiku", "anthropic", ModelStatus::Available, &[], &[Modality::Text]);
        let m1 = CapabilityMatrix::new(vec![opus.clone(), haiku.clone()]);
        let m2 = CapabilityMatrix::new(vec![haiku, opus]);
        // Stable (provider, model_id) tie-break → "haiku" < "opus" in both orders.
        assert_eq!(route(&m1, &req, 0).model.unwrap().model_id, "haiku");
        assert_eq!(route(&m2, &req, 0).model.unwrap().model_id, "haiku");
    }

    #[test]
    fn route_returns_none_when_only_candidate_is_suspended() {
        let m = CapabilityMatrix::new(vec![
            cap("fable", "anthropic", ModelStatus::Suspended, &["reasoning"], &[Modality::Text]),
        ]);
        let d = route(&m, &RouteRequest::default(), 0);
        assert!(d.model.is_none());
        assert!(d.note.is_some());
    }

    #[test]
    fn independence_reflects_family_diversity() {
        let two = CapabilityMatrix::new(vec![
            cap("opus", "anthropic", ModelStatus::Available, &[], &[Modality::Text]),
            cap("gpt", "openai", ModelStatus::Available, &[], &[Modality::Text]),
        ]);
        assert_eq!(independence(&two, 0), Independence::MultiFamily);
        assert!(bearing_independence_met(&two, 0));

        let one_cloud = CapabilityMatrix::new(vec![
            cap("opus", "anthropic", ModelStatus::Available, &[], &[Modality::Text]),
            cap("haiku", "anthropic", ModelStatus::Available, &[], &[Modality::Text]),
        ]);
        assert_eq!(independence(&one_cloud, 0), Independence::SingleFamily);
        assert!(!bearing_independence_met(&one_cloud, 0), "single family must NOT meet the bearing gate");

        let one_local = CapabilityMatrix::new(vec![
            cap("gpt-oss", "ollama", ModelStatus::Available, &[], &[Modality::Text]),
        ]);
        assert_eq!(independence(&one_local, 0), Independence::LocalOnly);
    }

    #[test]
    fn local_family_does_not_grant_cross_vendor_independence() {
        // One cloud + one local must NOT be MultiFamily — a local model provides no
        // third-party corroboration, so a bearing claim can't verify on this alone.
        assert_eq!(
            independence_of_families(&["anthropic".into(), "ollama".into()]),
            Independence::SingleFamily
        );
        // Two distinct CLOUD families IS multi-family.
        assert_eq!(
            independence_of_families(&["anthropic".into(), "openai".into()]),
            Independence::MultiFamily
        );
        // Local-only stays LocalOnly.
        assert_eq!(independence_of_families(&["ollama".into()]), Independence::LocalOnly);
    }
}
