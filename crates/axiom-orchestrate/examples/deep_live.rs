//! Live test of the deep (two-level) orchestration: goal → top sub-tasks → each fans out
//! into sub-agents, so many agents run through a bounded queue. Prints spawn/finish +
//! peak concurrency to prove the queue actually executes.
//!
//!   DEEPSEEK_API_KEY=... cargo run -p axiom-orchestrate --example deep_live -- "your goal"
use axiom_orchestrate::events::{EventSink, RunEvent, RunEventBody};
use axiom_orchestrate::{orchestrate_deep, OrchestrateOptions};
use axiom_runtime::supervisor::Cancel;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[derive(Default)]
struct CountSink {
    spawned: AtomicUsize,
    finished: AtomicUsize,
    running: AtomicUsize,
    peak: AtomicUsize,
}
impl EventSink for CountSink {
    fn emit(&self, ev: RunEvent) {
        match ev.body {
            RunEventBody::PlanReady { planned_agents, notes, .. } => println!("plan: ~{planned_agents} agents · {}", notes.join("; ")),
            RunEventBody::PhaseChanged { phase, index, total } => println!("── phase {index}/{total}: {phase}"),
            RunEventBody::AgentSpawned { role, .. } => {
                let s = self.spawned.fetch_add(1, Ordering::SeqCst) + 1;
                let r = self.running.fetch_add(1, Ordering::SeqCst) + 1;
                self.peak.fetch_max(r, Ordering::SeqCst);
                println!("  +[{s:>2}] {role}   (running now: {r})");
            }
            RunEventBody::AgentFinished { agent_id, outcome, .. } => {
                self.finished.fetch_add(1, Ordering::SeqCst);
                self.running.fetch_sub(1, Ordering::SeqCst);
                if agent_id.ends_with("-verify") {
                    println!("  reviewer: {outcome}");
                }
            }
            RunEventBody::RunFinished { status, agents_ok, agents_total, .. } => println!("■ run_finished: {status} ({agents_ok}/{agents_total} groups ok)"),
            _ => {}
        }
    }
}

#[tokio::main]
async fn main() {
    let arg = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    let goal = if arg.trim().is_empty() {
        "Produce a launch plan for a privacy-first note-taking app: market analysis, product scope, go-to-market, pricing, and key risks.".to_string()
    } else {
        arg
    };
    let model = std::env::var("AXIOM_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".into());
    let top: usize = std::env::var("AXIOM_TOP").ok().and_then(|s| s.parse().ok()).unwrap_or(4);
    let sub: usize = std::env::var("AXIOM_SUB").ok().and_then(|s| s.parse().ok()).unwrap_or(3);
    let opts = OrchestrateOptions { run_id: "deep".into(), provider: "deepseek".into(), model: model.clone(), concurrency: 3 };
    let sink = Arc::new(CountSink::default());

    println!("=== deep orchestration · model={model} · {top}×{sub} ===\n=== goal: {goal}\n");
    let run = orchestrate_deep(&goal, &opts, top, sub, sink.clone(), &Cancel::new()).await;

    println!(
        "\n=== agents spawned: {} · finished: {} · PEAK CONCURRENT: {} · groups: {} ===",
        sink.spawned.load(Ordering::SeqCst),
        sink.finished.load(Ordering::SeqCst),
        sink.peak.load(Ordering::SeqCst),
        run.steps.len(),
    );
    println!("status: {:?}", run.status);
    println!("\n=== SYNTHESIS (first 900 chars) ===\n{}", run.synthesis.unwrap_or_default().chars().take(900).collect::<String>());
}
