//! Fold completed step outputs into one result. Free-form synthesis ONLY — never an
//! adjudication or verdict (that lives in axiom-core, off-limits here). Falls back to
//! concatenation under role headers if the provider is unavailable or the call fails.
use crate::model::StepState;
use axiom_providers::message::{ContentBlock, Message, Request};
use axiom_runtime::supervisor::{is_retryable_error, retry_async, Cancel, RetryPolicy};
use axiom_runtime::{complete_with, Provider};
use std::time::Duration;

pub async fn synthesize(goal: &str, steps: &[StepState], provider: &str, model: &str, cancel: &Cancel) -> String {
    let concat = concat_fallback(steps);
    if steps.is_empty() || cancel.is_cancelled() {
        return concat;
    }
    let Some(p) = Provider::parse(provider) else {
        return concat;
    };
    let mut body = String::new();
    for s in steps {
        if let Some(out) = &s.output {
            body.push_str(&format!("\n## {} ({})\n{}\n", s.spec.role, s.spec.id, out));
        }
    }
    if body.is_empty() {
        return concat;
    }
    let system = "You fold sub-agent outputs into one coherent result for the user's goal. Be \
                  concise and faithful to the parts; do not invent. Return only the result.";
    let user = format!("GOAL:\n{goal}\n\nSUB-AGENT OUTPUTS:{body}");
    let req = Request {
        system: Some(system.into()),
        messages: vec![Message::user(vec![ContentBlock::text(user)])],
        model: model.to_string(),
        max_tokens: None,
        reasoning_effort: None,
        tools: Vec::new(),
    };
    let r = retry_async(&RetryPolicy::default(), is_retryable_error, || async {
        tokio::time::timeout(Duration::from_secs(45), complete_with(p, &req))
            .await
            .map_err(|_| anyhow::anyhow!("synthesis timed out after 45s"))?
    })
    .await;
    r.unwrap_or(concat)
}

fn concat_fallback(steps: &[StepState]) -> String {
    let mut s = String::new();
    for st in steps {
        if let Some(out) = &st.output {
            s.push_str(&format!("## {}\n{}\n\n", st.spec.role, out));
        }
    }
    if s.is_empty() {
        s.push_str("(no output)");
    }
    s
}
