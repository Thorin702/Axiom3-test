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
use axiom_runtime::web::WebClient;
use axiom_runtime::supervisor::{is_retryable_error, retry_async, Cancel, RetryPolicy};
use axiom_runtime::complete_by_name;
use sandbox::{Sandbox, SandboxMode};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct CodingBudget {
    pub max_steps: u32,
    pub per_call_timeout_s: u64,
    /// Timeout for a SHELL command (`run`) — kept tight; a hung shell should die fast.
    pub cmd_timeout_s: u64,
    /// Timeout for a PYTHON KERNEL cell. Distinct from `cmd_timeout_s` and much more generous:
    /// a slow-but-finite analysis cell (e.g. parsing a 126MB dirty dataframe) must be allowed to
    /// FINISH and keep its state, not be killed at a tight bound — a kernel restart wipes every
    /// loaded dataframe/import and forces the agent to redo work. Only a truly wedged cell should
    /// ever hit this.
    pub cell_timeout_s: u64,
}
impl Default for CodingBudget {
    fn default() -> Self {
        Self { max_steps: 24, per_call_timeout_s: 90, cmd_timeout_s: 45, cell_timeout_s: 600 }
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
            let req = Request {
                system: Some(system),
                messages: vec![Message::user(vec![ContentBlock::text(user)])],
                model,
                max_tokens: None,
                reasoning_effort: None,
                tools: Vec::new(),
            };
            // Dispatch by name: a user-registered connection label routes to its own
            // OpenAI-compatible endpoint; a built-in provider name uses its env key.
            retry_async(&RetryPolicy::default(), is_retryable_error, || async {
                tokio::time::timeout(Duration::from_secs(timeout_s), complete_by_name(&provider_s, &req))
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
// Snapshot / reload picklable kernel globals — used by the checkpoint/restore tools AND by the
// loop's AUTOMATIC resilience (checkpoint after an expensive cell, restore after a state-wiping
// kernel reset), so a slow real-data step can no longer cost the whole session its loaded state.
const CHECKPOINT_CODE: &str = "import pickle as _pk, os as _os\n_ck = {}\nfor _k, _v in list(globals().items()):\n    if _k.startswith('_'): continue\n    try:\n        _pk.dumps(_v, protocol=_pk.HIGHEST_PROTOCOL)\n        _ck[_k] = _v\n    except Exception:\n        pass\n_tmp = '.axiom_ckpt.pkl.tmp'\nwith open(_tmp, 'wb') as _f:\n    _pk.dump(_ck, _f, protocol=_pk.HIGHEST_PROTOCOL)\n_os.replace(_tmp, '.axiom_ckpt.pkl')\nprint('checkpointed', len(_ck), 'objects:', sorted(_ck.keys()))\n";
const RESTORE_CODE: &str = "import pickle as _pk, os\nif os.path.exists('.axiom_ckpt.pkl'):\n    _ck = _pk.load(open('.axiom_ckpt.pkl', 'rb'))\n    globals().update(_ck)\n    print('restored', len(_ck), 'objects:', sorted(_ck.keys()))\nelse:\n    print('no checkpoint found (.axiom_ckpt.pkl missing)')\n";

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

fn system_prompt(mode: SandboxMode, has_kernel: bool, can_delegate: bool) -> String {
    let mut s = String::from(
        "You are a research coding agent working in a sandboxed workspace. Each turn, FIRST write \
         1-2 short sentences of THINKING in the USER'S LANGUAGE (Chinese if they wrote Chinese) — say \
         what you are about to do and WHY, or what the last result told you and what you'll do next — \
         as plain prose with NO curly braces. This narration is streamed LIVE to the user so they can \
         follow your reasoning like a scientist thinking out loud; a bare tool call with no thinking \
         leaves them staring at a '运行代码' row with no idea what is happening, so ALWAYS narrate. \
         THEN, on a new line, give EXACTLY ONE JSON action object — one of:\n\
         {\"tool\":\"plan\",\"todos\":[\"[ ] first step\",\"[ ] second step\"]}   (for a multi-step \
         task, list 3-7 TODOs; RE-SEND the whole list each time you update it, marking finished items \
         [x], so the plan stays current)\n\
         {\"tool\":\"read_file\",\"path\":\"<relative path>\"}   (or {\"paths\":[\"a\",\"b\"]} to read several in ONE step)\n\
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
             draw with it. For PUBLICATION-GRADE figures, set a clean style ONCE at the top of your first \
             plotting cell — `import matplotlib as mpl; mpl.rcParams.update({'figure.dpi':140,'savefig.dpi':140,\
             'font.size':11,'axes.titlesize':12.5,'axes.labelsize':11,'axes.spines.top':False,\
             'axes.spines.right':False,'axes.grid':True,'grid.alpha':0.22,'legend.frameon':False,\
             'figure.autolayout':True})` — then label axes WITH UNITS, give a concise title, add a legend \
             for >=2 series, and use a restrained colorblind-safe palette (tab10 / a seaborn palette). \
             LABELS IN THE USER'S LANGUAGE: for Chinese/CJK labels set a CJK font FIRST — \
             `mpl.rcParams['font.sans-serif']=['PingFang SC','Heiti TC','Arial Unicode MS','DejaVu Sans']; \
             mpl.rcParams['axes.unicode_minus']=False` — then verify no missing-glyph boxes; only if no CJK \
             font is available fall back to English on the figure. \
             MECHANISM / PATHWAY / CONCEPT figures MUST be a PUBLICATION-GRADE matplotlib SCHEMATIC — draw \
             it like a scientific ILLUSTRATOR making a clean Nature/Cell figure, NOT a slide flowchart of \
             rectangles. Concretely: (a) CELLS = filled ELLIPSES/CIRCLES with a colored edge and a soft \
             pastel fill, e.g. `ax.add_patch(Ellipse((x,y),w,h,facecolor='#EAD9F2',edgecolor='#8A6FC4',\
             lw=1.6,alpha=.92,zorder=1))` — NOT rectangles for cells; (b) RECEPTORS = small rounded boxes \
             (FancyBboxPatch, boxstyle='round,pad=0.02') placed ON the cell membrane (sitting on the \
             ellipse's edge); (c) LIGANDS / small molecules = small circles; a repeating chain (glycans, \
             polymers, fibrils) = a ROW of small circles along a path, not one box labelled 'glycan'; \
             (d) SIGNALLING = curved `FancyArrowPatch(a,b,arrowstyle='-|>',connectionstyle='arc3,rad=0.2',\
             lw=2,color=...)`; INHIBITION = a blunt bar-end ('-['); (e) a SOFT professional palette (4-5 \
             muted pastels, never primary red/green/blue at full saturation), GENEROUS whitespace so \
             nothing overlaps, concise labels placed clear of shapes, and a small legend box in a corner; \
             set `ax.set_xlim/ylim` with margin and `ax.axis('off')`. Do NOT emit a ```network / ```chart \
             block of dots-and-lines as the mechanism figure — that reads as a placeholder. A run asked \
             for a 机理图/mechanism diagram that produced no matplotlib schematic has FAILED its deliverable. \
             ITERATE ON THE FIGURE: after a plotting cell, CHECK it actually ran (no Traceback) and LOOK at \
             the result — if labels overlap, glyphs are boxes, or it errored, FIX and redraw (1-2 refine \
             passes) before finishing; a good figure is iterated, never one-shot. \
             CODE-QUOTING — figure cells DIE on this constantly, so sanitize every label BEFORE running: \
             (a) write EVERY string label with DOUBLE quotes; (b) NEVER put a curly/smart quote (\u{201c} \u{201d} \u{2018} \u{2019}) \
             or a bare apostrophe INSIDE a label — write \"don't eat me\" with straight ASCII quotes (or \
             escape the apostrophe), and use straight \" not \u{201c}\u{201d}; (c) a line break inside a label must be \\n, \
             NEVER a real newline in the middle of a quoted string (that gives 'unterminated string \
             literal'); (d) matplotlib arrows: plt.arrow() has NO 'mutation_scale' — for a styled arrow \
             use FancyArrowPatch(posA,posB,arrowstyle=\"-|>\",mutation_scale=14) or ax.annotate(\"\",xy=..,\
             xytext=..,arrowprops=dict(arrowstyle=\"-|>\")). A smart-quote or unescaped apostrophe in a \
             label is the #1 cause of a wasted figure attempt. \
             numpy, pandas, scipy, matplotlib, statsmodels and scikit-learn are \
             already installed, plus requests — use them directly (e.g. scipy.stats / statsmodels \
             for tests, confidence intervals and regression; scikit-learn for models). The kernel HAS \
             NETWORK access: DOWNLOAD a public dataset straight into the workspace with requests, \
             urllib, or pandas.read_csv(url) — e.g. from GEO, UniProt, Ensembl, or a raw GitHub URL. \
             When you fetch real data, INSPECT it before parsing (peek at the first lines/columns) and \
             handle messy reality — wrong encodings (fall back to latin-1), extra header rows, trailing \
             delimiters that shift columns. For a LARGE or dirty file, sniff the shape on a small slice \
             first (nrows=/usecols=/chunksize=) to get the parse right in ONE pass, then load the full \
             frame. Your state is AUTO-checkpointed after each expensive cell and AUTO-restored if the \
             kernel restarts, so you rarely need to manage that by hand — just keep each heavy operation \
             in its own cell.)\n\
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
             {\"tool\":\"checkpoint\"}   (usually AUTOMATIC — state is snapshotted after each expensive \
             cell; call this only to force an extra snapshot before something especially risky.)  \
             {\"tool\":\"restore\"}   (also AUTOMATIC after a kernel restart; call it manually only if you \
             need to roll back to the last checkpoint on purpose.)\n\
             {\"tool\":\"ask_user\",\"questions\":[{\"header\":\"<=12 chars\",\"question\":\"<neutral one-line choice>\",\
             \"kind\":\"intent|fork|permission|integrity|scope|cost\",\"why_it_matters\":\"<how the branches change the verdict>\",\
             \"multiSelect\":false,\"allowOther\":true,\"options\":[{\"label\":\"<=40 chars\",\"description\":\"what it does + the tradeoff\",\"convention\":false}]}]}   \
             (ASK THE HUMAN a structured question and WAIT for the answer — the run pauses until they reply. Use 1-4 \
             questions per card, 2-4 options each; batch related forks into ONE card. See the asking policy below.)\n\
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
             {\"tool\":\"web_search\",\"query\":\"<search terms>\"}   (search the WHOLE WEB, not just \
             papers — official databases (GEO/SRA/UniProt/PDB/Ensembl/PubChem), datasets, tool docs, \
             preprints, standards, org/news pages. Returns titles + URLs + snippets. Use it to FIND a \
             source or dataset you don't yet have a URL for.)\n\
             {\"tool\":\"web_fetch\",\"url\":\"<https URL>\"}   (READ one public URL as text — an article, \
             a database record page, a documentation page you found via web_search. Returns the page's \
             cleaned text. Only public http(s) URLs.)\n\
             ── WHICH TOOL WHEN (pick the RIGHT one each step) ──\n\
             • Peer-reviewed papers / citations → litsearch.  • Anything else on the web (databases, \
             datasets, docs, preprints, orgs) you need to FIND → web_search.  • READ a specific page you \
             already have a URL for → web_fetch.  • DOWNLOAD + PARSE a structured dataset (CSV/TSV/JSON, \
             an accession, an FTP/HTTPS data file) or COMPUTE anything → python (pandas/urllib), NOT \
             web_fetch (which returns prose text, not a parsed table).  • A domain has a known pipeline → \
             search_skills + load_skill FIRST.  Don't write python to scrape a web page when web_fetch \
             does it; don't web_fetch a data table when python can parse it.\n\
             Use python to COMPUTE results and figures from real data — never state a number, table, \
             or trend you did not actually compute. In your final answer, describe what the computed \
             results and figures show; the figures themselves are attached automatically.\n\
             BUDGET RULE — you have a GENEROUS step budget, shown each turn as [budget] step N of M. \
             QUALITY FIRST: prioritize a CORRECT, COMPLETE analysis over speed — it is fine to take many \
             steps and a long time, so do NOT cut corners, skip validation, or rush to {\"final\":…}. \
             Still avoid WASTED steps: never re-run a search, re-read a file, or re-download something you \
             already have, and never repeat a failed approach UNCHANGED (change the approach or the inputs). \
             Get a first computed figure reasonably early to de-risk, then DEEPEN it. If a real dataset is \
             genuinely too big to process in one cell, SUBSAMPLE to a scientifically valid size (e.g. \
             ~800–2000 cells) and disclose it — a solid subsampled result beats a full one that never \
             finishes. Keep each heavy operation in its OWN cell so it completes and gets auto-checkpointed. \
             Finish with {\"final\":…} only when the analysis is actually complete and its figures are drawn.\n\
             LARGE / REAL DATA — real datasets can be huge and a naive full read hangs the run for \
             MANY MINUTES with no visible progress (a 100+ MB single-cell matrix is the classic \
             trap). So: (1) BEFORE reading, check the size and PRINT it — `import os; \
             print(os.path.getsize(path)/1e6, 'MB')`; (2) if a table is large (>~50 MB compressed, \
             or >~1e5 rows, e.g. a single-cell TPM matrix), do NOT read the whole thing at once — \
             read a HEAD first (`pd.read_csv(path, nrows=5)`) to learn the shape/columns, then load \
             only the columns/rows you need (`usecols=`), read in chunks, or SUBSAMPLE to the size \
             the task names (e.g. ~800 cells); a subsampled real dataset beats a full one that never \
             finishes; (3) put each heavy operation — download, size-check, read, process — in its \
             OWN python step and PRINT a one-line progress note ('downloaded 126 MB', 'matrix \
             16291x55738 → subsampling to 800 cells'), so the human sees forward motion instead of a \
             frozen 'running'. Never let one cell BOTH download a big file AND fully parse it.\n\
             STATISTICAL RIGOR — when the analysis involves statistics, hold yourself to: (1) CORRECT \
             for multiple testing (Benjamini-Hochberg / FDR via statsmodels.stats.multitest) whenever \
             you test many hypotheses, and report adjusted p-values; (2) never present an IN-SAMPLE \
             metric (accuracy/AUC/R² from the same data you fit on) as if it generalizes — use \
             cross-validation or a held-out set and say which; (3) aggregate to the true experimental \
             unit to avoid pseudoreplication (e.g. per-sample, not per-cell); (4) report effect sizes \
             with confidence intervals, not p-values alone. Before finishing, re-check these and state \
             any caveat honestly — surfacing a limitation is worth more than an overclaimed result.\n\
             ASKING THE HUMAN (ask_user) — you are a verification-first research agent, so you ask MORE \
             than a generic coding assistant: in science a silent default is a hidden assumption that \
             can invalidate a result. When a choice is genuinely FORKED and load-bearing you MUST call \
             ask_user and wait — never silently pick the option you think is best (that is the failure \
             this product exists to prevent). ASK at these moments: (1) INTAKE — once, at the start of a \
             non-trivial run BEFORE downloading data or heavy compute, batch up to 4 questions confirming \
             the research question, what a trustworthy answer looks like, and data provenance (attached vs \
             fetched vs synthetic changes trust); (2) TARGET NUMBER — if the request names an expected \
             result ('~150 genes'), ask if it's a hypothesis to TEST (report the honest number) or a hard \
             expectation (you will NOT tune thresholds to hit it) — never reverse-engineer results to a \
             target; (3) INTEGRITY — if the task as stated would need p-hacking/cherry-picking/overstating \
             certainty, surface it as a choice, never silently comply or refuse; (4) PERMISSION — before \
             network egress beyond curated sources, large downloads, long/background compute, or writing \
             outside your scratch, state the exact size/cost; (5) A methodological FORK where alternatives \
             are each defensible AND would plausibly move the conclusion: normalization/transform, the \
             statistical test, multiple-testing correction + alpha, significance/effect cutoff, data source \
             + VERSION/BUILD, QC/inclusion filters, batch correction, covariates, aggregation unit, \
             reference/control, imputation, seed-when-it-matters. Do NOT ask about cosmetic/reversible \
             choices or a truly-standard default whose alternative wouldn't change the conclusion — apply it \
             and DISCLOSE it in your assumptions. Present options NEUTRALLY (never 'recommended'/'best'/\
             'optimal'; mark at most one as the field `convention`); for a real fork pre-select NOTHING. \
             Budget: after intake, at most ~3 discretionary fork cards per run (permission/integrity asks \
             don't count) — when spent, fall back to conventions and DISCLOSE them so the run converges. \
             After the human answers, use exactly their choice and record it in your methods/assumptions.\n\
             For QUANTITATIVE DATA you computed (distributions, comparisons, correlations, time series, \
             ROC/PR, volcano, heatmaps, embeddings…), you may ALSO emit a NATIVE figure block filled with \
             your computed values — these are for DATA, NOT a substitute for a hand-drawn mechanism \
             schematic (draw those in matplotlib as above). Use these EXACT shapes: \
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
    if can_delegate {
        s.push_str(
            "\n{\"tool\":\"delegate\",\"task\":\"<a self-contained, non-trivial SUBTASK>\"}   (DEEP RESEARCH \
             only: hand a COMPLEX, separable subtask to a multi-agent sub-team — it is decomposed into \
             several angles, a focused sub-agent tackles each, and the results are synthesized back to \
             you. Use it like a scientist opening a working group: when ONE step is itself broad (e.g. \
             'survey all proposed mechanisms of X across fields', 'weigh the 4 competing hypotheses for \
             Y', 'map the evidence for and against Z'), delegate it rather than doing it shallowly \
             yourself; then build on what comes back. Don't delegate something you can answer in one \
             step, and don't delegate the WHOLE task — decompose first, delegate the hard branches.)",
        );
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
/// The `delegate` tool: spawn a BOUNDED multi-agent sub-workflow for a complex, self-contained
/// subtask — decompose it into 2-4 independent angles, have a focused sub-agent tackle each
/// (sequential; parallel GLM calls hit 429), then synthesize. Reuses `model.complete` (no tools, no
/// kernel, no recursion into the full agent — a tool-using sub-agent is a later upgrade). Returns the
/// synthesized result as the delegating step's observation. Bounded to keep a research run converging.
pub async fn run_delegate(model: &impl CodingModel, task: &str, cancel: &Cancel) -> String {
    let decomp_sys = "把给定的研究子任务拆成 2-4 个可以各自独立深挖的角度/子问题,每行一个,只输出角度本身(不要编号、不要解释、不要多余的话)。";
    let angles_raw = model
        .complete(decomp_sys, &format!("子任务:\n{task}"), cancel)
        .await
        .unwrap_or_default();
    let mut angles: Vec<String> = angles_raw
        .lines()
        .map(|l| l.trim().trim_start_matches(|c: char| c.is_ascii_digit() || matches!(c, '-' | '*' | '·' | '.' | ')' | '(' | ' ' | '、')).trim().to_string())
        .filter(|l| l.chars().count() > 3)
        .take(4)
        .collect();
    if angles.is_empty() {
        angles.push(task.to_string());
    }
    let mut parts: Vec<String> = Vec::new();
    for (i, a) in angles.iter().enumerate() {
        if cancel.is_cancelled() {
            break;
        }
        let sub_sys = "你是一个专注的研究子智能体,只深入回答分配给你的这一个角度:要具体、有据、简洁,可引用你确知的真实文献/机制,诚实区分已知与推测,绝不编造。不要跑题到别的角度。";
        let ans = model
            .complete(sub_sys, &format!("总子任务:{task}\n\n你负责的角度:{a}\n\n给出你这一角度的深入结论:"), cancel)
            .await
            .unwrap_or_default();
        if !ans.trim().is_empty() {
            parts.push(format!("### 角度 {}:{a}\n{}", i + 1, ans.trim()));
        }
    }
    if parts.is_empty() {
        return "[委派未产出结果 —— 请自己完成这个子任务]".to_string();
    }
    let body = parts.join("\n\n");
    let synth_sys = "把多个子智能体各自角度的结论综合成对总子任务的一个连贯、去重、有取舍的结论:忠实于各部分,指出它们的一致与冲突,不要编造。";
    let synth = model
        .complete(synth_sys, &format!("总子任务:{task}\n\n各角度结论:\n{body}\n\n综合结论:"), cancel)
        .await
        .unwrap_or_else(|_| body.clone());
    format!("[委派完成 · {} 个子智能体各攻一角度并综合]\n\n{}", parts.len(), synth.trim())
}

pub async fn run_coding_agent(
    goal: &str,
    sandbox: &Sandbox,
    model: &impl CodingModel,
    cancel: &Cancel,
    pause: &Pause,
    budget: &CodingBudget,
    kernel: Option<Arc<PyKernel>>,
    mut on_step: impl FnMut(CodingStepRec),
    // Called when the agent invokes `ask_user`: (card_id, card_json) — the shell emits an AskUser
    // event so the UI can render the question card. The loop then parks until the answer arrives.
    mut on_ask: impl FnMut(String, String),
) -> CodingResult {
    let sys = system_prompt(sandbox.mode(), kernel.is_some(), pause.delegate_allowed());
    let mut transcript = format!("GOAL:\n{goal}\n\nWorkspace root: {}\n", sandbox.root().display());
    let mut ran_command = false; // did the agent run a build/test command this run?
    let mut nudged = false; // pushed back once for finishing without verifying
    let mut plan_gated = false; // whether the one-time plan-approval gate has fired
    let mut malformed = 0u32; // consecutive un-parseable JSON envelopes (bounded retry, not a fatal "done")
    const MAX_MALFORMED: u32 = 3;
    let mut intake_done = !pause.intake_required(); // satisfied once the forced intake ask has fired
    let mut intake_nudges = 0u32; // bounded pushes to make the first action an ask_user intake card
    let mut all_figures: Vec<String> = Vec::new(); // computed figures across the whole run
    // Automatic kernel resilience: a cell timeout/crash restarts the kernel and WIPES in-memory state
    // (dataframes/adata). We auto-checkpoint after an expensive successful cell and auto-restore after
    // a state-wiping restart, so a heavy analysis survives instead of losing hours of loaded data.
    let mut have_ckpt = false; // a checkpoint file exists to restore from
    let mut cells_since_ckpt = 0u32; // periodic-checkpoint safety net counter
    let mut searched: std::collections::HashSet<String> = std::collections::HashSet::new(); // dedup repeated skill searches
    let mut skill_searched = false; // once true, HARD-block further search_skills until a load_skill (force-commit)
    let mut fig_seen: std::collections::HashSet<String> = std::collections::HashSet::new(); // dedup identical figures (re-run plots)
    let mut recent_errs: Vec<String> = Vec::new(); // last few error signatures — nudge on a repeat of the SAME failure
    let mut lit_queries: Vec<String> = Vec::new(); // litsearch queries so far — nudge for breadth when one angle is over-searched
    let mut web_queries: Vec<String> = Vec::new(); // web_search queries so far — dedup + soft breadth nudge
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
        // Budget meter: the model has no other way to know how many steps remain, so it can't
        // triage. Show step N/total and, in the final ~20%, an explicit wrap-up order so it stops
        // exploring and computes the deliverable instead of getting truncated mid-wrangle.
        let remaining = budget.max_steps.saturating_sub(n - 1);
        let warn = if remaining <= (budget.max_steps / 5).max(3) {
            "\n[budget] Few steps left — STOP exploring. Compute the core result/figure NOW and finish with {\"final\":\"…\"} (a native figure block + a short honest paragraph)."
        } else {
            ""
        };
        // Compact the context so input tokens don't grow ~quadratically over a long run (the
        // full transcript was re-sent verbatim every turn — the bloat that timed out flagship 03).
        // Keep the GOAL header + the most recent steps; elide the middle.
        let ctx = compact_transcript(&transcript, 26_000);
        let user = format!(
            "{ctx}\n[budget] step {n} of {} ({remaining} left).{warn}\n先用1-2句话说出你此刻的思考(你要做什么、为什么、上一步的结果说明了什么),再给出你的 JSON 动作 · First write 1-2 sentences of your thinking in the user's language, THEN your JSON action:",
            budget.max_steps
        );
        // Model calls over a long transcript occasionally time out or hit a transient provider
        // error; a single such blip should NOT kill a multi-minute analysis run. Retry a few times
        // with a short backoff before giving up (cancellation still wins immediately).
        let reply = {
            let mut attempt = 0u32;
            loop {
                match model.complete(&sys, &user, cancel).await {
                    Ok(r) => break r,
                    Err(e) => {
                        attempt += 1;
                        if attempt > 3 || cancel.is_cancelled() {
                            return CodingResult { status: "failed".into(), summary: format!("model error after {attempt} attempts: {e}"), steps: n - 1, figures: all_figures, artifacts: collect_artifacts(sandbox.root(), &start_files) };
                        }
                        // Escalating backoff — a 429/rate-limit window can last tens of seconds, so
                        // the old ~6s total (attempt*5*200ms) never outlasts it and all retries fail
                        // inside the same window. Wait ~2s, ~8s, ~20s across the three retries so a
                        // final call survives a provider rate-limit; cancellation still bails fast.
                        let wait_ms: u64 = [2000, 8000, 20000][(attempt as usize - 1).min(2)];
                        for _ in 0..(wait_ms / 200) {
                            if cancel.is_cancelled() { break; }
                            tokio::time::sleep(Duration::from_millis(200)).await;
                        }
                    }
                }
            }
        };
        let json = extract_json(&reply);
        // The model's reasoning = its reply minus the JSON action it chose this turn.
        let reasoning = match &json {
            Some(j) => reply.replacen(j, "", 1).trim().to_string(),
            None => String::new(),
        };
        // No JSON object at all → the model may be DONE (prose answer) OR it "thought out loud" and
        // forgot the action envelope. Don't silently end the run: nudge it (bounded, shares the
        // malformed budget) to emit one JSON object; only accept the prose as FINAL after the nudge
        // budget is spent. A strong model that narrates its plan then forgets `{...}` used to kill
        // the whole run here.
        let action = match &json {
            Some(j) => j.clone(),
            None => {
                if malformed < MAX_MALFORMED {
                    malformed += 1;
                    transcript.push_str(
                        "\n[system] I couldn't find a JSON action in your reply. If you're still \
                         working, respond with EXACTLY ONE JSON object — a tool call, or \
                         {\"final\":\"…\"} only when the analysis is truly complete (with its figure).\n",
                    );
                    continue;
                }
                return CodingResult { status: "done".into(), summary: reply.trim().to_string(), steps: n, figures: all_figures, artifacts: collect_artifacts(sandbox.root(), &start_files) };
            }
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
        // FORCED INTAKE GATE: GLM (and weaker models) won't call ask_user on their own discretion even
        // when the prompt demands it — so an analysis run's clarifying intake never fired. Enforce it:
        // the FIRST action MUST be an ask_user intake card. Reject anything else and nudge (bounded),
        // then relent so a stubborn model still converges instead of hanging.
        if !intake_done && !pause.asks_disabled() {
            if tool == "ask_user" {
                intake_done = true; // let the match below surface the card + park for the answer
            } else if tool == "final" && intake_nudges >= 2 {
                intake_done = true; // gave up nudging — don't block a finish
            } else if intake_nudges < 2 {
                intake_nudges += 1;
                transcript.push_str(
                    "\n[system] STOP — before ANY other tool, your FIRST action on this analysis MUST be \
                     an ask_user INTAKE card. Emit ONE ask_user with 1-4 questions confirming: the real \
                     research question, what a trustworthy answer would look like, and the data's \
                     provenance (attached vs fetched vs synthetic — it changes how much to trust the \
                     result). Present options NEUTRALLY and pre-select nothing. Emit that ask_user JSON \
                     now — do not run code, read files, or plan first.\n",
                );
                continue;
            } else {
                intake_done = true; // relent after the nudge budget; let this action proceed
            }
        }
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
                // Accept `{"paths":[…]}` to read several files in ONE step (or the single `path`).
                let mut paths: Vec<String> = v.get("paths").and_then(|x| x.as_array())
                    .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                    .unwrap_or_default();
                if paths.is_empty() {
                    let p = arg_str(&v, "path");
                    if !p.is_empty() { paths.push(p.to_string()); }
                }
                if paths.is_empty() {
                    ("read_file".to_string(), "ERROR: read_file needs a `path` (or a non-empty `paths` array)".to_string())
                } else if paths.len() > 1 {
                    let obs = paths.iter()
                        .map(|p| format!("=== {p} ===\n{}", sandbox.read_file(p).unwrap_or_else(|e| format!("ERROR: {e}"))))
                        .collect::<Vec<_>>().join("\n\n");
                    (paths.join(", "), obs)
                } else {
                    let p = &paths[0];
                    (p.clone(), sandbox.read_file(p).unwrap_or_else(|e| format!("ERROR: {e}")))
                }
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
                    // Recovery hint instead of a bare error: the classic thrash is resending a
                    // slightly-different `new` — the real fix is to re-read and widen `old`.
                    Err(e) => format!("ERROR: {e}\n[hint] `old` must match EXACTLY ONE place in the file. Re-read the file, then copy a longer unique `old` snippet — don't just resend a different `new`."),
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
                // FORCE-COMMIT: after the FIRST search (any query), hard-block every further
                // search_skills until a load_skill happens — the flagship traces burned 4–10 steps
                // re-searching (often the identical query). One search is enough to pick a skill.
                let obs = if skill_searched || !searched.insert(q.to_string()) {
                    format!(
                        "You already searched the skill registry — the matching skills are in the \
                         transcript above. STOP searching; call load_skill with a skill NAME now \
                         (or, if none fit, proceed without a skill)."
                    )
                } else {
                    skill_searched = true;
                    let hits = skills::search(q);
                    if hits.is_empty() {
                        format!("(no skill matched '{q}') Available skills:\n{}\nPick one and call load_skill.", skills::catalogue())
                    } else {
                        let list = hits.iter().map(|s| format!("- {} — {}", s.name, s.summary)).collect::<Vec<_>>().join("\n");
                        format!("{list}\n\nNow call load_skill with one of these NAMES (do not search again).")
                    }
                };
                (q.to_string(), obs)
            }
            "load_skill" => {
                let name = arg_str(&v, "name");
                skill_searched = false; // committed to a skill — allow a fresh search later if truly needed
                let obs = match skills::get(name) {
                    // Every card carries the same budget discipline: a first figure fast on a small
                    // slice, then scale — the flagship traces show over-scoping to full real data is
                    // what stops runs from ever reaching the deliverable.
                    Some(s) => format!(
                        "{}\n\nBUDGET RULE: reach a FIRST computed figure on a small/simulated slice within ~6 steps, then scale. \
                         If real data doesn't load cleanly in ≤2 steps, SUBSAMPLE to the size the task names (or simulate it) and proceed — don't get stuck perfecting the load.",
                        s.card
                    ),
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
                // Convergence guards — GLM OVER-searches (a run fired 6 near-identical IDO queries
                // and never reached a figure, while Claude Science converges in 2-3). Decide BEFORE
                // spending an API call: refuse a near-duplicate, and hard-cap total searches so the
                // run converges to the deliverable instead of looping in the literature.
                let n_done = lit_queries.len();
                let is_dup = lit_queries.iter().any(|prev| query_overlap(prev, &q) >= 0.8);
                let obs = if is_dup {
                    format!("[skip] 这条检索与之前某次高度重复,未执行。你已检索 {n_done} 次——够了。现在停止检索,基于已有文献:提出 2-3 个竞争假说、选出最强并说明新颖性、画机理图、给相关→因果→增益(gain-of-function)→阴性对照的分级验证,然后 {{\"final\":…}}. Duplicate query — not executed; STOP searching and produce the deliverable now.")
                } else if n_done >= 10 {
                    lit_queries.push(q.clone());
                    format!("[stop] litsearch 预算已用尽({n_done} 次),现已暂停。立即用已检索到的文献:提出 2-3 个竞争假说、选最强、**用 python+matplotlib 画一张自定义机理示意图(细胞/受体/箭头/图例,不要用抽象 network 块)**、给分级验证方案,然后 {{\"final\":…}}. 不要再检索。litsearch budget spent — it is DISABLED; DRAW the mechanism schematic in matplotlib (not a node-graph), give the verification plan, and finish; do not search again.")
                } else {
                    let hits = match LitSearch::new().search(&q, 5).await {
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
                    // Soft breadth nudge for a same-angle (0.6–0.8) follow-up that still gets to run.
                    let same_angle = lit_queries.iter().any(|prev| query_overlap(prev, &q) >= 0.6);
                    lit_queries.push(q.clone());
                    if same_angle {
                        format!("{hits}\n\n[breadth] 已搜过相近角度。换一个真正不同的机制/竞争解释,或停止检索去构建假说+画图。Search a genuinely DIFFERENT angle next, or stop and produce.")
                    } else {
                        hits
                    }
                };
                (q, obs)
            }
            "web_search" => {
                // General web search (DuckDuckGo) — for anything that is NOT a peer-reviewed paper:
                // official databases, datasets, docs, preprints, news, org pages. Distinct from
                // litsearch (papers) — the decision guide in the prompt tells the model which to use.
                let q = arg_str(&v, "query").to_string();
                let dup = web_queries.iter().any(|p| query_overlap(p, &q) >= 0.85);
                let obs = if dup {
                    "[skip] 这条 web 检索与之前高度重复,未执行。换一个真正不同的角度,或用 web_fetch 打开已找到的链接细读。".to_string()
                } else {
                    web_queries.push(q.clone());
                    match WebClient::new().search(&q, 8).await {
                        Ok(rs) if rs.is_empty() => "(web 检索无结果 — 换个说法,或改用 litsearch(论文)/直接 web_fetch 一个已知 URL)".to_string(),
                        Ok(rs) => rs
                            .iter()
                            .enumerate()
                            .map(|(i, r)| format!("[{}] {}\n    {}\n    {}", i + 1, r.title, r.url, r.snippet))
                            .collect::<Vec<_>>()
                            .join("\n\n"),
                        Err(e) => format!("ERROR: web_search failed: {e}. 可改用 litsearch 或 web_fetch 已知 URL。"),
                    }
                };
                (q, obs)
            }
            "web_fetch" => {
                // Read ONE specific public URL as text (HTML stripped, bounded, SSRF-guarded). Use it
                // to actually READ a page/doc found via web_search or a known database URL. For a
                // structured DATASET (csv/json by URL/accession) prefer python (pandas) so you can parse it.
                let url = arg_str(&v, "url").to_string();
                let obs = if url.trim().is_empty() {
                    "ERROR: web_fetch needs a `url`".to_string()
                } else {
                    match WebClient::new().fetch(&url).await {
                        Ok(t) if t.trim().is_empty() => "(fetched, but no readable text — likely a binary/dynamic page; try a different source or python)".to_string(),
                        Ok(t) => t,
                        Err(e) => format!("ERROR: web_fetch failed: {e}"),
                    }
                };
                (url, obs)
            }
            "delegate" if !pause.delegate_allowed() => {
                // Not deep-research (or already inside a sub-agent) — no delegation; do it yourself.
                ("delegate".to_string(), "[此处不开子智能体委派 —— 请自己完成这个子任务]".to_string())
            }
            "delegate" => {
                // Spawn a bounded multi-agent sub-workflow for ONE complex, separable subtask.
                let task = arg_str(&v, "task").to_string();
                if task.trim().is_empty() {
                    ("delegate".to_string(), "ERROR: delegate needs a `task` — a self-contained subtask worth a whole sub-team.".to_string())
                } else {
                    let obs = run_delegate(model, &task, cancel).await;
                    (task.chars().take(60).collect::<String>(), obs)
                }
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
                    let kc = k.clone();
                    // Blocking kernel call off the async worker. Use the generous CELL timeout (NOT
                    // the tight shell `cmd_timeout_s`) so a slow-but-finite parse finishes and keeps
                    // its loaded state instead of tripping a kernel restart that wipes every dataframe.
                    let timeout = Duration::from_secs(budget.cell_timeout_s.max(60));
                    let run_code = code.clone();
                    let t0 = Instant::now();
                    let obs = match tokio::task::spawn_blocking(move || kc.exec(&run_code, timeout)).await {
                        Ok(Ok(cell)) => {
                            step_figures = cell.figures;
                            let cell_ok = cell.error.is_none();
                            let mut o = cell.stdout;
                            if let Some(err) = cell.error {
                                if !o.is_empty() && !o.ends_with('\n') { o.push('\n'); }
                                o.push_str(&err);
                            }
                            if !step_figures.is_empty() {
                                o.push_str(&format!("\n[{} figure(s) captured into the report]", step_figures.len()));
                            }
                            if o.trim().is_empty() { o = "(ran; no output)".into(); }
                            // AUTO-CHECKPOINT: after a SUCCESSFUL, expensive cell (or every few cells),
                            // snapshot picklable globals so a later kernel restart can recover them.
                            // Best-effort + generous timeout; a checkpoint failure never fails the step.
                            let elapsed = t0.elapsed().as_secs();
                            cells_since_ckpt += 1;
                            if cell_ok && (elapsed >= 15 || cells_since_ckpt >= 4) {
                                let kk = k.clone();
                                match tokio::task::spawn_blocking(move || kk.exec(CHECKPOINT_CODE, Duration::from_secs(300))).await {
                                    Ok(Ok(c)) if c.stdout.contains("checkpointed") => { have_ckpt = true; cells_since_ckpt = 0; }
                                    Ok(Err(e)) if e.to_string().contains("state was reset") => {
                                        // Snapshotting a very large object ITSELF timed out / OOM'd and
                                        // restarted the kernel — so the state we just computed is gone.
                                        // Surface it (don't let the cell's "success" mislead the agent).
                                        o.push_str("\n[自动容错警告] 保存快照时内核超时/内存不足并重启,刚计算出的内存状态已丢失。请把这步拆更小或抽样后重做。");
                                        have_ckpt = false;
                                    }
                                    _ => {}
                                }
                            }
                            o
                        }
                        Ok(Err(e)) => {
                            let es = e.to_string();
                            // A cell timeout/crash restarts the kernel and WIPES memory. AUTO-RESTORE the
                            // last checkpoint so the agent doesn't silently lose adata/dataframes and
                            // doesn't have to do the restore dance by hand (which it kept fumbling).
                            if es.contains("kernel restart") || es.contains("state was reset") || es.contains("kernel had crashed") {
                                if have_ckpt {
                                    let kr = k.clone();
                                    // Only report a recovery if RESTORE_CODE actually SUCCEEDED — a raised
                                    // cell (corrupt checkpoint / un-unpicklable object) restores nothing,
                                    // so telling the agent "state recovered" would be a lie.
                                    let ok = match tokio::task::spawn_blocking(move || kr.exec(RESTORE_CODE, Duration::from_secs(300))).await {
                                        Ok(Ok(c)) if c.error.is_none() && c.stdout.contains("restored") => Some(c.stdout.trim().to_string()),
                                        _ => None,
                                    };
                                    match ok {
                                        Some(restored) => format!(
                                            "ERROR: {es}\n[自动容错] 内核重启后已自动从上一个检查点恢复状态：{restored}。但超时/崩溃的那步操作**没有完成** —— 不要原样重跑那步重活，改成先 SUBSAMPLE/抽样或拆成更小的 cell 分步做。"
                                        ),
                                        None => format!(
                                            "ERROR: {es}\n[自动容错] 内核重启后尝试恢复检查点但**失败了**(检查点可能损坏或对象无法反序列化),内存状态已丢失。请重新加载数据,并把刚才那步拆小/抽样再做。"
                                        ),
                                    }
                                } else {
                                    format!(
                                        "ERROR: {es}\n[自动容错] 内核重启前还没有检查点，所以内存里的数据(adata/DataFrame)丢了。请重新加载数据(加载成功后会被自动检查点保护),并把刚才超时的重活拆小/抽样再做。"
                                    )
                                }
                            } else {
                                format!("ERROR: {es}")
                            }
                        }
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
                    let code = CHECKPOINT_CODE;
                    let k = k.clone();
                    let obs = match tokio::task::spawn_blocking(move || k.exec(code, Duration::from_secs(240))).await {
                        Ok(Ok(cell)) => {
                            if cell.stdout.contains("checkpointed") { have_ckpt = true; cells_since_ckpt = 0; }
                            let mut o = cell.stdout; if let Some(e) = cell.error { o.push_str(&e); } o
                        }
                        Ok(Err(e)) => format!("ERROR: {e}"),
                        Err(e) => format!("ERROR: {e}"),
                    };
                    ("checkpoint".to_string(), obs)
                }
                None => ("checkpoint".to_string(), "ERROR: checkpoint needs the analysis kernel".to_string()),
            },
            "restore" => match &kernel {
                Some(k) => {
                    let code = RESTORE_CODE;
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
            "ask_user" if pause.asks_disabled() => {
                // Headless/secondary step with no UI to answer a card — never park; converge instead.
                ("ask_user".to_string(), "[clarifying questions are disabled in this background step — proceed with sensible, field-standard defaults and DISCLOSE each assumption]".to_string())
            }
            "ask_user" => {
                // Surface a structured clarifying question to the human and PARK until they answer.
                // The chosen answer becomes this tool's observation, feeding straight back to the model.
                let card_id = {
                    let c = arg_str(&v, "card_id");
                    if c.is_empty() { format!("ask-{n}") } else { c.to_string() }
                };
                // Normalize into { questions: [...] } — accept either a `questions` array or a single
                // top-level question, tolerating snake_case + camelCase keys from weaker models.
                let questions = if let Some(qs) = v.get("questions").and_then(|x| x.as_array()) {
                    serde_json::Value::Array(qs.clone())
                } else {
                    let multi = v.get("multi_select").or_else(|| v.get("multiSelect")).and_then(|x| x.as_bool()).unwrap_or(false);
                    let other = v.get("allow_other").or_else(|| v.get("allowOther")).and_then(|x| x.as_bool()).unwrap_or(true);
                    let mut q = serde_json::Map::new();
                    q.insert("header".into(), v.get("header").cloned().unwrap_or_else(|| serde_json::Value::String(String::new())));
                    q.insert("question".into(), serde_json::Value::String(arg_str(&v, "question").to_string()));
                    q.insert("kind".into(), v.get("kind").cloned().unwrap_or_else(|| serde_json::Value::String("fork".into())));
                    q.insert("options".into(), v.get("options").cloned().unwrap_or_else(|| serde_json::Value::Array(vec![])));
                    q.insert("multiSelect".into(), serde_json::Value::Bool(multi));
                    q.insert("allowOther".into(), serde_json::Value::Bool(other));
                    if let Some(w) = v.get("why_it_matters") { q.insert("why_it_matters".into(), w.clone()); }
                    serde_json::Value::Array(vec![serde_json::Value::Object(q)])
                };
                // Refuse to park on a card the UI can't render (no question with text) — otherwise
                // the run would hang behind an invisible card. Nudge the model instead of blocking.
                let renderable = questions.as_array().map(|qs| {
                    qs.iter().any(|q| q.get("question").and_then(|s| s.as_str()).map(|s| !s.trim().is_empty()).unwrap_or(false))
                }).unwrap_or(false);
                if !renderable {
                    ("ask_user".to_string(), "ERROR: ask_user needs at least one question with a non-empty \"question\" field (and 2-4 options). Re-emit a valid card, or proceed with sensible defaults and disclose them.".to_string())
                } else {
                    let card = serde_json::json!({ "card_id": card_id, "questions": questions }).to_string();
                    on_ask(card_id.clone(), card); // shell emits AskUser → UI renders the card
                    let _ = pause.take_answer(); // discard any stale answer
                    // Park until the human answers; cancellation still wins immediately.
                    let answer = loop {
                        if cancel.is_cancelled() { break None; }
                        if let Some(a) = pause.take_answer() { break Some(a); }
                        tokio::time::sleep(Duration::from_millis(150)).await;
                    };
                    match answer {
                        Some(a) => ("ask_user".to_string(), format!("The human answered your question:\n{a}\nUse exactly these choices and record them in the report's methods / assumptions.")),
                        None => ("ask_user".to_string(), "[question dismissed — proceed with sensible, field-standard defaults and DISCLOSE each assumption you make; do NOT re-ask this]".to_string()),
                    }
                }
            }
            other => (other.to_string(), format!("ERROR: unknown tool '{other}'")),
        };
        // Figure dedup: re-running a plot cell (common while tuning) would append duplicate PNGs to
        // the report — keep only figures we haven't already captured this run.
        step_figures.retain(|f| fig_seen.insert(f.clone()));
        all_figures.extend(step_figures.iter().cloned());
        // Error memory: if this observation repeats a failure we saw recently (same signature — the
        // exception line), flag it so the model changes approach instead of blindly retrying (the
        // flagship hit the identical UnicodeDecodeError 3×). Decide BEFORE truncating, and append the
        // nudge AFTER truncation so it can't be cut off the end of a long traceback.
        let repeat_err = match error_signature(&observation) {
            Some(sig) => {
                let rep = recent_errs.iter().any(|s| *s == sig);
                recent_errs.push(sig);
                if recent_errs.len() > 4 { recent_errs.remove(0); }
                rep
            }
            None => false,
        };
        let mut obs = truncate(&observation, 4000);
        if repeat_err {
            obs.push_str("\n[repeat] This is the SAME error you already hit — do NOT resend a near-identical call. Fix the actual cause (encoding, path, query field, dtype) or take a different route.");
        }
        // The full tool input, for the execution log (detail is only a short label).
        let input = match tool.as_str() {
            "python" => arg_str(&v, "code").to_string(),
            "read_file" | "list_dir" | "write_file" | "edit" => arg_str(&v, "path").to_string(),
            "search" | "litsearch" | "search_skills" | "web_search" => arg_str(&v, "query").to_string(),
            "web_fetch" => arg_str(&v, "url").to_string(),
            "delegate" => arg_str(&v, "task").to_string(),
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
        // Plan-approval gate: once the agent has surfaced its FIRST plan, pause (if the run opted in)
        // so the user can approve / edit / reject before any tool executes. The pause takes effect at
        // the top of the next turn; resuming = approve, cancelling = reject. Only gates once.
        if tool == "plan" && pause.approval_required() && !plan_gated {
            plan_gated = true;
            pause.pause();
        }
    }
    let artifacts = collect_artifacts(sandbox.root(), &start_files);
    // Budget exhausted without a {"final"}. Don't dump the raw transcript as the "report" — do ONE
    // wrap-up synthesis so the user still gets a proper writeup of what was actually computed + its
    // figures + an honest note on what's incomplete. Falls back to a transcript tail on failure.
    let wrap_ctx = compact_transcript(&transcript, 26_000);
    let wrap_user = format!(
        "{wrap_ctx}\n\n[你已用完步数预算,不能再调用任何工具] 现在直接写出**最终报告**:把你到目前为止真正计算出的结果和图讲清楚(已生成的图会自动附在报告里,不要再画),说明主要发现与结论,并诚实指出哪些部分还没完成、下一步该做什么。用 Markdown、用用户的语言。只输出报告正文,不要 JSON、不要工具调用。"
    );
    let summary = match model.complete(&sys, &wrap_user, cancel).await {
        Ok(r) if r.trim().chars().count() > 40 => r.trim().to_string(),
        _ => format!(
            "(达到 {} 步预算,分析未完全收尾;已计算出的图见报告。)最近进展:\n{}",
            budget.max_steps,
            truncate(&transcript, 1500)
        ),
    };
    CodingResult { status: "done".into(), summary, steps: budget.max_steps, figures: all_figures, artifacts }
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

/// Bound the context re-sent to the model each turn: if the transcript is longer than
/// `budget_chars`, keep the GOAL header + the most RECENT chars and elide the middle. char-based
/// (never splits a UTF-8 codepoint). Recent steps carry the live state the model needs next; the
/// deep middle is rarely load-bearing, and the final artifact is built from the real trace anyway.
fn compact_transcript(t: &str, budget_chars: usize) -> String {
    let total = t.chars().count();
    if total <= budget_chars {
        return t.to_string();
    }
    // Keep a generous head so the FULL goal (and usually the first plan) survives verbatim, plus a
    // large recent tail; only the deep middle is elided.
    let head_n = 2200usize.min(budget_chars / 3);
    let tail_n = budget_chars.saturating_sub(head_n);
    let head: String = t.chars().take(head_n).collect();
    let tail: String = t.chars().skip(total.saturating_sub(tail_n)).collect();
    format!(
        "{head}\n\n[... {} earlier characters elided to stay within context — the goal above and the recent steps below are kept verbatim ...]\n{tail}",
        total.saturating_sub(head_n + tail_n)
    )
}

/// Jaccard-ish word overlap between two search queries in [0,1] — used to detect when the agent is
/// re-searching the SAME angle instead of broadening. Lowercased, split on whitespace, ignores a few
/// stopwords so "STAT3 HLA-E senescence" vs "STAT3 HLA-E promoter senescence" reads as near-identical.
fn query_overlap(a: &str, b: &str) -> f64 {
    fn toks(s: &str) -> std::collections::HashSet<String> {
        const STOP: &[&str] = &["the", "and", "of", "in", "for", "a", "to", "via", "on", "with"];
        s.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.len() > 2 && !STOP.contains(w))
            .map(|w| w.to_string())
            .collect()
    }
    let (x, y) = (toks(a), toks(b));
    if x.is_empty() || y.is_empty() {
        return 0.0;
    }
    let inter = x.intersection(&y).count() as f64;
    inter / (x.union(&y).count() as f64)
}

/// If `obs` reads as an error, return a stable signature (the Python exception line, e.g.
/// `UnicodeDecodeError: …`, else the first line) so the loop can detect the SAME failure recurring.
/// None when it isn't an error.
fn error_signature(obs: &str) -> Option<String> {
    let is_err = obs.starts_with("ERROR") || obs.contains("Traceback (most recent call last)");
    if !is_err {
        return None;
    }
    // The last "Error:" line in a traceback is the actual exception — the most specific signature.
    let line = obs.lines().rev().find(|l| l.contains("Error:")).or_else(|| obs.lines().next());
    line.map(|l| l.trim().chars().take(80).collect())
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
        let res = run_coding_agent("review main.rs", &sb, &model, &Cancel::new(), &Pause::new(), &CodingBudget::default(), None, |_| steps += 1, |_, _| {}).await;
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
        let res = run_coding_agent("x", &sb, &model, &Cancel::new(), &Pause::new(), &CodingBudget::default(), None, |_| steps += 1, |_, _| {}).await;
        assert_eq!(res.status, "done", "summary: {}", res.summary);
        assert!(res.summary.contains("looked at the dir"), "must reach the post-nudge final, got: {}", res.summary);
        assert_eq!(steps, 1, "only the valid list_dir ran as a step (malformed reply executed nothing)");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn plan_approval_gate_pauses_until_resumed() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let root = tmp("gate");
        let sb = Sandbox::new(&root).unwrap();
        let model = ScriptModel {
            replies: Mutex::new(vec![
                r#"{"tool":"plan","todos":["[ ] look","[ ] done"]}"#.into(),
                r#"{"tool":"list_dir","path":"."}"#.into(),
                r#"{"final":"looked"}"#.into(),
            ]),
        };
        let pause = Pause::new();
        pause.require_plan_approval();
        let pause2 = pause.clone();
        // Watcher: once the gate has paused the run (after the first plan), approve by resuming.
        let approved = std::sync::Arc::new(AtomicBool::new(false));
        let approved2 = approved.clone();
        let watcher = tokio::spawn(async move {
            for _ in 0..300 {
                if pause2.is_paused() {
                    approved2.store(true, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(60)).await; // let the loop actually wait
                    pause2.resume(); // approve
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });
        let res = run_coding_agent("x", &sb, &model, &Cancel::new(), &pause, &CodingBudget::default(), None, |_| {}, |_, _| {}).await;
        watcher.await.unwrap();
        assert!(approved.load(Ordering::SeqCst), "the gate must have paused the run after the first plan");
        assert_eq!(res.status, "done", "summary: {}", res.summary);
        assert!(res.summary.contains("looked"), "run must continue after approval, got: {}", res.summary);
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
        let res = run_coding_agent("make hello.txt", &sb, &model, &Cancel::new(), &Pause::new(), &CodingBudget::default(), None, |_| {}, |_, _| {}).await;
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
        let res = run_coding_agent("try to write", &sb, &model, &Cancel::new(), &Pause::new(), &CodingBudget::default(), None, |_| {}, |_, _| {}).await;
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
        let res = run_coding_agent("x", &sb, &model, &cancel, &Pause::new(), &CodingBudget::default(), None, |_| {}, |_, _| {}).await;
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
            |_, _| {},
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
