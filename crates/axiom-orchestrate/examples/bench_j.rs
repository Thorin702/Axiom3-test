//! Verifies P2 background jobs: the agent launches a DETACHED job that runs while it does other
//! work, polls job_status until DONE, and reads the result.
//!   DEEPSEEK_API_KEY=... cargo run -p axiom-orchestrate --example bench_j
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
    let dir = std::env::temp_dir().join(format!("axiom-bench-j-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let sb = Sandbox::with_mode(&dir, SandboxMode::ReadWrite).unwrap();
    let kernel = Arc::new(PyKernel::start(&py, &dir).expect("kernel"));
    let model_id = std::env::var("AXIOM_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".into());
    let model = ProviderModel { provider: "deepseek".into(), model: model_id.clone(), timeout_s: 120 };
    let budget = CodingBudget { max_steps: 16, per_call_timeout_s: 120, cmd_timeout_s: 60 };

    println!("=== P2 验证:后台作业 job_start/job_status (model={model_id}) ===");
    let (mut started, mut saw_done) = (false, false);
    let res = run_coding_agent(
        "Start a BACKGROUND job with the job_start tool whose code sleeps 2 seconds then writes the \
         number 1234 into a file called answer.txt (code: `import time; time.sleep(2); \
         open('answer.txt','w').write('1234')`). Then repeatedly call job_status with that job's id \
         until it reports DONE. Once DONE, read answer.txt and report the number. Do not use the \
         python tool to sleep yourself.",
        &sb, &model, &Cancel::new(), &Pause::new(), &budget, Some(kernel),
        |s| {
            if s.tool == "job_start" { started = true; }
            if s.tool == "job_status" && s.observation.contains("DONE") { saw_done = true; }
            let o: String = s.observation.lines().next().unwrap_or("").chars().take(70).collect();
            println!("  [{}] {} {} → {}", s.n, s.tool, s.detail, o);
        },
    )
    .await;

    let sy = &res.summary;
    let has_answer = sy.contains("1234");
    let file_written = res.artifacts.iter().any(|a| a.path.ends_with("answer.txt"))
        || std::fs::read_to_string(dir.join("answer.txt")).map(|s| s.contains("1234")).unwrap_or(false);
    println!("\n=== status {} ({} steps) · job_start:{} · 见到 DONE:{} ===", res.status, res.steps, started, saw_done);
    println!("=== 报告含 1234:{} · answer.txt 已写:{} ===", has_answer, file_written);
    println!("\nSUMMARY(截断):\n{}", sy.chars().take(700).collect::<String>());
    let ok = started && saw_done && file_written;
    println!("\n=== 判定: 后台作业启动→轮询→完成 = {} ===", if ok { "PASS ✓" } else { "未达标" });
    let _ = std::fs::remove_dir_all(&dir);
    std::process::exit(if ok { 0 } else { 1 });
}
