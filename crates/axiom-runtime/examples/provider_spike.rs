//! Live provider-integration spike (needs a key — makes a real API call).
//!
//! Run:
//!   DEEPSEEK_API_KEY=sk-... cargo run -p axiom-runtime --example provider_spike

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let model = std::env::var("AXIOM_SPIKE_MODEL")
        .unwrap_or_else(|_| rig_core::providers::deepseek::DEEPSEEK_V4_FLASH.to_string());
    let has_key = std::env::var("DEEPSEEK_API_KEY").map(|k| k.len()).unwrap_or(0);
    println!("→ Rig DeepSeek provider  model={model}  (DEEPSEEK_API_KEY {has_key} chars)");

    let reply = axiom_runtime::deepseek_complete(&model, "Reply with exactly one word: ok").await?;
    println!("← reply: {reply:?}");
    Ok(())
}
