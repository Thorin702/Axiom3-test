//! The agent-execution seam. `AgentStep` generalizes the verification fleet's
//! `FleetBackend::judge` into a kind-agnostic step runner: workflow agents and (later)
//! coding agents are just `AgentStep` impls.
use crate::model::StepSpec;
use axiom_providers::message::{ContentBlock, Message, Request};
use axiom_runtime::supervisor::{is_retryable_error, retry_async, Cancel, RetryPolicy};
use axiom_runtime::{complete_with, Provider};
use std::time::Duration;

/// What a step receives: its spec plus the outputs of the steps it depended on.
pub struct StepInput {
    pub spec: StepSpec,
    pub dep_outputs: Vec<(String, String)>, // (role, output)
}

/// What a step produces. Tokens are 0 for the workflow slice (the text seam reports no
/// usage yet); the coding seam will fill them.
pub struct StepOutcome {
    pub output: String,
    pub tokens_in: u64,
    pub tokens_out: u64,
}

/// One unit of agent work. `run` returns a `Send` future so it composes with
/// `run_batch`'s `buffer_unordered`.
pub trait AgentStep: Send + Sync {
    fn run(
        &self,
        input: StepInput,
        cancel: &Cancel,
    ) -> impl std::future::Future<Output = anyhow::Result<StepOutcome>> + Send;
}

/// The real executor: dispatches a step's goal to its provider/model through the shared
/// runtime seam, copying the fleet backend's `retry_async` + 45s-timeout template
/// verbatim so transient blips recover but a hung call fails closed.
pub struct ProviderStep;

impl AgentStep for ProviderStep {
    fn run(
        &self,
        input: StepInput,
        _cancel: &Cancel,
    ) -> impl std::future::Future<Output = anyhow::Result<StepOutcome>> + Send {
        async move {
            let provider = Provider::parse(&input.spec.provider)
                .ok_or_else(|| anyhow::anyhow!("unknown provider: {}", input.spec.provider))?;
            let req = build_request(&input);
            let reply = retry_async(&RetryPolicy::default(), is_retryable_error, || async {
                tokio::time::timeout(Duration::from_secs(45), complete_with(provider, &req))
                    .await
                    .map_err(|_| anyhow::anyhow!("step timed out after 45s"))?
            })
            .await?;
            Ok(StepOutcome { output: reply, tokens_in: 0, tokens_out: 0 })
        }
    }
}

fn build_request(input: &StepInput) -> Request {
    let mut user = String::new();
    if !input.dep_outputs.is_empty() {
        user.push_str("Context from earlier steps:\n");
        for (role, out) in &input.dep_outputs {
            user.push_str(&format!("\n[{role}]\n{out}\n"));
        }
        user.push_str("\n---\n");
    }
    user.push_str("Task: ");
    user.push_str(&input.spec.goal);
    let system = format!(
        "You are a focused sub-agent in a multi-agent workflow. Your role: {}. Do exactly \
         your task and return only the result — concise and self-contained.",
        input.spec.role
    );
    Request {
        system: Some(system),
        messages: vec![Message::user(vec![ContentBlock::text(user)])],
        model: input.spec.model.clone(),
        max_tokens: input.spec.max_tokens,
        reasoning_effort: None,
        tools: Vec::new(),
    }
}

/// Deterministic offline executor for tests. A goal containing `PANIC` panics (to test
/// isolation) and one containing `FAIL` returns an error.
pub struct MockStep;

impl AgentStep for MockStep {
    fn run(
        &self,
        input: StepInput,
        _cancel: &Cancel,
    ) -> impl std::future::Future<Output = anyhow::Result<StepOutcome>> + Send {
        async move {
            if input.spec.goal.contains("PANIC") {
                panic!("mock step panicked for {}", input.spec.id);
            }
            if input.spec.goal.contains("FAIL") {
                anyhow::bail!("mock step failed for {}", input.spec.id);
            }
            Ok(StepOutcome { output: format!("done: {}", input.spec.goal), tokens_in: 0, tokens_out: 0 })
        }
    }
}
