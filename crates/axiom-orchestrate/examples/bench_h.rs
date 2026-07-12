//! Verifies P1 statistical rigor: given MANY hypothesis tests but NOT told to correct for it, does
//! the agent apply multiple-testing (BH/FDR) correction on its own (from the rigor checklist)?
//!   DEEPSEEK_API_KEY=... cargo run -p axiom-orchestrate --example bench_h
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
    let dir = std::env::temp_dir().join(format!("axiom-bench-h-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // 15 genes x (4 control + 4 treatment). Most null; g1,g2 clearly up in treatment.
    let mut csv = String::from("gene,c1,c2,c3,c4,t1,t2,t3,t4\n");
    for g in 1..=15 {
        let (base, delta) = if g <= 2 { (10.0, 12.0) } else { (10.0 + g as f64 * 0.2, 0.0) };
        let c: Vec<String> = (0..4).map(|k| format!("{:.1}", base + (k as f64 - 1.5) * 0.4)).collect();
        let t: Vec<String> = (0..4).map(|k| format!("{:.1}", base + delta + (k as f64 - 1.5) * 0.4)).collect();
        csv.push_str(&format!("g{g},{},{}\n", c.join(","), t.join(",")));
    }
    std::fs::write(dir.join("expr.csv"), csv).unwrap();

    let sb = Sandbox::with_mode(&dir, SandboxMode::ReadWrite).unwrap();
    let kernel = Arc::new(PyKernel::start(&py, &dir).expect("kernel"));
    let model_id = std::env::var("AXIOM_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".into());
    let model = ProviderModel { provider: "deepseek".into(), model: model_id.clone(), timeout_s: 150 };
    let budget = CodingBudget { max_steps: 16, per_call_timeout_s: 150, cmd_timeout_s: 120, cell_timeout_s: 600 };

    println!("=== P1 验证:统计严谨(未点名的多重检验校正) (model={model_id}) ===");
    let res = run_coding_agent(
        "expr.csv holds expression for 15 genes across 4 control (c1..c4) and 4 treatment (t1..t4) \
         samples. Test EACH gene for a difference between the control and treatment groups, and report \
         which genes are significantly different. Present the result with a native figure block. Keep \
         prose short.",
        &sb, &model, &Cancel::new(), &Pause::new(), &budget, Some(kernel),
        |s| {
            let o: String = s.observation.lines().next().unwrap_or("").chars().take(70).collect();
            println!("  [{}] {} {} → {}", s.n, s.tool, s.detail, o);
        },
    )
    .await;

    let sy_all = format!("{}", res.summary);
    let low = sy_all.to_lowercase();
    // Did it apply multiple-testing correction WITHOUT being told to?
    let rigor_terms = ["fdr", "benjamini", "hochberg", "multipletests", "adjusted p", "p-adj",
        "padj", "q-value", "q value", "corrected p", "multiple test", "multiple compar"];
    let hit: Vec<&str> = rigor_terms.iter().filter(|t| low.contains(**t)).copied().collect();
    println!("\n=== status {} ({} steps) ===", res.status, res.steps);
    println!("=== 未点名却做了多重检验校正,命中术语: {:?} ===", hit);
    println!("\nSUMMARY(截断):\n{}", res.summary.chars().take(1100).collect::<String>());
    let ok = !hit.is_empty();
    println!("\n=== 判定: 自主统计严谨(多重检验校正) = {} ===", if ok { "PASS ✓" } else { "未达标" });
    let _ = std::fs::remove_dir_all(&dir);
    std::process::exit(if ok { 0 } else { 1 });
}
