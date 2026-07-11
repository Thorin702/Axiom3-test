//! The coding agent: a bounded tool-use loop over a fail-closed [`Sandbox`]. Uses a
//! strict-JSON action envelope over the text seam (native tool-use / token usage is the
//! `complete_blocks` follow-up). In read-only mode it reads/lists/searches and proposes;
//! in read-write mode it can also write_file and run build/test commands, so it can
//! implement a change, run the tests, and iterate to green (verified live in
//! examples/coding_live.rs). The sandbox keeps every path inside the workspace and runs
//! commands shell-free against a mode-gated allowlist.
pub mod sandbox;

use crate::control::Pause;
use crate::kernel::PyKernel;
use crate::skills;
use axiom_providers::message::{ContentBlock, Message, Request};
use axiom_runtime::litsearch::LitSearch;
use axiom_runtime::supervisor::{is_retryable_error, retry_async, Cancel, RetryPolicy};
use axiom_runtime::{complete_with, Provider};
use sandbox::{Sandbox, SandboxMode};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

pub struct CodingBudget {
    pub max_steps: u32,
    pub per_call_timeout_s: u64,
    pub cmd_timeout_s: u64,
}
impl Default for CodingBudget {
    fn default() -> Self {
        Self { max_steps: 24, per_call_timeout_s: 90, cmd_timeout_s: 45 }
    }
}

#[derive(Clone, Debug)]
pub struct CodingStepRec {
    pub n: u32,
    pub tool: String,
    pub detail: String,
    pub observation: String,
    /// The FULL tool input (complete python source / command / path / query), for an
    /// auditable execution log — `detail` is only a short label. Empty when not applicable.
    pub input: String,
    /// The model's free-text reasoning around the action (its "thinking" this turn).
    pub reasoning: String,
    /// Matplotlib figures (base64 PNG) captured from a `python` step this turn — computed
    /// from real data, never asserted. Empty for non-python steps.
    pub figures: Vec<String>,
}

pub struct CodingResult {
    pub status: String, // "done" | "failed" | "cancelled"
    pub summary: String,
    pub steps: u32,
    /// All figures the agent computed across the run, in order — surfaced into the artifact.
    pub figures: Vec<String>,
    /// Files the agent created or grew in the workspace during the run (data tables, FASTA,
    /// Newick, saved PNGs, …) — a per-file artifact manifest the app can surface, list, and
    /// reference by name (big files) instead of inlining.
    pub artifacts: Vec<ArtifactFile>,
}

/// One agent-produced workspace file, for the per-file artifact registry.
#[derive(Clone, Debug)]
pub struct ArtifactFile {
    /// Workspace-relative path (e.g. `results/markers.csv`).
    pub path: String,
    pub bytes: u64,
}

/// The model seam for the loop — abstracted so the loop is testable offline.
pub trait CodingModel: Send + Sync {
    fn complete(
        &self,
        system: &str,
        user: &str,
        cancel: &Cancel,
    ) -> impl std::future::Future<Output = anyhow::Result<String>> + Send;
}

/// Real model: one text completion per turn (history is flattened into `user` since the
/// text seam keeps no history), wrapped in the fleet's retry + timeout template.
pub struct ProviderModel {
    pub provider: String,
    pub model: String,
    pub timeout_s: u64,
}
impl CodingModel for ProviderModel {
    fn complete(
        &self,
        system: &str,
        user: &str,
        _cancel: &Cancel,
    ) -> impl std::future::Future<Output = anyhow::Result<String>> + Send {
        let provider_s = self.provider.clone();
        let model = self.model.clone();
        let system = system.to_string();
        let user = user.to_string();
        let timeout_s = self.timeout_s;
        async move {
            let provider = Provider::parse(&provider_s)
                .ok_or_else(|| anyhow::anyhow!("unknown provider: {provider_s}"))?;
            let req = Request {
                system: Some(system),
                messages: vec![Message::user(vec![ContentBlock::text(user)])],
                model,
                max_tokens: None,
                reasoning_effort: None,
                tools: Vec::new(),
            };
            retry_async(&RetryPolicy::default(), is_retryable_error, || async {
                tokio::time::timeout(Duration::from_secs(timeout_s), complete_with(provider, &req))
                    .await
                    .map_err(|_| anyhow::anyhow!("model timed out after {timeout_s}s"))?
            })
            .await
        }
    }
}

/// Kernel code for `conda_install`: bootstrap a standalone micromamba (once), then launch the
/// slow bioconda create/install as a DETACHED background job (conda solves are minutes-long), and
/// pre-wire the env's bin onto the kernel PATH so the binaries resolve once the job finishes.
/// `__PKGS__` → a python list literal; `__ID__` → the job id.
const CONDA_INSTALL: &str = r#"import os, sys, subprocess, platform, tarfile, io, requests
ROOT = os.path.expanduser('~/.axiom3'); MM = os.path.join(ROOT, 'bin', 'micromamba'); ENV = os.path.join(ROOT, 'bioenv')
os.makedirs(os.path.join(ROOT, 'bin'), exist_ok=True); os.makedirs('.axiom_jobs', exist_ok=True)
if not os.path.exists(MM):
    _s = platform.system().lower(); _m = platform.machine().lower()
    _key = ('osx-arm64' if _m in ('arm64','aarch64') else 'osx-64') if _s == 'darwin' else (('linux-aarch64' if _m in ('aarch64','arm64') else 'linux-64') if _s == 'linux' else None)
    if not _key: raise SystemExit('unsupported platform ' + _s + ' ' + _m)
    _d = requests.get('https://micro.mamba.pm/api/micromamba/%s/latest' % _key, timeout=120).content
    with tarfile.open(fileobj=io.BytesIO(_d), mode='r:bz2') as _tf:
        _mem = next(x for x in _tf.getmembers() if x.name.endswith('bin/micromamba'))
        _mem.name = os.path.basename(_mem.name); _tf.extract(_mem, os.path.join(ROOT, 'bin'))
    os.chmod(MM, 0o755)
_sub = 'install' if os.path.isdir(ENV) else 'create'
_pkgs = __PKGS__
_cmd = [MM, _sub, '-y', '-p', ENV, '-c', 'conda-forge', '-c', 'bioconda'] + _pkgs
_wrap = ("import subprocess, sys\n" "r = subprocess.run(%r, capture_output=True, text=True)\n" "sys.stdout.write(r.stdout or ''); sys.stderr.write(r.stderr or '')\n" "open('.axiom_jobs/__ID__.status', 'w').write('done' if r.returncode == 0 else 'failed')\n") % (_cmd,)
open('.axiom_jobs/__ID__.py', 'w').write(_wrap)
try: os.remove('.axiom_jobs/__ID__.status')
except OSError: pass
_p = subprocess.Popen([sys.executable, '.axiom_jobs/__ID__.py'], stdout=open('.axiom_jobs/__ID__.log', 'w'), stderr=subprocess.STDOUT, start_new_session=True)
_eb = os.path.join(ENV, 'bin'); os.environ['PATH'] = _eb + os.pathsep + os.environ.get('PATH', '')
print('micromamba ready; installing', _pkgs, 'as background job __ID__ (pid', _p.pid, ')')
print('poll with job_status id=__ID__; binaries land on PATH at', _eb, 'once DONE')
"#;

fn system_prompt(mode: SandboxMode, has_kernel: bool) -> String {
    let mut s = String::from(
        "You are a coding agent working inside a sandboxed workspace. Each turn respond with \
         EXACTLY ONE JSON object and nothing else — one of:\n\
         {\"tool\":\"plan\",\"todos\":[\"[ ] first step\",\"[ ] second step\"]}   (for a multi-step \
         task, list 3-7 TODOs; RE-SEND the whole list each time you update it, marking finished items \
         [x], so the plan stays current)\n\
         {\"tool\":\"read_file\",\"path\":\"<relative path>\"}\n\
         {\"tool\":\"list_dir\",\"path\":\"<relative path>\"}\n\
         {\"tool\":\"glob\",\"pattern\":\"<glob, e.g. src/**/*.rs>\"}   (find files by name/pattern)\n\
         {\"tool\":\"search\",\"query\":\"<text to grep for>\"}\n",
    );
    if mode == SandboxMode::ReadWrite {
        s.push_str(
            "{\"tool\":\"edit\",\"path\":\"<relative path>\",\"old\":\"<exact snippet to replace>\",\"new\":\"<replacement>\"}\n\
             {\"tool\":\"write_file\",\"path\":\"<relative path>\",\"content\":\"<COMPLETE new file contents>\"}\n\
             {\"tool\":\"run\",\"cmd\":[\"prog\",\"arg\",...]}   (build/test, e.g. [\"cargo\",\"test\"] or [\"pytest\",\"-q\"])\n\
             {\"final\":\"<what you changed and the test result>\"}\n\
             Workflow: use glob/search to LOCATE, read_file to understand, then make SMALL precise \
             changes with `edit` — its `old` must match a UNIQUE spot verbatim (include enough \
             surrounding lines), and it fails rather than misapply if `old` is missing or ambiguous. \
             Prefer edit over write_file; use write_file only to CREATE a file or rewrite it wholesale \
             (it replaces the entire file). After a SUCCESSFUL edit, do NOT repeat the same edit — move \
             on to the next change or run the tests. After changing, RUN the tests, read failures, and \
             iterate until they pass — THEN finish.",
        );
    } else {
        s.push_str(
            "{\"tool\":\"run\",\"cmd\":[\"prog\",\"arg\",...]}   (read-only allowlisted commands only)\n\
             {\"final\":\"<your findings and proposed changes>\"}\n\
             You CANNOT write files or run build/test commands here — describe any changes in your final answer.",
        );
    }
    if has_kernel {
        s.push_str(
            "\n{\"tool\":\"python\",\"code\":\"<python source>\"}   (a PERSISTENT Python kernel: \
             variables/dataframes/models stay in memory across python calls, so build up an analysis \
             step by step. stdout and the last expression's value come back to you; any matplotlib \
             figure you create is CAPTURED into the report — do `import matplotlib.pyplot as plt` and \
             draw with it. numpy, pandas, scipy, matplotlib, statsmodels and scikit-learn are \
             already installed, plus requests — use them directly (e.g. scipy.stats / statsmodels \
             for tests, confidence intervals and regression; scikit-learn for models). The kernel HAS \
             NETWORK access: DOWNLOAD a public dataset straight into the workspace with requests, \
             urllib, or pandas.read_csv(url) — e.g. from GEO, UniProt, Ensembl, or a raw GitHub URL. \
             When you fetch real data, INSPECT it before parsing (peek at the first lines/columns) and \
             handle messy reality — wrong encodings (fall back to latin-1), extra header rows, trailing \
             delimiters that shift columns.)\n\
             {\"tool\":\"pip_install\",\"packages\":[\"biopython\",\"dendropy\"]}   (install more Python \
             packages ON DEMAND into this kernel — pip, so pure-Python / wheels work: biopython, \
             dendropy, scikit-bio, umap-learn, statsmodels extras, etc. They become importable in the \
             SAME kernel right after. Prefer the pre-installed stack when it suffices; reach for this \
             when a task genuinely needs a domain library. Heavy builds may be slow.)\n\
             {\"tool\":\"conda_install\",\"packages\":[\"mafft\",\"iqtree\"]}   (install BINARY tools that \
             pip can't — MAFFT, IQ-TREE, trimAl, FastTree, Cas-OFFinder, samtools, etc. — from bioconda \
             via micromamba. The solve runs as a background job (minutes): poll job_status with the \
             returned id; once DONE the binaries are on PATH, call them from python with subprocess.)\n\
             {\"tool\":\"job_start\",\"code\":\"<python>\",\"label\":\"<what it does>\"}   (run a LONG \
             computation as a DETACHED background job that keeps going while you do other work — good \
             for a big download, a long fit, or a genome-scale scan. It runs in a FRESH process, so \
             read inputs from and write results to FILES in the workspace, not the kernel's memory. \
             Returns a job id.)\n\
             {\"tool\":\"job_status\",\"id\":\"<job id>\"}   (check a background job: RUNNING / DONE / \
             FAILED plus the tail of its log. Poll it between other steps; when DONE, read its output files.)\n\
             {\"tool\":\"checkpoint\"}   (snapshot the kernel's variables to disk before a risky or huge \
             step, so you can recover them.)  {\"tool\":\"restore\"}   (reload them after a kernel restart \
             or crash — in-memory state is otherwise lost when the kernel restarts.)\n\
             {\"tool\":\"search_skills\",\"query\":\"<domain/task>\"}   (search the domain SKILL library \
             for an expert playbook — e.g. single-cell, phylogenetics, differential expression, CRISPR \
             screen, protein engineering, GWAS. Returns matching skill names.)\n\
             {\"tool\":\"load_skill\",\"name\":\"<skill name>\"}   (load a skill card: the exact packages \
             to pip_install, the canonical pipeline, which native figure block to emit, and the rigor \
             pitfalls. When a task is in a known domain, SEARCH + LOAD the skill BEFORE writing code.)\n\
             {\"tool\":\"litsearch\",\"query\":\"<search terms>\"}   (search the peer-reviewed \
             literature via OpenAlex — returns real titles, years, authors, DOIs and abstracts. Use \
             it to FRAME an analysis around a real published study, to find the canonical dataset/method \
             for a task, or to ground a claim in a citation. Retrieving an abstract is evidence, not \
             proof; cite what you found and stay honest about what it does and doesn't show.)\n\
             Use python to COMPUTE results and figures from real data — never state a number, table, \
             or trend you did not actually compute. In your final answer, describe what the computed \
             results and figures show; the figures themselves are attached automatically.\n\
             STATISTICAL RIGOR — when the analysis involves statistics, hold yourself to: (1) CORRECT \
             for multiple testing (Benjamini-Hochberg / FDR via statsmodels.stats.multitest) whenever \
             you test many hypotheses, and report adjusted p-values; (2) never present an IN-SAMPLE \
             metric (accuracy/AUC/R² from the same data you fit on) as if it generalizes — use \
             cross-validation or a held-out set and say which; (3) aggregate to the true experimental \
             unit to avoid pseudoreplication (e.g. per-sample, not per-cell); (4) report effect sizes \
             with confidence intervals, not p-values alone. Before finishing, re-check these and state \
             any caveat honestly — surfacing a limitation is worth more than an overclaimed result.\n\
             For a crisp, interactive figure in the report, you may instead emit a NATIVE figure block \
             in your final answer, filled with values you computed in python. Use these EXACT shapes: \
             ```chart bar = {\"type\":\"bar\",\"title\":..,\"xlabel\":..,\"ylabel\":..,\"data\":[{\"label\":\"Jan\",\"value\":12}, …]}; \
             ```chart scatter (also UMAP/PCA/t-SNE embeddings) = {\"type\":\"scatter\",...,\"points\":[{\"x\":1,\"y\":2,\"group\":?}, …]}; \
             ```chart line = {\"type\":\"line\",...,\"series\":[{\"name\":..,\"points\":[[x,y], …]}]}. \
             MANY more native blocks — pick the one that fits your data; each its own fenced JSON, EXACT shapes: \
             ```heatmap {\"rows\":[..],\"cols\":[..],\"values\":[[..]],\"scale\":\"sequential|diverging\"}; \
             ```forest {\"rows\":[{\"label\":..,\"estimate\":..,\"lo\":..,\"hi\":..}],\"reference\"?:0}; \
             ```dist {\"groups\":[{\"label\":..,\"values\":[..]}],\"show\":[\"box\",\"points\"]}; \
             ```confusion {\"labels\":[..],\"matrix\":[[counts]],\"normalize\":\"row|col|none\"}; \
             ```roc {\"kind\":\"roc|pr\",\"curves\":[{\"name\":..,\"auc\"?:..,\"points\":[[x,y]]}]}; \
             ```band {\"series\":[{\"name\":..,\"points\":[[x,y,lo,hi]]}],\"xlabel\":..,\"ylabel\":..}; \
             ```dotplot {\"rows\":[..],\"cols\":[..],\"cells\":[{\"r\":..,\"c\":..,\"size\":..,\"color\":..}]}; \
             ```volcano {\"genes\":[{\"gene\":..,\"lfc\":..,\"padj\":..}],\"lfcThreshold\"?:1,\"pThreshold\"?:0.05}; \
             ```manhattan {\"points\":[{\"chr\":..,\"pos\":..,\"p\":..,\"snp\"?:..}],\"threshold\"?:..}; \
             ```track {\"length\":..,\"domains\":[{\"name\":..,\"from\":..,\"to\":..}],\"markers\":[{\"pos\":..,\"label\":..,\"count\":..,\"class\":..}]}; \
             ```tree {\"newick\":\"((A:.1,B:.2):.3,C:.4);\"}; \
             ```seqlogo {\"type\":\"dna|protein\",\"columns\":[{\"A\":..,\"C\":..,\"G\":..,\"T\":..}]}; \
             ```msa {\"type\":\"protein|dna\",\"seqs\":[{\"id\":..,\"seq\":..}],\"color\":\"clustal|identity\"}; \
             ```network {\"nodes\":[{\"id\":..,\"group\"?:..,\"value\"?:..}],\"edges\":[{\"source\":..,\"target\":..,\"weight\"?:..,\"sign\"?:\"+|-\"}],\"directed\"?:false}; \
             ```sankey {\"nodes\":[..],\"links\":[{\"source\":..,\"target\":..,\"value\":..}]}; \
             ```field {\"x\":[..],\"y\":[..],\"z\":[[..]],\"contours\"?:..}; \
             ```quiver {\"vectors\":[{\"x\":..,\"y\":..,\"u\":..,\"v\":..}]}; \
             ```slope {\"left\":..,\"right\":..,\"rows\":[{\"label\":..,\"from\":..,\"to\":..}]}; \
             ```dose {\"curves\":[{\"name\":..,\"points\":[[log10dose,resp]],\"ic50\"?:..,\"hill\"?:..}]}; \
             ```spectrum {\"kind\":\"ms|nmr|ir\",\"peaks\":[[x,intensity]],\"invertX\"?:false}; \
             ```protein <4-char PDB id like 1CRN, or inline PDB text>; \
             ```smiles <one SMILES string>; \
             ```rxn {\"reactants\":[SMILES],\"products\":[SMILES]}; \
             ```math <LaTeX>; ```mermaid <diagram>. \
             Prefer the DOMAIN block that fits (phylogeny → ```tree + ```seqlogo + ```msa; differential \
             expression → ```volcano + ```heatmap; single-cell/embeddings → ```chart scatter; a classifier → \
             ```roc + ```confusion; GWAS → ```manhattan; dose-response → ```dose; a structure → ```protein). \
             Use real computed numbers only — never fabricated ones, and match the shapes exactly.",
        );
        // Host awareness: let the agent size heavier compute to the machine.
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        s.push_str(&format!(
            "\nThis host has {cores} CPU cores available — parallelize heavier compute (e.g. \
             scikit-learn n_jobs=-1, or set worker/thread counts) rather than running single-threaded."
        ));
    }
    s.push_str(
        " For any task beyond a couple of steps, START by writing a plan (the plan tool), then work \
         through it, re-sending the updated list as you check items off. Be efficient; do not repeat a \
         read you already did.",
    );
    s
}

/// Run the loop. `on_step` is called after each tool execution (the shell streams it as
/// an event). Cancellation is checked every turn (cooperative — an in-flight command
/// still finishes or hits its timeout).
pub async fn run_coding_agent(
    goal: &str,
    sandbox: &Sandbox,
    model: &impl CodingModel,
    cancel: &Cancel,
    pause: &Pause,
    budget: &CodingBudget,
    kernel: Option<Arc<PyKernel>>,
    mut on_step: impl FnMut(CodingStepRec),
) -> CodingResult {
    let sys = system_prompt(sandbox.mode(), kernel.is_some());
    let mut transcript = format!("GOAL:\n{goal}\n\nWorkspace root: {}\n", sandbox.root().display());
    let mut ran_command = false; // did the agent run a build/test command this run?
    let mut nudged = false; // pushed back once for finishing without verifying
    let mut malformed = 0u32; // consecutive un-parseable JSON envelopes (bounded retry, not a fatal "done")
    const MAX_MALFORMED: u32 = 3;
    let mut all_figures: Vec<String> = Vec::new(); // computed figures across the whole run
    // Snapshot the workspace so we can report the files the agent produces (per-file artifacts).
    let start_files = snapshot_files(sandbox.root());
    for n in 1..=budget.max_steps {
        // Cooperative pause: wait here between turns while paused (cancel still wins).
        while pause.is_paused() && !cancel.is_cancelled() {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        if cancel.is_cancelled() {
            return CodingResult { status: "cancelled".into(), summary: "cancelled".into(), steps: n - 1, figures: all_figures, artifacts: collect_artifacts(sandbox.root(), &start_files) };
        }
        let user = format!("{transcript}\nNext action (one JSON object only):");
        let reply = match model.complete(&sys, &user, cancel).await {
            Ok(r) => r,
            Err(e) => return CodingResult { status: "failed".into(), summary: format!("model error: {e}"), steps: n - 1, figures: all_figures, artifacts: collect_artifacts(sandbox.root(), &start_files) },
        };
        let json = extract_json(&reply);
        // The model's reasoning = its reply minus the JSON action it chose this turn.
        let reasoning = match &json {
            Some(j) => reply.replacen(j, "", 1).trim().to_string(),
            None => String::new(),
        };
        // No JSON object at all → the model answered in prose; take that as the final answer.
        let action = match &json {
            Some(j) => j.clone(),
            None => return CodingResult { status: "done".into(), summary: reply.trim().to_string(), steps: n, figures: all_figures, artifacts: collect_artifacts(sandbox.root(), &start_files) },
        };
        // Weaker models routinely emit a valid-INTENT tool call whose JSON is technically
        // invalid — a multi-line `code`/`content`/`new` value with RAW newlines/tabs (JSON forbids
        // literal control chars in strings), or a stray bracket/quote from truncation. First try to
        // repair control chars; if it STILL won't parse, that's a MALFORMED tool call, NOT a final
        // answer — nudge the model to re-emit valid JSON and continue (bounded), rather than
        // silently ending the run on what was clearly an attempted action.
        let v: Value = match serde_json::from_str(&action)
            .or_else(|_| serde_json::from_str(&sanitize_json_strings(&action)))
        {
            Ok(v) => { malformed = 0; v }
            Err(e) => {
                if malformed < MAX_MALFORMED {
                    malformed += 1;
                    transcript.push_str(&format!(
                        "\n[system] Your last message was not valid JSON ({e}). Respond with EXACTLY \
                         ONE well-formed JSON object — a tool call or {{\"final\":\"…\"}} — and nothing \
                         else. If it carries code, escape newlines as \\n inside the string.\n"
                    ));
                    continue;
                }
                return CodingResult { status: "done".into(), summary: reply.trim().to_string(), steps: n, figures: all_figures, artifacts: collect_artifacts(sandbox.root(), &start_files) };
            }
        };
        if let Some(f) = v.get("final").and_then(|x| x.as_str()) {
            // In write mode, don't accept "done" until the agent has actually RUN a
            // build/test command at least once — nudge it once, like Codex verifying.
            if sandbox.mode() == SandboxMode::ReadWrite && !ran_command && !nudged {
                nudged = true;
                transcript.push_str(
                    "\n[system] You haven't run any build/test command yet. Before finishing, RUN \
                     the tests (e.g. `python3 check.py`, `pytest -q`, or `cargo test`) to confirm \
                     your change actually works — then finish.\n",
                );
                continue;
            }
            return CodingResult { status: "done".into(), summary: f.to_string(), steps: n, figures: all_figures, artifacts: collect_artifacts(sandbox.root(), &start_files) };
        }
        let tool = v.get("tool").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let mut step_figures: Vec<String> = Vec::new(); // figures this turn (python only)
        let (detail, observation) = match tool.as_str() {
            "plan" => {
                // The agent's TODO snapshot (checklist). It re-submits the full list each time
                // with items marked [x]/[ ]; we just normalize + surface it (no state kept).
                let todos: Vec<String> = v
                    .get("todos")
                    .and_then(|x| x.as_array())
                    .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                    .unwrap_or_default();
                let done = todos.iter().filter(|t| {
                    let s = t.trim_start().trim_start_matches("- ");
                    s.starts_with("[x]") || s.starts_with("[X]")
                }).count();
                let rendered: String = todos
                    .iter()
                    .map(|t| {
                        let s = t.trim().trim_start_matches("- ").trim_start();
                        if let Some(r) = s.strip_prefix("[x]").or_else(|| s.strip_prefix("[X]")) {
                            format!("- [x] {}", r.trim())
                        } else if let Some(r) = s.strip_prefix("[ ]") {
                            format!("- [ ] {}", r.trim())
                        } else {
                            format!("- [ ] {s}")
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let detail = if todos.is_empty() { "plan".into() } else { format!("{done}/{} done", todos.len()) };
                (detail, if rendered.is_empty() { "(empty plan)".to_string() } else { rendered })
            }
            "read_file" => {
                let p = arg_str(&v, "path");
                (p.to_string(), sandbox.read_file(p).unwrap_or_else(|e| format!("ERROR: {e}")))
            }
            "list_dir" => {
                let p = arg_str(&v, "path");
                (p.to_string(), sandbox.list_dir(p).unwrap_or_else(|e| format!("ERROR: {e}")))
            }
            "write_file" => {
                let p = arg_str(&v, "path").to_string();
                let content = arg_str(&v, "content");
                let obs = match sandbox.write_file(&p, content) {
                    Ok(()) => format!("wrote {p} ({} bytes)", content.len()),
                    Err(e) => format!("ERROR: {e}"),
                };
                (p, obs)
            }
            "edit" => {
                let p = arg_str(&v, "path").to_string();
                let old = arg_str(&v, "old");
                let new = arg_str(&v, "new");
                let obs = match sandbox.edit_file(&p, old, new) {
                    Ok(()) => format!("edited {p} (1 replacement)"),
                    Err(e) => format!("ERROR: {e}"),
                };
                (p, obs)
            }
            "glob" => {
                let pat = arg_str(&v, "pattern").to_string();
                let obs = sandbox.glob(&pat).unwrap_or_else(|e| format!("ERROR: {e}"));
                (pat, obs)
            }
            "search" => {
                let q = arg_str(&v, "query").to_string();
                let to = Duration::from_secs(budget.cmd_timeout_s);
                let rg = vec!["rg".into(), "-n".into(), "--max-count".into(), "40".into(), q.clone()];
                let obs = match sandbox.run_command(&rg, to).await {
                    Ok(r) => if r.output.trim().is_empty() { "(no matches)".into() } else { r.output },
                    // ripgrep binary may be absent — fall back to grep (also allowlisted).
                    Err(_) => {
                        let grep = vec!["grep".into(), "-rnI".into(), "--".into(), q.clone(), ".".into()];
                        match sandbox.run_command(&grep, to).await {
                            Ok(r) => if r.output.trim().is_empty() { "(no matches)".into() } else { r.output },
                            Err(e) => format!("ERROR: {e}"),
                        }
                    }
                };
                (q, obs)
            }
            "search_skills" => {
                let q = arg_str(&v, "query");
                let hits = skills::search(q);
                let obs = if hits.is_empty() {
                    format!("(no skill matched '{q}') Available skills:\n{}", skills::catalogue())
                } else {
                    let list = hits.iter().map(|s| format!("- {} — {}", s.name, s.summary)).collect::<Vec<_>>().join("\n");
                    format!("{list}\n\nLoad one with load_skill.")
                };
                (q.to_string(), obs)
            }
            "load_skill" => {
                let name = arg_str(&v, "name");
                let obs = match skills::get(name) {
                    Some(s) => s.card.to_string(),
                    None => format!("(no skill named '{name}') Available skills:\n{}", skills::catalogue()),
                };
                (name.to_string(), obs)
            }
            "pip_install" => match &kernel {
                Some(k) => {
                    let pkgs: Vec<String> = v
                        .get("packages")
                        .and_then(|x| x.as_array())
                        .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                        .unwrap_or_default();
                    // Reject anything that isn't a plain package spec, so the list can't smuggle
                    // pip flags (e.g. "--index-url …", "-e .") into the install.
                    let bad = pkgs.iter().find(|p| p.trim().is_empty() || p.starts_with('-'));
                    if pkgs.is_empty() {
                        ("pip_install".to_string(), "ERROR: no packages given".to_string())
                    } else if let Some(b) = bad {
                        ("pip_install".to_string(), format!("ERROR: refusing suspicious package spec '{b}'"))
                    } else {
                        ran_command = true;
                        // Install into the KERNEL's own interpreter (sys.executable = the managed venv
                        // python), inheriting its SSL env, so the packages are importable in THIS same
                        // persistent kernel immediately after.
                        let list_py = pkgs.iter().map(|p| format!("{p:?}")).collect::<Vec<_>>().join(", ");
                        let names = pkgs.join(", ");
                        let code = format!(
                            "import subprocess, sys\n\
                             _r = subprocess.run([sys.executable,'-m','pip','install','--quiet',\
                             '--disable-pip-version-check', {list_py}], capture_output=True, text=True)\n\
                             print('pip exit', _r.returncode)\n\
                             _msg = (_r.stderr or _r.stdout).strip()\n\
                             print(_msg[-1500:] if _msg else 'installed: {names}')"
                        );
                        let k = k.clone();
                        let obs = match tokio::task::spawn_blocking(move || k.exec(&code, Duration::from_secs(300))).await {
                            Ok(Ok(cell)) => {
                                let mut o = cell.stdout;
                                if let Some(e) = cell.error { if !o.is_empty() { o.push('\n'); } o.push_str(&e); }
                                if o.trim().is_empty() { o = "(done)".into(); }
                                o
                            }
                            Ok(Err(e)) => format!("ERROR: {e}"),
                            Err(e) => format!("ERROR: pip task failed: {e}"),
                        };
                        (pkgs.join(" "), obs)
                    }
                }
                None => ("pip_install".to_string(), "ERROR: pip_install needs the analysis kernel".to_string()),
            },
            "litsearch" => {
                // First-class literature tool (OpenAlex) — not a shell command, so it works
                // regardless of the sandbox network fence. Grounds analysis in real citations.
                let q = arg_str(&v, "query").to_string();
                let obs = match LitSearch::new().search(&q, 5).await {
                    Ok(works) if works.is_empty() => "(no matching works)".to_string(),
                    Ok(works) => works
                        .iter()
                        .enumerate()
                        .map(|(i, w)| {
                            let authors = if w.authors.is_empty() { "—".into() } else { w.authors.join(", ") };
                            let year = w.year.map(|y| y.to_string()).unwrap_or_else(|| "—".into());
                            let doi = w.doi.clone().unwrap_or_else(|| "—".into());
                            let abs: String = w.abstract_text.chars().take(280).collect();
                            format!("[{}] {} ({year}) — {authors}\n    DOI: {doi}\n    {abs}", i + 1, w.title)
                        })
                        .collect::<Vec<_>>()
                        .join("\n\n"),
                    Err(e) => format!("ERROR: litsearch failed: {e}"),
                };
                (q, obs)
            }
            "run" => {
                ran_command = true;
                let argv: Vec<String> = v
                    .get("cmd")
                    .and_then(|x| x.as_array())
                    .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                    .unwrap_or_default();
                let label = argv.join(" ");
                let obs = match sandbox.run_command(&argv, Duration::from_secs(budget.cmd_timeout_s)).await {
                    Ok(r) => format!("(exit {:?})\n{}", r.code, r.output),
                    Err(e) => format!("ERROR: {e}"),
                };
                (label, obs)
            }
            "python" => match &kernel {
                Some(k) => {
                    ran_command = true; // executing code counts as "ran something" (nudge)
                    let code = arg_str(&v, "code").to_string();
                    let label: String = code.lines().next().unwrap_or("").chars().take(60).collect();
                    let k = k.clone();
                    // Blocking kernel call off the async worker; generous timeout for analysis.
                    let timeout = Duration::from_secs(budget.cmd_timeout_s.max(60));
                    let obs = match tokio::task::spawn_blocking(move || k.exec(&code, timeout)).await {
                        Ok(Ok(cell)) => {
                            step_figures = cell.figures;
                            let mut o = cell.stdout;
                            if let Some(err) = cell.error {
                                if !o.is_empty() && !o.ends_with('\n') { o.push('\n'); }
                                o.push_str(&err);
                            }
                            if !step_figures.is_empty() {
                                o.push_str(&format!("\n[{} figure(s) captured into the report]", step_figures.len()));
                            }
                            if o.trim().is_empty() { o = "(ran; no output)".into(); }
                            o
                        }
                        Ok(Err(e)) => format!("ERROR: {e}"),
                        Err(e) => format!("ERROR: python kernel task failed: {e}"),
                    };
                    (label, obs)
                }
                None => ("python".to_string(), "ERROR: python tool not available in this run".to_string()),
            },
            "conda_install" => match &kernel {
                // Binary bioinformatics tools (MAFFT/IQ-TREE/Cas-OFFinder/…) aren't pip-installable.
                // Bootstrap a standalone micromamba, then install from bioconda as a DETACHED job
                // (conda solves take minutes) — poll it with job_status; the env bin is pre-wired
                // onto PATH so the binaries resolve once the job is DONE.
                Some(k) => {
                    let pkgs: Vec<String> = v
                        .get("packages")
                        .and_then(|x| x.as_array())
                        .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                        .unwrap_or_default();
                    let bad = pkgs.iter().find(|p| p.trim().is_empty() || p.starts_with('-'));
                    if pkgs.is_empty() {
                        ("conda_install".to_string(), "ERROR: no packages given".to_string())
                    } else if let Some(b) = bad {
                        ("conda_install".to_string(), format!("ERROR: refusing suspicious package spec '{b}'"))
                    } else {
                        ran_command = true;
                        let id = format!("conda{n}");
                        let list_py = format!("[{}]", pkgs.iter().map(|p| format!("{p:?}")).collect::<Vec<_>>().join(", "));
                        let code = CONDA_INSTALL.replace("__PKGS__", &list_py).replace("__ID__", &id);
                        let k = k.clone();
                        // Generous timeout only for the micromamba bootstrap; the install itself is detached.
                        let obs = match tokio::task::spawn_blocking(move || k.exec(&code, Duration::from_secs(180))).await {
                            Ok(Ok(cell)) => { let mut o = cell.stdout; if let Some(e) = cell.error { o.push_str(&e); } o }
                            Ok(Err(e)) => format!("ERROR: {e}"),
                            Err(e) => format!("ERROR: conda bootstrap failed: {e}"),
                        };
                        (format!("{id}: {}", pkgs.join(" ")), obs)
                    }
                }
                None => ("conda_install".to_string(), "ERROR: conda_install needs the analysis kernel".to_string()),
            },
            "job_start" => match &kernel {
                // Launch a long-running computation as a DETACHED process (from inside the kernel,
                // so it inherits the interpreter + SSL env) that keeps running while the agent does
                // other work. Status/output are file-based, so they survive a kernel restart.
                Some(k) => {
                    ran_command = true;
                    let code = arg_str(&v, "code").to_string();
                    let label = arg_str(&v, "label");
                    let id = format!("job{n}");
                    let indented: String = code.lines().map(|l| format!("    {l}")).collect::<Vec<_>>().join("\n");
                    let wrapped = format!(
                        "import sys, traceback\ntry:\n{indented}\n    open('.axiom_jobs/{id}.status','w').write('done')\nexcept Exception:\n    traceback.print_exc()\n    open('.axiom_jobs/{id}.status','w').write('failed')\n"
                    );
                    let launcher = format!(
                        "import os, subprocess, sys\nos.makedirs('.axiom_jobs', exist_ok=True)\n\
                         open('.axiom_jobs/{id}.py','w').write({wrapped:?})\n\
                         try: os.remove('.axiom_jobs/{id}.status')\nexcept OSError: pass\n\
                         _p = subprocess.Popen([sys.executable, '.axiom_jobs/{id}.py'], \
                         stdout=open('.axiom_jobs/{id}.log','w'), stderr=subprocess.STDOUT, start_new_session=True)\n\
                         print('started {id} (pid', _p.pid, ')')\n"
                    );
                    let k = k.clone();
                    let obs = match tokio::task::spawn_blocking(move || k.exec(&launcher, Duration::from_secs(30))).await {
                        Ok(Ok(cell)) => format!("{}\n[background job {id} running — poll it with job_status id={id}]", cell.stdout.trim()),
                        Ok(Err(e)) => format!("ERROR: {e}"),
                        Err(e) => format!("ERROR: job launch failed: {e}"),
                    };
                    (format!("{id} {label}").trim().to_string(), obs)
                }
                None => ("job_start".to_string(), "ERROR: job_start needs the analysis kernel".to_string()),
            },
            "job_status" => {
                // Pure file read — doesn't touch the kernel, so you can poll a job while other
                // kernel work proceeds.
                let id = arg_str(&v, "id");
                let jobdir = sandbox.root().join(".axiom_jobs");
                let log = std::fs::read_to_string(jobdir.join(format!("{id}.log"))).unwrap_or_default();
                let tail: String = {
                    let lines: Vec<&str> = log.lines().collect();
                    let start = lines.len().saturating_sub(40);
                    lines[start..].join("\n")
                };
                let status = std::fs::read_to_string(jobdir.join(format!("{id}.status"))).unwrap_or_default();
                let state = if status.contains("done") { "DONE ✓" } else if status.contains("failed") { "FAILED ✗" } else { "RUNNING…" };
                (id.to_string(), format!("{state}\n--- log tail ---\n{tail}"))
            }
            "checkpoint" => match &kernel {
                // Snapshot the kernel's picklable globals to disk, so a later restart (or crash) can
                // reload them with `restore` instead of losing all in-memory state.
                Some(k) => {
                    let code = "import pickle as _pk\n_ck = {}\nfor _k, _v in list(globals().items()):\n    if _k.startswith('_'): continue\n    try:\n        _pk.dumps(_v)\n        _ck[_k] = _v\n    except Exception:\n        pass\n_pk.dump(_ck, open('.axiom_ckpt.pkl', 'wb'))\nprint('checkpointed', len(_ck), 'objects:', sorted(_ck.keys()))\n";
                    let k = k.clone();
                    let obs = match tokio::task::spawn_blocking(move || k.exec(code, Duration::from_secs(60))).await {
                        Ok(Ok(cell)) => { let mut o = cell.stdout; if let Some(e) = cell.error { o.push_str(&e); } o }
                        Ok(Err(e)) => format!("ERROR: {e}"),
                        Err(e) => format!("ERROR: {e}"),
                    };
                    ("checkpoint".to_string(), obs)
                }
                None => ("checkpoint".to_string(), "ERROR: checkpoint needs the analysis kernel".to_string()),
            },
            "restore" => match &kernel {
                Some(k) => {
                    let code = "import pickle as _pk, os\nif os.path.exists('.axiom_ckpt.pkl'):\n    _ck = _pk.load(open('.axiom_ckpt.pkl', 'rb'))\n    globals().update(_ck)\n    print('restored', len(_ck), 'objects:', sorted(_ck.keys()))\nelse:\n    print('no checkpoint found (.axiom_ckpt.pkl missing)')\n";
                    let k = k.clone();
                    let obs = match tokio::task::spawn_blocking(move || k.exec(code, Duration::from_secs(60))).await {
                        Ok(Ok(cell)) => { let mut o = cell.stdout; if let Some(e) = cell.error { o.push_str(&e); } o }
                        Ok(Err(e)) => format!("ERROR: {e}"),
                        Err(e) => format!("ERROR: {e}"),
                    };
                    ("restore".to_string(), obs)
                }
                None => ("restore".to_string(), "ERROR: restore needs the analysis kernel".to_string()),
            },
            other => (other.to_string(), format!("ERROR: unknown tool '{other}'")),
        };
        all_figures.extend(step_figures.iter().cloned());
        let obs = truncate(&observation, 4000);
        // The full tool input, for the execution log (detail is only a short label).
        let input = match tool.as_str() {
            "python" => arg_str(&v, "code").to_string(),
            "read_file" | "list_dir" | "write_file" | "edit" => arg_str(&v, "path").to_string(),
            "search" | "litsearch" | "search_skills" => arg_str(&v, "query").to_string(),
            "load_skill" => arg_str(&v, "name").to_string(),
            "glob" => arg_str(&v, "pattern").to_string(),
            "job_start" => arg_str(&v, "code").to_string(),
            "job_status" => arg_str(&v, "id").to_string(),
            "pip_install" | "conda_install" => detail.clone(), // the joined package list
            _ => detail.clone(), // "run": the joined argv
        };
        on_step(CodingStepRec { n, tool: tool.clone(), detail: detail.clone(), observation: obs.clone(), input, reasoning: reasoning.clone(), figures: step_figures });
        let think = if reasoning.is_empty() { String::new() } else { format!("THINKING: {}\n", truncate(&reasoning, 500)) };
        transcript.push_str(&format!("\n--- step {n}: {tool} {detail} ---\n{think}{obs}\n"));
    }
    let artifacts = collect_artifacts(sandbox.root(), &start_files);
    CodingResult {
        status: "done".into(),
        summary: format!("Reached the {}-step budget without finishing. Latest context:\n{}", budget.max_steps, truncate(&transcript, 2000)),
        steps: budget.max_steps,
        figures: all_figures,
        artifacts,
    }
}

/// Extract the first balanced top-level `{...}` object from a reply (tolerate fences /
/// prose around it).
fn extract_json(s: &str) -> Option<String> {
    let start = s.find('{')?;
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    for i in start..s.len() {
        let c = bytes[i] as char;
        if in_str {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(s[start..=i].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

/// Escape RAW control characters that appear *inside* JSON string literals (literal
/// newlines/tabs a model emitted in a multi-line `code`/`content` value), so an otherwise
/// well-formed tool call parses instead of being misread as a final answer. Characters
/// outside string literals — and already-escaped sequences — are left untouched.
fn sanitize_json_strings(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    let mut in_str = false;
    let mut esc = false;
    for c in s.chars() {
        if in_str {
            if esc {
                out.push(c);
                esc = false;
            } else {
                match c {
                    '\\' => { out.push(c); esc = true; }
                    '"' => { out.push(c); in_str = false; }
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                    _ => out.push(c),
                }
            }
        } else {
            if c == '"' { in_str = true; }
            out.push(c);
        }
    }
    out
}

/// Snapshot every regular file under `root` as path→size (bounded), skipping VCS/build/cache
/// dirs and dotfiles. Two snapshots (before/after a run) diff to the files the agent produced.
fn snapshot_files(root: &Path) -> HashMap<String, u64> {
    fn walk(dir: &Path, root: &Path, out: &mut HashMap<String, u64>, depth: usize) {
        if depth > 8 || out.len() > 500 {
            return;
        }
        let rd = match std::fs::read_dir(dir) {
            Ok(r) => r,
            Err(_) => return,
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name == "target" || name == "node_modules" || name == "__pycache__" {
                continue;
            }
            let p = e.path();
            match e.file_type() {
                Ok(ft) if ft.is_dir() => walk(&p, root, out, depth + 1),
                Ok(ft) if ft.is_file() => {
                    if let Ok(rel) = p.strip_prefix(root) {
                        let sz = e.metadata().map(|m| m.len()).unwrap_or(0);
                        out.insert(rel.to_string_lossy().replace('\\', "/"), sz);
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = HashMap::new();
    walk(root, root, &mut out, 0);
    out
}

/// Files present now that weren't in `start` (or grew) — the run's produced artifacts.
fn collect_artifacts(root: &Path, start: &HashMap<String, u64>) -> Vec<ArtifactFile> {
    let mut v: Vec<ArtifactFile> = snapshot_files(root)
        .into_iter()
        .filter(|(p, sz)| start.get(p) != Some(sz))
        .map(|(path, bytes)| ArtifactFile { path, bytes })
        .collect();
    v.sort_by(|a, b| a.path.cmp(&b.path));
    v.truncate(100);
    v
}

fn arg_str<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(|x| x.as_str()).unwrap_or("")
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let kept: String = s.chars().take(n).collect();
        format!("{kept}\n…[truncated]")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Model that returns canned replies in order.
    struct ScriptModel {
        replies: Mutex<Vec<String>>,
    }
    impl CodingModel for ScriptModel {
        fn complete(
            &self,
            _system: &str,
            _user: &str,
            _cancel: &Cancel,
        ) -> impl std::future::Future<Output = anyhow::Result<String>> + Send {
            let next = {
                let mut r = self.replies.lock().unwrap();
                if r.is_empty() {
                    None
                } else {
                    Some(r.remove(0))
                }
            };
            async move { next.ok_or_else(|| anyhow::anyhow!("script exhausted")) }
        }
    }

    fn tmp(label: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("axiom-coding-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("main.rs"), "fn main() { println!(\"hi\"); }").unwrap();
        d
    }

    #[tokio::test]
    async fn loop_reads_then_finalizes() {
        let root = tmp("reads");
        let sb = Sandbox::new(&root).unwrap();
        let model = ScriptModel {
            replies: Mutex::new(vec![
                r#"{"tool":"list_dir","path":"."}"#.into(),
                r#"{"tool":"read_file","path":"main.rs"}"#.into(),
                r#"{"final":"main.rs prints hi; no issues."}"#.into(),
            ]),
        };
        let mut steps = 0;
        let res = run_coding_agent("review main.rs", &sb, &model, &Cancel::new(), &Pause::new(), &CodingBudget::default(), None, |_| steps += 1).await;
        assert_eq!(res.status, "done");
        assert!(res.summary.contains("prints hi"));
        assert_eq!(steps, 2); // list_dir + read_file (final isn't a tool step)
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn malformed_json_is_retried_not_finalized() {
        // A weaker model's malformed tool call (here: missing comma) must NOT be mistaken for a
        // final answer — the loop nudges it and continues to the next, valid action.
        let root = tmp("malformed");
        let sb = Sandbox::new(&root).unwrap();
        let model = ScriptModel {
            replies: Mutex::new(vec![
                r#"{"tool":"read_file" "path":"main.rs"}"#.into(), // invalid JSON (missing comma)
                r#"{"tool":"list_dir","path":"."}"#.into(),        // valid action after the nudge
                r#"{"final":"looked at the dir"}"#.into(),
            ]),
        };
        let mut steps = 0;
        let res = run_coding_agent("x", &sb, &model, &Cancel::new(), &Pause::new(), &CodingBudget::default(), None, |_| steps += 1).await;
        assert_eq!(res.status, "done", "summary: {}", res.summary);
        assert!(res.summary.contains("looked at the dir"), "must reach the post-nudge final, got: {}", res.summary);
        assert_eq!(steps, 1, "only the valid list_dir ran as a step (malformed reply executed nothing)");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn rw_loop_writes_a_file() {
        let root = tmp("rw");
        let sb = Sandbox::with_mode(&root, SandboxMode::ReadWrite).unwrap();
        let model = ScriptModel {
            replies: Mutex::new(vec![
                r#"{"tool":"write_file","path":"hello.txt","content":"hi there"}"#.into(),
                r#"{"tool":"run","cmd":["cat","hello.txt"]}"#.into(), // verifies → satisfies the "run before final" nudge
                r#"{"final":"created hello.txt"}"#.into(),
            ]),
        };
        let res = run_coding_agent("make hello.txt", &sb, &model, &Cancel::new(), &Pause::new(), &CodingBudget::default(), None, |_| {}).await;
        assert_eq!(res.status, "done");
        assert_eq!(std::fs::read_to_string(root.join("hello.txt")).unwrap(), "hi there");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn readonly_loop_cannot_write() {
        let root = tmp("ro");
        let sb = Sandbox::new(&root).unwrap(); // read-only
        let model = ScriptModel {
            replies: Mutex::new(vec![
                r#"{"tool":"write_file","path":"x.txt","content":"nope"}"#.into(),
                r#"{"final":"could not write"}"#.into(),
            ]),
        };
        let res = run_coding_agent("try to write", &sb, &model, &Cancel::new(), &Pause::new(), &CodingBudget::default(), None, |_| {}).await;
        assert_eq!(res.status, "done");
        assert!(!root.join("x.txt").exists(), "read-only sandbox must not create files");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn cancel_stops_the_loop() {
        let root = tmp("cancel");
        let sb = Sandbox::new(&root).unwrap();
        let model = ScriptModel { replies: Mutex::new(vec![r#"{"tool":"list_dir","path":"."}"#.into()]) };
        let cancel = Cancel::new();
        cancel.cancel();
        let res = run_coding_agent("x", &sb, &model, &cancel, &Pause::new(), &CodingBudget::default(), None, |_| {}).await;
        assert_eq!(res.status, "cancelled");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn python_tool_runs_in_persistent_kernel() {
        // Needs a real python3; skip cleanly where absent (CI images without it).
        let py = match ["python3", "python"].iter().find(|p| {
            std::process::Command::new(p).arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
        }) {
            Some(p) => p.to_string(),
            None => return,
        };
        let root = tmp("py");
        let sb = Sandbox::with_mode(&root, SandboxMode::ReadWrite).unwrap();
        let kernel = Arc::new(crate::kernel::PyKernel::start(&py, &root).unwrap());
        let model = ScriptModel {
            replies: Mutex::new(vec![
                r#"{"tool":"python","code":"x = 6 * 7\nprint(x)"}"#.into(),
                r#"{"tool":"python","code":"print(x + 1)"}"#.into(), // x persists across calls
                r#"{"final":"computed 42 then 43"}"#.into(),
            ]),
        };
        let mut obs: Vec<String> = Vec::new();
        let mut inputs: Vec<String> = Vec::new();
        let res = run_coding_agent(
            "compute", &sb, &model, &Cancel::new(), &Pause::new(), &CodingBudget::default(),
            Some(kernel), |s| { obs.push(s.observation); inputs.push(s.input); },
        )
        .await;
        assert_eq!(res.status, "done", "summary: {}", res.summary);
        assert!(obs.len() >= 2, "expected two python steps, got {obs:?}");
        assert!(obs[0].contains("42"), "first python step stdout: {}", obs[0]);
        assert!(obs[1].contains("43"), "kernel state must persist across python calls: {}", obs[1]);
        // The execution log needs the FULL code, not just the first line.
        assert!(inputs[0].contains("x = 6 * 7") && inputs[0].contains("print(x)"), "input must be the full code, got: {}", inputs[0]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn extract_json_handles_fences_and_braces() {
        assert_eq!(extract_json("```json\n{\"a\":1}\n```").as_deref(), Some("{\"a\":1}"));
        assert_eq!(extract_json("sure: {\"final\":\"a {nested} brace\"} done").as_deref(), Some("{\"final\":\"a {nested} brace\"}"));
        assert_eq!(extract_json("no json here"), None);
    }

    #[test]
    fn sanitize_repairs_raw_newlines_in_code_so_tool_call_parses() {
        // A model emitted a python tool call with LITERAL newlines/tabs inside `code`
        // (invalid JSON). Raw parse fails; sanitized parse must recover the tool call.
        let raw = "{\"tool\":\"python\",\"code\":\"import numpy as np\n\tx = 6*7\nprint(x)\"}";
        assert!(serde_json::from_str::<Value>(raw).is_err(), "raw should be invalid JSON");
        let v: Value = serde_json::from_str(&sanitize_json_strings(raw)).expect("repaired JSON parses");
        assert_eq!(v.get("tool").and_then(|x| x.as_str()), Some("python"));
        assert!(v.get("code").and_then(|x| x.as_str()).unwrap().contains("print(x)"));
        // Already-valid JSON (escaped \n, braces in strings) must survive untouched.
        let good = "{\"final\":\"line1\\nline2 with a {brace}\"}";
        assert_eq!(sanitize_json_strings(good), good);
        assert!(serde_json::from_str::<Value>(&sanitize_json_strings(good)).is_ok());
    }
}
