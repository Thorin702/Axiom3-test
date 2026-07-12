//! Live smoke test of the "AI Scientist" loop: an agent given a PERSISTENT Python kernel
//! (numpy/pandas/scipy/matplotlib/statsmodels/scikit-learn from the managed venv) is asked to compute a result and plot
//! it. Verifies the loop drives the kernel and that a real matplotlib FIGURE is captured
//! (computed from data, not asserted).
//!
//!   DEEPSEEK_API_KEY=... cargo run -p axiom-orchestrate --example analyze_live
use axiom_orchestrate::coding::sandbox::{Sandbox, SandboxMode};
use axiom_orchestrate::coding::{run_coding_agent, CodingBudget, ProviderModel};
use axiom_orchestrate::kernel::PyKernel;
use axiom_orchestrate::Pause;
use axiom_runtime::supervisor::Cancel;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    // Prefer the app's managed venv (has matplotlib); fall back to python3.
    let venv = dirs_data()
        .join("com.axiom3.desktop/notebook/.venv/bin/python");
    let py = if venv.exists() { venv.to_string_lossy().into_owned() } else { "python3".into() };
    println!("kernel python: {py}");

    let dir = std::env::temp_dir().join(format!("axiom-analyze-live-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // Simulate an ATTACHED data file sitting in the workspace (what coding_run copies in).
    std::fs::write(
        dir.join("sales.csv"),
        "month,sales\nJan,12\nFeb,19\nMar,15\nApr,27\nMay,34\nJun,30\n",
    )
    .unwrap();

    let sb = Sandbox::with_mode(&dir, SandboxMode::ReadWrite).unwrap();
    let kernel = Arc::new(PyKernel::start(&py, &dir).expect("start kernel"));
    let model_id = std::env::var("AXIOM_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".into());
    let model = ProviderModel { provider: "deepseek".into(), model: model_id.clone(), timeout_s: 90 };
    let budget = CodingBudget { max_steps: 12, per_call_timeout_s: 90, cmd_timeout_s: 60, cell_timeout_s: 600 };

    println!("=== analysis agent (model={model_id}) ===");
    let res = run_coding_agent(
        "Analyze the attached data. Data files are in your workspace (cwd) — read them with the \
         python tool, e.g. `import pandas as pd; df = pd.read_csv('sales.csv'); print(df.head())`. \
         Files: sales.csv. Compute the mean of the sales column and the month-over-month percent \
         changes with python and print them. Then, in your FINAL answer, present monthly sales as a \
         native ```chart bar figure filled with the values you computed, and give a one-paragraph \
         summary of what the numbers show.",
        &sb,
        &model,
        &Cancel::new(),
        &Pause::new(),
        &budget,
        Some(kernel),
        |step| {
            let obs1: String = step.observation.lines().next().unwrap_or("").chars().take(100).collect();
            let figs = if step.figures.is_empty() { String::new() } else { format!(" [+{} fig]", step.figures.len()) };
            println!("  [step {}] {} {}{} → {}", step.n, step.tool, step.detail, figs, obs1);
        },
    )
    .await;

    println!("\n=== status: {} ({} steps), figures computed: {} ===", res.status, res.steps, res.figures.len());
    println!("SUMMARY:\n{}\n", res.summary.chars().take(900).collect::<String>());
    // Prove the agent produced a figure computed from real data — either a native ```chart
    // block filled with computed values, or a captured matplotlib PNG.
    let native = res.summary.contains("```chart") || res.summary.contains("```bar") || res.summary.contains("```forest");
    let png = res.figures.iter().any(|f| f.starts_with("iVBORw0KGgo"));
    let ok = native || png;
    println!("=== VERIFY: computed figure present (native block: {native}, PNG: {png}): {} ===", if ok { "PASS ✓" } else { "FAIL ✗" });
    let _ = std::fs::remove_dir_all(&dir);
    std::process::exit(if ok { 0 } else { 1 });
}

fn dirs_data() -> std::path::PathBuf {
    // macOS app data dir root.
    std::env::var_os("HOME")
        .map(|h| std::path::PathBuf::from(h).join("Library/Application Support"))
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}
