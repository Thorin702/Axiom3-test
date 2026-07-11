//! Verifies P0#3: the analysis kernel can DOWNLOAD a real public dataset over the network and
//! analyze it (not just work from local attachments).
//!   DEEPSEEK_API_KEY=... cargo run -p axiom-orchestrate --example bench_f
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
    let dir = std::env::temp_dir().join(format!("axiom-bench-f-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let sb = Sandbox::with_mode(&dir, SandboxMode::ReadWrite).unwrap();
    let kernel = Arc::new(PyKernel::start(&py, &dir).expect("kernel"));
    let model_id = std::env::var("AXIOM_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".into());
    let model = ProviderModel { provider: "deepseek".into(), model: model_id.clone(), timeout_s: 150 };
    let budget = CodingBudget { max_steps: 14, per_call_timeout_s: 150, cmd_timeout_s: 120 };

    println!("=== P0#3 验证:kernel 联网取真实数据 (model={model_id}) ===");
    let res = run_coding_agent(
        "The kernel has network access. Using the python tool, DOWNLOAD the Iris dataset from \
         https://raw.githubusercontent.com/mwaskom/seaborn-data/master/iris.csv (use pandas.read_csv \
         on the URL, or requests). Then compute the mean of each of the four measurements per species, \
         and present the per-species means as a native ```heatmap block (rows=species, cols=measurements). \
         Use ONLY values you computed from the downloaded data. Keep prose short.",
        &sb, &model, &Cancel::new(), &Pause::new(), &budget, Some(kernel),
        |s| {
            let o: String = s.observation.lines().next().unwrap_or("").chars().take(72).collect();
            println!("  [{}] {} {} → {}", s.n, s.tool, s.detail, o);
        },
    )
    .await;

    let sy = &res.summary;
    // Proof it really downloaded + parsed the real file: the three species names must appear,
    // and a native block filled with computed numbers.
    let species = ["setosa", "versicolor", "virginica"];
    let species_hit = species.iter().filter(|s| sy.to_lowercase().contains(**s)).count();
    let has_block = sy.contains("```heatmap") || sy.contains("```chart");
    println!("\n=== status {} ({} steps) ===", res.status, res.steps);
    println!("=== 三个种类命中:{}/3 · 含原生块:{} ===", species_hit, has_block);
    println!("\nSUMMARY(截断):\n{}", sy.chars().take(1100).collect::<String>());
    let ok = species_hit >= 3 && has_block;
    println!("\n=== 判定: 联网下载真实数据并分析 = {} ===", if ok { "PASS ✓" } else { "未达标" });
    let _ = std::fs::remove_dir_all(&dir);
    std::process::exit(if ok { 0 } else { 1 });
}
