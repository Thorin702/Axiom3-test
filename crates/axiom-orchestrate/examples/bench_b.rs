//! Comparison scenario B (hard): multi-file codebase. A small package has TWO bugs in
//! DIFFERENT files (discount computed wrong in pricing.py; quantity dropped in cart.py). The
//! failing test only shows the wrong cart total — the agent must LOCATE both root causes across
//! files (glob/search → read), fix each with a precise `edit`, and run the test to green.
//!   DEEPSEEK_API_KEY=... cargo run -p axiom-orchestrate --example bench_b
use axiom_orchestrate::coding::sandbox::{Sandbox, SandboxMode};
use axiom_orchestrate::coding::{run_coding_agent, CodingBudget, ProviderModel};
use axiom_orchestrate::Pause;
use axiom_runtime::supervisor::Cancel;

#[tokio::main]
async fn main() {
    let dir = std::env::temp_dir().join(format!("axiom-bench-b-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("shop")).unwrap();
    std::fs::create_dir_all(dir.join("tests")).unwrap();
    std::fs::write(dir.join("shop/__init__.py"), "").unwrap();
    // BUG 1: discount is applied as price*pct instead of price*(1 - pct/100).
    std::fs::write(
        dir.join("shop/pricing.py"),
        "def apply_discount(price, pct):\n    \"\"\"Return price after a pct% discount.\"\"\"\n    return price * pct\n",
    )
    .unwrap();
    // BUG 2: line total forgets to multiply by quantity.
    std::fs::write(
        dir.join("shop/cart.py"),
        "from shop.pricing import apply_discount\n\n\n\
         def cart_total(items, discount_pct):\n    \
         subtotal = 0\n    for it in items:\n        subtotal += it[\"price\"]  # per-line total\n    \
         return apply_discount(subtotal, discount_pct)\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("tests/test_shop.py"),
        "import sys, os\nsys.path.insert(0, os.path.dirname(os.path.dirname(__file__)))\n\
         from shop.cart import cart_total\n\
         items = [{\"name\": \"a\", \"price\": 100, \"qty\": 2}, {\"name\": \"b\", \"price\": 50, \"qty\": 1}]\n\
         got = cart_total(items, 10)\n\
         assert got == 225.0, f\"expected 225.0 got {got}\"\n\
         print(\"ALL TESTS PASSED\")\n",
    )
    .unwrap();

    let model_id = std::env::var("AXIOM_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".into());
    let sb = Sandbox::with_mode(&dir, SandboxMode::ReadWrite).unwrap();
    let model = ProviderModel { provider: "deepseek".into(), model: model_id.clone(), timeout_s: 120 };
    let budget = CodingBudget { max_steps: 22, per_call_timeout_s: 120, cmd_timeout_s: 40, cell_timeout_s: 600 };

    println!("=== 场景B 多文件代码库 (model={model_id}) on {} ===", dir.display());
    let (mut plan, mut edit, mut glob, mut search, mut edits) = (false, false, false, false, 0);
    let res = run_coding_agent(
        "This project's test fails: `python3 tests/test_shop.py` should print ALL TESTS PASSED but \
         the cart total is wrong. There are TWO bugs in DIFFERENT files under shop/. Make a plan, \
         locate both root causes (use glob/search to find the code, then read it), fix each with a \
         precise edit, and run the test until it passes. Do not rewrite whole files.",
        &sb, &model, &Cancel::new(), &Pause::new(), &budget, None,
        |s| {
            match s.tool.as_str() { "plan"=>plan=true, "edit"=>{edit=true; edits+=1;}, "glob"=>glob=true, "search"=>search=true, _=>{} }
            let o: String = s.observation.lines().next().unwrap_or("").chars().take(70).collect();
            println!("  [{}] {} {} → {}", s.n, s.tool, s.detail, o);
        },
    )
    .await;

    println!("\n=== status {} ({} steps) · plan:{} glob:{} search:{} edit:{}(×{}) ===",
        res.status, res.steps, plan, glob, search, edit, edits);
    println!("SUMMARY:\n{}\n", res.summary.chars().take(600).collect::<String>());

    // Independent verify with a fresh interpreter.
    let out = std::process::Command::new("python3").arg("tests/test_shop.py").current_dir(&dir).output().unwrap();
    let so = String::from_utf8_lossy(&out.stdout);
    let ok = so.contains("ALL TESTS PASSED");
    println!("=== 独立复验 (python3 tests/test_shop.py): {} ===", if ok { "PASS ✓" } else { "FAIL ✗" });
    if !ok {
        println!("pricing.py:\n{}", std::fs::read_to_string(dir.join("shop/pricing.py")).unwrap_or_default());
        println!("cart.py:\n{}", std::fs::read_to_string(dir.join("shop/cart.py")).unwrap_or_default());
        println!("stderr: {}", String::from_utf8_lossy(&out.stderr));
    }
    let _ = std::fs::remove_dir_all(&dir);
    std::process::exit(if ok { 0 } else { 1 });
}
