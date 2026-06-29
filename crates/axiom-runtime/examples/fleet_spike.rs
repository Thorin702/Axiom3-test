//! Live cross-vendor fleet smoke (needs ≥2 provider keys — makes real API calls).
//!
//! Set the standard env vars for the vendors you have, e.g.:
//!   export ANTHROPIC_API_KEY=...
//!   export OPENAI_API_KEY=...
//! then:
//!   cargo run -p axiom-runtime --example fleet_spike
//!
//! Builds a fleet from whichever provider keys are present and runs one bearing
//! claim through the real [`MultiProviderRigBackend`] + the independence gate.

use axiom_runtime::fleet::{verify_bearing_claim, FleetMember, MultiProviderRigBackend};
use axiom_runtime::Provider;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // A default model per vendor — override via the AXIOM_FLEET_* env vars.
    let candidates = [
        (Provider::Anthropic, "claude-haiku-4-5-20251001"),
        (Provider::OpenAi, "gpt-4o-mini"),
        (Provider::DeepSeek, "deepseek-chat"),
        (Provider::Gemini, "gemini-1.5-flash"),
        (Provider::Qwen, "qwen-plus"),
    ];

    let fleet: Vec<FleetMember> = candidates
        .iter()
        .filter(|(p, _)| std::env::var(p.env_var()).is_ok())
        .map(|(p, model)| FleetMember {
            family: p.family().to_string(),
            provider: p.family().to_string(),
            model: (*model).to_string(),
        })
        .collect();

    println!("fleet ({} vendors with keys present):", fleet.len());
    for m in &fleet {
        println!("  - {} / {}", m.family, m.model);
    }
    if fleet.len() < 2 {
        println!("\n⚠ need ≥2 vendor keys for a real cross-vendor verdict; with <2 the gate keeps it Uncertain.");
    }

    let claim = std::env::var("AXIOM_FLEET_CLAIM")
        .unwrap_or_else(|_| "Water boils at 100 degrees Celsius at sea level.".to_string());
    println!("\nclaim: {claim}");

    let verdict = verify_bearing_claim(&claim, &[], &fleet, &MultiProviderRigBackend).await;
    println!("\n→ status:       {}", verdict.status.code());
    println!("→ independence: {}", verdict.independence.code());
    println!("→ reason:       {}", verdict.reason);
    for j in &verdict.judgements {
        println!("  · {} ({}): {}", j.family, j.model, j.status.code());
    }
    Ok(())
}
