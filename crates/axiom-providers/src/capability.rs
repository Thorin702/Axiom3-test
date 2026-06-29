//! The capability matrix — the single source of truth per model (SSOT 设计对齐 §点4).
//!
//! Connection is per-PROVIDER; the matrix holds the per-MODEL rows discovered via
//! each provider's `/models` endpoint. `status` + `sunset_date` let the router
//! auto-drop retired models — the live lesson being Fable 5 (export-suspended) and
//! the Sora API sunset.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Modality {
    Text,
    Image,
    Audio,
    Video,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelStatus {
    Available,
    Deprecated,
    /// Past or scheduled retirement (e.g. the Sora API sunset).
    Sunset,
    /// Pulled / export-restricted (e.g. Fable 5).
    Suspended,
}

/// A discovered model and what it can do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelCapability {
    pub provider: String,
    pub model_id: String,
    /// Model FAMILY — the unit of cross-vendor independence (e.g. `anthropic`,
    /// `openai`, `gemini`, `local`). Two models of the same family do NOT
    /// decorrelate errors.
    pub family: String,
    pub input_modalities: Vec<Modality>,
    pub output_modalities: Vec<Modality>,
    /// Free-form capability tags: `reasoning`, `function_calling`, `mcp`,
    /// `realtime`, `native_av_sync`, `prompt_cache`, …
    pub features: Vec<String>,
    pub status: ModelStatus,
    /// Retirement instant (epoch ms); the router drops the model once `now` passes it.
    #[serde(default)]
    pub sunset_date: Option<i64>,
    #[serde(default)]
    pub cost_input_per_mtok: Option<f64>,
    #[serde(default)]
    pub cost_output_per_mtok: Option<f64>,
}

impl ModelCapability {
    /// Usable right now: Available and not past its sunset.
    pub fn is_usable(&self, now_ms: i64) -> bool {
        self.status == ModelStatus::Available
            && self.sunset_date.map_or(true, |s| now_ms < s)
    }
    pub fn supports_input(&self, m: Modality) -> bool {
        self.input_modalities.contains(&m)
    }
    pub fn supports_output(&self, m: Modality) -> bool {
        self.output_modalities.contains(&m)
    }
    pub fn has_feature(&self, f: &str) -> bool {
        self.features.iter().any(|x| x == f)
    }
}

/// Model families that run locally — their verdicts carry a "local, single-family"
/// independence caveat.
pub fn is_local_family(family: &str) -> bool {
    matches!(family, "local" | "ollama" | "vllm" | "llamafile")
}

/// The full set of discovered models across all connected providers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CapabilityMatrix {
    pub models: Vec<ModelCapability>,
}

impl CapabilityMatrix {
    pub fn new(models: Vec<ModelCapability>) -> Self {
        Self { models }
    }

    /// Currently-usable models.
    pub fn usable(&self, now_ms: i64) -> impl Iterator<Item = &ModelCapability> {
        self.models.iter().filter(move |m| m.is_usable(now_ms))
    }

    /// Whether any model is usable right now (distinguishes "nothing connected"
    /// from "one vendor" for independence attribution).
    pub fn has_usable(&self, now_ms: i64) -> bool {
        self.usable(now_ms).next().is_some()
    }

    /// Distinct usable model families (the unit of independence), sorted/deduped.
    pub fn usable_families(&self, now_ms: i64) -> Vec<String> {
        let mut fams: Vec<String> = self.usable(now_ms).map(|m| m.family.clone()).collect();
        fams.sort();
        fams.dedup();
        fams
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cap(model: &str, family: &str, status: ModelStatus, sunset: Option<i64>) -> ModelCapability {
        ModelCapability {
            provider: family.into(),
            model_id: model.into(),
            family: family.into(),
            input_modalities: vec![Modality::Text, Modality::Image],
            output_modalities: vec![Modality::Text],
            features: vec!["reasoning".into(), "prompt_cache".into()],
            status,
            sunset_date: sunset,
            cost_input_per_mtok: None,
            cost_output_per_mtok: None,
        }
    }

    #[test]
    fn suspended_and_sunset_models_are_unusable() {
        let now = 1_000;
        assert!(cap("opus", "anthropic", ModelStatus::Available, None).is_usable(now));
        // Fable-5-style suspension.
        assert!(!cap("fable", "anthropic", ModelStatus::Suspended, None).is_usable(now));
        // Sora-style sunset already passed.
        assert!(!cap("sora", "openai", ModelStatus::Available, Some(500)).is_usable(now));
        // Sunset still in the future → usable.
        assert!(cap("veo", "gemini", ModelStatus::Available, Some(2_000)).is_usable(now));
    }

    #[test]
    fn usable_families_dedupes_and_skips_retired() {
        let m = CapabilityMatrix::new(vec![
            cap("opus", "anthropic", ModelStatus::Available, None),
            cap("haiku", "anthropic", ModelStatus::Available, None),
            cap("gpt", "openai", ModelStatus::Available, None),
            cap("fable", "anthropic", ModelStatus::Suspended, None),
        ]);
        assert_eq!(m.usable_families(1_000), vec!["anthropic", "openai"]);
    }

    #[test]
    fn feature_and_modality_predicates() {
        let c = cap("opus", "anthropic", ModelStatus::Available, None);
        assert!(c.has_feature("prompt_cache"));
        assert!(!c.has_feature("realtime"));
        assert!(c.supports_input(Modality::Image));
        assert!(!c.supports_output(Modality::Audio));
    }
}
