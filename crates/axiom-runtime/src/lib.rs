//! Agent-runtime integration (Rig) + the canonical-model → Rig adapter.
//!
//! P0.3/P0.4 increment: map our provider-agnostic [`axiom_providers::Request`]
//! onto a Rig completion and run it. This is the seam where everything downstream
//! (the verification engine) talks to a model through OUR types, not Rig's.
//!
//! Scope of this increment: the text path (system prompt + the last user turn's
//! text) through Rig's DeepSeek provider, using `from_env` for the key. The
//! follow-ups are explicit BYO-key construction across providers, multi-turn
//! history, tool / signed-thinking blocks, and usage extraction (needs Rig's
//! lower-level CompletionModel, which carries `Usage`).

pub mod fleet;

use axiom_providers::message::{ContentBlock, Request, Role};
use rig_core::client::{CompletionClient, ProviderClient};
use rig_core::completion::Prompt;
use rig_core::providers::deepseek;

/// Flatten the last user message's text blocks into a prompt string.
/// (Multi-turn history, tool, and thinking blocks are TODO — P0.5.)
fn last_user_text(req: &Request) -> String {
    req.messages
        .iter()
        .rev()
        .find(|m| m.role == Role::User)
        .map(|m| {
            m.content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// Run a canonical [`Request`] through Rig (DeepSeek provider; key from
/// `DEEPSEEK_API_KEY`). `req.system` becomes the agent preamble; the last user
/// turn's text becomes the prompt.
pub async fn complete(req: &Request) -> anyhow::Result<String> {
    let client = deepseek::Client::from_env().map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let agent = {
        let b = client.agent(&req.model);
        let b = match &req.system {
            Some(sys) => b.preamble(sys),
            None => b,
        };
        b.build()
    };
    let reply = agent
        .prompt(last_user_text(req))
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    Ok(reply)
}

/// Minimal text completion (the original integration spike helper).
pub async fn deepseek_complete(model: &str, prompt: &str) -> anyhow::Result<String> {
    let client = deepseek::Client::from_env().map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let agent = client.agent(model).build();
    let reply = agent
        .prompt(prompt)
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axiom_providers::message::{ContentBlock, Message};

    #[test]
    fn last_user_text_flattens_text_blocks_of_the_last_user_turn() {
        let req = Request {
            system: Some("be terse".into()),
            messages: vec![
                Message::user(vec![ContentBlock::text("ignored older turn")]),
                Message::assistant(vec![ContentBlock::text("...")]),
                Message::user(vec![
                    ContentBlock::text("line one"),
                    // a non-text block is skipped in the text path
                    ContentBlock::Image { media_type: "image/png".into(), data_ref: "blake3:abc".into() },
                    ContentBlock::text("line two"),
                ]),
            ],
            model: "deepseek-v4-flash".into(),
            max_tokens: None,
            reasoning_effort: None,
            tools: vec![],
        };
        assert_eq!(last_user_text(&req), "line one\nline two");
    }
}
