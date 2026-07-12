//! Research-innovation test WITH the intent→directive reframe layer + depth nudges. Feeds a CASUAL
//! request (what a normal user types), reframes it into a rigorous directive (same REFRAME_SYSTEM as
//! the app's analysis mode), then runs the agent — so we can see whether reframe + breadth nudges
//! produce DEEPER research (diverse litsearch, competing hypotheses) than the bare prompt did.
//!
//!   DEEPSEEK_API_KEY=... HTTPS_PROXY=... cargo run -p axiom-orchestrate --example research_test
use axiom_orchestrate::coding::sandbox::{Sandbox, SandboxMode};
use axiom_orchestrate::coding::{run_coding_agent, CodingBudget, CodingModel, ProviderModel};
use axiom_orchestrate::kernel::PyKernel;
use axiom_orchestrate::Pause;
use axiom_runtime::supervisor::Cancel;
use std::sync::{Arc, Mutex};

#[derive(serde::Serialize, Clone)]
struct StepOut { n: u32, tool: String, detail: String, code: String, observation: String, reasoning: String }
#[derive(serde::Serialize)]
struct RunOut { casual: String, directive: String, status: String, summary: String, total_steps: usize, steps: Vec<StepOut> }

// What a normal user actually types — plain, short, non-expert.
const CASUAL: &str = "帮我想一个衰老细胞逃避免疫清除的新机制,画个机理图,顺便说说怎么验证";

const REFRAME_SYSTEM: &str = "\
You convert a user's plain-language request into a rigorous, self-contained RESEARCH DIRECTIVE that a \
capable research agent will execute. The user speaks casually; make their intent PRECISE and DEMANDING \
— never invent scope, domain, or specifics they did not ask for, and keep their actual question and any \
concrete details. Into the directive, bake the depth a good researcher brings:\n\
- DELIVERABLE: state exactly what the final answer must contain (e.g. a mechanism diagram + testable \
predictions).\n\
- BREADTH FIRST: have the agent survey SEVERAL DISTINCT angles / candidate mechanisms / competing \
explanations via litsearch — DIVERSE queries, not variations of one — before committing to any.\n\
- COMPETING HYPOTHESES: require at least 2-3 genuinely different hypotheses, each weighed against the \
evidence, then an explicit, justified choice of the strongest.\n\
- SELF-CRITIQUE: require naming the chosen answer's weaknesses and what would falsify it.\n\
- GROUNDING + HONESTY: ground key claims in REAL literature via litsearch and cite; separate clearly \
what is supported from what is speculative.\n\
- USE THE STEP BUDGET FOR DEPTH: do not stop at the first plausible answer.\n\
HARD RULES: The agent EXECUTES REAL TOOLS. REQUIRE it to actually run litsearch and cite real papers it \
retrieves — NEVER tell it to 'imagine' / 'assume' / generate a 'hypothetical' report. Do NOT invent figure \
syntax or an output template — the agent knows its native figure blocks; just NAME the deliverable (a \
mechanism network figure + testable predictions). Keep the directive CONCISE, no padding.\n\
Output ONLY the directive itself, in the user's language, phrased as an instruction.";

#[tokio::main]
async fn main() {
    let out = std::path::PathBuf::from("/tmp/science_compare/research_test.json");
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    let venv = std::path::PathBuf::from(std::env::var_os("HOME").unwrap())
        .join("Library/Application Support/com.axiom3.desktop/notebook/.venv/bin/python");
    let py = if venv.exists() { venv.to_string_lossy().into_owned() } else { "python3".into() };
    let model_id = std::env::var("AXIOM_MODEL").unwrap_or_else(|_| "glm-5.2".into());
    let model = ProviderModel { provider: "zhipu".into(), model: model_id.clone(), timeout_s: 180 };

    // ── reframe: casual → rigorous directive (what the app's analysis mode now does) ──
    println!("CASUAL: {CASUAL}\n--- reframing ---");
    let user = format!("User's request:\n{CASUAL}\n\nRewrite it as the research directive:");
    let directive = model.complete(REFRAME_SYSTEM, &user, &Cancel::new()).await.unwrap_or_else(|_| CASUAL.into());
    println!("DIRECTIVE:\n{directive}\n--- running ---");

    let dir = std::env::temp_dir().join("axiom-research-test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let sb = Sandbox::with_mode(&dir, SandboxMode::ReadWrite).unwrap();
    let kernel = Arc::new(PyKernel::start(&py, &dir).unwrap());
    let budget = CodingBudget { max_steps: 30, per_call_timeout_s: 120, cmd_timeout_s: 60, cell_timeout_s: 300 };
    let steps: Arc<Mutex<Vec<StepOut>>> = Arc::new(Mutex::new(Vec::new()));
    let cb = steps.clone();
    let res = run_coding_agent(
        &directive, &sb, &model, &Cancel::new(), &Pause::new(), &budget, Some(kernel),
        move |s| {
            let o: String = s.observation.lines().next().unwrap_or("").chars().take(100).collect();
            println!("  [{}] {} {} → {}", s.n, s.tool, s.detail, o);
            cb.lock().unwrap().push(StepOut {
                n: s.n, tool: s.tool.clone(), detail: s.detail.chars().take(200).collect(),
                code: s.input.chars().take(3000).collect(),
                observation: s.observation.chars().take(2500).collect(),
                reasoning: s.reasoning.chars().take(1500).collect(),
            });
        },
        |_, _| {},
    ).await;
    let collected = steps.lock().unwrap().clone();
    let o = RunOut { casual: CASUAL.into(), directive, status: res.status.clone(), summary: res.summary.clone(), total_steps: res.steps as usize, steps: collected };
    std::fs::write(&out, serde_json::to_vec_pretty(&o).unwrap()).unwrap();
    let lit = o.steps.iter().filter(|s| s.tool == "litsearch").count();
    println!("\n=== status={} steps={} litsearch={} network_block={} → {}", res.status, res.steps, lit, res.summary.contains("```network"), out.display());
    let _ = std::fs::remove_dir_all(&dir);
}
