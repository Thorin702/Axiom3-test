//! Verifies P0#1: after advertising all 26 native blocks to the analysis agent, does it pick a
//! DOMAIN-appropriate block UNPROMPTED? Two results are dropped in; the agent must choose the block
//! type itself (never told "manhattan"/"slope"). Success = it emits domain blocks that were NOT in
//! the old 4-block vocabulary (chart/forest/heatmap/dist).
//!   DEEPSEEK_API_KEY=... cargo run -p axiom-orchestrate --example bench_d
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

    let dir = std::env::temp_dir().join(format!("axiom-bench-d-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // A GWAS scan: most SNPs null, a clear peak on chr8.
    let mut gwas = String::from("snp,chr,pos,p\n");
    for (i, (c, base)) in [(1u32, 0.4f64), (2, 0.6), (3, 0.3), (8, 0.5)].iter().enumerate() {
        for k in 0..6 {
            let p = if *c == 8 && k >= 3 { 1e-9 * (k as f64) } else { base / (1.0 + k as f64) };
            gwas.push_str(&format!("rs{}{},{},{},{}\n", c, k, c, 100000 * (k as u32 + 1) + i as u32, p.max(1e-12)));
        }
    }
    std::fs::write(dir.join("gwas.csv"), gwas).unwrap();
    // Paired before/after per subject (a treatment measured twice).
    std::fs::write(
        dir.join("paired.csv"),
        "subject,before,after\nS1,4.2,6.8\nS2,3.9,7.1\nS3,5.0,5.4\nS4,4.5,7.9\nS5,3.2,6.0\nS6,4.8,6.6\n",
    )
    .unwrap();

    let sb = Sandbox::with_mode(&dir, SandboxMode::ReadWrite).unwrap();
    let kernel = Arc::new(PyKernel::start(&py, &dir).expect("kernel"));
    let model_id = std::env::var("AXIOM_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".into());
    let model = ProviderModel { provider: "deepseek".into(), model: model_id.clone(), timeout_s: 120 };
    let budget = CodingBudget { max_steps: 16, per_call_timeout_s: 120, cmd_timeout_s: 90, cell_timeout_s: 600 };

    println!("=== P0#1 验证:未点名下的领域块选择 (model={model_id}) ===");
    let res = run_coding_agent(
        "Two result files are in your workspace. Using the python tool, load each and then present \
         EACH with the SINGLE most appropriate NATIVE figure block — choose the block type yourself. \
         Do NOT use ```chart bar/line/scatter, and do NOT just print a table. gwas.csv is a \
         genome-wide association scan (snp, chr, pos, p-value). paired.csv is a paired before/after \
         measurement per subject. Use only values you computed. Keep prose short.",
        &sb, &model, &Cancel::new(), &Pause::new(), &budget, Some(kernel),
        |s| {
            let o: String = s.observation.lines().next().unwrap_or("").chars().take(64).collect();
            println!("  [{}] {} {} → {}", s.n, s.tool, s.detail, o);
        },
    )
    .await;

    let sy = &res.summary;
    // Blocks that DID NOT exist in the old 4-block vocabulary.
    let new_blocks = ["```manhattan", "```slope", "```volcano", "```roc", "```track", "```tree",
        "```seqlogo", "```msa", "```network", "```dose", "```confusion", "```band", "```spectrum"];
    let found: Vec<&str> = new_blocks.iter().filter(|b| sy.contains(**b)).copied().collect();
    println!("\n=== status {} ({} steps) ===", res.status, res.steps);
    println!("=== 未点名却用到的【新广告】领域块: {:?} ===", found);
    println!("\nSUMMARY(截断):\n{}", sy.chars().take(1200).collect::<String>());
    // Ideal: manhattan for GWAS AND slope for paired — but any newly-advertised domain block proves the unlock.
    let ideal = sy.contains("```manhattan") && sy.contains("```slope");
    let ok = !found.is_empty();
    println!("\n=== 判定: 自主选用新领域块 = {} {}===",
        if ok { "PASS ✓" } else { "未达标" },
        if ideal { "(理想:manhattan+slope 都中) " } else { "" });
    let _ = std::fs::remove_dir_all(&dir);
    std::process::exit(if ok { 0 } else { 1 });
}
