//! Comparison scenario C (hard): omics differential-expression analysis. A gene×sample
//! expression matrix (3 control + 3 treatment) is given; the agent must compute per-gene log2
//! fold change + a two-sample p-value (no scipy → hand-rolled t-test), then render a native
//! volcano + a z-scored heatmap + a co-expression network, and give a biology-grounded readout
//! — all from COMPUTED values. Exercises the native scientific-figure stack end to end.
//!   DEEPSEEK_API_KEY=... cargo run -p axiom-orchestrate --example bench_c
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

    let dir = std::env::temp_dir().join(format!("axiom-bench-c-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // 3 up (IL6/TNF/STAT3, pro-inflammatory), 2 down (FOXP3/IL10, regulatory), 3 NS housekeeping.
    std::fs::write(
        dir.join("expr.csv"),
        "gene,c1,c2,c3,t1,t2,t3\n\
         IL6,5,6,5,50,55,48\nTNF,8,7,9,40,44,38\nSTAT3,10,11,9,30,28,33\n\
         FOXP3,40,42,38,8,7,9\nIL10,35,33,37,10,12,9\n\
         ACTB,100,102,98,101,99,100\nGAPDH,90,92,88,91,89,90\nB2M,50,51,49,50,52,48\n",
    )
    .unwrap();

    let sb = Sandbox::with_mode(&dir, SandboxMode::ReadWrite).unwrap();
    let kernel = Arc::new(PyKernel::start(&py, &dir).expect("kernel"));
    let model_id = std::env::var("AXIOM_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".into());
    let model = ProviderModel { provider: "deepseek".into(), model: model_id.clone(), timeout_s: 150 };
    let budget = CodingBudget { max_steps: 24, per_call_timeout_s: 150, cmd_timeout_s: 90, cell_timeout_s: 600 };

    println!("=== 场景C 组学差异表达 (model={model_id}) ===");
    let (mut plan, mut py_steps) = (false, 0);
    let res = run_coding_agent(
        "expr.csv holds gene expression: columns c1..c3 are control, t1..t3 are treatment. Using the \
         python tool: (1) for each gene compute log2 fold change (mean treatment / mean control) and \
         a two-sample p-value (there is NO scipy — implement Welch's t-test with numpy yourself, \
         approximating the p-value); (2) render a native ```volcano block with each gene's gene/lfc/padj; \
         (3) render a native ```heatmap of z-scored expression (rows=genes, cols=the 6 samples); \
         (4) build a co-expression ```network among the differentially-expressed genes (nodes=DE genes \
         grouped up/down, edges between genes whose expression correlates |r|>0.8). Use ONLY computed \
         values. Finish with a short biology-grounded interpretation of what the treatment does.",
        &sb, &model, &Cancel::new(), &Pause::new(), &budget, Some(kernel),
        |s| {
            if s.tool == "plan" { plan = true; }
            if s.tool == "python" { py_steps += 1; }
            let o: String = s.observation.lines().next().unwrap_or("").chars().take(72).collect();
            println!("  [{}] {} {} → {}", s.n, s.tool, s.detail, o);
        },
    )
    .await;

    let sy = &res.summary;
    let blocks = ["```volcano", "```heatmap", "```network"];
    let found: Vec<&str> = blocks.iter().filter(|b| sy.contains(**b)).copied().collect();
    println!("\n=== status {} ({} steps) · plan:{} python步:{} ===", res.status, res.steps, plan, py_steps);
    println!("=== 原生图块: {:?} · matplotlib图:{} ===", found, res.figures.len());
    println!("\nSUMMARY(截断):\n{}", sy.chars().take(1600).collect::<String>());
    let ok = found.len() >= 2;
    println!("\n=== 判定: 组学三图产出≥2 = {} ===", if ok { "PASS ✓" } else { "未达标" });
    let _ = std::fs::remove_dir_all(&dir);
    std::process::exit(if ok { 0 } else { 1 });
}
