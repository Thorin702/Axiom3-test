//! Comparison scenario A (hard): end-to-end data analysis. A multi-column patient dataset
//! is dropped in the workspace; the agent must clean, describe, draw a correlation heatmap
//! + per-group distributions + a regression with CIs, and report findings — computed, not
//! asserted. Verifies our platform's AI-Scientist loop on a realistic task.
//!   DEEPSEEK_API_KEY=... cargo run -p axiom-orchestrate --example bench_a
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

    let dir = std::env::temp_dir().join(format!("axiom-bench-a-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // A small but real-ish patient table (with a couple of missing cells to force cleaning).
    std::fs::write(
        dir.join("patients.csv"),
        "id,age,bmi,ldl,hdl,glucose,group\n\
         1,54,28.1,150,42,95,treatment\n2,61,31.4,172,38,110,treatment\n3,47,24.6,132,55,88,control\n\
         4,58,29.8,,45,102,treatment\n5,66,33.2,190,35,140,treatment\n6,43,22.1,120,60,84,control\n\
         7,51,26.7,145,48,92,control\n8,72,30.5,168,40,,treatment\n9,39,21.8,118,62,80,control\n\
         10,64,32.0,180,37,128,treatment\n11,48,25.3,138,52,90,control\n12,56,27.9,155,44,98,control\n",
    )
    .unwrap();

    let sb = Sandbox::with_mode(&dir, SandboxMode::ReadWrite).unwrap();
    let kernel = Arc::new(PyKernel::start(&py, &dir).expect("kernel"));
    let model_id = std::env::var("AXIOM_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".into());
    let model = ProviderModel { provider: "deepseek".into(), model: model_id.clone(), timeout_s: 120 };
    let budget = CodingBudget { max_steps: 20, per_call_timeout_s: 120, cmd_timeout_s: 90, cell_timeout_s: 600 };

    println!("=== 场景A 端到端数据分析 (model={model_id}) ===");
    let (mut used_plan, mut used_edit, mut py_steps) = (false, false, 0);
    let res = run_coding_agent(
        "patients.csv is in your workspace. Do a full analysis with the python tool: (1) load it \
         with pandas and clean missing values; (2) print descriptive stats; (3) compute the \
         correlation matrix of the numeric columns and present it as a native ```heatmap block; \
         (4) compare glucose distribution between the treatment and control groups as a native \
         ```dist block; (5) fit a linear regression of glucose on age+bmi+ldl and report each \
         coefficient with its 95% CI as a native ```forest block. Use ONLY values you computed. \
         Finish with a short paragraph of findings.",
        &sb, &model, &Cancel::new(), &Pause::new(), &budget, Some(kernel),
        |s| {
            if s.tool == "plan" { used_plan = true; }
            if s.tool == "edit" { used_edit = true; }
            if s.tool == "python" { py_steps += 1; }
            let o: String = s.observation.lines().next().unwrap_or("").chars().take(80).collect();
            println!("  [{}] {} {} → {}", s.n, s.tool, s.detail, o);
        },
    )
    .await;

    let sy = &res.summary;
    let blocks = ["```heatmap", "```dist", "```forest", "```chart"];
    let found: Vec<&str> = blocks.iter().filter(|b| sy.contains(**b)).copied().collect();
    println!("\n=== status {} ({} steps) · plan:{} edit:{} python步:{} ===", res.status, res.steps, used_plan, used_edit, py_steps);
    println!("=== 报告里的原生图块: {:?} · matplotlib图:{} ===", found, res.figures.len());
    println!("\nSUMMARY(截断):\n{}", sy.chars().take(1400).collect::<String>());
    let ok = found.len() >= 2 || res.figures.len() >= 2;
    println!("\n=== 判定: 计算并产出多图 = {} ===", if ok { "PASS ✓" } else { "未达标" });
    let _ = std::fs::remove_dir_all(&dir);
    std::process::exit(if ok { 0 } else { 1 });
}
