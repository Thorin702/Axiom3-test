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
pub mod supervisor;

use axiom_providers::message::{ContentBlock, Request, Role};
use rig_core::client::{CompletionClient, ProviderClient};
use rig_core::completion::Prompt;
use rig_core::providers::{anthropic, deepseek, gemini, openai};

/// A cloud model vendor the runtime can dispatch to. Keys are never in code: a
/// client is built from the provider's standard env var (see [`Provider::env_var`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Anthropic,
    OpenAi,
    DeepSeek,
    Gemini,
    /// Alibaba Qwen via the DashScope OpenAI-compatible endpoint (Chat Completions).
    Qwen,
}

/// DashScope's OpenAI-compatible base URL (Chat Completions API), used for Qwen.
const QWEN_BASE_URL: &str = "https://dashscope.aliyuncs.com/compatible-mode/v1";

/// Canonical OpenAI base URL — PINNED. rig's `openai::Client::from_env()` honors an
/// `OPENAI_BASE_URL` env override; if that pointed the `openai` family at a shared
/// gateway (e.g. the same DashScope endpoint that serves Qwen, or any OpenRouter/
/// LiteLLM proxy), two "independent" families could resolve to ONE physical model and
/// fake the cross-vendor corroboration H2 requires. We build the OpenAI client with
/// this fixed base URL so the env cannot redirect it. A user who wants a custom
/// OpenAI-compatible endpoint connects it as its OWN distinct family, never as `openai`.
const OPENAI_BASE_URL: &str = "https://api.openai.com/v1";

impl Provider {
    /// Parse a provider label (case-insensitive); `google` aliases Gemini,
    /// `dashscope` aliases Qwen.
    pub fn parse(s: &str) -> Option<Provider> {
        match s.to_ascii_lowercase().as_str() {
            "anthropic" => Some(Provider::Anthropic),
            "openai" => Some(Provider::OpenAi),
            "deepseek" => Some(Provider::DeepSeek),
            "gemini" | "google" => Some(Provider::Gemini),
            "qwen" | "dashscope" => Some(Provider::Qwen),
            _ => None,
        }
    }
    /// Canonical model-family name — the unit of cross-vendor independence.
    pub fn family(self) -> &'static str {
        match self {
            Provider::Anthropic => "anthropic",
            Provider::OpenAi => "openai",
            Provider::DeepSeek => "deepseek",
            Provider::Gemini => "gemini",
            Provider::Qwen => "qwen",
        }
    }
    /// The env var its API key is read from.
    pub fn env_var(self) -> &'static str {
        match self {
            Provider::Anthropic => "ANTHROPIC_API_KEY",
            Provider::OpenAi => "OPENAI_API_KEY",
            Provider::DeepSeek => "DEEPSEEK_API_KEY",
            Provider::Gemini => "GEMINI_API_KEY",
            Provider::Qwen => "QWEN_API_KEY",
        }
    }
}

fn anyerr<E: std::fmt::Display>(e: E) -> anyhow::Error {
    anyhow::anyhow!(e.to_string())
}

/// Build an agent from a concrete provider client and run the canonical request
/// through it. A macro (not a generic fn) keeps the client type CONCRETE per
/// provider, sidestepping the trait-bound puzzle of `Agent<M, P>: Prompt`.
macro_rules! run_provider_agent {
    ($client:expr, $req:expr) => {{
        let client = $client;
        let mut b = client.agent(&$req.model);
        if let Some(sys) = &$req.system {
            b = b.preamble(sys);
        }
        let agent = b.build();
        agent.prompt(last_user_text($req)).await.map_err(anyerr)
    }};
}

/// Run a canonical [`Request`] against `provider`. The client is built from the
/// provider's standard env var; a missing key surfaces as an error (the fleet
/// treats that as fail-closed — a verifier that can't run can't corroborate).
pub async fn complete_with(provider: Provider, req: &Request) -> anyhow::Result<String> {
    match provider {
        Provider::Anthropic => run_provider_agent!(anthropic::Client::from_env().map_err(anyerr)?, req),
        // Build explicitly with the PINNED base URL (see OPENAI_BASE_URL) rather than
        // from_env, which would honor an OPENAI_BASE_URL override and could collapse
        // cross-vendor independence.
        Provider::OpenAi => {
            let key = std::env::var(Provider::OpenAi.env_var())
                .map_err(|_| anyhow::anyhow!("{} not set", Provider::OpenAi.env_var()))?;
            run_provider_agent!(
                openai::Client::builder()
                    .api_key(&key)
                    .base_url(OPENAI_BASE_URL)
                    .build()
                    .map_err(anyerr)?,
                req
            )
        }
        Provider::DeepSeek => run_provider_agent!(deepseek::Client::from_env().map_err(anyerr)?, req),
        Provider::Gemini => run_provider_agent!(gemini::Client::from_env().map_err(anyerr)?, req),
        // OpenAI-compatible (Chat Completions) at DashScope's endpoint.
        Provider::Qwen => {
            let key = std::env::var(Provider::Qwen.env_var())
                .map_err(|_| anyhow::anyhow!("{} not set", Provider::Qwen.env_var()))?;
            run_provider_agent!(
                openai::CompletionsClient::builder()
                    .api_key(&key)
                    .base_url(QWEN_BASE_URL)
                    .build()
                    .map_err(anyerr)?,
                req
            )
        }
    }
}

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
    complete_with(Provider::DeepSeek, req).await
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
