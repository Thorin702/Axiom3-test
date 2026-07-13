//! Probe: does the analysis agent actually EMIT ask_user (the intake clarifying question)?
//! Runs a deliberately VAGUE analysis request against a real model and prints whether on_ask ever
//! fires (auto-answering so the run proceeds), plus each step's tool + a snippet of its reasoning.
//!
//!   ZHIPU_API_KEY=... AXIOM_PROVIDER=zhipu AXIOM_MODEL=glm-5.2 \
//!     cargo run -p axiom-orchestrate --example ask_probe
use axiom_orchestrate::coding::sandbox::{Sandbox, SandboxMode};
use axiom_orchestrate::coding::{run_coding_agent, CodingBudget, ProviderModel};
use axiom_orchestrate::kernel::PyKernel;
use axiom_orchestrate::Pause;
use axiom_runtime::supervisor::Cancel;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let venv = dirs_data().join("com.axiom3.desktop/notebook/.venv/bin/python");
    let py = if venv.exists() { venv.to_string_lossy().into_owned() } else { "python3".into() };

    let dir = std::env::temp_dir().join(format!("axiom-ask-probe-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("sales.csv"),
        "month,sales\nJan,12\nFeb,19\nMar,15\nApr,27\nMay,34\nJun,30\n",
    )
    .unwrap();

    let sb = Sandbox::with_mode(&dir, SandboxMode::ReadWrite).unwrap();
    let kernel = Arc::new(PyKernel::start(&py, &dir).expect("start kernel"));
    let provider = std::env::var("AXIOM_PROVIDER").unwrap_or_else(|_| "zhipu".into());
    let model_id = std::env::var("AXIOM_MODEL").unwrap_or_else(|_| "glm-5.2".into());
    let model = ProviderModel { provider: provider.clone(), model: model_id.clone(), timeout_s: 120 };
    let budget = CodingBudget { max_steps: 8, per_call_timeout_s: 120, cmd_timeout_s: 60, cell_timeout_s: 300 };

    // A VAGUE, casual request — exactly the kind an intake ask_user is meant to clarify.
    let goal = std::env::var("AXIOM_GOAL").unwrap_or_else(|_| {
        "帮我分析一下工作区里的 sales.csv,看看有什么值得关注的。Data file sales.csv is in your workspace (cwd).".into()
    });

    let pause = Pause::new();
    if std::env::var("AXIOM_NO_INTAKE").is_err() {
        pause.require_intake(); // force the intake ask (what production does for a first-round analysis)
    }
    let pause_ans = pause.clone();
    let ask_count = Arc::new(AtomicUsize::new(0));
    let ask_count2 = ask_count.clone();

    println!("=== ask_probe (provider={provider} model={model_id}) ===\n=== goal: {goal}\n");
    let res = run_coding_agent(
        &goal,
        &sb,
        &model,
        &Cancel::new(),
        &pause,
        &budget,
        Some(kernel),
        |step| {
            let think: String = step.reasoning.lines().next().unwrap_or("").chars().take(90).collect();
            let obs: String = step.observation.lines().next().unwrap_or("").chars().take(70).collect();
            println!("  [step {}] tool={} · think=\"{}\" → {}", step.n, step.tool, think, obs);
        },
        |card_id, card| {
            ask_count2.fetch_add(1, Ordering::Relaxed);
            println!("\n  >>> ✅ ASK_USER FIRED (card_id={card_id}):\n      {card}\n      (auto-answering to continue)\n");
            pause_ans.set_answer("用最合适的默认设置继续，并披露假设。".into());
        },
    )
    .await;

    println!("\n=== status: {} ({} steps) ===", res.status, res.steps);
    println!("=== ask_user fired: {} time(s) → {} ===",
        ask_count.load(Ordering::Relaxed),
        if ask_count.load(Ordering::Relaxed) > 0 { "ASKING WORKS ✓" } else { "NEVER ASKED ✗" });
    let _ = std::fs::remove_dir_all(&dir);
}

fn dirs_data() -> std::path::PathBuf {
    std::env::var_os("HOME")
        .map(|h| std::path::PathBuf::from(h).join("Library/Application Support"))
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}
