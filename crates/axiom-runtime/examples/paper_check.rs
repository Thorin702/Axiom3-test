//! Run a real document through the full verification pipeline from the CLI — for
//! manual verification runs on real papers.
//!
//! Usage:
//!   cargo run -p axiom-runtime --example paper_check -- <path-to-doc> [max_fleet_claims]
//!
//! Extracts text (pdf/docx/txt/md), runs the deterministic tier, then sends the
//! first N bearing claims through the real cross-vendor fleet (knowledge mode — no
//! references — i.e. the "researcher self-check" flow). Needs >=2 cloud provider
//! keys in env (e.g. DEEPSEEK_API_KEY, QWEN_API_KEY) for an assertive fleet verdict.

use std::collections::BTreeMap;

use axiom_core::verify::verify_draft;
use axiom_runtime::fleet::{verify_bearing_claim, FleetMember, MultiProviderRigBackend};
use axiom_runtime::Provider;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: paper_check <path> [max_fleet_claims]");
    let max: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(8);

    let text = axiom_ingest::extract_text(std::path::Path::new(&path))?;
    let name = std::path::Path::new(&path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.clone());
    println!("\n========================================================");
    println!("PAPER: {name}");
    println!("extracted: {} chars", text.len());

    let reports = verify_draft("paper-test", &text);
    let total = reports.len();
    let bearing: Vec<_> = reports.iter().filter(|r| r.claim.is_bearing).collect();
    let flagged = reports.iter().filter(|r| !r.findings.is_empty()).count();
    let mut by_checker: BTreeMap<&str, usize> = BTreeMap::new();
    for r in &reports {
        for f in &r.findings {
            *by_checker.entry(f.checker).or_insert(0) += 1;
        }
    }
    println!("\n--- deterministic tier ---");
    println!("claims extracted: {total}");
    println!("bearing (fleet-eligible): {}", bearing.len());
    println!("claims with >=1 risk flag: {flagged}");
    println!("findings by checker: {by_checker:?}");

    // Build the fleet from whichever cloud keys are present.
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
    println!(
        "\n--- cross-vendor fleet ({} vendors: {:?}) ---",
        fleet.len(),
        fleet.iter().map(|m| m.family.as_str()).collect::<Vec<_>>()
    );
    if fleet.len() < 2 {
        println!("need >=2 cloud keys for an assertive verdict; skipping fleet.");
        return Ok(());
    }

    let n = max.min(bearing.len());
    println!("judging the first {n} bearing claims (knowledge mode, no references)…\n");
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for (i, r) in bearing.iter().take(n).enumerate() {
        let v = verify_bearing_claim(&r.claim.text, &[], &fleet, &MultiProviderRigBackend).await;
        *counts.entry(v.status.code()).or_insert(0) += 1;
        let snippet: String = r.claim.text.chars().take(90).collect();
        println!(
            "[{:>2}/{n}] {:>9} ({:<13}) :: {snippet}",
            i + 1,
            v.status.code(),
            v.independence.code()
        );
    }
    println!("\nfleet verdict tally: {counts:?}");
    println!("========================================================\n");
    Ok(())
}
