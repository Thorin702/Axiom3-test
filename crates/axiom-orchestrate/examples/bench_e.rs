//! Verifies P0#2: the analysis agent can ground work in real literature via the `litsearch`
//! tool (OpenAlex), reachable regardless of the sandbox network fence.
//!   DEEPSEEK_API_KEY=... cargo run -p axiom-orchestrate --example bench_e
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
    let dir = std::env::temp_dir().join(format!("axiom-bench-e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let sb = Sandbox::with_mode(&dir, SandboxMode::ReadWrite).unwrap();
    let kernel = Arc::new(PyKernel::start(&py, &dir).expect("kernel"));
    let model_id = std::env::var("AXIOM_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".into());
    let model = ProviderModel { provider: "deepseek".into(), model: model_id.clone(), timeout_s: 120 };
    let budget = CodingBudget { max_steps: 10, per_call_timeout_s: 120, cmd_timeout_s: 60 };

    println!("=== P0#2 验证:litsearch 文献检索工具 (model={model_id}) ===");
    let mut used_litsearch = false;
    let mut lit_returned_real = false;
    let res = run_coding_agent(
        "Using the litsearch tool (NOT python), find a seminal peer-reviewed paper introducing CRISPR-Cas9 \
         for programmable genome editing. From the search results, report the exact title, the year, and \
         the DOI. Be brief; cite only what the search actually returned.",
        &sb, &model, &Cancel::new(), &Pause::new(), &budget, Some(kernel),
        |s| {
            if s.tool == "litsearch" {
                used_litsearch = true;
                if s.observation.contains("DOI:") && s.observation.contains("10.") { lit_returned_real = true; }
            }
            let o: String = s.observation.lines().next().unwrap_or("").chars().take(80).collect();
            println!("  [{}] {} {} → {}", s.n, s.tool, s.detail, o);
        },
    )
    .await;

    println!("\n=== status {} ({} steps) · 调用了 litsearch:{} · 返回真实条目(含DOI 10.x):{} ===",
        res.status, res.steps, used_litsearch, lit_returned_real);
    println!("\nSUMMARY(截断):\n{}", res.summary.chars().take(900).collect::<String>());
    let ok = used_litsearch && lit_returned_real;
    println!("\n=== 判定: 分析循环内可检索真实文献 = {} ===", if ok { "PASS ✓" } else { "未达标" });
    let _ = std::fs::remove_dir_all(&dir);
    std::process::exit(if ok { 0 } else { 1 });
}
