//! SELF-TEST of the report FINALIZE protocol against the REAL model (SCNet GLM-5.2). Drives the
//! research report path end-to-end and inspects the delivered report for EVERY leak class the recent
//! audit fixed — the ===AXIOM3_FINAL=== marker, a raw {"final":…} envelope, un-unescaped `\n`, and
//! leaked THINKING/tool-call/code scaffolding. Prints a PASS/FAIL verdict + writes the full report.
//!
//!   SCNET_API_KEY=... cargo run -p axiom-orchestrate --example finalize_selftest -- "<topic>" /tmp/out.json
use axiom_orchestrate::coding::sandbox::{Sandbox, SandboxMode};
use axiom_orchestrate::coding::{run_coding_agent_steered, CodingBudget, ProviderModel};
use axiom_orchestrate::kernel::PyKernel;
use axiom_orchestrate::Pause;
use axiom_runtime::supervisor::Cancel;
use std::sync::Arc;

// The SAME research steer the app appends in research mode: demand real litsearch, competing
// hypotheses, [Sn] citations, and a substantial cited report as the deliverable.
const STEER: &str = "\
你在研究模式:执行真实工具、检索真实文献。要求:①先用 litsearch 广度检索若干不同角度(不是同一角度的变体);\
②给出机制/关键证据/争议点;③关键结论与数字都要有真实 [Sn] 来源支撑,没来源的显式标注为估计;④最后产出一份\
完整、分节、带引用的 Markdown 研究简报作为最终答案,并给出可验证的下一步实验。别在第一个看似合理的答案就停。";

fn strip_fences(s: &str) -> String {
    let mut out = String::new();
    let mut in_fence = false;
    for line in s.lines() {
        if line.trim_start().starts_with("```") { in_fence = !in_fence; continue; }
        if !in_fence { out.push_str(line); out.push('\n'); }
    }
    out
}

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let topic = args.next().unwrap_or_else(|| "PCSK9 抑制剂降低 LDL 的机制与心血管获益证据".into());
    let out_path = args.next().unwrap_or_else(|| "/tmp/finalize_selftest.json".into());

    let venv = std::path::PathBuf::from(std::env::var_os("HOME").unwrap())
        .join("Library/Application Support/com.axiom3.desktop/notebook/.venv/bin/python");
    let py = if venv.exists() { venv.to_string_lossy().into_owned() } else { "python3".into() };
    let model_id = std::env::var("AXIOM_MODEL").unwrap_or_else(|_| "GLM-5.2".into());
    let provider = std::env::var("AXIOM_PROVIDER").unwrap_or_else(|_| "scnet".into());
    let model = ProviderModel { provider: provider.clone(), model: model_id.clone(), timeout_s: 220 };

    let dir = std::env::temp_dir().join(format!("axiom-finalize-selftest-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let sb = Sandbox::with_mode(&dir, SandboxMode::ReadWrite).unwrap();
    let kernel = Arc::new(PyKernel::start(&py, &dir).expect("start kernel"));
    let budget = CodingBudget { max_steps: 22, per_call_timeout_s: 220, cmd_timeout_s: 60, cell_timeout_s: 300 };
    let pause = Pause::new();
    pause.encourage_delegate(); // research mode: activates the self-critique + figure + report gates

    eprintln!("=== finalize self-test · {provider}/{model_id} · py={py}\n=== topic: {topic}\n");
    let t0 = std::time::Instant::now();
    let mut litsearch = 0u32;
    let res = run_coding_agent_steered(
        &topic, STEER, &sb, &model, &Cancel::new(), &pause, &budget, Some(kernel),
        |s| {
            if s.tool == "litsearch" || s.tool == "multi_search" { litsearch += 1; }
            let o: String = s.observation.lines().next().unwrap_or("").chars().take(90).collect();
            eprintln!("  [{}] {} {} → {}", s.n, s.tool, s.detail.chars().take(50).collect::<String>(), o);
        },
        |_, _| {},
    ).await;
    let dur = t0.elapsed().as_secs();
    let sum = &res.summary;

    // ── LEAK CHECKS: every class the audit fixed ──────────────────────────────────────────────────
    let prose = strip_fences(sum);
    let pt = prose.trim();
    let marker_leaked = sum.contains("AXIOM3_FINAL");
    let head: String = sum.trim_start().chars().take(30).collect();
    let envelope_leaked = head.contains("{\"final\"") || head.contains("{ \"final\"");
    // A raw un-unescaped envelope shows many LITERAL backslash-n in the REPORT BODY. Count only OUTSIDE
    // fenced code blocks — a `\n` inside a ```python/```chart fence is legitimate code content, not a leak.
    let literal_bs_n = prose.matches("\\n").count();
    let literal_bs_n_total = sum.matches("\\n").count();
    let thinking_leaked = pt.starts_with("THINKING:") || pt.starts_with("THINKING：")
        || pt.contains("<tool_call>") || pt.contains("<think>")
        || pt.contains("\"tool\":") || pt.contains("tool\":\"")
        || pt.contains("\"queries\":") || pt.contains("\"todos\":") || pt.contains("\"code\":");
    let prose_chars = pt.chars().filter(|c| !c.is_whitespace()).count();
    let has_heading = sum.lines().any(|l| l.trim_start().starts_with('#'));
    // DANGLING CITATION: any [Sn] whose n exceeds the sources actually retrieved (dead provenance link).
    // After the new gate + strip backstop this must be false.
    let max_cited: usize = {
        let b = sum.as_bytes();
        let mut mx = 0usize;
        let mut i = 0;
        while i + 2 < b.len() {
            if b[i] == b'[' && b[i + 1] == b'S' && b[i + 2].is_ascii_digit() {
                let mut j = i + 2;
                let mut v = 0usize;
                while j < b.len() && b[j].is_ascii_digit() { v = v * 10 + (b[j] - b'0') as usize; j += 1; }
                if j < b.len() && b[j] == b']' { mx = mx.max(v); }
                i = j;
            } else { i += 1; }
        }
        mx
    };
    let dangling = max_cited > res.sources.len();
    let no_report = res.status == "no_report"
        || sum.contains("报告未自动生成");

    let clean = !marker_leaked && !envelope_leaked && literal_bs_n < 5 && !thinking_leaked && !dangling;
    let is_real_report = clean && prose_chars >= 200 && has_heading;
    let verdict = if is_real_report { "PASS" } else if no_report { "SALVAGED(no_report — clean banner)" } else { "FAIL" };

    let report = serde_json::json!({
        "topic": topic, "provider": provider, "model": model_id,
        "status": res.status, "steps": res.steps, "dur_s": dur, "litsearch": litsearch,
        "summary_len": sum.chars().count(), "prose_chars": prose_chars, "figures": res.figures.len(), "sources": res.sources.len(),
        "checks": {
            "marker_leaked": marker_leaked, "envelope_leaked": envelope_leaked,
            "literal_backslash_n": literal_bs_n, "literal_backslash_n_total": literal_bs_n_total, "thinking_leaked": thinking_leaked,
            "max_cited": max_cited, "dangling_citation": dangling,
            "has_heading": has_heading, "clean": clean, "is_real_report": is_real_report,
        },
        "verdict": verdict,
        "head": sum.chars().take(600).collect::<String>(),
    });
    std::fs::write(&out_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    // Dump the FULL report next to the JSON so a leak's exact location is inspectable.
    let md_path = format!("{}.md", out_path.trim_end_matches(".json"));
    std::fs::write(&md_path, sum.as_bytes()).unwrap();

    println!("\n================ FINALIZE SELF-TEST VERDICT ================");
    println!("topic:   {topic}");
    println!("status={} steps={} dur={}s litsearch={} figs={} srcs={} len={} prose={}",
        res.status, res.steps, dur, litsearch, res.figures.len(), res.sources.len(), sum.chars().count(), prose_chars);
    println!("checks:  marker_leaked={marker_leaked} envelope_leaked={envelope_leaked} literal_\\n={literal_bs_n} thinking_leaked={thinking_leaked} max_cited=[S{max_cited}]/{} dangling={dangling} heading={has_heading}", res.sources.len());
    println!(">>> VERDICT: {verdict}");
    println!("--- report head ---\n{}", sum.chars().take(500).collect::<String>());
    println!("=== full JSON → {out_path}");
    let _ = std::fs::remove_dir_all(&dir);
}
