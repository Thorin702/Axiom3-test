//! Side-by-side benchmark harness: run the SAME casual research questions through OUR platform
//! (the app's real pipeline — the workbench REFRAME_SYSTEM → run_coding_agent on GLM → figures +
//! synthesis) so the output can be compared against Claude Science run on the identical prompts.
//!
//! Each question writes /tmp/science_compare/q<N>_ours.json (casual, directive, every step with its
//! code/observation/reasoning, the final synthesis, and the base64 figures) so we can score
//! similarity on: deliverable structure, thinking depth, code, figure richness, model-reply style.
//!
//!   ZHIPU_API_KEY=... cargo run -p axiom-orchestrate --example science_compare        # all
//!   ZHIPU_API_KEY=... AXIOM_Q=0 cargo run -p axiom-orchestrate --example science_compare  # one
use axiom_orchestrate::coding::sandbox::{Sandbox, SandboxMode};
use axiom_orchestrate::coding::{run_coding_agent, CodingBudget, CodingModel, ProviderModel};
use axiom_orchestrate::kernel::PyKernel;
use axiom_orchestrate::Pause;
use axiom_runtime::supervisor::Cancel;
use std::sync::{Arc, Mutex};

// The app's real reframe prompt (kept in sync with workbench.rs REFRAME_SYSTEM), so the directive
// the agent runs here is exactly what the app produces for a casual ask.
const REFRAME_SYSTEM: &str = "\
You convert a user's plain-language request into a rigorous, self-contained RESEARCH DIRECTIVE that a \
capable research agent will execute. The user speaks casually; make their intent PRECISE and DEMANDING \
— never invent scope, domain, or specifics they did not ask for, and keep their actual question and any \
concrete details. Into the directive, bake the depth a good researcher brings:\n\
- DELIVERABLE: state exactly what the final answer must contain (e.g. a mechanism diagram + testable \
predictions; a ranked comparison; a decision with its justification).\n\
- BREADTH FIRST: have the agent survey SEVERAL DISTINCT angles / candidate mechanisms / competing \
explanations via litsearch — DIVERSE queries, not variations of one — before committing to any.\n\
- COMPETING HYPOTHESES: require at least 2-3 genuinely different hypotheses/approaches, each weighed \
against the evidence, then an explicit, justified choice of the strongest.\n\
- NOVELTY POSITIONING: require the agent to FIRST establish what is already known / standard in the \
field, then deliberately target the GAP and propose something genuinely under-explored — say plainly \
what makes it new versus the existing work. And be HONEST about novelty like a good reviewer: do NOT \
assert 'entirely unexplored' / 'completely novel' / 'the most novel' unless the searches genuinely \
turned up nothing; run a self-check search on the PROPOSED mechanism itself, and if it is already \
published, ACKNOWLEDGE that and narrow the truly-new part rather than overclaiming.\n\
- CROSS-DOMAIN TRANSFER: encourage borrowing a mechanism/approach from an ADJACENT field that has not \
yet been applied here (the strongest novel hypotheses often come from such a transfer), and to state the \
analogy explicitly.\n\
- SELF-CRITIQUE: require naming the chosen answer's weaknesses and what would falsify it (for an empirical \
claim, a verification plan that climbs from correlation → causal, incl. a gain-of-function/sufficiency \
test and the key negative controls).\n\
- GROUNDING + HONESTY: ground key claims in REAL literature via litsearch and cite; separate clearly \
what is supported from what is speculative.\n\
- USE THE STEP BUDGET FOR DEPTH: do not stop at the first plausible answer; keep going until the space \
is adequately covered.\n\
HARD RULES: The agent EXECUTES REAL TOOLS. REQUIRE it to actually run litsearch and cite the real papers \
it retrieves — NEVER tell it to 'imagine', 'assume', or generate a 'hypothetical' report, and never say \
'you don't need to run real queries'. Do NOT invent figure syntax, node/edge notation, or an output \
template — the agent already knows its own native figure blocks; just NAME the deliverable (e.g. 'a \
mechanism network figure + testable predictions'). Keep the directive CONCISE — one focused paragraph or \
a short list, no padding.\n\
Output ONLY the directive itself (no preamble, no 'Here is', no meta-notes), in the user's language, \
phrased as an instruction to the agent. If the request is ALREADY a detailed rigorous directive, return it unchanged.";

// Five casual asks (what a normal user types), spanning the capability surface we want to match
// Claude Science on: novel mechanism + diagram, experiment design + stats, modeling, cross-domain
// transfer, methodological rigor.
const QUESTIONS: &[&str] = &[
    "帮我想一个衰老细胞逃避免疫清除的新机制,画个机理图,说说怎么验证",
    "比较两种教学方法的效果,帮我设计一个实验并模拟分析给出结论",
    "用一个简单模型解释为什么抗生素耐药性会越来越普遍,画图说明",
    "免疫检查点抑制剂治肿瘤的思路,有没有可能迁移到阿尔茨海默这类神经退行病上",
    "单细胞RNA-seq里怎么判断两群细胞是不是真的不同,别被批次效应骗了",
];

#[derive(serde::Serialize, Clone)]
struct StepOut { n: u32, tool: String, detail: String, code: String, observation: String, reasoning: String, figures: usize }
#[derive(serde::Serialize)]
struct RunOut {
    idx: usize, casual: String, directive: String, status: String, summary: String,
    total_steps: usize, litsearch: usize, total_figures: usize,
    steps: Vec<StepOut>, figures_b64: Vec<String>,
}

async fn run_one(idx: usize, py: &str, model: &ProviderModel) {
    let casual = QUESTIONS[idx];
    println!("\n========== Q{idx}: {casual}\n--- reframing ---");
    let user = format!("User's request:\n{casual}\n\nRewrite it as the research directive:");
    let directive = model.complete(REFRAME_SYSTEM, &user, &Cancel::new()).await.unwrap_or_else(|_| casual.into());
    println!("DIRECTIVE:\n{directive}\n--- running (GLM) ---");

    let dir = std::env::temp_dir().join(format!("axiom-sci-q{idx}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let sb = Sandbox::with_mode(&dir, SandboxMode::ReadWrite).unwrap();
    let kernel = Arc::new(PyKernel::start(py, &dir).unwrap());
    // Match the app's analysis budget so the run is representative.
    let budget = CodingBudget { max_steps: 40, per_call_timeout_s: 120, cmd_timeout_s: 120, cell_timeout_s: 600 };
    let steps: Arc<Mutex<Vec<StepOut>>> = Arc::new(Mutex::new(Vec::new()));
    let cb = steps.clone();
    let res = run_coding_agent(
        &directive, &sb, model, &Cancel::new(), &Pause::new(), &budget, Some(kernel),
        move |s| {
            let o: String = s.observation.lines().next().unwrap_or("").chars().take(90).collect();
            println!("  [{}] {} {} ({} fig) → {}", s.n, s.tool, s.detail, s.figures.len(), o);
            cb.lock().unwrap().push(StepOut {
                n: s.n, tool: s.tool.clone(), detail: s.detail.chars().take(200).collect(),
                code: s.input.chars().take(4000).collect(),
                observation: s.observation.chars().take(2500).collect(),
                reasoning: s.reasoning.chars().take(2000).collect(),
                figures: s.figures.len(),
            });
        },
        |_, _| {},
    ).await;
    let collected = steps.lock().unwrap().clone();
    let lit = collected.iter().filter(|s| s.tool == "litsearch").count();
    let o = RunOut {
        idx, casual: casual.into(), directive, status: res.status.clone(), summary: res.summary.clone(),
        total_steps: res.steps as usize, litsearch: lit, total_figures: res.figures.len(),
        steps: collected, figures_b64: res.figures.clone(),
    };
    let out = std::path::PathBuf::from(format!("/tmp/science_compare/q{idx}_ours.json"));
    std::fs::write(&out, serde_json::to_vec_pretty(&o).unwrap()).unwrap();
    println!("=== Q{idx} done: status={} steps={} litsearch={} figures={} → {}",
        res.status, res.steps, lit, res.figures.len(), out.display());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::main]
async fn main() {
    std::fs::create_dir_all("/tmp/science_compare").unwrap();
    let venv = std::path::PathBuf::from(std::env::var_os("HOME").unwrap())
        .join("Library/Application Support/com.axiom3.desktop/notebook/.venv/bin/python");
    let py = if venv.exists() { venv.to_string_lossy().into_owned() } else { "python3".into() };
    let model_id = std::env::var("AXIOM_MODEL").unwrap_or_else(|_| "glm-5.2".into());
    let model = ProviderModel { provider: "zhipu".into(), model: model_id, timeout_s: 180 };

    let which: Vec<usize> = match std::env::var("AXIOM_Q") {
        Ok(v) => v.split(',').filter_map(|s| s.trim().parse().ok()).filter(|&i: &usize| i < QUESTIONS.len()).collect(),
        Err(_) => (0..QUESTIONS.len()).collect(),
    };
    println!("running questions: {which:?} with {}", model.model);
    for i in which {
        run_one(i, &py, &model).await;
    }
}
