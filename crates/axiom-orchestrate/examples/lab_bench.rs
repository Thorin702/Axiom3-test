//! LAB-Bench–style self-eval harness. Runs OUR analysis agent on a set of multiple-choice
//! questions (each answerable from the databases / kernel), extracts its chosen letter, and
//! reports accuracy — so we get an OBJECTIVE, repeatable quality number comparable to Biomni's
//! LAB-Bench DbQA 74.4% / SeqQA 81.9%.
//!
//! Runs LIVE (needs a model key + network). Each question is a full agent run (slow) — use a subset.
//!
//!   ZHIPU_API_KEY=... cargo run -p axiom-orchestrate --example lab_bench            # sample set
//!   ZHIPU_API_KEY=... cargo run -p axiom-orchestrate --example lab_bench -- questions.jsonl  [N]
//!
//! JSONL line format: {"id":"q1","question":"...","choices":["A text","B text",...],"answer":"A text"}
//!   `answer` is the FULL TEXT of the correct choice (or a 1-letter A/B/C… — both accepted).
//! To compare with Biomni: drop in the real LAB-Bench DbQA/SeqQA questions as JSONL and run.
use axiom_orchestrate::coding::sandbox::{Sandbox, SandboxMode};
use axiom_orchestrate::coding::{run_coding_agent, CodingBudget, ProviderModel};
use axiom_orchestrate::kernel::PyKernel;
use axiom_orchestrate::Pause;
use axiom_runtime::supervisor::Cancel;
use serde_json::Value;
use std::sync::Arc;

const SAMPLE: &str = include_str!("sample_dbqa.jsonl");

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let raw = match args.get(1) {
        Some(p) if !p.chars().all(|c| c.is_ascii_digit()) => std::fs::read_to_string(p).expect("read questions file"),
        _ => SAMPLE.to_string(),
    };
    let limit: usize = args.iter().find_map(|a| a.parse::<usize>().ok()).unwrap_or(usize::MAX);
    let questions: Vec<Value> = raw.lines().filter(|l| l.trim().starts_with('{')).filter_map(|l| serde_json::from_str(l).ok()).take(limit).collect();
    println!("=== LAB-Bench self-eval · {} questions ===", questions.len());

    // Model: prefer Zhipu GLM (our primary), else SCNet, else DeepSeek.
    let (provider, model_id) = if std::env::var("ZHIPU_API_KEY").is_ok() {
        ("zhipu".to_string(), "glm-5.2".to_string())
    } else if std::env::var("SCNET_API_KEY").is_ok() {
        ("scnet".to_string(), "GLM-5.2".to_string())
    } else {
        ("deepseek".to_string(), "deepseek-v4-flash".to_string())
    };
    println!("model: {provider}/{model_id}\n");

    let venv = std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join("Library/Application Support/com.axiom3.desktop/notebook/.venv/bin/python")).unwrap_or_default();
    let py = if venv.exists() { venv.to_string_lossy().into_owned() } else { "python3".into() };

    let mut correct = 0usize;
    let mut answered = 0usize;
    for (i, q) in questions.iter().enumerate() {
        let question = q.get("question").and_then(|x| x.as_str()).unwrap_or("");
        let choices: Vec<&str> = q.get("choices").and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|x| x.as_str()).collect()).unwrap_or_default();
        let answer = q.get("answer").and_then(|x| x.as_str()).unwrap_or("");
        if question.is_empty() || choices.is_empty() {
            continue;
        }
        // Gold letter: `answer` is either the option TEXT or a letter.
        let gold_letter = if answer.len() == 1 {
            answer.to_uppercase().chars().next().unwrap()
        } else {
            choices.iter().position(|c| c.trim() == answer.trim()).map(|p| (b'A' + p as u8) as char).unwrap_or('?')
        };
        let lettered: String = choices.iter().enumerate().map(|(k, c)| format!("{}) {}", (b'A' + k as u8) as char, c)).collect::<Vec<_>>().join("\n");
        let prompt = format!(
            "回答下面这道单选题。可以用内核里的 scidb 数据库工具(import scidb)与 python 去查证,不要凭记忆猜。\n\n\
             【问题】{question}\n【选项】\n{lettered}\n\n\
             先用 scidb/python 查证(如变异频率:import scidb; scidb.gnomad(...) 拿到 af,再把它换算成百分比与选项区间比较),\
             再作答。**你的 FINAL 回答里必须单独有一行写 `ANSWER: X`(X 是 A/B/C/D 之一)**,例如最后写 `ANSWER: C`。这一行是必须的。",
        );
        let dir = std::env::temp_dir().join(format!("axiom-labbench-{}-{}", std::process::id(), i));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sb = Sandbox::with_mode(&dir, SandboxMode::ReadWrite).unwrap();
        let kernel = PyKernel::start(&py, &dir).ok().map(Arc::new);
        let model = ProviderModel { provider: provider.clone(), model: model_id.clone(), timeout_s: 120 };
        let budget = CodingBudget { max_steps: 14, per_call_timeout_s: 120, cmd_timeout_s: 60, cell_timeout_s: 600 };
        let res = run_coding_agent(&prompt, &sb, &model, &Cancel::new(), &Pause::new(), &budget, kernel, |_| {}, |_, _| {}).await;
        let picked = parse_answer(&res.summary, &choices);
        let ok = picked == Some(gold_letter);
        if picked.is_some() {
            answered += 1;
        }
        if ok {
            correct += 1;
        }
        println!("[{:>2}/{}] {} gold={} picked={} · {}", i + 1, questions.len(), if ok { "✓" } else { "✗" }, gold_letter, picked.map(|c| c.to_string()).unwrap_or_else(|| "—".into()), question.chars().take(60).collect::<String>());
        let _ = std::fs::remove_dir_all(&dir);
    }
    let n = questions.len().max(1);
    println!("\n=== ACCURACY: {}/{} = {:.1}%  (answered {}/{}) ===", correct, questions.len(), 100.0 * correct as f64 / n as f64, answered, questions.len());
    println!("(Biomni 参照:LAB-Bench DbQA 74.4% · SeqQA 81.9%;把真实 LAB-Bench JSONL 传进来即可直接对比)");
}

/// Extract the chosen option letter from a free-form final answer — robust to how a model phrases it
/// (weak models often don't obey an exact format). Tries, in order: an explicit answer keyword +
/// letter; a near-the-end standalone letter ("D" / "D)" / "D."); finally, the correct-option TEXT
/// appearing verbatim. Returns None only if truly nothing resembles an answer.
fn parse_answer(s: &str, choices: &[&str]) -> Option<char> {
    let up = s.to_uppercase();
    let n = choices.len().min(5);
    let is_opt = |c: char| c.is_ascii_uppercase() && (c as usize - 'A' as usize) < n.max(1);
    // 1) explicit answer keyword followed by a letter (scan from the END for the LAST one).
    for kw in ["ANSWER:", "ANSWER IS", "ANSWER", "答案是", "答案:", "答案：", "正确答案", "THE ANSWER IS", "我选", "选择", "应选"] {
        if let Some(idx) = up.rfind(kw) {
            if let Some(c) = up[idx + kw.len()..].chars().take(10).map(|c| c.to_ascii_uppercase()).find(|&c| is_opt(c)) {
                return Some(c);
            }
        }
    }
    // 2) a standalone option letter on one of the last lines ("D", "**D)**", "- D."):
    for line in s.lines().rev().take(8) {
        let t = line.trim().trim_matches(|c: char| c == '*' || c == '-' || c == ' ' || c == '·' || c == '>');
        let mut it = t.chars();
        if let Some(c0) = it.next() {
            let c = c0.to_ascii_uppercase();
            if is_opt(c) {
                match it.next() {
                    None => return Some(c),
                    Some(nx) if matches!(nx, ')' | '.' | '、' | ':' | '：' | ' ' | '\t') => return Some(c),
                    _ => {}
                }
            }
        }
    }
    // 3) fallback: the correct-option TEXT appears verbatim in the last stretch of the answer.
    let tail: String = s.chars().rev().take(400).collect::<String>().chars().rev().collect();
    for (k, opt) in choices.iter().enumerate() {
        if opt.trim().len() > 4 && tail.contains(opt.trim()) {
            return Some((b'A' + k as u8) as char);
        }
    }
    None
}
