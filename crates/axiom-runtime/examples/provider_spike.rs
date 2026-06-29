//! Live adapter spike (needs a key — makes a real API call).
//!
//! Maps a canonical axiom_providers::Request through Rig and prints the reply.
//! Run:
//!   DEEPSEEK_API_KEY=sk-... cargo run -p axiom-runtime --example provider_spike

use axiom_providers::message::{ContentBlock, Message, Request, Role};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let model = std::env::var("AXIOM_SPIKE_MODEL")
        .unwrap_or_else(|_| rig_core::providers::deepseek::DEEPSEEK_V4_FLASH.to_string());
    let has_key = std::env::var("DEEPSEEK_API_KEY").map(|k| k.len()).unwrap_or(0);

    let req = Request {
        system: Some("You are terse. Answer in one word.".into()),
        messages: vec![Message {
            role: Role::User,
            content: vec![ContentBlock::text("Is water wet? Reply yes or no.")],
        }],
        model: model.clone(),
        max_tokens: None,
        reasoning_effort: None,
        tools: vec![],
    };

    println!("→ canonical Request → Rig DeepSeek  model={model}  (DEEPSEEK_API_KEY {has_key} chars)");
    let reply = axiom_runtime::complete(&req).await?;
    println!("← reply: {reply:?}");
    Ok(())
}
