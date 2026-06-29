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
/// P0.3 default is "first usable match" (deterministic); cost/latency/quality
/// ranking and the user's per-role pin are layered on later. Returns `model: None`
/// with a note when nothing qualifies (e.g. the only candidate is sunset).
pub fn route(matrix: &CapabilityMatrix, req: &RouteRequest, now_ms: i64) -> RouteDecision {
    let pick = matrix
        .usable(now_ms)
        .find(|m| {
            req.required_input.iter().all(|&x| m.supports_input(x))
                && req.required_output.iter().all(|&x| m.supports_output(x))
                && req.required_features.iter().all(|f| m.has_feature(f))
        })
        .cloned();

    match pick {
        Some(m) => RouteDecision { model: Some(m), note: None },
        None => RouteDecision {
            model: None,
            note: Some("no usable model satisfies the required modalities/features".into()),
        },
    }
}

/// The independence currently achievable across connected providers.
pub fn independence(matrix: &CapabilityMatrix, now_ms: i64) -> Independence {
    let fams = matrix.usable_families(now_ms);
    match fams.len() {
        0 => Independence::SingleFamily, // nothing connected → conservative
        1 if is_local_family(&fams[0]) => Independence::LocalOnly,
        1 => Independence::SingleFamily,
        _ => Independence::MultiFamily,
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
}
