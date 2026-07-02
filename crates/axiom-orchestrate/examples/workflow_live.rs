//! Live smoke test of the multi-agent WORKFLOW: a real goal is decomposed into sub-
//! agents, run concurrently, reviewed, and synthesized — printing the full event stream.
//!
//!   DEEPSEEK_API_KEY=... cargo run -p axiom-orchestrate --example workflow_live -- "your goal"
use axiom_orchestrate::events::{EventSink, RunEvent, RunEventBody};
use axiom_orchestrate::step::ProviderStep;
use axiom_orchestrate::supervisor::LlmDecomposer;
use axiom_orchestrate::{orchestrate, OrchestrateOptions};
use axiom_runtime::supervisor::Cancel;
use std::sync::Arc;

struct PrintSink;
impl EventSink for PrintSink {
    fn emit(&self, ev: RunEvent) {
        match ev.body {
            RunEventBody::RunStarted { title, .. } => println!("[{:>2}] ▶ run_started: {title}", ev.seq),
            RunEventBody::PlanReady { planned_agents, .. } => println!("[{:>2}] ✓ plan_ready: {planned_agents} sub-agents", ev.seq),
            RunEventBody::PhaseChanged { phase, index, total } => println!("[{:>2}] ── phase {index}/{total}: {phase}", ev.seq),
            RunEventBody::AgentSpawned { role, agent_id, .. } => println!("[{:>2}]    + {role} ({agent_id})", ev.seq),
            RunEventBody::AgentFinished { agent_id, outcome, summary, .. } => {
                println!("[{:>2}]    ✓ {agent_id}: {outcome} — {}", ev.seq, summary.unwrap_or_default())
            }
            RunEventBody::RunProgress { agents_done, agents_total, .. } => println!("[{:>2}]    progress {agents_done}/{agents_total}", ev.seq),
            RunEventBody::RunFinished { status, agents_ok, agents_total, .. } => {
                println!("[{:>2}] ■ run_finished: {status} ({agents_ok}/{agents_total} ok)", ev.seq)
            }
            _ => {}
        }
    }
}

#[tokio::main]
async fn main() {
    let arg = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    let goal = if arg.trim().is_empty() {
        "Compare REST vs GraphQL for a mobile-app backend across latency, caching, versioning, and tooling; end with a one-line recommendation.".to_string()
    } else {
        arg
    };
    let model = std::env::var("AXIOM_MODEL").unwrap_or_else(|_| "deepseek-chat".into());
    let opts = OrchestrateOptions { run_id: "run-live".into(), provider: "deepseek".into(), model: model.clone(), concurrency: 4 };
    let target = std::env::var("AXIOM_AGENTS").ok().and_then(|s| s.parse().ok());
    let dec = LlmDecomposer { provider: "deepseek".into(), model: model.clone(), target };
    let cancel = Cancel::new();

    println!("=== workflow (model={model})\n=== goal: {goal}\n");
    let run = orchestrate(&goal, &opts, &dec, &ProviderStep, Arc::new(PrintSink), &cancel).await;
    println!("\n=== status: {:?} · {} sub-agents ===", run.status, run.steps.len());
    for s in &run.steps {
        println!("  - [{}] {} → {:?}", s.spec.role, s.spec.goal.chars().take(60).collect::<String>(), s.status);
    }
    println!("\n=== SYNTHESIS ===\n{}", run.synthesis.unwrap_or_default());
}
