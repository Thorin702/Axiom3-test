//! Live smoke test of the read-write coding agent against a real model, in an ISOLATED
//! temp dir (never a real repo). Seeds a failing check, asks the agent to implement +
//! run it to green, then independently verifies.
//!
//!   DEEPSEEK_API_KEY=... cargo run -p axiom-orchestrate --example coding_live
use axiom_orchestrate::coding::sandbox::{Sandbox, SandboxMode};
use axiom_orchestrate::coding::{run_coding_agent, CodingBudget, ProviderModel};
use axiom_orchestrate::Pause;
use axiom_runtime::supervisor::Cancel;

#[tokio::main]
async fn main() {
    let dir = std::env::temp_dir().join(format!("axiom-coding-live-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("solution.py"), "def fib(n):\n    pass  # TODO: implement\n").unwrap();
    std::fs::write(
        dir.join("check.py"),
        "from solution import fib\n\
         assert fib(0) == 0, fib(0)\n\
         assert fib(1) == 1, fib(1)\n\
         assert fib(10) == 55, fib(10)\n\
         assert fib(20) == 6765, fib(20)\n\
         print('ALL TESTS PASSED')\n",
    )
    .unwrap();

    let model_id = std::env::var("AXIOM_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".into());
    let sb = Sandbox::with_mode(&dir, SandboxMode::ReadWrite).unwrap();
    let model = ProviderModel { provider: "deepseek".into(), model: model_id.clone(), timeout_s: 90 };
    let budget = CodingBudget { max_steps: 14, per_call_timeout_s: 90, cmd_timeout_s: 30 };
    let cancel = Cancel::new();
    let pause = Pause::new();

    println!("=== coding agent (model={model_id}) on {} ===", dir.display());
    let res = run_coding_agent(
        "Implement fib(n) in solution.py as the 0-indexed Fibonacci sequence (fib(0)=0, fib(1)=1). \
         Then run `python3 check.py` and ensure it prints ALL TESTS PASSED. Iterate until it passes.",
        &sb,
        &model,
        &cancel,
        &pause,
        &budget,
        None,
        |step| {
            let obs1: String = step.observation.lines().next().unwrap_or("").chars().take(90).collect();
            println!("  [step {}] {} {} → {}", step.n, step.tool, step.detail, obs1);
        },
    )
    .await;

    println!("\n=== status: {} ({} steps) ===", res.status, res.steps);
    println!("SUMMARY:\n{}\n", res.summary.chars().take(800).collect::<String>());

    let out = std::process::Command::new("python3")
        .arg("check.py")
        .current_dir(&dir)
        .output()
        .expect("python3 not found");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let ok = stdout.contains("ALL TESTS PASSED");
    println!("=== INDEPENDENT VERIFY (python3 check.py): {} ===", if ok { "PASS ✓" } else { "FAIL ✗" });
    if !ok {
        println!("written solution.py:\n{}", std::fs::read_to_string(dir.join("solution.py")).unwrap_or_default());
    }
    let _ = std::fs::remove_dir_all(&dir);
    std::process::exit(if ok { 0 } else { 1 });
}
