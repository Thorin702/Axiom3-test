//! Verifies #2 skills registry: given a domain task, the agent searches + loads the right skill
//! card and follows it (packages, pipeline, native block, rigor).
//!   DEEPSEEK_API_KEY=... cargo run -p axiom-orchestrate --example bench_l
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
    let dir = std::env::temp_dir().join(format!("axiom-bench-l-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // 12 genes x (4 ctrl + 4 treat); g1..g3 up in treatment.
    let mut csv = String::from("gene,c1,c2,c3,c4,t1,t2,t3,t4\n");
    for g in 1..=12 {
        let delta = if g <= 3 { 10.0 } else { 0.0 };
        let c: Vec<String> = (0..4).map(|k| format!("{:.1}", 8.0 + (k as f64 - 1.5) * 0.5)).collect();
        let t: Vec<String> = (0..4).map(|k| format!("{:.1}", 8.0 + delta + (k as f64 - 1.5) * 0.5)).collect();
        csv.push_str(&format!("g{g},{},{}\n", c.join(","), t.join(",")));
    }
    std::fs::write(dir.join("counts.csv"), csv).unwrap();

    let sb = Sandbox::with_mode(&dir, SandboxMode::ReadWrite).unwrap();
    let kernel = Arc::new(PyKernel::start(&py, &dir).expect("kernel"));
    let model_id = std::env::var("AXIOM_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".into());
    let model = ProviderModel { provider: "deepseek".into(), model: model_id.clone(), timeout_s: 150 };
    let budget = CodingBudget { max_steps: 18, per_call_timeout_s: 150, cmd_timeout_s: 120, cell_timeout_s: 600 };

    println!("=== #2 验证:skills 注册表 (model={model_id}) ===");
    let (mut searched, mut loaded, mut loaded_name) = (false, false, String::new());
    let res = run_coding_agent(
        "counts.csv is an expression matrix (columns c1..c4 control, t1..t4 treatment). This is a \
         differential-expression task. BEFORE writing any analysis code, consult the skill library: \
         use search_skills to find the right playbook, then load_skill to read it, and FOLLOW it. \
         Then run the analysis and present the result. Keep prose short.",
        &sb, &model, &Cancel::new(), &Pause::new(), &budget, Some(kernel),
        |s| {
            if s.tool == "search_skills" { searched = true; }
            if s.tool == "load_skill" { loaded = true; loaded_name = s.detail.clone(); }
            let o: String = s.observation.lines().next().unwrap_or("").chars().take(66).collect();
            println!("  [{}] {} {} → {}", s.n, s.tool, s.detail, o);
        },
    )
    .await;

    let low = res.summary.to_lowercase();
    let followed_rigor = ["fdr", "benjamini", "hochberg", "padj", "adjusted", "multipletests"].iter().any(|t| low.contains(*t));
    let has_block = res.summary.contains("```volcano") || res.summary.contains("```heatmap");
    println!("\n=== status {} ({} steps) ===", res.status, res.steps);
    println!("=== search_skills:{} · load_skill:{}({}) · 照卡做FDR:{} · 原生块:{} ===",
        searched, loaded, loaded_name, followed_rigor, has_block);
    println!("\nSUMMARY(截断):\n{}", res.summary.chars().take(900).collect::<String>());
    let ok = searched && loaded && (followed_rigor || has_block);
    println!("\n=== 判定: 查+载技能卡并遵循 = {} ===", if ok { "PASS ✓" } else { "未达标" });
    let _ = std::fs::remove_dir_all(&dir);
    std::process::exit(if ok { 0 } else { 1 });
}
