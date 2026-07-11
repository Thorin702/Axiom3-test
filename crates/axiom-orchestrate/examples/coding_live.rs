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
    // Multi-step: fib has a one-line BUG (start 1,1 not 0,1) AND factorial is unimplemented.
    // The agent should PLAN, then precisely edit fib and implement factorial.
    std::fs::write(
        dir.join("solution.py"),
        "def fib(n):\n    a, b = 1, 1  # start values\n    for _ in range(n):\n        a, b = b, a + b\n    return a\n\n\ndef factorial(n):\n    pass  # TODO\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("check.py"),
        "from solution import fib, factorial\n\
         assert fib(0) == 0 and fib(10) == 55, 'fib'\n\
         assert factorial(0) == 1 and factorial(5) == 120, 'factorial'\n\
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
    let mut used_edit = false;
    let mut used_plan = false;
    let res = run_coding_agent(
        "solution.py has TWO issues: fib(n) has a bug, and factorial(n) is unimplemented. Make a \
         plan, then fix both (edit fib precisely, implement factorial), and run `python3 check.py` \
         until it prints ALL TESTS PASSED.",
        &sb,
        &model,
        &cancel,
        &pause,
        &budget,
        None,
        |step| {
            if step.tool == "edit" { used_edit = true; }
            if step.tool == "plan" { used_plan = true; }
            let obs1: String = step.observation.lines().next().unwrap_or("").chars().take(90).collect();
            println!("  [step {}] {} {} → {}", step.n, step.tool, step.detail, obs1);
        },
    )
    .await;
    println!("=== used `plan`: {} · used `edit`: {} ===", if used_plan { "YES ✓" } else { "no" }, if used_edit { "YES ✓" } else { "no" });

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
