//! Verifies P1 per-file artifact registry: files the agent WRITES into the workspace (CSV, saved
//! PNG, …) are enumerated in CodingResult.artifacts (input files present at start are excluded).
//!   DEEPSEEK_API_KEY=... cargo run -p axiom-orchestrate --example bench_i
use axiom_orchestrate::coding::sandbox::{Sandbox, SandboxMode};
use axiom_orchestrate::coding::{run_coding_agent, CodingBudget, ProviderModel};
use axiom_orchestrate::kernel::PyKernel;
use axiom_orchestrate::Pause;
use axiom_runtime::supervisor::Cancel;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let venv = std::path::PathBuf::from(std::env::var_os("HOME").unwrap())
        .join("Library/Application Support/com.axiom3.desktop/notebook/.venv/bin/python");
    let py = if venv.exists() { venv.to_string_lossy().into_owned() } else { "python3".into() };
    let dir = std::env::temp_dir().join(format!("axiom-bench-i-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("data.csv"),
        "group,value\nA,10\nA,14\nB,20\nB,26\nB,23\nA,12\n",
    )
    .unwrap();

    let sb = Sandbox::with_mode(&dir, SandboxMode::ReadWrite).unwrap();
    let kernel = Arc::new(PyKernel::start(&py, &dir).expect("kernel"));
    let model_id = std::env::var("AXIOM_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".into());
    let model = ProviderModel { provider: "deepseek".into(), model: model_id.clone(), timeout_s: 150 };
    let budget = CodingBudget { max_steps: 14, per_call_timeout_s: 150, cmd_timeout_s: 120, cell_timeout_s: 600 };

    println!("=== P1 验证:按文件产物注册表 (model={model_id}) ===");
    let res = run_coding_agent(
        "data.csv has columns group,value. Using the python tool: compute the mean value per group, \
         WRITE the per-group means to a new file results.csv in the workspace (use df.to_csv), and \
         SAVE a bar chart of the means to a file chart.png (matplotlib savefig). Then briefly report.",
        &sb, &model, &Cancel::new(), &Pause::new(), &budget, Some(kernel),
        |s| {
            let o: String = s.observation.lines().next().unwrap_or("").chars().take(66).collect();
            println!("  [{}] {} {} → {}", s.n, s.tool, s.detail, o);
        },
    )
    .await;

    println!("\n=== status {} ({} steps) ===", res.status, res.steps);
    println!("=== CodingResult.artifacts ({} 个文件): ===", res.artifacts.len());
    for a in &res.artifacts {
        println!("   - {} ({} bytes)", a.path, a.bytes);
    }
    let has_csv = res.artifacts.iter().any(|a| a.path.ends_with("results.csv"));
    let has_png = res.artifacts.iter().any(|a| a.path.ends_with(".png"));
    let excludes_input = !res.artifacts.iter().any(|a| a.path == "data.csv");
    println!("\n=== results.csv:{} · *.png:{} · 排除了输入 data.csv:{} ===", has_csv, has_png, excludes_input);
    let ok = has_csv && has_png && excludes_input;
    println!("\n=== 判定: 按文件枚举 agent 产物 = {} ===", if ok { "PASS ✓" } else { "未达标" });
    let _ = std::fs::remove_dir_all(&dir);
    std::process::exit(if ok { 0 } else { 1 });
}
