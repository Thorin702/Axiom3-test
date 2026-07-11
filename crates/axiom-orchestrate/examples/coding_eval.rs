//! A small offline coding eval (mini-HumanEval style): the read-write agent solves N
//! self-contained Python tasks in isolated scratch dirs; each is INDEPENDENTLY scored by
//! running its check. Prints pass@1. This is the first number to watch climb.
//!
//!   DEEPSEEK_API_KEY=... cargo run -p axiom-orchestrate --example coding_eval
use axiom_orchestrate::coding::sandbox::{Sandbox, SandboxMode};
use axiom_orchestrate::coding::{run_coding_agent, CodingBudget, ProviderModel};
use axiom_orchestrate::Pause;
use axiom_runtime::supervisor::Cancel;

struct Task {
    name: &'static str,
    stub: &'static str,
    check: &'static str,
    goal: &'static str,
}

const TASKS: &[Task] = &[
    Task {
        name: "fibonacci",
        stub: "def fib(n):\n    pass\n",
        check: "from solution import fib\nassert fib(0)==0\nassert fib(1)==1\nassert fib(10)==55\nassert fib(20)==6765\nprint('PASS')\n",
        goal: "Implement fib(n) in solution.py as 0-indexed Fibonacci (fib(0)=0, fib(1)=1).",
    },
    Task {
        name: "palindrome",
        stub: "def is_palindrome(s):\n    pass\n",
        check: "from solution import is_palindrome\nassert is_palindrome('racecar')\nassert not is_palindrome('hello')\nassert is_palindrome('')\nprint('PASS')\n",
        goal: "Implement is_palindrome(s) returning True iff s reads the same forwards and backwards.",
    },
    Task {
        name: "two_sum",
        stub: "def two_sum(nums, target):\n    pass\n",
        check: "from solution import two_sum\nassert sorted(two_sum([2,7,11,15],9))==[0,1]\nassert sorted(two_sum([3,2,4],6))==[1,2]\nprint('PASS')\n",
        goal: "Implement two_sum(nums, target) returning the indices of the two numbers adding to target.",
    },
    Task {
        name: "fizzbuzz",
        stub: "def fizzbuzz(n):\n    pass\n",
        check: "from solution import fizzbuzz\nassert fizzbuzz(3)=='Fizz'\nassert fizzbuzz(5)=='Buzz'\nassert fizzbuzz(15)=='FizzBuzz'\nassert fizzbuzz(7)=='7'\nprint('PASS')\n",
        goal: "Implement fizzbuzz(n): 'Fizz' if divisible by 3, 'Buzz' if by 5, 'FizzBuzz' if both, else str(n).",
    },
    Task {
        name: "merge_sort",
        stub: "def merge_sort(a):\n    pass\n",
        check: "from solution import merge_sort\nimport random\nfor _ in range(30):\n    x=[random.randint(0,99) for _ in range(random.randint(0,40))]\n    assert merge_sort(list(x))==sorted(x)\nprint('PASS')\n",
        goal: "Implement merge_sort(a) returning a NEW ascending-sorted list (do not mutate the input).",
    },
    Task {
        name: "roman",
        stub: "def to_roman(n):\n    pass\n",
        check: "from solution import to_roman\nassert to_roman(4)=='IV'\nassert to_roman(9)=='IX'\nassert to_roman(58)=='LVIII'\nassert to_roman(1994)=='MCMXCIV'\nprint('PASS')\n",
        goal: "Implement to_roman(n) converting an integer 1..3999 to a Roman numeral string.",
    },
];

#[tokio::main]
async fn main() {
    let model_id = std::env::var("AXIOM_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".into());
    let model = ProviderModel { provider: "deepseek".into(), model: model_id.clone(), timeout_s: 90 };
    let budget = CodingBudget { max_steps: 14, per_call_timeout_s: 90, cmd_timeout_s: 30 };

    println!("=== coding eval · model={model_id} · {} tasks ===\n", TASKS.len());
    let mut passed = 0;
    for t in TASKS {
        let dir = std::env::temp_dir().join(format!("axiom-eval-{}-{}", t.name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("solution.py"), t.stub).unwrap();
        std::fs::write(dir.join("check.py"), t.check).unwrap();

        let sb = Sandbox::with_mode(&dir, SandboxMode::ReadWrite).unwrap();
        let goal = format!("{} Then run `python3 check.py` and iterate until it prints PASS.", t.goal);
        let res = run_coding_agent(&goal, &sb, &model, &Cancel::new(), &Pause::new(), &budget, None, |_| {}).await;

        let out = std::process::Command::new("python3").arg("check.py").current_dir(&dir).output();
        let ok = out
            .map(|o| String::from_utf8_lossy(&o.stdout).contains("PASS"))
            .unwrap_or(false);
        if ok {
            passed += 1;
        }
        println!("  {:<12} {}  ({} steps, {})", t.name, if ok { "PASS ✓" } else { "FAIL ✗" }, res.steps, res.status);
        let _ = std::fs::remove_dir_all(&dir);
    }
    let pct = (passed as f64) / (TASKS.len() as f64) * 100.0;
    println!("\n=== pass@1: {passed}/{} ({pct:.0}%) ===", TASKS.len());
}
