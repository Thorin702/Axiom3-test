//! Live Multi-LLM AB-MCTS search. Requires at least one provider key in env
//! (DEEPSEEK_API_KEY, and optionally QWEN_API_KEY to exercise the multi-LLM axis).
//! Prints the adaptive search tree (explore/refine + score per node) + the final synthesis.
//!
//!   DEEPSEEK_API_KEY=... [QWEN_API_KEY=...] cargo run -p axiom-orchestrate --example search_live -- "your task"
use axiom_orchestrate::events::{EventSink, RunEvent, RunEventBody};
use axiom_orchestrate::{orchestrate_search, ModelArm, OrchestrateOptions, SearchConfig};
use axiom_runtime::supervisor::Cancel;
use std::sync::Arc;

struct PrintSink;
impl EventSink for PrintSink {
    fn emit(&self, ev: RunEvent) {
        match ev.body {
            RunEventBody::PlanReady { notes, .. } => {
                for n in notes {
                    println!("plan: {n}");
                }
            }
            RunEventBody::PhaseChanged { phase, .. } => println!("── {phase}"),
            RunEventBody::AgentSpawned { role, .. } => println!("  + {role}"),
            RunEventBody::AgentFinished { summary, outcome, .. } => match summary {
                Some(s) => println!("      {s}"),
                None => println!("      ({outcome})"),
            },
            RunEventBody::RunFinished { agents_ok, agents_err, .. } => {
                println!("■ done: {agents_ok} ok / {agents_err} err")
            }
            _ => {}
        }
    }
}

#[tokio::main]
async fn main() {
    let goal = std::env::args().nth(1).unwrap_or_else(|| {
        "Write a crisp 150-word explainer on why agreement between two AI models is weak \
         evidence that a claim is true."
            .into()
    });

    let mut models = Vec::new();
    if std::env::var("DEEPSEEK_API_KEY").is_ok() {
        models.push(ModelArm { provider: "deepseek".into(), model: "deepseek-v4-flash".into() });
    }
    if std::env::var("QWEN_API_KEY").is_ok() {
        models.push(ModelArm { provider: "qwen".into(), model: "qwen-plus".into() });
    }
    if models.is_empty() {
        eprintln!("set DEEPSEEK_API_KEY (and optionally QWEN_API_KEY) to run");
        return;
    }

    let opts = OrchestrateOptions {
        run_id: "search".into(),
        provider: models[0].provider.clone(),
        model: models[0].model.clone(),
        concurrency: 4,
    };
    let budget: usize = std::env::var("AXIOM_SEARCH_BUDGET").ok().and_then(|s| s.parse().ok()).unwrap_or(8);
    let cfg = SearchConfig { budget, max_depth: 3, models: models.clone(), seed: 42 };

    println!("=== AB-MCTS · budget {budget} · {} model(s) ===\n=== goal: {goal}\n", models.len());
    let run = orchestrate_search(&goal, &opts, cfg, Arc::new(PrintSink), &Cancel::new()).await;
    println!("\n=== status: {:?} · {} candidates kept ===", run.status, run.steps.len());
    for s in &run.steps {
        println!("  - {}", s.spec.role);
    }
    let synth = run.synthesis.unwrap_or_default();
    let head: String = synth.chars().take(1200).collect();
    println!("\n=== SYNTHESIS (first 1200 chars) ===\n{head}");
}
