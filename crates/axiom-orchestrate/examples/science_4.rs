//! Flagship comparison: run our platform's coding/analysis agent (the SAME engine the desktop
//! app's "分析" mode uses) on the 4 Claude Science official example tasks, capturing each run's
//! full trace — thinking chain (reasoning), executed code, observations, computed figures, and
//! artifact files — to JSON so we can compare item-by-item against Claude Science and assess our
//! capability level. Scoped to the CORE analysis of each example (Claude Science ran them at full
//! GB/genome scale over hours); we honestly record what our engine actually produces.
//!
//!   DEEPSEEK_API_KEY=... HTTPS_PROXY=... cargo run -p axiom-orchestrate --example science_4
use axiom_orchestrate::coding::sandbox::{Sandbox, SandboxMode};
use axiom_orchestrate::coding::{run_coding_agent, CodingBudget, ProviderModel};
use axiom_orchestrate::kernel::PyKernel;
use axiom_orchestrate::Pause;
use axiom_runtime::supervisor::Cancel;
use std::sync::{Arc, Mutex};

#[derive(serde::Serialize, Clone)]
struct StepOut {
    n: u32,
    tool: String,
    detail: String,
    code: String,        // full tool input (the code/command the agent ran)
    observation: String, // truncated result
    reasoning: String,   // the agent's narration for this step
    figures: usize,
}

#[derive(serde::Serialize)]
struct RunOut {
    name: String,
    task: String,
    status: String,
    summary: String,
    total_steps: usize,
    figure_count: usize,
    artifacts: Vec<String>,
    steps: Vec<StepOut>,
}

const TASKS: &[(&str, &str)] = &[
    (
        "01_scrnaseq_immunotherapy",
        "You are analyzing single-cell RNA-seq of melanoma immunotherapy (the Sade-Feldman 2018 study, \
         GEO GSE120575). First SEARCH the skill registry and LOAD the single-cell RNA-seq skill, then \
         follow its pipeline. Obtain a representative expression matrix (download a small real dataset \
         if you can, otherwise simulate a realistic counts matrix of ~800 cells x ~500 genes across \
         responder/non-responder patients with a few marker genes). Run the standard pipeline: QC \
         filtering, normalization, highly-variable genes, PCA, neighbors, Leiden clustering, and a \
         UMAP; annotate clusters by marker genes. Then compare responders vs non-responders with a \
         PSEUDOBULK differential-expression test, apply Benjamini-Hochberg/FDR correction, and report \
         effect sizes HONESTLY (guard against pseudoreplication; if you fit any classifier, report a \
         cross-validated metric, not the in-sample one). In your FINAL answer present a UMAP scatter \
         and a volcano figure of the DE result (native ```chart / ```volcano / ```dist blocks filled \
         with the values you COMPUTED, or matplotlib), a short results paragraph, and honest limitations.",
    ),
    (
        "02_phylogenetics_ancestral",
        "You are doing a phylogenetics + ancestral-sequence-reconstruction analysis of the DUF34/NIF3 \
         protein family (like MJ0927). SEARCH the skill registry and LOAD the phylogenetics skill, then \
         follow it. Obtain a set of family protein sequences — fetch a few real ones from UniProt via \
         the network if you can (requests is available), otherwise use a representative set of ~12 \
         homologous sequences you construct. Align them (use MAFFT if you can install it, else fall \
         back to Biopython's pairwise/`Bio.Align`), build a maximum-likelihood or neighbor-joining tree \
         with support values, track a conserved metal-binding residue (e.g. a His) across the tree, and \
         reconstruct the ancestral residue at key internal nodes. In your FINAL answer present a \
         phylogenetic ```tree (Newick) and a ```seqlogo of the conserved motif filled with your \
         computed data, plus the ancestral-state call with its support, and note caveats.",
    ),
    (
        "03_crispr_kinase_library",
        "You are designing a genome-wide CRISPR knockout library targeting the human KINOME. SEARCH the \
         skill registry and LOAD the CRISPR-screen skill, then follow it. Enumerate a representative set \
         of human kinase genes (use a built-in list or fetch one), design several sgRNAs per gene, and \
         SCORE each guide with an on-target rule plus a CFD-style off-target proxy (compute the scores \
         in python from the guide sequences; simulate candidate protospacers if you have no genome). \
         Handle gene aliases/paralogs and flag guides containing a BsmBI (CGTCTC) site that would break \
         Golden-Gate cloning. Plan sequencing depth for the pooled screen. In your FINAL answer present \
         a ```dist or ```chart of guides-per-gene and of the score distribution filled with COMPUTED \
         values, a short design summary, and the counts (genes, total guides, flagged guides).",
    ),
    (
        "04_protein_engineering_is621",
        "You are doing protein engineering on the IS621 recombinase to propose stabilizing / \
         activity-improving mutations. SEARCH the skill registry and LOAD the protein-engineering skill, \
         then follow it. Take a representative IS621 sequence (construct one if you cannot fetch it). \
         Score candidate single mutations with MULTIPLE methods and rank by CONSENSUS: at minimum a \
         conservation/substitution-matrix proxy (BLOSUM) and a simple physicochemical/structure proxy; \
         if you can pip_install a protein language model (e.g. fair-esm) do so and add its score, else \
         self-heal by falling back to the proxies and say so. Produce a RANKED table of the top mutations \
         with per-method and consensus scores. In your FINAL answer present the ranked table and a \
         ```chart / ```network / ```track figure of the scores filled with COMPUTED values, plus a short \
         rationale and honest limitations about the proxies used.",
    ),
];

#[tokio::main]
async fn main() {
    let out_dir = std::path::PathBuf::from("/tmp/science_compare");
    std::fs::create_dir_all(&out_dir).unwrap();
    let venv = std::path::PathBuf::from(std::env::var_os("HOME").unwrap())
        .join("Library/Application Support/com.axiom3.desktop/notebook/.venv/bin/python");
    let py = if venv.exists() { venv.to_string_lossy().into_owned() } else { "python3".into() };
    let model_id = std::env::var("AXIOM_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".into());
    println!("kernel python: {py}\nmodel: {model_id}\nout: {}", out_dir.display());

    let only = std::env::var("ONLY").ok(); // optional: run a single example by index prefix, e.g. ONLY=01

    for (name, task) in TASKS {
        if let Some(o) = &only {
            if !name.starts_with(o.as_str()) { continue; }
        }
        println!("\n\n════════════════════ {name} ════════════════════");
        let dir = std::env::temp_dir().join(format!("axiom-sci-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sb = Sandbox::with_mode(&dir, SandboxMode::ReadWrite).unwrap();
        let kernel = match PyKernel::start(&py, &dir) {
            Ok(k) => Arc::new(k),
            Err(e) => { eprintln!("kernel start failed: {e}"); continue; }
        };
        let model = ProviderModel { provider: "deepseek".into(), model: model_id.clone(), timeout_s: 120 };
        // Post-optimization budget: matches the app's analyze path (40 steps) so a re-run is
        // representative of what "分析" mode now does after the search-dedup + budget fixes.
        let budget = CodingBudget { max_steps: 40, per_call_timeout_s: 120, cmd_timeout_s: 180, cell_timeout_s: 600 };
        let steps: Arc<Mutex<Vec<StepOut>>> = Arc::new(Mutex::new(Vec::new()));
        let steps_cb = steps.clone();
        let res = run_coding_agent(
            task, &sb, &model, &Cancel::new(), &Pause::new(), &budget, Some(kernel),
            move |step| {
                let obs1: String = step.observation.lines().next().unwrap_or("").chars().take(90).collect();
                let figs = if step.figures.is_empty() { String::new() } else { format!(" [+{} fig]", step.figures.len()) };
                println!("  [{}] {} {}{} → {}", step.n, step.tool, step.detail, figs, obs1);
                steps_cb.lock().unwrap().push(StepOut {
                    n: step.n,
                    tool: step.tool.clone(),
                    detail: step.detail.chars().take(200).collect(),
                    code: step.input.chars().take(4000).collect(),
                    observation: step.observation.chars().take(2500).collect(),
                    reasoning: step.reasoning.chars().take(1500).collect(),
                    figures: step.figures.len(),
                });
            },
        |_, _| {},
        )
        .await;
        let collected = steps.lock().unwrap().clone();
        let out = RunOut {
            name: name.to_string(),
            task: task.to_string(),
            status: res.status.clone(),
            summary: res.summary.clone(),
            total_steps: res.steps as usize,
            figure_count: res.figures.len(),
            artifacts: res.artifacts.iter().map(|a| format!("{} ({} bytes)", a.path, a.bytes)).collect(),
            steps: collected,
        };
        let path = out_dir.join(format!("{name}.json"));
        std::fs::write(&path, serde_json::to_vec_pretty(&out).unwrap()).unwrap();
        println!("=== {name}: status={} steps={} figures={} → {}", res.status, res.steps, res.figures.len(), path.display());
        let _ = std::fs::remove_dir_all(&dir);
    }
    println!("\n\nALL DONE. JSON results in {}", out_dir.display());
}
