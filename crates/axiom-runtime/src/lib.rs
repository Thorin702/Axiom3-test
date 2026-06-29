//! Agent-runtime integration (Rig).
//!
//! P0.3 integration spike: confirm Rig can drive a BYO provider end-to-end with a
//! live completion. Validates that Rig works as our runtime and a custom provider
//! (DeepSeek here) is reachable — it does NOT validate Anthropic-specific
//! cache_control / signed-thinking replay (needs a real Anthropic key).

use rig_core::client::{CompletionClient, ProviderClient};
use rig_core::completion::Prompt;
use rig_core::providers::deepseek;

/// Complete a single prompt via Rig's built-in DeepSeek provider.
/// Reads the key from `DEEPSEEK_API_KEY` (Rig's `from_env`).
pub async fn deepseek_complete(model: &str, prompt: &str) -> anyhow::Result<String> {
    let client = deepseek::Client::from_env().map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let agent = client.agent(model).build();
    let reply = agent
        .prompt(prompt)
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    Ok(reply)
}
