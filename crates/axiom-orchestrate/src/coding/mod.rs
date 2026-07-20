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
use axiom_runtime::scitools::{is_sci_tool, run_sci_tool, sci_tool_arg, sci_tools};
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
    /// Every external source the agent RETRIEVED during the run (papers, web pages, database
    /// records), each with a stable [Sn] id + an excerpt — so a report's [Sn] markers are clickable
    /// and trace back to the original text. Provenance, never a truth verdict.
    pub sources: Vec<Source>,
}

/// One retrieved external source, addressable by its `[S{id}]` marker in the report.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Source {
    pub id: u32,
    pub kind: String, // "paper" | "web" | "db"
    pub title: String,
    pub url: String,     // DOI url / page url / db record url ("" if none)
    pub excerpt: String, // the abstract / page text / record — the "original passage" to trace to
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
    /// Long-form completion for the make-or-break FINAL REPORT. A full report (long output + heavy
    /// hidden reasoning) routinely needs far more wall-clock than a per-step call, so this variant
    /// gets a generous timeout. Default delegates to `complete` (offline test models don't care).
    fn complete_report(
        &self,
        system: &str,
        user: &str,
        cancel: &Cancel,
    ) -> impl std::future::Future<Output = anyhow::Result<String>> + Send {
        self.complete(system, user, cancel)
    }
}

/// Real model: one text completion per turn (history is flattened into `user` since the
/// text seam keeps no history), wrapped in the fleet's retry + timeout template.
pub struct ProviderModel {
    pub provider: String,
    pub model: String,
    pub timeout_s: u64,
}
impl ProviderModel {
    /// Shared completion body, parameterized by the timeout so per-step calls stay tight while the
    /// final report gets a generous budget.
    fn run_timeout(
        &self,
        system: &str,
        user: &str,
        timeout_s: u64,
    ) -> impl std::future::Future<Output = anyhow::Result<String>> + Send {
        let provider_s = self.provider.clone();
        let model = self.model.clone();
        let system = system.to_string();
        let user = user.to_string();
        async move {
            let req = Request {
                system: Some(system),
                messages: vec![Message::user(vec![ContentBlock::text(user)])],
                model,
                // Generous output budget — GLM-5.2 is a reasoning model that spends most tokens on
                // hidden reasoning, so a small/absent cap truncated long reports before the answer.
                // Quality-first: 32k leaves ample room for heavy reasoning PLUS a long final report.
                max_tokens: Some(32000),
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
impl CodingModel for ProviderModel {
    fn complete(
        &self,
        system: &str,
        user: &str,
        _cancel: &Cancel,
    ) -> impl std::future::Future<Output = anyhow::Result<String>> + Send {
        self.run_timeout(system, user, self.timeout_s)
    }
    fn complete_report(
        &self,
        system: &str,
        user: &str,
        _cancel: &Cancel,
    ) -> impl std::future::Future<Output = anyhow::Result<String>> + Send {
        // The final report needs generous wall-clock — at LEAST 5 min — no matter how tight the
        // per-step timeout is, so a long GLM-5.2 report isn't cut off and forced to the fallback.
        self.run_timeout(system, user, self.timeout_s.max(300))
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

// CROSS-PLATFORM: darwin/linux verified; the Windows (win-64) branch is written but must be
// re-verified on a real Windows box (micromamba there ships as micromamba.exe under Library/bin,
// and a conda env's executables live in the env root + Scripts + Library/bin, not bin/).
const CONDA_INSTALL: &str = r#"import os, sys, subprocess, platform, tarfile, io, requests, time
_win = platform.system().lower() == 'windows'
ROOT = os.path.expanduser('~/.axiom3'); MM = os.path.join(ROOT, 'bin', 'micromamba.exe' if _win else 'micromamba'); ENV = os.path.join(ROOT, 'bioenv')
os.makedirs(os.path.join(ROOT, 'bin'), exist_ok=True); os.makedirs('.axiom_jobs', exist_ok=True)
if not os.path.exists(MM):
    _s = platform.system().lower(); _m = platform.machine().lower()
    if _s == 'darwin': _key = 'osx-arm64' if _m in ('arm64','aarch64') else 'osx-64'
    elif _s == 'linux': _key = 'linux-aarch64' if _m in ('aarch64','arm64') else 'linux-64'
    elif _s == 'windows': _key = 'win-64'
    else: raise SystemExit('unsupported platform ' + _s + ' ' + _m)
    _d = None; _err = None
    for _try in range(3):   # mainland networks can drop the CDN mid-fetch; retry before giving up
        try:
            _d = requests.get('https://micro.mamba.pm/api/micromamba/%s/latest' % _key, timeout=300).content; break
        except Exception as _e:
            _err = _e; time.sleep(3)
    if _d is None: raise SystemExit('micromamba download failed after 3 tries: ' + str(_err))
    with tarfile.open(fileobj=io.BytesIO(_d), mode='r:bz2') as _tf:
        _mem = next(x for x in _tf.getmembers() if x.name.replace('\\','/').endswith('bin/micromamba') or x.name.replace('\\','/').endswith('bin/micromamba.exe'))
        _mem.name = os.path.basename(_mem.name); _tf.extract(_mem, os.path.join(ROOT, 'bin'))
    try: os.chmod(MM, 0o755)
    except OSError: pass
_pkgs = __PKGS__
# CACHE SHORT-CIRCUIT: if EVERY requested package is already in the bioenv, skip entirely. A re-install
# re-loads conda-forge repodata (tens of seconds → minutes) even when there's nothing to do — this is
# the "又在安装 / 正在加载 repodata" the user saw on a second structure task. `micromamba list` is a
# fast LOCAL read (no network), so we check it first and mark the job done without touching the network.
try:
    if os.path.exists(MM):
        _out = subprocess.run([MM, 'list', '-p', ENV], capture_output=True, text=True, timeout=60).stdout
        _installed = set()
        for _ln in _out.splitlines():
            if _ln and not _ln.startswith('#'):
                _tok = _ln.split()
                if _tok:
                    _installed.add(_tok[0].lower())
        def _base(_p):
            for _sep in ('==', '>=', '<=', '=', '>', '<', ' '):
                _p = _p.split(_sep)[0]
            return _p.strip().lower()
        _names = [_base(_p) for _p in _pkgs if _base(_p)]
        if _names and all(_n in _installed for _n in _names):
            open(os.path.join('.axiom_jobs', '__ID__.status'), 'w').write('done')
            print('[cached] 依赖已在本地环境中,跳过安装(直接可用):' + ', '.join(_names))
            raise SystemExit(0)
except SystemExit:
    raise
except Exception:
    pass
# `micromamba install -p ENV` creates the env if it's missing, so retrying across channel sets is
# safe. Try the China (TUNA) mirror FIRST — conda-forge/bioconda direct are slow/blocked on mainland
# networks (this is what made PyMOL installs error + poll forever) — then fall back to the official
# channels. Robust in every region: the mirror is reachable abroad too, and the fallback covers an
# outage. Each attempt is hard-capped at 30 min so a stalled network fails cleanly instead of looping.
_CN = 'https://mirrors.tuna.tsinghua.edu.cn/anaconda/cloud'
_mir = [MM, 'install', '-y', '-p', ENV, '-c', _CN + '/conda-forge', '-c', _CN + '/bioconda'] + _pkgs
_off = [MM, 'install', '-y', '-p', ENV, '-c', 'conda-forge', '-c', 'bioconda'] + _pkgs
# STREAM micromamba's output straight to the job log (NOT capture_output, which buffers until the
# whole install finishes → the log stayed EMPTY for the several minutes a cold PyMOL download takes,
# so job_status showed "RUNNING + empty log" and the agent wrongly concluded it had hung and gave up).
# Streaming means the log shows live download/link progress, so a long-but-healthy install is visible.
_wrap = (
    "import subprocess, sys, os\n"
    "CF = 0x08000000 if os.name == 'nt' else 0   # CREATE_NO_WINDOW: no console flash on Windows\n"
    "for _i, _cmd in enumerate([%r, %r]):\n"
    "    sys.stdout.write('>>> conda install attempt %%d (live micromamba output follows)\\n' %% (_i+1)); sys.stdout.flush()\n"
    "    try:\n"
    "        rc = subprocess.run(_cmd, stdout=sys.stdout, stderr=subprocess.STDOUT, timeout=1800, creationflags=CF).returncode\n"
    "    except subprocess.TimeoutExpired:\n"
    "        sys.stdout.write('attempt %%d timed out after 30 min\\n' %% _i); sys.stdout.flush(); continue\n"
    "    sys.stdout.flush()\n"
    "    if rc == 0:\n"
    "        open('.axiom_jobs/__ID__.status', 'w').write('done'); break\n"
    "    sys.stdout.write('\\n--- channel set %%d failed (rc=%%d), trying next ---\\n' %% (_i, rc)); sys.stdout.flush()\n"
    "else:\n"
    "    open('.axiom_jobs/__ID__.status', 'w').write('failed')\n"
) % (_mir, _off)
open('.axiom_jobs/__ID__.py', 'w').write(_wrap)
try: os.remove('.axiom_jobs/__ID__.status')
except OSError: pass
_flags = 0x08000008 if _win else 0   # DETACHED_PROCESS | CREATE_NO_WINDOW (no black console flash on Windows)
_p = subprocess.Popen([sys.executable, '.axiom_jobs/__ID__.py'], stdout=open('.axiom_jobs/__ID__.log', 'w'), stderr=subprocess.STDOUT, start_new_session=not _win, creationflags=_flags)
_eb = os.pathsep.join([ENV, os.path.join(ENV,'Scripts'), os.path.join(ENV,'Library','bin')]) if _win else os.path.join(ENV, 'bin')
os.environ['PATH'] = _eb + os.pathsep + os.environ.get('PATH', '')
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
         [x], so the plan stays current. LOCK THE CONTRACT FIRST: make TODO #1 restate the user's \
         ORIGINAL question in one line AND name the SPECIFIC deliverables it demands — the exact answer/ \
         verdict, and which figures — so you never drift into a tangential angle. Your final report is \
         checked against this contract before it's accepted.)\n\
         {\"tool\":\"read_file\",\"path\":\"<relative path>\"}   (or {\"paths\":[\"a\",\"b\"]} to read several in ONE step)\n\
         {\"tool\":\"list_dir\",\"path\":\"<relative path>\"}\n\
         {\"tool\":\"glob\",\"pattern\":\"<glob, e.g. src/**/*.rs>\"}   (find files by name/pattern)\n\
         {\"tool\":\"search\",\"query\":\"<text to grep for>\"}\n\
         ── 如何收尾(最终报告 · 重要)──\n\
         当且仅当任务真正完成时,**不要**把报告包进 JSON。请这样收尾:先单独一行写:\n\
         ===AXIOM3_FINAL===\n\
         紧接着直接写出**完整的报告正文(裸 Markdown)**——正常换行、正常标点、正常引号,**不转义、不放进 JSON 字符串**。\
         标记行之后的全部内容都会被当作最终报告。(一份长报告无法可靠地转义进 {\"final\":\"…\"} 字符串——用这个标记就彻底避开了转义问题。)\n\
         WHEN DONE, do NOT wrap the report in JSON. Output a line with exactly ===AXIOM3_FINAL=== then your \
         FULL report as plain Markdown (real newlines, no escaping, NOT inside a JSON string).\n",
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
        // scidb is ALWAYS on the kernel PYTHONPATH — but the model won't use it unless the BASE prompt
        // says so; without this, agents hand-rolled a database's GraphQL/REST and burned many turns
        // fighting its schema (observed on Open Targets). Surface it up front with the hard rule.
        s.push_str(
            "\n── PRE-INSTALLED DATABASE TOOLKIT (`import scidb`) — USE IT, don't hand-roll DB APIs ──\n\
             A `scidb` module is already on the kernel path (no pip needed). Every call returns a REAL \
             record with a source_url to cite [Sn]; on failure it returns {'error':…} (surface it, never \
             fabricate). **RULE: if scidb has a function for a database, CALL IT — do NOT hand-write that \
             database's GraphQL/REST/urllib yourself (you will waste many turns fighting its schema).** \
             Functions: scidb.lit_search / pubmed / crossref (literature); scidb.uniprot / pdb / pdb_search / \
             alphafold / interpro(domains) / protein_atlas(expression) (protein/structure); scidb.pubchem / chembl / chembl_activity (chemistry); scidb.gene / \
             ensembl / geo / entrez_search (genomics); scidb.kegg / reactome / string_db (pathways); \
             scidb.opentargets(gene) (target→disease scores); scidb.clinvar / gnomad(variant_id) / \
             gwas_catalog(gene) (clinical variants/genetics); scidb.clinical_trials(query); scidb.blast(seq); \
             and scidb.rest(url, params) for ANY other public REST DB (provenance-tracked). Run \
             `scidb.catalog()` once to see them all. Prefer a real scidb record over a remembered fact.\n\
             **溯源铁律:要在报告里用 [Sn] 引用一条 scidb 取到的记录,必须先用 `scidb.cite(记录)` 登记 \
             (可链式:`rows = scidb.cite(scidb.chembl(query=...))`、`scidb.cite(scidb.pdb('7XKJ'))`)—— \
             它会回一行『已登记…可引用来源:[Sn]…』,你只能引用这些真实登记过的 [Sn]。文献同理:用 \
             scidb.lit_search 后对要引用的条目 scidb.cite 一下。native litsearch/multi_search 工具已自动登记, \
             无需再 cite。⚠️ 绝不要自己编 [Sn] 编号、也绝不要写『检索结果:…关键文献包括…』这种打包式引用—— \
             凡没有真实登记来源的 [Sn] 会被系统当死链剥掉。**\n\
             {\"tool\":\"python\",\"code\":\"<python source>\"}   (a PERSISTENT Python kernel: \
             variables/dataframes/models stay in memory across python calls, so build up an analysis \
             step by step. stdout and the last expression's value come back to you; any matplotlib \
             figure you create is CAPTURED into the report — do `import matplotlib.pyplot as plt` and \
             draw with it. For PUBLICATION-GRADE figures, set a clean style ONCE at the top of your first \
             plotting cell — `import matplotlib as mpl; mpl.rcParams.update({'figure.dpi':140,'savefig.dpi':140,\
             'font.size':11,'axes.titlesize':12.5,'axes.labelsize':11,'axes.spines.top':False,\
             'axes.spines.right':False,'axes.grid':True,'grid.alpha':0.22,'legend.frameon':False,\
             'figure.autolayout':True})` — then label axes WITH UNITS, give a concise title, add a legend \
             for >=2 series, and use a restrained colorblind-safe palette (tab10 / a seaborn palette). \
             ANTI-COLLISION (a crowded header/labels reads as sloppy): put the legend OUTSIDE the axes \
             (`legend(loc='upper left', bbox_to_anchor=(1.02,1))` or below), never on top of the plot; for a \
             RADAR / polar chart reserve top margin (`subplots_adjust(top=0.82)`) and wrap a long title so the \
             title, legend and the top axis label don't overlap — this is the #1 radar defect; for a TIMELINE / \
             scatter with many point labels, stagger them (alternate vertical offsets, or rotate ~30°, or use \
             adjustText) so author/event labels don't pile on top of each other. \
             LABELS IN THE USER'S LANGUAGE: for Chinese/CJK labels set a CJK font FIRST — \
             `mpl.rcParams['font.sans-serif']=['PingFang SC','Heiti TC','Arial Unicode MS','DejaVu Sans']; \
             mpl.rcParams['axes.unicode_minus']=False` — then verify no missing-glyph boxes; only if no CJK \
             font is available fall back to English on the figure. \
             NO BLANK FIGURES — self-check + redraw: a fetch or a curve-fit can return NOTHING, and an \
             axes with no curve/scatter/bars ships as an empty, scaffold-only figure (this has happened: \
             blank dose-response grids). BEFORE plotting, confirm the data is non-empty; if you plot from \
             empty data the kernel DROPS the figure and prints `[axiom] ⚠ 丢弃了 …空图` — do NOT ignore that: \
             fix the data (re-fetch / correct the query / recompute the fit) and REDRAW. If a specific group \
             truly has no data, annotate that panel `无数据 / no data` instead of leaving it blank, and \
             `plt.close()` any half-built or failed figure before drawing the final one. \
             STRUCTURE figures (a PROTEIN / enzyme / complex / active site / mutation / binding pocket / \
             fold): NEVER hand-draw these in matplotlib — matplotlib physically cannot render a cartoon \
             ribbon or ray-trace, and the attempt always looks amateurish. Instead `load_skill` \
             `protein-structure-figure` and follow it: it installs headless PyMOL via `conda_install` and \
             ray-traces a publication-grade panel from a REAL PDB entry, with every distance MEASURED from \
             the coordinates. Whenever the subject of a figure is a 3-D structure, this is the route. \
             SMALL-MOLECULE / DRUG / LIGAND figures (a 2-D chemical structure, an inhibitor series, a \
             pharmacophore/warhead/scaffold highlight, computed descriptors): `load_skill` \
             `small-molecule-figure` — it `pip_install`s RDKit and draws proper stereochemistry with \
             computed MW/logP/TPSA, far better than the inline ```smiles block when you need a labelled, \
             multi-molecule, descriptor-annotated figure. GENE-CONTEXT figures (an operon, a resistance \
             cassette, a mobile genetic element, a gene neighbourhood / plasmid map): `load_skill` \
             `gene-context-figure` — it `pip_install`s dna_features_viewer and draws strand-aware gene \
             arrows on a ruler. \
             MECHANISM / PATHWAY / CONCEPT diagrams: PREFER a NATIVE figure block over hand-drawn \
             matplotlib — the native renderers emit clean publication-grade SVG, whereas hand-placing \
             matplotlib patches (Ellipse/FancyArrowPatch/…) reliably looks amateurish. Pick by what the \
             diagram IS: a signalling / regulatory / causal PATHWAY or process FLOW → ```mermaid (e.g. \
             `graph LR; A[受体激活]-->B[下游信号]; B-->|抑制|C[凋亡]`); a map of entities with \
             ACTIVATION vs INHIBITION edges → ```network (nodes + edges, tag each edge `\"sign\":\"+\"` or \
             `\"-\"`); a CHEMICAL reaction / transformation → ```reaction; a SIMPLE molecular structure → \
             ```smiles — BUT the SMILES MUST be a real, valid string copied from PubChem/ChEMBL, NEVER \
             written from memory: a malformed SMILES (e.g. a wrong ring size) renders as '无法解析该 SMILES'. \
             For any DRUG / INHIBITOR / non-trivial molecule, or if you are at all unsure the SMILES is \
             exactly right, use the `small-molecule-figure` skill instead — RDKit VALIDATES it \
             (`assert Chem.MolFromSmiles(smi) is not None`) and draws real stereochemistry, so it cannot \
             silently ship a wrong structure. Give real, specific node labels in the user's language — never a generic \
             dots-and-lines placeholder. ONLY when no native block can express the idea (e.g. a spatial \
             cell-membrane cartoon) fall back to a matplotlib schematic, drawn like a clean Nature/Cell \
             illustration: cells = soft-filled ellipses (not rectangles), receptors = small rounded boxes on \
             the membrane, ligands = small circles, activation = curved `FancyArrowPatch(...arrowstyle='-|>'\
             ...)`, inhibition = blunt '-[', a muted pastel palette, generous whitespace, `ax.axis('off')`. \
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
             returned id; once DONE the binaries are on PATH, call them from python with subprocess. \
             PATIENCE: a first-time PyMOL install downloads ~200MB and can take 3-5 minutes — as long as \
             job_status returns RUNNING and its log tail is ADVANCING (download / extract / link lines), \
             it is HEALTHY, so keep polling; do NOT conclude it has hung or fall back to a lesser figure \
             while it is still progressing.)\n\
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
             {\"tool\":\"multi_search\",\"queries\":[\"<q1>\",\"<q2>\",\"<q3>\"]}   (PREFERRED when you have \
             several INDEPENDENT literature queries — runs them ALL AT ONCE (concurrently) and returns the \
             combined hits in ONE step, far faster than firing litsearch one-by-one. Give 2-4 genuinely \
             different angles up front; results carry the same [Sn] citations. Use single litsearch only \
             when the next query depends on what the last one returned.)\n\
             {\"tool\":\"extract_evidence\"}   或 {\"tool\":\"extract_evidence\",\"ids\":[3,7,12]}   \
             (★**把已登记来源榨成定量证据表**。它会对每条来源的原文摘录跑一次专门的抽取,回给你形如 \
             `[Sn] | 结论 | 数字+单位 | 人群/n | 终点 | 证据等级` 的行。**检索完、动笔前先调用它**——\
             深度报告的价值在于具体数字(样本量、效应量 HR/OR/DC50/IC50、终点 mPFS/OS/ORR、剂量、毒性等级、\
             p/95%CI),而不是「X 等报道了…」这类书目式转述。写正文时只用表里的数字,每个数字紧跟它的 [Sn]。\
             不传 ids 就抽全部已登记来源。)\n\
             {\"tool\":\"web_search\",\"query\":\"<search terms>\"}   (search the WHOLE WEB, not just \
             papers — official databases (GEO/SRA/UniProt/PDB/Ensembl/PubChem), datasets, tool docs, \
             preprints, standards, org/news pages. Returns titles + URLs + snippets. Use it to FIND a \
             source or dataset you don't yet have a URL for.)\n\
             {\"tool\":\"web_fetch\",\"url\":\"<https URL>\"}   (READ one public URL as text — an article, \
             a database record page, a documentation page you found via web_search. Returns the page's \
             cleaned text. Only public http(s) URLs.)\n\
             {\"tool\":\"multi_fetch\",\"urls\":[\"<url1>\",\"<url2>\",\"<url3>\"]}   (PREFERRED when you have \
             SEVERAL URLs to read — fetches them ALL AT ONCE (concurrently) in ONE step, far faster than \
             web_fetch one-by-one. Give 2-6 URLs you already have from web_search / a database result. Use \
             single web_fetch only when the next URL depends on what the last page said.)\n\
             ── WHICH TOOL WHEN (pick the RIGHT one each step) ──\n\
             • Peer-reviewed papers / citations → litsearch.  • Anything else on the web (databases, \
             datasets, docs, preprints, orgs) you need to FIND → web_search.  • READ a specific page you \
             already have a URL for → web_fetch.  • DOWNLOAD + PARSE a structured dataset (CSV/TSV/JSON, \
             an accession, an FTP/HTTPS data file) or COMPUTE anything → python (pandas/urllib), NOT \
             web_fetch (which returns prose text, not a parsed table).  • A domain has a known pipeline → \
             search_skills + load_skill FIRST.  Don't write python to scrape a web page when web_fetch \
             does it; don't web_fetch a data table when python can parse it.\n\
             ── CITATIONS (traceable provenance) ── Every result from litsearch / web_search / web_fetch / \
             a database tool is tagged with a source id like [S3]. When you state a fact, finding, or claim \
             that came from a source, put its marker at the END of that sentence, e.g. \"…下调 PD-L1 [S3]。\" \
             Use the EXACT [Sn] the tool returned, cite only sources you actually retrieved, and never \
             invent a marker. These markers become CLICKABLE and let the reader trace back to the original \
             passage — citing precisely is part of the rigor.\n\
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
             Finish ONLY when the analysis is actually complete and its figures are drawn — and finish with the \
             ===AXIOM3_FINAL=== marker + your report as raw Markdown (NOT a {\"final\":…} JSON string, which \
             can't reliably escape a long report).\n\
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
             ROC/PR, volcano, heatmaps, embeddings…), emit a NATIVE figure block filled with your computed \
             values (these render as clean SVG). Native blocks are ALSO the preferred way to draw a \
             mechanism/pathway (```mermaid / ```network / ```reaction — see above). Use these EXACT shapes: \
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
             ```protein <4-char PDB id like 1CRN, or inline PDB text>  ← renders an INTERACTIVE, rotatable \
             3-D viewer INSTANTLY, no install. MATCH THE TOOL TO WHAT THE USER ASKED FOR (don't reflexively \
             pick one): if they want to SEE / DISPLAY / VISUALIZE / EXPLORE a 3-D structure → ```protein (just \
             emit ```protein\\n<PDB id>\\n```); if they specifically want a publication-grade ANNOTATED STATIC \
             figure (a labeled active-site close-up, measured distances) → the PyMOL `protein-structure-figure` \
             skill; if they want structural ANALYSIS/COMPUTATION (contacts, RMSD, pockets) → compute it, a \
             figure may not even be needed. Read the user's actual intent first; \
             ```smiles <one VALID SMILES copied from PubChem/ChEMBL — NOT from memory; a malformed one \
             renders as '无法解析该 SMILES'; for drugs/inhibitors prefer the RDKit small-molecule-figure skill>; \
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
    if has_kernel {
        // Auto-listed from the science-tool registry — add a tool = add a registry entry, no prompt edit.
        let mut db = String::from(
            "\n── SPECIALIZED SCIENCE DATABASES (first-class tools — call these to look up an entity \
             precisely; prefer them over hand-writing a fetch, and DON'T fabricate an id/SMILES/accession) ──\n",
        );
        for t in sci_tools() {
            db.push_str("             ");
            db.push_str(t.desc);
            db.push('\n');
        }
        s.push_str(&db);
    }
    if can_delegate {
        s.push_str(
            "\n{\"tool\":\"delegate\",\"task\":\"<a self-contained, non-trivial SUBTASK>\"}   (DEEP RESEARCH \
             only, and SPARINGLY — a sub-team is EXPENSIVE, adding many minutes. **Most topics need ZERO \
             delegation**: a single-focus task — SHOW/RENDER/COMPUTE/ANALYZE one thing (display a protein \
             structure, plot one dataset, compare a fixed set of items, answer one focused question) — you \
             do DIRECTLY and finish FAST; do NOT open a sub-team for it. Delegate ONLY when the topic \
             genuinely splits into ≥2 INDEPENDENT hard research branches that each need their own deep \
             literature/web dig (e.g. 'survey all proposed mechanisms of X across fields' AND 'weigh the 4 \
             competing hypotheses for Y' — two separate deep digs). Decompose first, delegate only the hard \
             branches, never the whole task. HARD LIMIT: at most 2 delegations per run.)",
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

/// The TOOL-USING delegate (upgrade of `run_delegate`): decompose the subtask into ≤2 angles and give
/// EACH to a focused sub-agent that runs the FULL tool loop (litsearch / web_search / web_fetch /
/// python / figures), then synthesize their reports. Sub-agents SHARE the parent's sandbox + kernel
/// (the parent is parked awaiting, so no concurrent kernel access), get a bounded budget, and CANNOT
/// delegate again (their Pause never sets delegate_allowed → depth guard, no infinite recursion).
///
/// The recursion into the generic `run_coding_agent` is made finite two ways: the sub-agent's
/// callbacks are passed as `&mut (dyn FnMut + Send)` trait objects (so every level instantiates the
/// SAME closure types — monomorphization converges) and the recursive call is `Box::pin`'d (so the
/// future has a fixed size instead of containing itself). Returns (observation, sub-agent figures).
/// Rewrite `[Sk]` markers to `[S(k+offset)]` (char-safe — Chinese text is multi-byte — and
/// dependency-free). Lifts a delegated sub-agent's LOCAL source ids (which restart at 1 per
/// sub-agent) into the parent's global `[Sn]` space, so the sub-agent's real retrieved sources
/// become correctly-attributed, clickable citations in the report instead of colliding with the
/// parent's (or sibling sub-agents') ids. A `[S…]` with no digit run or no closing `]` is left
/// verbatim; `offset == 0` is a no-op (the local ids are already the global ids).
fn remap_cite_markers(s: &str, offset: u32) -> String {
    if offset == 0 {
        return s.to_string();
    }
    let cs: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < cs.len() {
        if cs[i] == '[' && i + 2 < cs.len() && cs[i + 1] == 'S' && cs[i + 2].is_ascii_digit() {
            let mut j = i + 2;
            while j < cs.len() && cs[j].is_ascii_digit() {
                j += 1;
            }
            if j < cs.len() && cs[j] == ']' {
                let k: u32 = cs[i + 2..j].iter().collect::<String>().parse().unwrap_or(0);
                if k > 0 {
                    out.push_str(&format!("[S{}]", k + offset));
                    i = j + 1;
                    continue;
                }
            }
        }
        out.push(cs[i]);
        i += 1;
    }
    out
}

pub async fn delegate_tooled(
    model: &impl CodingModel,
    sandbox: &Sandbox,
    kernel: &Option<Arc<PyKernel>>,
    sub_budget: &CodingBudget,
    cancel: &Cancel,
    task: &str,
    base: u32, // parent's current src_n — sub-agent sources are renumbered from base+1 upward
    // Lifecycle sink: emitted once per sub-agent as a sentinel step (tool "agent_spawn"/"agent_done",
    // `input` = a unique id, `detail` = a display name) so the shell can surface each delegated expert
    // as its OWN live agent (its own status lamp), not hidden inside the parent's step stream.
    on_life: &mut (dyn FnMut(CodingStepRec) + Send),
) -> (String, Vec<String>, Vec<Source>) {
    let decomp = model
        .complete(
            "把给定的研究子任务拆成 1-2 个可以各自独立、用工具(检索/联网/计算)深挖的角度,每行一个,只输出角度本身。若本身已足够聚焦,就只输出一行。",
            &format!("子任务:\n{task}"),
            cancel,
        )
        .await
        .unwrap_or_default();
    let mut angles: Vec<String> = decomp
        .lines()
        .map(|l| l.trim().trim_start_matches(|c: char| c.is_ascii_digit() || matches!(c, '-' | '*' | '·' | '.' | ')' | '(' | ' ' | '、')).trim().to_string())
        .filter(|l| l.chars().count() > 3)
        .take(2)
        .collect();
    if angles.is_empty() {
        angles.push(task.to_string());
    }
    let mut reports: Vec<String> = Vec::new();
    let mut figs: Vec<String> = Vec::new();
    // Renumber-merge accumulators: every sub-agent's real retrieved sources are lifted into the
    // PARENT's global [Sn] id space (offset by ids already consumed) so delegated findings keep
    // clickable, correctly-attributed provenance — no collision with the parent or sibling sub-agents.
    let mut merged: Vec<Source> = Vec::new();
    let mut next = base; // ids consumed so far (parent + earlier sub-agents); this sub's offset
    for (i, a) in angles.iter().enumerate() {
        if cancel.is_cancelled() {
            break;
        }
        // Announce this expert's START so it lights up as its own live agent in the UI.
        let sub_id = format!("sub-{base}-{i}");
        let short: String = a.chars().take(22).collect();
        on_life(CodingStepRec {
            n: 0,
            tool: "agent_spawn".into(),
            detail: format!("子专家 {}·{}", i + 1, short),
            observation: String::new(),
            input: sub_id.clone(),
            reasoning: String::new(),
            figures: Vec::new(),
        });
        let sub_pause = Pause::new();
        sub_pause.disable_asks(); // headless sub-agent; delegate_allowed NOT set → cannot re-delegate
        let sub_goal = format!(
            "你是深度研究团队里的一个专注子智能体,负责下面这一个子问题。请**用工具**(litsearch 找论文、web_search 找其它来源、web_fetch 读网页、python 计算/画图)去查证与深挖,不要只凭记忆:\n【子问题】{a}\n\n【总课题背景】{task}\n\n深入、有据、带引用地攻坚它,最后给出这一子问题的清晰结论。"
        );
        let mut log: Vec<String> = Vec::new();
        let mut on_s = |r: CodingStepRec| {
            if log.len() < 40 {
                log.push(format!("· {} {}", r.tool, r.detail.chars().take(48).collect::<String>()));
            }
        };
        let mut on_a = |_: String, _: String| {};
        let sd: &mut (dyn FnMut(CodingStepRec) + Send) = &mut on_s;
        let ad: &mut (dyn FnMut(String, String) + Send) = &mut on_a;
        let sub = Box::pin(run_coding_agent_steered(&sub_goal, "", sandbox, model, cancel, &sub_pause, sub_budget, kernel.clone(), sd, ad)).await;
        figs.extend(sub.figures);
        // Lift this sub-agent's LOCAL source ids (1..M) into the parent's global space at `offset`,
        // and rewrite its summary's [Sk] markers by the same offset so text and sources stay in sync.
        let offset = next;
        let sub_src_count = sub.sources.len() as u32;
        for mut src in sub.sources {
            src.id += offset;
            merged.push(src);
        }
        next += sub_src_count;
        reports.push(format!("### 子智能体角度:{a}\n{}", remap_cite_markers(sub.summary.trim(), offset)));
        // Announce this expert's END → its lamp turns green.
        on_life(CodingStepRec {
            n: 0,
            tool: "agent_done".into(),
            detail: format!("完成 · {short}"),
            observation: String::new(),
            input: sub_id,
            reasoning: String::new(),
            figures: Vec::new(),
        });
    }
    if reports.is_empty() {
        return ("[委派未产出结果 —— 请自己完成这个子任务]".to_string(), figs, merged);
    }
    if reports.len() == 1 {
        return (format!("[委派完成 · 1 个工具型子智能体]\n\n{}", reports[0]), figs, merged);
    }
    let body = reports.join("\n\n");
    let synth = model
        .complete(
            "把多个工具型子智能体各自子问题的结论综合成对总子任务的一个连贯、去重、有取舍的结论:忠实于各部分与其引用,指出一致与冲突,不要编造。原文里形如 [S12] 的引用角标必须原样保留、不得改动或删除。",
            &format!("总子任务:{task}\n\n各子智能体结论:\n{body}\n\n综合结论:"),
            cancel,
        )
        .await
        .unwrap_or_else(|_| body.clone());
    (format!("[委派完成 · {} 个工具型子智能体各攻一角度并综合]\n\n{}", reports.len(), synth.trim()), figs, merged)
}

/// Salvage a proper report from everything gathered so far. Used BOTH at step-budget exhaustion and
/// when a step's model call keeps failing — a long, expensive run must never collapse and throw away
/// its gathered material. Retries the synthesis a few times (a transient timeout/429 shouldn't force
/// the thinner fallback); on total failure builds a CLEAN report from the retrieved SOURCES — it never
/// dumps the raw transcript (which holds internal steering / the GOAL header) into a user-facing report.
async fn synthesize_wrapup(model: &impl CodingModel, _sys: &str, transcript: &str, sources: &[Source], cancel: &Cancel) -> String {
    // DEDICATED report-writer system prompt — NOT the agent system prompt. The agent prompt orders the
    // model to emit tool calls / {"final":…} JSON, so reusing it here made the model answer "write the
    // report" with a `{"tool":"python","code":…}` step or a {"final":…} envelope instead of prose — which
    // then shipped verbatim as a "done" deliverable (THINKING + raw matplotlib code). A focused writer
    // prompt makes it actually produce the report BODY and nothing else.
    let report_sys = "你是一名严谨的科研报告撰写助手。你的唯一任务:根据下面给出的研究过程与已检索证据,直接写出一份完整的最终报告【正文】。硬性规则:只输出报告正文本身的 Markdown;绝对不要输出任何 JSON、工具调用(如 {\"tool\":…}、{\"final\":…})、<think> 或 <tool_call> 标签、也不要写 \"THINKING:\"/思考过程/元说明,更不要再写 Python 代码去画图(图已经生成好,会自动附在报告后面)。用用户的语言书写。";
    let instr = "现在直接写出一份**完整的最终报告正文**:把已经真正检索到 / 计算出的证据、结果和图,系统组织成报告(已生成的图会自动附上,不要再画、不要再写代码)。**按固定五段**:①**直接回答**——开篇正面、简明回答用户最初问的那个问题(先给答案/判断/推荐,别跑偏成另一个新颖角度);②机制/证据分析——主要发现、竞争假说权衡、带 [Sn] 引用;③**可执行方案(必给)**——把分析落到用户能照着做的具体方案:机制/假说题给决定性实验方案(体系材料、方法与关键参数、读出指标、判定判据、对照、预期),选型/决策题给明确推荐+分步落地路径,可行性题给可行性判定+关键卡点+立即可做的下一步动作,how-to 题给操作规程;**具体到能照做、给参数与判据,别写「建议进一步验证」这类空话**;④关键风险与可证伪缺口;⑤参考文献。用 Markdown、用用户的语言。**只输出报告正文,不要 JSON、不要工具调用、不要思考过程。**";
    // The final report is the make-or-break deliverable — a timeout here is the #1 reason a run that
    // did all the work still shows "综述未能自动合成". So: (a) each attempt uses `complete_report`,
    // which grants a generous (>=5 min) timeout — a long GLM-5.2 report over 90s no longer gets cut
    // off; (b) attempts use a PROGRESSIVELY SMALLER context, so if the fullest prompt still times out,
    // a compacter one generates faster and lands. Only after all of these do we fall back.
    for (ctx_chars, wait_ms) in [(22_000usize, 0u64), (14_000, 3000), (8_000, 6000), (5_000, 6000)] {
        if wait_ms > 0 {
            for _ in 0..(wait_ms / 200) {
                if cancel.is_cancelled() { break; }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
        if cancel.is_cancelled() { break; }
        let wrap_user = format!("{}\n\n{instr}", compact_transcript(transcript, ctx_chars));
        if let Ok(r) = model.complete_report(report_sys, &wrap_user, cancel).await {
            // Recover a report the model wrapped in {"final":…} (unescapes \n, drops a "我已…" preamble).
            let r = unwrap_report_envelope(&r);
            // VALIDATE the salvage's OWN output: never return the model's thinking narration or a
            // tool-call echo (THINKING: … {"tool":"python","code":…}) as the report. If this attempt
            // still produced one, fall through to the next (smaller-context) try, then the honest banner.
            // (Fixes: a "done" deliverable that was THINKING + raw matplotlib code from the salvage path.)
            if r.trim().chars().count() > 80 && looks_like_real_report(&r) {
                return r.trim().to_string();
            }
        }
    }
    fallback_report_from_sources(sources)
}

/// Recover a real report when the model wrapped it in a `{"final":"…"}` (or `{"tool":…,"…"}`) action
/// instead of returning raw prose — extract the `final` string (serde UNESCAPES its `\n`), dropping any
/// leading thinking narration. If there's no such envelope, the input already IS the report prose.
fn unwrap_report_envelope(s: &str) -> String {
    if let Some(obj) = extract_json(s) {
        // Only unwrap when the JSON dominates the text (the report IS the envelope, modulo a short
        // preamble) — never strip a ```json example that a real prose report happens to contain.
        if obj.len() * 100 >= s.trim().len() * 60 {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&obj)
                .or_else(|_| serde_json::from_str(&sanitize_json_strings(&obj)))
            {
                if let Some(fin) = v.get("final").and_then(|x| x.as_str()) {
                    if fin.trim().chars().count() > 40 {
                        return fin.trim().to_string();
                    }
                }
            }
        }
    }
    // TEXTUAL fallback: the model emitted {"final":"…"} but the JSON is too broken to parse — a RAW
    // control character inside the value, unescaped interior quotes, or a garbled/missing closing quote
    // (observed: an 8KB report whose value ended `[S15]」}` with a stray newline → serde + sanitize both
    // fail). Recover the report body by hand so it isn't shipped as a raw `{"final":"…\n…"}` blob.
    if let Some(body) = textual_final(s) {
        if body.trim().chars().count() > 40 {
            return body.trim().to_string();
        }
    }
    s.trim().to_string()
}

/// Recover the value of a `"final"` key from text too broken to JSON-parse. Takes from after
/// `"final" : "` to the last `"}` (else the last `}`, else the end — tolerating a missing close quote),
/// then unescapes the common JSON string escapes (\n \t \" \\ \/). Robust to raw control chars and
/// unescaped interior quotes in the report body.
fn textual_final(s: &str) -> Option<String> {
    let k = s.find("\"final\"")?;
    let after_key = &s[k + "\"final\"".len()..];
    let colon = after_key.find(':')?;
    let body = after_key[colon + 1..].trim_start();
    let body = body.strip_prefix('"')?;
    // Find the envelope's END. Prefer the proper `"}` close. When it's ABSENT (the truncated/broken
    // case this fn exists for), do NOT fall back to the last bare `}` — that is almost always INTERIOR
    // content (LaTeX `\frac{a}{b}`, set notation `{x}`, a ```json example) and would silently truncate
    // the report tail. The report was cut off anyway → take it to end-of-string, only shedding a single
    // trailing `}` if it's the very last non-whitespace char (a real, if unquoted, envelope close).
    let end = if let Some(p) = body.rfind("\"}") {
        p
    } else {
        let trimmed = body.trim_end();
        if trimmed.ends_with('}') { trimmed.len() - 1 } else { body.len() }
    };
    let raw = body[..end].trim_end().trim_end_matches('"').trim_end();
    let mut out = String::with_capacity(raw.len());
    let mut it = raw.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => {}
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('/') => out.push('/'),
                Some(o) => { out.push('\\'); out.push(o); }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    Some(out.trim().to_string())
}

/// When the model can't synthesize the final report, build a clean, readable report from the retrieved
/// sources (with clickable [Sn] markers) — NEVER a raw transcript dump. Figures are attached separately.
/// Stable leading marker for a fallback (report-not-synthesized) deliverable. Kept in sync with
/// [`is_fallback_report`], which detects it to mark the run `no_report`.
const FALLBACK_BANNER: &str = "> ⚠️ **报告未自动生成** — 本次已完成检索与绘图,但最终文字报告合成未成功。**已算出的图和来源都已保存,不会丢失**(见下方 / 产物库)。点右下角【继续 / 重试】即可在已有材料基础上重新生成报告。\n\n";
pub(crate) fn fallback_report_from_sources(sources: &[Source]) -> String {
    if sources.is_empty() {
        return format!("{FALLBACK_BANNER}下方为本次已生成的图。");
    }
    let mut s = format!("{FALLBACK_BANNER}## 已检索到的关键来源(角标可点击溯源)\n\n");
    for src in sources {
        let kind = match src.kind.as_str() { "paper" => "文献", "db" => "数据库", _ => "网页" };
        let head = if src.title.trim().is_empty() { src.url.clone() } else { src.title.clone() };
        s.push_str(&format!("- **[S{}]** ({kind}) {}\n", src.id, head.chars().take(160).collect::<String>()));
        let ex: String = src.excerpt.lines().filter(|l| !l.trim().is_empty()).take(2).collect::<Vec<_>>().join(" ");
        if !ex.trim().is_empty() {
            s.push_str(&format!("  - {}\n", ex.chars().take(220).collect::<String>()));
        }
    }
    s.push_str("\n**后续可深化**:再运行一次即可让模型把上述来源综合成完整、连贯、带引用的深度报告。");
    s
}

/// True when a summary is one of the "final report couldn't be synthesized" fallbacks. The run still
/// gathered real figures + sources (kept), but there is NO narrative report — so the caller marks the
/// run `no_report` instead of `done`, and the UI presents it as "报告未生成 · 可重新生成" rather than a
/// clean deliverable (a user must never think an hour of work produced a complete report when it didn't).
fn is_fallback_report(s: &str) -> bool {
    s.trim_start().starts_with("> ⚠️ **报告未自动生成**")
}

/// The FINALIZE MARKER for the raw-Markdown report protocol. When the agent is done it writes a line
/// containing exactly this, then the report as plain Markdown — NO JSON, NO escaping. This removes the
/// #1 recurring failure (GLM can't reliably escape an 8KB report into a `{"final":"…"}` JSON string).
const FINAL_MARKER: &str = "===AXIOM3_FINAL===";

/// Remove fenced ```…``` blocks (base64 ```pyfig figures, ```json tool echoes) so only NARRATIVE
/// prose is measured. An unterminated fence (a truncated reply) drops everything after the opener.
fn strip_fenced_blocks(s: &str) -> String {
    let mut out = String::new();
    let mut in_fence = false;
    for line in s.lines() {
        if line.trim_start().starts_with("```") { in_fence = !in_fence; continue; }
        if !in_fence { out.push_str(line); out.push('\n'); }
    }
    out
}

/// The highest source id `n` cited as `[Sn]` in the text (0 if none). Used to catch DANGLING citations —
/// a `[Sn]` whose `n` exceeds the number of sources actually retrieved means the model invented a
/// citation it can't back. Requires the closing `]` so `[S12` inside other prose doesn't false-match.
fn max_cited_marker(s: &str) -> u32 {
    let b = s.as_bytes();
    let mut max = 0u32;
    let mut i = 0;
    while i + 2 < b.len() {
        if b[i] == b'[' && b[i + 1] == b'S' && b[i + 2].is_ascii_digit() {
            let mut j = i + 2;
            let mut val: u32 = 0;
            while j < b.len() && b[j].is_ascii_digit() {
                val = val.saturating_mul(10).saturating_add((b[j] - b'0') as u32);
                j += 1;
            }
            if j < b.len() && b[j] == b']' {
                max = max.max(val);
            }
            i = j;
        } else {
            i += 1;
        }
    }
    max
}

/// Backstop for DANGLING citations: remove every `[Sn]` marker whose `n` exceeds `max_valid` (the number
/// of sources actually retrieved), so a dead provenance link never ships even if the model ignores the
/// dangling-citation nudge. Only the unbacked marker is dropped; the claim text stays. UTF-8 safe — it
/// skips whole ASCII `[S…]` tokens and copies every other byte verbatim (incl. multi-byte CJK).
fn strip_dangling_markers(s: &str, max_valid: u32) -> String {
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'[' && i + 2 < b.len() && b[i + 1] == b'S' && b[i + 2].is_ascii_digit() {
            let mut j = i + 2;
            let mut val: u32 = 0;
            while j < b.len() && b[j].is_ascii_digit() {
                val = val.saturating_mul(10).saturating_add((b[j] - b'0') as u32);
                j += 1;
            }
            if j < b.len() && b[j] == b']' && val > max_valid {
                i = j + 1; // drop the whole [Sn] token
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

/// Strip a residual `===AXIOM3_FINAL===` marker from a summary. The loop's own detection already takes
/// the text AFTER the marker, but the SALVAGE writers (synthesize_wrapup / the budget-exhausted wrap-up)
/// ask the model to write a report and it sometimes re-prepends the marker line — which would then ship
/// verbatim. Take everything after the (first) marker; no-op when there is none.
fn strip_final_marker(s: &str) -> String {
    match s.find(FINAL_MARKER) {
        Some(i) => s[i + FINAL_MARKER.len()..].trim_start().to_string(),
        None => s.to_string(),
    }
}

/// Normalize MULTI-reference citation groups into consecutive single markers: `[S2, S3]` / `[S2,S3]` /
/// `[S2; S3]` → `[S2][S3]`. The model routinely bundles several refs in one bracket, but the dangling
/// gate, the strip backstop, AND the frontend clickable-source regex (`/\[S\d+\]/`) only recognize a
/// SINGLE `[Sn]` — so a bundled `[S1, S16]` was invisible to the anti-fabrication gate (a fabricated
/// [S16] with 0 sources shipped) and rendered as dead, unclickable text. Normalizing once here makes
/// every downstream consumer work unchanged. Only rewrites strict `S<digits>`-and-separator groups, so
/// `[START]`, `[Section 2]`, `[SI]` are untouched.
fn normalize_citations(s: &str) -> String {
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'[' {
            // Try to parse a citation group: [ (S digits) (sep S digits)* ] with ≥1 ref.
            let mut j = i + 1;
            let mut nums: Vec<u32> = Vec::new();
            loop {
                while j < b.len() && (b[j] == b' ' || b[j] == b'\t') { j += 1; }
                if j >= b.len() || b[j] != b'S' || j + 1 >= b.len() || !b[j + 1].is_ascii_digit() {
                    break;
                }
                j += 1; // past 'S'
                let mut v: u32 = 0;
                while j < b.len() && b[j].is_ascii_digit() {
                    v = v.saturating_mul(10).saturating_add((b[j] - b'0') as u32);
                    j += 1;
                }
                nums.push(v);
                while j < b.len() && (b[j] == b' ' || b[j] == b'\t') { j += 1; }
                if j < b.len() && (b[j] == b',' || b[j] == b';') { j += 1; continue; }
                break;
            }
            if !nums.is_empty() && j < b.len() && b[j] == b']' {
                for n in &nums {
                    out.extend_from_slice(format!("[S{n}]").as_bytes());
                }
                i = j + 1;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

/// Final cleanup applied to EVERY shipped report summary, from ANY path (accept, salvage, wrap-up,
/// budget-exhaustion): strip a leaked ===AXIOM3_FINAL=== marker, NORMALIZE bundled `[S2, S3]` citations
/// into `[S2][S3]` (so the dangling strip + frontend see every ref), then strip DANGLING [Sn] citations
/// (n > sources retrieved). Idempotent; a no-op on a clean report or the honest fallback banner.
fn clean_summary(summary: &str, n_sources: usize) -> String {
    let s = normalize_citations(&strip_final_marker(summary));
    strip_dangling_markers(&s, n_sources as u32)
}

/// Count QUANTITATIVE claims: text segments carrying BOTH a digit and a unit/statistic marker — an
/// effect size, sample size, dose, endpoint, or potency. This is the real signal of evidence DEPTH, as
/// opposed to a bibliographic pointer ("X et al. reported …"). Benchmarking against a web-research
/// baseline: 126 registered sources yielded ~4 such numbers while the 23-source baseline produced 37 —
/// retrieval VOLUME is not evidence depth, so the gate below measures this instead of source count.
fn quant_claim_count(s: &str) -> usize {
    const UNITS: &[&str] = &[
        "n=", "n =", "%", "nM", "µM", "uM", "μM", "pM", "mg/kg", "mg", "kDa", "Å", "HR", "OR", "CI",
        "p<", "p =", "p=", "P<", "IC50", "DC50", "Dmax", "EC50", "PFS", "OS", "ORR", "倍", "个月", "±", "≥",
    ];
    strip_fenced_blocks(s)
        .split(|c| c == '。' || c == '\n' || c == '；' || c == ';')
        .filter(|seg| {
            seg.chars().any(|c| c.is_ascii_digit()) && UNITS.iter().any(|u| seg.contains(u))
        })
        .count()
}

/// Every `[Sn]` id cited anywhere in `s`.
fn cited_ids(s: &str) -> std::collections::BTreeSet<u32> {
    let b = s.as_bytes();
    let mut out = std::collections::BTreeSet::new();
    let mut i = 0;
    while i + 2 < b.len() {
        if b[i] == b'[' && b[i + 1] == b'S' && b[i + 2].is_ascii_digit() {
            let mut j = i + 2;
            let mut v: u32 = 0;
            while j < b.len() && b[j].is_ascii_digit() {
                v = v.saturating_mul(10).saturating_add((b[j] - b'0') as u32);
                j += 1;
            }
            if j < b.len() && b[j] == b']' {
                out.insert(v);
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

/// Guarantee every cited `[Sn]` is RESOLVABLE by the reader: append any cited-but-unlisted source
/// (title + URL, taken from the REGISTERED provenance list) to the report's reference section.
/// The dangling gate only compares an id against the COUNT of registered sources — benchmarking found a
/// report citing [S91] (≤ the 126 registered, so it passed) whose own 参考文献 listed just 30 entries, so
/// the reader could not look it up; and the listed entries carried no URL/DOI at all. Deterministic, no
/// model call: it closes both holes at once.
fn ensure_reference_list(summary: &str, sources: &[Source]) -> String {
    if sources.is_empty() {
        return summary.to_string();
    }
    // Split body vs the reference section (if the model wrote one).
    let head_at = ["参考文献", "参考资料", "References", "REFERENCES"]
        .iter()
        .filter_map(|k| summary.find(k))
        .min();
    let (body, refs) = match head_at {
        Some(i) => (&summary[..i], &summary[i..]),
        None => (summary, ""),
    };
    let cited = cited_ids(body);
    if cited.is_empty() {
        return summary.to_string();
    }
    // What the reference section already resolves — accept `[S3]` or a bare `S3` entry marker.
    let mut listed = cited_ids(refs);
    let rb = refs.as_bytes();
    let mut i = 0;
    while i + 1 < rb.len() {
        if rb[i] == b'S' && rb[i + 1].is_ascii_digit() {
            let mut j = i + 1;
            let mut v: u32 = 0;
            while j < rb.len() && rb[j].is_ascii_digit() {
                v = v.saturating_mul(10).saturating_add((rb[j] - b'0') as u32);
                j += 1;
            }
            listed.insert(v);
            i = j;
        } else {
            i += 1;
        }
    }
    let missing: Vec<u32> = cited.difference(&listed).copied().collect();
    if missing.is_empty() {
        return summary.to_string();
    }
    let mut out = summary.to_string();
    if head_at.is_none() {
        out.push_str("\n\n## 参考文献\n");
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    for id in missing {
        if let Some(src) = sources.iter().find(|s| s.id == id) {
            let url = if src.url.trim().is_empty() { String::new() } else { format!(" — {}", src.url) };
            out.push_str(&format!("- [S{}] {}{}\n", id, src.title.trim(), url));
        }
    }
    out
}

/// Capture sources the model registered via `scidb.cite(...)` (which prints a `#AXIOM_CITE#<json>` line):
/// parse them into the provenance list so DB/lit records fetched through the python+scidb path become
/// CITABLE [Sn]. Previously ONLY the native litsearch/multi_search tools pushed sources, so a report
/// built on `import scidb` in python had 0 registered sources and the model fabricated [Sn] to fill the
/// gap (observed in benchmarking: a full report citing [S1..S16] with 0 real sources). Returns the
/// observation with the marker line(s) replaced by the assigned [Sn] ids, so the model knows which
/// numbers it may now cite. De-duplicates by URL.
fn capture_cited_sources(o: &str, sources: &mut Vec<Source>, src_n: &mut u32) -> String {
    const TAG: &str = "#AXIOM_CITE#";
    if !o.contains(TAG) {
        return o.to_string();
    }
    let mut kept = String::new();
    let mut assigned: Vec<u32> = Vec::new();
    for line in o.lines() {
        if let Some(js) = line.trim_start().strip_prefix(TAG) {
            if let Ok(items) = serde_json::from_str::<Vec<Value>>(js.trim()) {
                for it in items {
                    let url = it.get("url").and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
                    if url.is_empty() {
                        continue;
                    }
                    if let Some(existing) = sources.iter().find(|s| s.url == url) {
                        assigned.push(existing.id); // already registered → reuse its [Sn]
                        continue;
                    }
                    *src_n += 1;
                    sources.push(Source {
                        id: *src_n,
                        kind: "db".into(),
                        title: it.get("title").and_then(|x| x.as_str()).unwrap_or("record").to_string(),
                        url,
                        excerpt: it.get("excerpt").and_then(|x| x.as_str()).unwrap_or("").chars().take(4000).collect(),
                    });
                    assigned.push(*src_n);
                }
            }
            continue; // drop the raw marker line from what the model sees
        }
        kept.push_str(line);
        kept.push('\n');
    }
    if !assigned.is_empty() {
        let ids: Vec<String> = assigned.iter().map(|n| format!("[S{n}]")).collect();
        kept.push_str(&format!("[已登记 {} 条可引用来源;在报告正文引用它们时用:{}]\n", assigned.len(), ids.join(" ")));
    }
    kept
}

/// Heuristic guard: does `summary` read like a genuine final report, or is it the agent's thinking
/// narration + a {"tool":…}/{"todos":…} action echo, an empty/trivial string, or a raw JSON/base64
/// blob? Deliberately LENIENT — a real markdown report must pass; only obvious non-reports fail. The
/// figure/files appendix is appended by the CALLER later, so `summary` here is the narrative only.
/// (Fixes: a truncated first-step "thinking + {plan}" reply was shipped as a `done` deliverable.)
/// True if the text carries the agent's raw-scaffolding markers OUTSIDE any ```fence``` — thinking
/// narration ("THINKING:"), tool-call tags (`<tool_call>`/`<think>`), or a tool-call JSON echo
/// (`"tool":`/`"code":`/`"queries":`/`"todos":`). Their presence means a raw reply leaked instead of a
/// real report. Split out from `looks_like_real_report` so the {"final"} ACCEPT path can gate on markers
/// ALONE — there a short but CLEAN final ("created hello.txt") must still pass (the >200-char prose check
/// would wrongly reject it), but a final that smuggled thinking+code inside its value must be caught.
pub(crate) fn has_journal_markers(s: &str) -> bool {
    let prose = strip_fenced_blocks(s.trim());
    let pt = prose.trim();
    pt.starts_with("THINKING:") || pt.starts_with("THINKING：")
        || pt.contains("<tool_call>") || pt.contains("<think>") || pt.contains("</think>")
        || pt.contains("\"tool\":") || pt.contains("tool\":\"")
        || pt.contains("\"queries\":") || pt.contains("\"todos\":") || pt.contains("\"code\":")
}

pub(crate) fn looks_like_real_report(summary: &str) -> bool {
    let t = summary.trim();
    // The honest no-report banner is short by design — treat as "real" so we don't re-salvage it
    // (the caller maps the banner to `no_report`).
    if is_fallback_report(t) { return true; }
    // A raw reply / truncated tool-call echo leaked as the "report" — reject regardless of length
    // (a LONG thinking narration is >200 prose chars, so the length check alone can't catch it).
    if has_journal_markers(t) { return false; }
    // Strip fenced blocks + a leading bare JSON action object, then require substantial narrative prose;
    // a "thinking + {plan}" echo (or empty/JSON-only reply) leaves almost nothing once stripped.
    let mut prose = strip_fenced_blocks(t);
    if let Some(obj) = extract_json(&prose) { prose = prose.replacen(&obj, "", 1); }
    prose.chars().filter(|c| !c.is_whitespace()).count() >= 200
}

/// Guard a candidate "done" summary taken from a RAW model reply (the non-`{"final":…}` return
/// paths): if it isn't a real report, salvage one from the transcript; if the salvage also fails it
/// returns the honest fallback banner, which the caller detects and marks `no_report`. This is why a
/// run can never again ship the agent's thinking+plan echo as a finished report.
async fn guard_done_summary(
    candidate: String, model: &impl CodingModel, sys: &str, transcript: &str,
    sources: &[Source], cancel: &Cancel,
) -> String {
    // First recover a report the model wrapped in {"final":…} (unescapes \n, drops a thinking preamble);
    // if that yields a real report use it, else synthesize a fresh one from the transcript.
    let candidate = unwrap_report_envelope(&candidate);
    if looks_like_real_report(&candidate) { candidate }
    else { synthesize_wrapup(model, sys, transcript, sources, cancel).await }
}

/// Back-compat entry: run with no extra system steering (the `goal` is the whole task).
pub async fn run_coding_agent(
    goal: &str,
    sandbox: &Sandbox,
    model: &impl CodingModel,
    cancel: &Cancel,
    pause: &Pause,
    budget: &CodingBudget,
    kernel: Option<Arc<PyKernel>>,
    on_step: impl FnMut(CodingStepRec) + Send,
    on_ask: impl FnMut(String, String),
) -> CodingResult {
    run_coding_agent_steered(goal, "", sandbox, model, cancel, pause, budget, kernel, on_step, on_ask).await
}

/// Like `run_coding_agent`, but `steer` (mode directives like the deep-research brief) is appended to
/// the SYSTEM prompt instead of being concatenated into `goal`. This keeps `goal` — which becomes the
/// transcript's `GOAL:` header, the report subject, the recap input and the session title — the CLEAN
/// user topic, so internal steering can never leak into a salvage dump or any user-facing string.
#[allow(clippy::too_many_arguments)]
pub async fn run_coding_agent_steered(
    goal: &str,
    steer: &str,
    sandbox: &Sandbox,
    model: &impl CodingModel,
    cancel: &Cancel,
    pause: &Pause,
    budget: &CodingBudget,
    kernel: Option<Arc<PyKernel>>,
    mut on_step: impl FnMut(CodingStepRec) + Send,
    // Called when the agent invokes `ask_user`: (card_id, card_json) — the shell emits an AskUser
    // event so the UI can render the question card. The loop then parks until the answer arrives.
    mut on_ask: impl FnMut(String, String),
) -> CodingResult {
    let base_sys = system_prompt(sandbox.mode(), kernel.is_some(), pause.delegate_allowed());
    let sys = if steer.trim().is_empty() {
        base_sys
    } else {
        // Internal steering, explicitly fenced off from the deliverable so the model never echoes it.
        format!("{base_sys}\n\n=== 本次运行的模式指令(仅供你遵循,绝不要把这段文字复述进报告/结论)===\n{steer}")
    };
    let mut transcript = format!("GOAL:\n{goal}\n\nWorkspace root: {}\n", sandbox.root().display());
    let mut ran_command = false; // did the agent run a build/test command this run?
    let mut nudged = false; // pushed back once for finishing without verifying
    let mut plan_gated = false; // whether the one-time plan-approval gate has fired
    let mut malformed = 0u32; // consecutive un-parseable JSON envelopes (bounded retry, not a fatal "done")
    const MAX_MALFORMED: u32 = 3;
    let mut intake_done = !pause.intake_required(); // satisfied once the forced intake ask has fired
    let mut intake_nudges = 0u32; // bounded pushes to make the first action an ask_user intake card
    let mut delegated_once = false; // a delegate sub-team has been spawned this run
    let mut delegate_calls = 0u32; // hard cap on delegations per run — a sub-team costs minutes
    let mut delegate_nudges = 0u32; // bounded push to open a sub-team on a complex research topic
    let mut all_figures: Vec<String> = Vec::new(); // computed figures across the whole run
    // Automatic kernel resilience: a cell timeout/crash restarts the kernel and WIPES in-memory state
    // (dataframes/adata). We auto-checkpoint after an expensive successful cell and auto-restore after
    // a state-wiping restart, so a heavy analysis survives instead of losing hours of loaded data.
    let mut have_ckpt = false; // a checkpoint file exists to restore from
    let mut cells_since_ckpt = 0u32; // periodic-checkpoint safety net counter
    let mut searched: std::collections::HashSet<String> = std::collections::HashSet::new(); // dedup repeated skill searches
    let mut skill_searched = false; // once true, HARD-block further search_skills until a load_skill (force-commit)
    let mut fig_seen: std::collections::HashSet<String> = std::collections::HashSet::new(); // dedup identical figures (re-run plots)
    // FIGURE-ENGINE GATE state. GLM under-triggers the figure skills: given a topic that explicitly
    // asks for a structure / molecule / gene-context figure, it draws a bar chart or an inline block
    // instead of loading the right engine (observed: mcr gene-context → bar chart; EGFR kinase → no
    // PyMOL). Detect the demand from the GOAL, track whether the matching skill was loaded, nudge ONCE
    // at the finish if a demanded engine was never loaded. Keyword lists are deliberately SPECIFIC so a
    // casual mention ("含突变") doesn't trigger a heavy install.
    let (needs_struct, needs_mol, needs_gene) = figure_needs(goal);
    let (mut loaded_struct, mut loaded_mol, mut loaded_gene) = (false, false, false);
    let mut figure_nudges = 0u32;
    let mut reflected = false; // whether the ONE-time self-critique gate has fired before finalizing
    let mut cite_nudged = false; // whether the ONE-time "you gathered sources but cited none" gate has fired
    let mut dangling_nudged = false; // whether the ONE-time "you cited [Sn] beyond the sources you retrieved" gate has fired
    let mut breadth_nudged = false; // whether the ONE-time "too few sources for a deep report" gate has fired
    let mut depth_nudges = 0u32; // "many sources but no numbers extracted" gate — bounded to 2 passes
    let mut last_compact_len = 0usize; // transcript length at the last context-compaction (avoid re-compacting every step)
    let mut recent_errs: Vec<String> = Vec::new(); // last few error signatures — nudge on a repeat of the SAME failure
    let mut lit_queries: Vec<String> = Vec::new(); // litsearch queries so far — nudge for breadth when one angle is over-searched
    // QUALITY-FIRST search budget. Dedup already SKIPS near-identical queries (they don't count), so this
    // caps only DISTINCT productive searches. A deep-research topic (meta-analysis needing many trials)
    // was being starved by a flat cap of 10 → forced early wrap-up → quality drop. Give research mode
    // generous headroom; analysis mode (local, fewer steps) naturally does far fewer anyway.
    let lit_cap: usize = if pause.delegate_encouraged() { 24 } else { 12 };
    let mut web_queries: Vec<String> = Vec::new(); // web_search queries so far — dedup + soft breadth nudge
    // Provenance: every retrieved source gets a stable [Sn] id + excerpt, so the report's [Sn] markers
    // are clickable and trace back to the original text. `src_n` is the running global counter.
    let mut sources: Vec<Source> = Vec::new();
    let mut src_n: u32 = 0;
    // Snapshot the workspace so we can report the files the agent produces (per-file artifacts).
    let start_files = snapshot_files(sandbox.root());
    for n in 1..=budget.max_steps {
        // Cooperative pause: wait here between turns while paused (cancel still wins).
        while pause.is_paused() && !cancel.is_cancelled() {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        if cancel.is_cancelled() {
            return CodingResult { status: "cancelled".into(), summary: "cancelled".into(), steps: n - 1, figures: all_figures, artifacts: collect_artifacts(sandbox.root(), &start_files), sources: sources.clone() };
        }
        // Budget meter: the model has no other way to know how many steps remain, so it can't
        // triage. Show step N/total and, in the final ~20%, an explicit wrap-up order so it stops
        // exploring and computes the deliverable instead of getting truncated mid-wrangle.
        let remaining = budget.max_steps.saturating_sub(n - 1);
        // Framed as PROGRESS, not a scarce budget: the model used to echo "[budget] few steps left"
        // into its user-facing thinking as "超预算/预算快用完了", which alarmed the user. In the final
        // stretch we still steer it to converge — but as "entering wrap-up", and we never expose a
        // remaining-count/total, so it wraps up a COMPLETE deliverable instead of narrating scarcity.
        let warn = if remaining <= (budget.max_steps / 5).max(3) {
            "\n[收尾阶段] 深度探索已经充分,现在请停止继续检索/探索,集中把核心结果与图算全,然后**先写一行 ===AXIOM3_FINAL=== 再直接写裸 Markdown 报告**(不要包进 JSON)产出一份**完整**的报告(原生图块 + 有据的结论段落);不要停在半成品。"
        } else {
            ""
        };
        // CONTEXT COMPACTION (keep running, don't lose the middle). Once the raw transcript grows large,
        // SUMMARIZE its old middle into a durable state block via one model call — instead of letting
        // compact_transcript() silently TRUNCATE it away (which made the agent forget mid-run findings +
        // re-search). Keeps the goal + a state summary (conclusions / sources [Sn] / open sub-questions)
        // + recent steps verbatim, so memory is retained while input tokens stay bounded (also faster).
        // Rare (only past ~40k chars and after ~14k of new growth); fail-safe (leaves the transcript as-is
        // on any error — compact_transcript below still bounds the per-call prompt).
        let tlen = transcript.chars().count();
        if tlen > 40_000 && tlen.saturating_sub(last_compact_len) > 14_000 && !cancel.is_cancelled() {
            let chars: Vec<char> = transcript.chars().collect();
            let head_n = 1600usize.min(chars.len());
            let tail_n = 9000usize.min(chars.len().saturating_sub(head_n));
            let mid_end = chars.len().saturating_sub(tail_n);
            if mid_end > head_n + 4000 {
                let head: String = chars[..head_n].iter().collect();
                let middle: String = chars[head_n..mid_end].iter().collect();
                let tail: String = chars[mid_end..].iter().collect();
                let sum_sys = "把下面这段研究过程的中间记录压缩成一段【状态摘要】,只保留后续推理必须记住的东西:已确认的关键发现与数字(带 [Sn] 角标)、已检索到的重要来源、已生成的图、尚未解决的子问题、已做的关键决定。丢弃冗长的工具原始输出与来回寒暄。用紧凑要点,**务必保留 [Sn] 角标编号不变**。只输出状态摘要本身。";
                if let Ok(summary) = model.complete(sum_sys, &middle, cancel).await {
                    if summary.trim().chars().count() > 40 {
                        transcript = format!(
                            "{head}\n\n[已压缩的中间状态 · 保留结论/来源/未决项]\n{}\n\n[最近步骤]\n{tail}",
                            summary.trim()
                        );
                    }
                }
            }
            last_compact_len = transcript.chars().count();
        }
        // Final safety net: bound the per-call prompt regardless (keep GOAL + recent; elide any residual middle).
        let ctx = compact_transcript(&transcript, 26_000);
        let user = format!(
            "{ctx}\n[进度] 第 {n} 步{warn}\n先用1-2句话说出你此刻的思考(你要做什么、为什么、上一步的结果说明了什么),再给出你的 JSON 动作 · First write 1-2 sentences of your thinking in the user's language, THEN your JSON action:"
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
                        if cancel.is_cancelled() {
                            return CodingResult { status: "cancelled".into(), summary: "cancelled".into(), steps: n - 1, figures: all_figures, artifacts: collect_artifacts(sandbox.root(), &start_files), sources: sources.clone() };
                        }
                        if attempt > 3 {
                            // The model kept failing at THIS step (e.g. repeated timeouts). Don't throw
                            // away a long run's gathered material by returning a bare error — SALVAGE a
                            // report from the transcript so far (one attempt; falls back to a readable
                            // tail). The run ends gracefully with real content + its figures.
                            let _ = &e; // error is captured in the transcript; the salvage speaks for itself
                            let artifacts = collect_artifacts(sandbox.root(), &start_files);
                            let summary = clean_summary(&synthesize_wrapup(model, &sys, &transcript, &sources, cancel).await, sources.len());
                            let status = if is_fallback_report(&summary) { "no_report" } else { "done" };
                            return CodingResult { status: status.into(), summary, steps: n - 1, figures: all_figures, artifacts, sources: sources.clone() };
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
        // The model's reasoning = its reply minus the JSON action (or minus the ===AXIOM3_FINAL=== marker + report).
        let reasoning = if let Some(i) = reply.find(FINAL_MARKER) {
            reply[..i].trim().to_string()
        } else {
            match &json {
                Some(j) => reply.replacen(j, "", 1).trim().to_string(),
                None => String::new(),
            }
        };
        // ── FINAL-REPORT DETECTION ─────────────────────────────────────────────────────────────────
        // NEW PROTOCOL (preferred, unbreakable): the report is RAW Markdown after a `===AXIOM3_FINAL===`
        // line — no JSON, no escaping. LEGACY (still accepted): a {"final":"…"} JSON action, parsed or
        // recovered textually when GLM's escaping of a long report is broken (raw control char / missing
        // closing quote). Either yields a clean report. Detected BEFORE the tool-call parse so a
        // marker-only reply (no JSON object) is finalized, not mistaken for a "no action" nudge.
        let final_text: Option<String> = if let Some(i) = reply.find(FINAL_MARKER) {
            // NEW protocol: raw Markdown after the marker.
            Some(reply[i + FINAL_MARKER.len()..].trim().to_string())
        } else {
            // LEGACY {"final":"…report…"} envelope. Must be distinguished from a TOOL CALL whose string
            // args merely contain the word "final" (e.g. write_file of a config with a "final" key, or
            // python building a dict `{"final": …}`) — misreading that as a final would ship a garbage
            // fragment AND skip the real action.
            let parsed = json.as_deref().and_then(|j| {
                serde_json::from_str::<Value>(j)
                    .ok()
                    .or_else(|| serde_json::from_str::<Value>(&sanitize_json_strings(j)).ok())
            });
            match parsed {
                // Parsed cleanly → a final ONLY if the top-level object has `final` and is NOT a tool
                // call (a `tool` key means it's an action, even if "final" appears nested in its args).
                Some(v) => v
                    .as_object()
                    .filter(|o| !o.contains_key("tool"))
                    .and_then(|o| o.get("final"))
                    .and_then(|x| x.as_str())
                    .map(str::to_string)
                    .filter(|s| !s.trim().is_empty()),
                // Unparseable JSON (GLM's escaping of a long report broke). Recover textually ONLY when
                // this is a broken final ENVELOPE — the `final` key precedes any tool-arg key — AND the
                // recovered body actually looks like a report. A broken TOOL CALL (tool/path/code/… key
                // first) recovers to a garbage fragment and must fall through to the malformed-JSON nudge.
                None if reply.contains("\"final\"") => {
                    let j = json.as_deref().unwrap_or(reply.as_str());
                    let is_envelope = j.find("\"final\"").is_some_and(|fp| {
                        ["\"tool\"", "\"content\"", "\"code\"", "\"cmd\"", "\"path\"", "\"query\"",
                         "\"queries\"", "\"url\"", "\"packages\"", "\"task\"", "\"pattern\"", "\"todos\""]
                            .iter()
                            .filter_map(|k| j.find(k))
                            .min()
                            .is_none_or(|tp| fp < tp)
                    });
                    if is_envelope {
                        textual_final(&reply).filter(|b| looks_like_real_report(b))
                    } else {
                        None
                    }
                }
                None => None,
            }
        };
        // If a FINAL report was detected (the ===AXIOM3_FINAL=== marker OR a {"final":…} action, incl.
        // textual recovery of broken JSON), normalize it into a {"final":…} value so the SINGLE finalize
        // path below handles both protocols uniformly. Otherwise parse the tool call as before.
        let v: Value = if let Some(ft) = final_text {
            serde_json::json!({ "final": ft })
        } else {
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
                        "\n[system] I couldn't find a JSON action in your reply. If you're still WORKING, \
                         respond with EXACTLY ONE JSON tool-call object. If you are DONE, do NOT use JSON — \
                         write a line with exactly ===AXIOM3_FINAL=== then your full report as raw Markdown.\n",
                    );
                    continue;
                }
                // SALVAGE: don't ship an EMPTY report. If the model gave up with a blank/trivial reply but
                // the run actually gathered material (>=3 steps of data), synthesize a report from the
                // transcript instead of returning nothing ("无产物内容"). This turned an empty analysis run
                // (agent absorbed in data-gathering, never wrote the report) into a real deliverable.
                // GUARD: never ship a raw non-report reply (thinking + {plan}/{tool} echo, or an
                // empty/JSON-only blob) as a `done` deliverable — salvage a real report, or mark
                // `no_report` honestly. (Was: `if reply.is_empty() && n>=3` — too narrow; a truncated
                // first-step plan echo slipped through as a 151KB "done" report that was 99% base64.)
                let summary = clean_summary(&guard_done_summary(reply.trim().to_string(), model, &sys, &transcript, &sources, cancel).await, sources.len());
                let status = if is_fallback_report(&summary) { "no_report" } else { "done" };
                return CodingResult { status: status.into(), summary, steps: n, figures: all_figures, artifacts: collect_artifacts(sandbox.root(), &start_files), sources: sources.clone() };
            }
        };
        // Weaker models routinely emit a valid-INTENT tool call whose JSON is technically
        // invalid — a multi-line `code`/`content`/`new` value with RAW newlines/tabs (JSON forbids
        // literal control chars in strings), or a stray bracket/quote from truncation. First try to
        // repair control chars; if it STILL won't parse, that's a MALFORMED tool call, NOT a final
        // answer — nudge the model to re-emit valid JSON and continue (bounded), rather than
        // silently ending the run on what was clearly an attempted action.
        match serde_json::from_str(&action)
            .or_else(|_| serde_json::from_str(&sanitize_json_strings(&action)))
        {
            Ok(v) => { malformed = 0; v }
            Err(e) => {
                if malformed < MAX_MALFORMED {
                    malformed += 1;
                    transcript.push_str(&format!(
                        "\n[system] Your last message was not valid JSON ({e}). If it was a TOOL CALL, \
                         re-emit EXACTLY ONE well-formed JSON object (escape newlines as \\n in code strings). \
                         If it was your FINAL REPORT, do NOT use JSON at all — write a line with exactly \
                         ===AXIOM3_FINAL=== then the report as raw Markdown (real newlines, no escaping).\n"
                    ));
                    continue;
                }
                // SALVAGE: don't ship an EMPTY report. If the model gave up with a blank/trivial reply but
                // the run actually gathered material (>=3 steps of data), synthesize a report from the
                // transcript instead of returning nothing ("无产物内容"). This turned an empty analysis run
                // (agent absorbed in data-gathering, never wrote the report) into a real deliverable.
                // GUARD: never ship a raw non-report reply (thinking + {plan}/{tool} echo, or an
                // empty/JSON-only blob) as a `done` deliverable — salvage a real report, or mark
                // `no_report` honestly. (Was: `if reply.is_empty() && n>=3` — too narrow; a truncated
                // first-step plan echo slipped through as a 151KB "done" report that was 99% base64.)
                let summary = clean_summary(&guard_done_summary(reply.trim().to_string(), model, &sys, &transcript, &sources, cancel).await, sources.len());
                let status = if is_fallback_report(&summary) { "no_report" } else { "done" };
                return CodingResult { status: status.into(), summary, steps: n, figures: all_figures, artifacts: collect_artifacts(sandbox.root(), &start_files), sources: sources.clone() };
            }
        }
        };
        if let Some(f) = v.get("final").and_then(|x| x.as_str()) {
            // EMPTY-FINAL SALVAGE: an empty/trivial "final" would ship as "无产物内容". If the run gathered
            // real material (>=3 steps), synthesize a proper report from the transcript instead of accepting
            // the blank finish (usually the model fumbling the write, not "nothing to say").
            let f: String = if f.trim().is_empty() && n >= 3 {
                synthesize_wrapup(model, &sys, &transcript, &sources, cancel).await
            } else {
                f.to_string()
            };
            // LAST-STEP GUARD: every gate below `continue`s to nudge the model for one more turn. On the
            // FINAL step there is no next turn — nudging would DISCARD the model's actual final report and
            // force the inferior budget-exhausted wrap-up path (observed: a good report thrown away, then
            // re-synthesized with a leaked ===AXIOM3_FINAL=== marker + a dangling [S7]). So on the last
            // step, skip the nudges and ACCEPT the report — the accept-path guard + clean_summary below
            // still sanitize it. (nudge_ok = there is budget for another turn.)
            let nudge_ok = n < budget.max_steps;
            // In write mode, don't accept "done" until the agent has actually RUN a
            // build/test command at least once — nudge it once, like Codex verifying.
            if nudge_ok && sandbox.mode() == SandboxMode::ReadWrite && !ran_command && !nudged {
                nudged = true;
                transcript.push_str(
                    "\n[system] You haven't run any build/test command yet. Before finishing, RUN \
                     the tests (e.g. `python3 check.py`, `pytest -q`, or `cargo test`) to confirm \
                     your change actually works — then finish.\n",
                );
                continue;
            }
            // FIGURE-ENGINE GATE: the topic explicitly asked for a structure/molecule/gene-context
            // figure but the matching engine skill was never loaded — the agent is about to finish
            // with a bar-chart stand-in or an inline block. Push it ONCE to load the real engine and
            // draw the publication figure, then relent. Research mode only (deep reports, where the
            // figure IS the differentiator) and only when a kernel exists to actually render.
            if nudge_ok && pause.delegate_encouraged() && kernel.is_some() && figure_nudges < 1 {
                let mut missing: Vec<&str> = Vec::new();
                if needs_struct && !loaded_struct { missing.push("protein-structure-figure(蛋白/酶/复合物结构 → PyMOL 无头光追)"); }
                if needs_mol && !loaded_mol { missing.push("small-molecule-figure(药物/抑制剂/配体 2D 结构 + 实测描述符 → RDKit)"); }
                if needs_gene && !loaded_gene { missing.push("gene-context-figure(操纵子/耐药基因盒/质粒/可移动元件 → dna_features_viewer)"); }
                if !missing.is_empty() {
                    figure_nudges += 1;
                    transcript.push_str(&format!(
                        "\n[system] 先别收尾。这个课题点名要出下面这种出版级图,但你还没加载对应的图形引擎技能,\
                         柱状图或内联 ```smiles 块替代不了它们:\n- {}\n请现在 {{\"tool\":\"load_skill\",\"name\":\"…\"}} \
                         加载对应技能并严格照卡出图(结构图/分子图/基因图各走各自的真引擎),出完再收尾。\
                         **例外:若只是【展示】某个 PDB 的三维结构,直接输出 ```protein 块(交互式 3D,秒出、免安装)即可,\
                         不必装 PyMOL;PyMOL 只留给带标注的出版级静态特写。** \
                         Load the missing figure-engine skill(s) and render the real publication figure before finishing.\n",
                        missing.join("\n- ")
                    ));
                    continue;
                }
            }
            // CITATION GATE: the run gathered real sources but the report cites NONE of them ([Sn]=0).
            // Push once to thread the markers in — a report making claims with zero clickable provenance
            // defeats the product's core promise (observed: an 84-source PCSK9 report with 0 [Sn]).
            if nudge_ok && sources.len() >= 5 && !f.contains("[S") && !cite_nudged {
                cite_nudged = true;
                transcript.push_str(&format!(
                    "\n[system] 你已检索到 {} 条来源,但报告正文里一个 [Sn] 角标都没有 —— 关键结论/数字必须挂上对应来源的 [Sn](如 [S3]、[S12],用你实际检索到的 sources 的真实编号),否则读者无法溯源,这违背本产品最核心的承诺。请给正文的关键论断补上 [Sn] 角标,补完再 {{\"final\":…}}。You gathered sources but cited none — add [Sn] markers to the key claims before finalizing.\n",
                    sources.len()
                ));
                continue;
            }
            // BREADTH GATE: a DEEP research report standing on very few registered sources is thin — the
            // model has either under-searched or leaned on memory. (Benchmarking vs a web-research baseline:
            // 8/4/0 registered sources where the baseline had 15-19.) Push ONCE to broaden: several DISTINCT
            // angles via litsearch/multi_search + scidb.cite on the records, especially landmark clinical /
            // competing-approach / negative results. Research mode only; one nudge; only if budget remains.
            // NOTE: threshold deliberately LOW (5, was 8). Benchmarking showed a high bar here drove the
            // model to keep RETRIEVING (126-144 registered sources) without deepening the argument — the
            // evidence-DEPTH gate below is what carries quality now; this one only catches a starved run.
            if nudge_ok && pause.delegate_encouraged() && !breadth_nudged && n >= 3 && sources.len() < 5 {
                breadth_nudged = true;
                transcript.push_str(&format!(
                    "\n[system] 收尾前先补检索广度。你目前只登记了 {} 条可引用来源,对一份深度研究简报偏少——很容易漏掉里程碑证据或变成凭记忆。请再做几组【不同角度】的检索(litsearch/multi_search + 对要引用的记录 scidb.cite 登记),重点补:关键临床读出/III期数据、竞争性方案/路线、以及反面或阴性结果;把关键结论都挂到真实登记的 [Sn] 上,再收尾。别为了收尾而少检索。Too few registered sources for a deep report — run several MORE distinct searches (landmark/clinical, competing approaches, negative results), scidb.cite the records, then finalize.\n",
                    sources.len()
                ));
                continue;
            }
            // EVIDENCE-DEPTH GATE (the one that actually carries quality). Retrieval volume ≠ depth:
            // benchmarking found a run with 126 registered sources citing 29 of them yet stating only ~4
            // real effect sizes, while a 23-source baseline produced 37 — and for the SAME drug the
            // baseline extracted 6 numbers from ONE paper where this run extracted none and got the trial
            // facts wrong. So when the run HAS sources but the draft is quantitatively thin, do NOT send
            // it to search more (that made it worse) — send it BACK INTO the sources it already cited to
            // pull the numbers out. Research mode, one nudge, budget permitting.
            // Threshold calibrated against the web-research baseline (52-57 quantitative claims on these
            // topics vs 5-9 here). Measured behaviour drove both constants: with the bar at 20 one run
            // stopped at EXACTLY 20 (the bar is what binds, so raise it) and another stalled at 13 after a
            // single push (so allow a second pass). 30 with 2 passes; still met by mining cited sources.
            if nudge_ok && pause.delegate_encouraged() && depth_nudges < 2 && n >= 3
                && sources.len() >= 5 && quant_claim_count(&f) < 30
            {
                depth_nudges += 1;
                transcript.push_str(&format!(
                    "\n[system] 先别收尾:你检索到了 {} 条来源,但报告里只有约 {} 处带具体数字的论断——这是「宽而薄」。\
                     **现在调用 {{\"tool\":\"extract_evidence\"}}**——它会把你已登记的来源逐条榨成 \
                     `[Sn] | 结论 | 数字+单位 | 人群/n | 终点 | 证据等级` 的证据表,然后你【照着表】把数字写进正文。\
                     **不要再去检索新文献**,回到你【已经引用】的那几条来源里,把数字抽出来写进正文:样本量(n=)、\
                     效应量(HR/OR/DC50/IC50/Dmax/%降解)、终点读出(mPFS/OS/ORR)、剂量与浓度、毒性等级与发生率、\
                     统计量(p/95%CI)。一条高质量来源应该能榨出好几个数字。每个数字紧跟它的 [Sn]。\
                     同时:凡是写进「临床试验」类表格的行,NCT号/期别/状态/适应症必须来自你真实检索到的记录,\
                     没有真实 NCT 的化合物不许放进临床表(放到临床前小节);别把设计原理写成已观测到的结果。\
                     ⚠️**不许为了凑数硬塞数字**:每个数字必须是来源里真实报告的量(不是你推算/举例/编的),\
                     必须紧跟它的 [Sn],且要服务于论断(说明该证据支持或反对什么)。凑出来的空数字比没有更糟。\
                     补完再收尾。Don't search more — mine the sources you already cited for real numbers; never pad.\n",
                    sources.len(), quant_claim_count(&f)
                ));
                continue;
            }
            // SELF-CRITIQUE GATE (quality > budget cutoff): before accepting the FINAL report, make the
            // agent critique its own draft ONCE against the ORIGINAL question + an evidence checklist.
            // This is the "verify before done" pattern (grok /goal, biomni self_critic) — it catches
            // drift (答非所问), unsupported claims, and figures-not-from-data WITHOUT a heavy extra model,
            // and it's the quality net that replaces a hard budget cutoff. Bounded to one pass. RESEARCH
            // mode only (deep reports, where drift/quality is the risk) — analysis mode stays fast; a
            // sub-agent (delegate not encouraged) never reflects. Skipped for a tiny run (nothing to check).
            if nudge_ok && !reflected && pause.delegate_encouraged() && n >= 3 {
                reflected = true;
                transcript.push_str(&format!(
                    "\n[system] 收尾前做一次自检(只此一次,别拖延)。对照你正要提交的报告逐条核对,不达标的先补齐;都达标就收尾 —— **收尾方式:先一行 ===AXIOM3_FINAL=== 再直接写裸 Markdown 报告(不要 JSON、不要转义)**:\n\
                     1) 答的是不是用户【原题】「{}」?开篇第一段有没有正面、直接回答它(而不是跑偏到别的角度)?\n\
                     2) 每条关键结论/数字是不是都有真实 [Sn] 来源支撑?有没有编造或凭记忆?没来源的先补检索或删掉。**特别检查具体数字——效应量(Cohen's d/HR/OR)、百分比、剂量、统计量、p 值这类,凡是没挂 [Sn] 的,要么补上真实来源,要么显式标注为「估计/示意值」,绝不把无源数字当既定事实列出。绝不写『检索结果:…关键文献包括』这种打包式引用,只引用真实登记过的 [Sn]。**\n\
                     3) **证据分级**:人体研究有没有按【真实设计】标注(首次人体/开放标签/单臂/pilot/样本量 n/II期RCT)?**绝不把开放标签 pilot 或早期小样本说成「II 期」「已验证疗效」**;机制/临床两环因果分别说清证据强度。药名/NCT号/ChEMBL ID 只写真实检索回填到的,拼写与记录一致,别自撰试验臂细节。**临床试验表纪律:没有真实 NCT 号的化合物不许进「临床试验」表(放临床前小节);同一个药全文的 NCT/期别/适应症必须唯一且自洽(别出现「已完成…状态招募中」这种自相矛盾);别把设计原理(rationale)写成已观测到的结果(outcome)——尤其当你引用的那篇文献其实报告了相反的不良事件。**\n\
                     3b) **数字密度**:每个一级章节至少要有几条【带具体数字】的证据(n=/效应量/剂量/终点/毒性等级/统计量),而不是通篇「X 等报道了…」这种书目式指针。数字不够就回到已引用的来源里去抽,别靠再检索凑数。\n\
                     3c) **禁止关于「自己核验过程」的全称陈述**:不许写「所有引用均经检索验证」「所有状态均已核实」这类话——除非每条都真的逐条核过。要写就逐条标注真实状态(已读全文/仅核对元数据/仅检索命中未读全文)。**对核验过程本身说大话,比措辞含糊更严重。**\n\
                     3d) **参考文献可点击**:每条参考文献尽量带 DOI 或 URL;拿不到全文的条目显式标「未获取全文,仅据摘要/二次转述」。\n\
                     4) **竞争假说**:是不是给了 ≥3 条【真正不同的路线级】假说/技术路线(不是同一路线内部的一次二选一),每条带支持证据 + 软肋 + 鉴别性证据 + 有理由的取舍/采信,并给出可证伪该判断的观测?\n\
                     5) 图是不是都来自真实检索/计算的数据(不是示意图)?课题点名要的图都出了吗?手里已检索到结构(PDB)却没出结构图吗?\n\
                     6) 有没有该用而没用的数据库(靶点→opentargets、变异→clinvar/gnomad、试验→clinical_trials、GWAS→gwas_catalog)?有没有明显缺口/自相矛盾?诚实标注局限,别硬撑。\n\
                     哪几条不达标就补哪几条,补完再 final。Self-critique against the ORIGINAL question + evidence; fix gaps; THEN finalize.\n",
                    goal.chars().take(220).collect::<String>()
                ));
                continue;
            }
            // DANGLING-CITATION GATE: the report cites a [Sn] whose n EXCEEDS the number of sources
            // actually retrieved — the model invented a citation number it can't back (observed under
            // thin retrieval: a 2-source run citing [S7]). A [Sn] with no matching source is a dead
            // provenance link + reads as a fabricated citation, violating the product's anti-fabrication
            // promise. Push ONCE to ground the claim with a real search or drop the unsupported marker.
            // Research mode only (deep cited reports); the strip backstop below is the hard guarantee.
            if nudge_ok && pause.delegate_encouraged() && !dangling_nudged
                && max_cited_marker(&normalize_citations(&f)) as usize > sources.len()
            {
                dangling_nudged = true;
                let ns = sources.len();
                transcript.push_str(&format!(
                    "\n[system] 报告里出现了超出范围的引用(如 [S{}]),但你实际只检索到 {ns} 条来源,有效编号只有 [S1]–[S{ns}]。超范围的 [Sn] 指向不存在的来源 = 悬空/编造引用,会变成点不开的死链,违背本产品最核心的溯源承诺。二选一:①对这些论断补一次真实检索、用拿到的真实编号引用;②删掉无据的引用角标(或把该数字/论断标注为「估计」)。改完再收尾(先一行 ===AXIOM3_FINAL=== 再裸 Markdown)。Some [Sn] exceed the {ns} sources you retrieved — ground them with a real search or remove the unsupported markers, then finalize.\n",
                    max_cited_marker(&f)
                ));
                continue;
            }
            // GUARD the accepted final: if the agent's thinking/tool-call echo leaked INTO the "final"
            // value (e.g. {"final":"THINKING: … {\"tool\":\"python\",\"code\":…}"}), or the final is empty,
            // salvage a real report / honest banner instead of shipping the raw blob. A short but CLEAN
            // final (no markers, e.g. "created hello.txt") ships as-is, so write-mode short finals still
            // work. (Closes the primary residual leak the audit found — the {"final"} accept path.)
            // RESEARCH mode (a deep, multi-section cited report IS the deliverable — the same signal the
            // self-critique and figure gates use) gets one more gate: reject a "final" that doesn't look
            // like a real report (e.g. raw Python dumped right after ===AXIOM3_FINAL===, which carries no
            // journal markers) → salvage instead. A real research report is always ≥200 prose chars, so
            // this never over-rejects one; a short compute/analysis final (no deep-report demand) and a
            // terse write-mode final ("created hello.txt") keep their exemption.
            let report_mode = pause.delegate_encouraged();
            let summary = if has_journal_markers(&f) || f.trim().is_empty() || (report_mode && !looks_like_real_report(&f)) {
                guard_done_summary(f.to_string(), model, &sys, &transcript, &sources, cancel).await
            } else {
                f.to_string()
            };
            // BACKSTOP: never ship a leaked ===AXIOM3_FINAL=== marker or a dead provenance link. Strip a
            // residual marker + any DANGLING [Sn] (n > sources retrieved) that survived the nudge — claim
            // text stays, only the unbacked marker is dropped. No-op for a clean report / fallback banner.
            let summary = clean_summary(&summary, sources.len());
            // RESOLVABILITY: every cited [Sn] must be lookup-able by the reader. Append any cited-but-
            // unlisted source (with its URL) to the reference section — closes the "[S91] cited but the
            // bibliography only lists 30 entries" hole AND gives entries a real URL to click.
            let summary = ensure_reference_list(&summary, &sources);
            let status = if is_fallback_report(&summary) { "no_report" } else { "done" };
            return CodingResult { status: status.into(), summary, steps: n, figures: all_figures, artifacts: collect_artifacts(sandbox.root(), &start_files), sources: sources.clone() };
        }
        let mut tool = v.get("tool").and_then(|x| x.as_str()).unwrap_or("").to_string();
        // ROBUSTNESS: models sometimes emit a tool call but FORGET the "tool" key (just the args) —
        // which landed as `unknown tool ''` and trapped them in a repeat loop (the model can't guess
        // what the harness wanted). Infer the intended tool from the args' distinctive signature so the
        // call still runs. Order matters: check the most specific keys first.
        if tool.is_empty() {
            tool = if v.get("queries").is_some() {
                "multi_search"
            } else if v.get("url").is_some() {
                "web_fetch"
            } else if v.get("code").is_some() {
                "python"
            } else if v.get("cmd").is_some() {
                "run"
            } else if v.get("packages").is_some() {
                "conda_install"
            } else if v.get("task").is_some() {
                "delegate"
            } else if v.get("pattern").is_some() {
                "glob"
            } else if v.get("todos").is_some() {
                "plan"
            } else if v.get("path").is_some() {
                if v.get("content").is_some() { "write_file" } else { "read_file" }
            } else if v.get("query").is_some() {
                "litsearch"
            } else if v.get("name").is_some() {
                "load_skill"
            } else {
                ""
            }
            .to_string();
        }
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
        // DELEGATE NUDGE: like the intake gate, GLM won't open a multi-agent sub-team on its own even
        // in research mode — so the dynamic multi-agent capability never actually fires. A few steps in
        // (after it has scoped + done some initial breadth), push it ONCE to fan out its hardest/broadest
        // branch, then relent so a genuinely single-angle topic isn't forced to sub-divide.
        if pause.delegate_encouraged()
            && !delegated_once
            && delegate_nudges < 1
            && n >= 4
            && !matches!(tool.as_str(), "delegate" | "ask_user" | "plan")
        {
            delegate_nudges += 1;
            transcript.push_str(
                "\n[system] 你在深度研究模式,手上有一支能并行工作的子智能体团队(delegate 工具)。这个课题是多面的:\
                 现在挑出你最宽、最硬的一个分支——竞争假说的权衡、跨领域机制普查、或正反证据的系统梳理——用一个 \
                 {\"tool\":\"delegate\",\"task\":\"…(自足、值得一支小队深挖的子问题)\"} 开子队去攻,等它带回结论再综合。\
                 现在就发这个 delegate 动作,先别做别的。You are in deep-research mode with a sub-agent team available; \
                 delegate your single broadest/hardest branch NOW via one {\"tool\":\"delegate\",\"task\":…} call.\n",
            );
            continue;
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
                match name { // satisfy the figure-engine gate once the right card is loaded
                    "protein-structure-figure" => loaded_struct = true,
                    "small-molecule-figure" => loaded_mol = true,
                    "gene-context-figure" => loaded_gene = true,
                    _ => {}
                }
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
                        // China mirror (TUNA) first, official pypi.org as EXTRA fallback — see
                        // pip_mirror_env in workbench.rs. Mainland networks otherwise stall on pypi.org.
                        let code = format!(
                            "import subprocess, sys, os\n\
                             _env = dict(os.environ)\n\
                             _env.setdefault('PIP_INDEX_URL', 'https://pypi.tuna.tsinghua.edu.cn/simple')\n\
                             _env.setdefault('PIP_EXTRA_INDEX_URL', 'https://pypi.org/simple')\n\
                             _r = subprocess.run([sys.executable,'-m','pip','install','--quiet',\
                             '--disable-pip-version-check', {list_py}], capture_output=True, text=True, env=_env)\n\
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
            // ORCHESTRATION: evidence extraction as an explicit MAP-REDUCE stage. Benchmarking showed the
            // core weakness is not retrieval but EXTRACTION: a single loop juggling search+read+reason+
            // write registers 126 sources yet states ~4 real numbers, while a stronger reference model
            // pulls 6 numbers out of ONE paper. Asking the same model to "read deeper" inside the big loop
            // does not fix that. Instead: MAP a tight, bounded sub-call over each stored source excerpt
            // (a small focused task — what a mid-tier model is actually good at), REDUCE into a structured
            // evidence table, then let the agent compose the report FROM that table.
            "extract_evidence" => {
                let want: Vec<u32> = v
                    .get("ids")
                    .and_then(|x| x.as_array())
                    .map(|a| a.iter().filter_map(|x| x.as_u64().map(|n| n as u32)).collect())
                    .unwrap_or_default();
                let pool: Vec<&Source> = sources
                    .iter()
                    .filter(|s| !s.excerpt.trim().is_empty())
                    .filter(|s| want.is_empty() || want.contains(&s.id))
                    .collect();
                if pool.is_empty() {
                    ("extract_evidence".to_string(),
                     "ERROR: 还没有带原文摘录的已登记来源。先 litsearch/multi_search 或对 scidb 记录 scidb.cite 登记,再抽取。".to_string())
                } else {
                    const EX_SYS: &str = "你是证据抽取器。只做一件事:从给定的来源原文摘录里,抽出可用于论证的【定量事实】。\n\
                        每条一行,严格用这个格式:[Sn] | 结论(一句) | 数字+单位 | 人群或样本量 | 终点/指标 | 证据等级\n\
                        规则:①只写摘录里【真实出现】的数字,绝不推算、绝不补全、绝不凭常识填;②一条来源可以抽多行;\
                        ③摘录里没有任何定量信息就写 `[Sn] | NONE`;④证据等级按摘录写(如 III期RCT/II期/开放标签pilot/临床前/体外/综述);\
                        ⑤不要输出除这些行以外的任何内容(不要前言、不要总结)。";
                    // Bounded map: cap the fan-out and batch a few sources per sub-call to keep it cheap.
                    let capped: Vec<&&Source> = pool.iter().take(12).collect();
                    let mut table = String::new();
                    let mut got = 0usize;
                    for chunk in capped.chunks(3) {
                        if cancel.is_cancelled() { break; }
                        let mut user = String::new();
                        for s in chunk {
                            user.push_str(&format!(
                                "\n=== [S{}] {} ===\n{}\n",
                                s.id,
                                s.title.chars().take(200).collect::<String>(),
                                s.excerpt.chars().take(2500).collect::<String>()
                            ));
                        }
                        user.push_str("\n按格式抽取上面每条来源的定量事实:");
                        if let Ok(out) = model.complete(EX_SYS, &user, cancel).await {
                            for line in out.lines() {
                                let l = line.trim();
                                if l.starts_with("[S") && !l.contains("| NONE") {
                                    table.push_str(l);
                                    table.push('\n');
                                    got += 1;
                                }
                            }
                        }
                    }
                    let obs = if got == 0 {
                        "抽取完成:这些来源的摘录里没有可用的定量事实。别编数字——要么换更具体的检索(补临床读出/效应量),要么在报告里如实说明该处缺定量证据。".to_string()
                    } else {
                        format!(
                            "从 {} 条来源抽出 {} 条定量证据(每条已绑定 [Sn],可直接写进报告正文):\n{}\n\
                             ⚠️ 只用上表里的数字,别添别的;每个数字在正文里都要紧跟它的 [Sn]。",
                            capped.len(), got, table.trim_end()
                        )
                    };
                    (format!("{} 条来源", capped.len()), obs)
                }
            }
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
                } else if n_done >= lit_cap {
                    lit_queries.push(q.clone());
                    format!("[stop] litsearch 已达 {n_done} 次,文献广度足够了——**别再发新的文献检索**。但不要因此牺牲质量就草草收尾:若还有关键数据缺口(某个亚组的逐试验 HR、某个具体数字),改用 **web_fetch 精确打开你已知的那几个试验/数据库页面**把缺口补齐;补齐后再综合。确认覆盖到用户原始诉求的每一问、关键数字有交叉核对,才 {{\"final\":…}}. litsearch cap reached — stop NEW literature queries, but DON'T sacrifice quality: fill any remaining data gaps via web_fetch on the specific known pages, cross-check the key numbers, then synthesize.")
                } else {
                    let hits = match LitSearch::new().search(&q, 5).await {
                        Ok(works) if works.is_empty() => "(no matching works)".to_string(),
                        Ok(works) => works
                            .iter()
                            .map(|w| {
                                let authors = if w.authors.is_empty() { "—".into() } else { w.authors.join(", ") };
                                let year = w.year.map(|y| y.to_string()).unwrap_or_else(|| "—".into());
                                let doi = w.doi.clone().unwrap_or_default();
                                let abs: String = w.abstract_text.chars().take(280).collect();
                                src_n += 1;
                                sources.push(Source {
                                    id: src_n,
                                    kind: "paper".into(),
                                    title: w.title.clone(),
                                    url: if doi.is_empty() { String::new() } else if doi.starts_with("http") { doi.clone() } else { format!("https://doi.org/{doi}") },
                                    excerpt: format!("{} ({year}) — {authors}\n{}", w.title, w.abstract_text.chars().take(1200).collect::<String>()),
                                });
                                format!("[S{}] {} ({year}) — {authors}\n    DOI: {}\n    {abs}", src_n, w.title, if doi.is_empty() { "—".into() } else { doi })
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
            "multi_search" => {
                // Concurrent literature search — fire several INDEPENDENT queries AT ONCE. These are
                // external HTTP calls (safe to parallelize, unlike model calls which 429), so this
                // collapses N sequential litsearch steps + N model round-trips into a SINGLE step,
                // cutting the biggest sequential wait in a research run. Same dedup + 10-total cap.
                let qs: Vec<String> = v
                    .get("queries")
                    .and_then(|x| x.as_array())
                    .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.trim().to_string())).filter(|s| s.chars().count() > 2).collect())
                    .unwrap_or_default();
                let mut fresh: Vec<String> = Vec::new();
                for q in qs {
                    if fresh.iter().any(|p| query_overlap(p, &q) >= 0.8) { continue; }
                    if lit_queries.iter().any(|p| query_overlap(p, &q) >= 0.8) { continue; }
                    fresh.push(q);
                }
                let remaining = lit_cap.saturating_sub(lit_queries.len());
                fresh.truncate(remaining.min(4)); // <=4 concurrent, and never past the 10-total litsearch cap
                let obs = if fresh.is_empty() {
                    format!("[skip] 这些检索与已检索高度重复或预算已用尽({} 次)。停止检索,基于已有文献出可交付物。", lit_queries.len())
                } else {
                    let mut set = tokio::task::JoinSet::new();
                    for (i, q) in fresh.iter().cloned().enumerate() {
                        set.spawn(async move {
                            let r = LitSearch::new().search(&q, 5).await;
                            (i, q, r)
                        });
                    }
                    let mut done = Vec::new();
                    while let Some(res) = set.join_next().await {
                        if let Ok(t) = res {
                            done.push(t);
                        }
                    }
                    done.sort_by_key(|(i, _, _)| *i);
                    let mut out = String::new();
                    for (_, q, res) in done {
                        lit_queries.push(q.clone());
                        out.push_str(&format!("\n### 检索:{q}\n"));
                        match res {
                            Ok(works) if works.is_empty() => out.push_str("(no matching works)\n"),
                            Ok(works) => {
                                for w in works.iter() {
                                    let authors = if w.authors.is_empty() { "—".into() } else { w.authors.join(", ") };
                                    let year = w.year.map(|y| y.to_string()).unwrap_or_else(|| "—".into());
                                    let doi = w.doi.clone().unwrap_or_default();
                                    let abs: String = w.abstract_text.chars().take(240).collect();
                                    src_n += 1;
                                    sources.push(Source {
                                        id: src_n,
                                        kind: "paper".into(),
                                        title: w.title.clone(),
                                        url: if doi.is_empty() { String::new() } else if doi.starts_with("http") { doi.clone() } else { format!("https://doi.org/{doi}") },
                                        excerpt: format!("{} ({year}) — {authors}\n{}", w.title, w.abstract_text.chars().take(1200).collect::<String>()),
                                    });
                                    out.push_str(&format!("[S{}] {} ({year}) — {authors}\n    {abs}\n", src_n, w.title));
                                }
                            }
                            Err(e) => out.push_str(&format!("ERROR: {e}\n")),
                        }
                    }
                    out.push_str("\n[并发检索完成]基于以上来源继续深挖或收敛出可交付物。");
                    out
                };
                (format!("并发检索 {} 条", fresh.len()), obs)
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
                            .map(|r| {
                                src_n += 1;
                                sources.push(Source {
                                    id: src_n,
                                    kind: "web".into(),
                                    title: r.title.clone(),
                                    url: r.url.clone(),
                                    excerpt: format!("{}\n{}\n{}", r.title, r.url, r.snippet),
                                });
                                format!("[S{}] {}\n    {}\n    {}", src_n, r.title, r.url, r.snippet)
                            })
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
                        Ok(t) => {
                            src_n += 1;
                            let title = t.lines().find(|l| !l.trim().is_empty()).unwrap_or(&url).chars().take(120).collect::<String>();
                            sources.push(Source {
                                id: src_n,
                                kind: "web".into(),
                                title,
                                url: url.clone(),
                                excerpt: t.chars().take(4000).collect(),
                            });
                            format!("[S{src_n}] 来源 {url}\n\n{t}")
                        }
                        Err(e) => format!("ERROR: web_fetch failed: {e}"),
                    }
                };
                (url, obs)
            }
            "multi_fetch" => {
                // Concurrent read of SEVERAL known URLs AT ONCE (external HTTP — safe to parallelize; 429
                // only hits model calls). Collapses N sequential web_fetch steps + N model round-trips into
                // ONE — a big wall-clock win when you have a list of pages/trial records to read. Give the
                // URLs you ALREADY have (from web_search / a database result).
                let mut seen = std::collections::HashSet::new();
                let urls: Vec<String> = v
                    .get("urls")
                    .and_then(|x| x.as_array())
                    .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.trim().to_string())).filter(|s| s.starts_with("http")).collect::<Vec<_>>())
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|u| seen.insert(u.clone()))
                    .take(6)
                    .collect();
                let obs = if urls.is_empty() {
                    "ERROR: multi_fetch 需要 `urls`: [\"https://…\", …](你已经拿到的 URL 列表)".to_string()
                } else {
                    let mut set = tokio::task::JoinSet::new();
                    for (i, u) in urls.iter().cloned().enumerate() {
                        set.spawn(async move {
                            let r = WebClient::new().fetch(&u).await;
                            (i, u, r)
                        });
                    }
                    let mut done = Vec::new();
                    while let Some(res) = set.join_next().await {
                        if let Ok(t) = res {
                            done.push(t);
                        }
                    }
                    done.sort_by_key(|(i, _, _)| *i);
                    let mut out = String::new();
                    for (_, u, res) in done {
                        match res {
                            Ok(t) if t.trim().is_empty() => out.push_str(&format!("\n### {u}\n(无可读文本)\n")),
                            Ok(t) => {
                                src_n += 1;
                                let title = t.lines().find(|l| !l.trim().is_empty()).unwrap_or(&u).chars().take(120).collect::<String>();
                                sources.push(Source { id: src_n, kind: "web".into(), title, url: u.clone(), excerpt: t.chars().take(4000).collect() });
                                out.push_str(&format!("\n### [S{}] {}\n{}\n", src_n, u, t.chars().take(3000).collect::<String>()));
                            }
                            Err(e) => out.push_str(&format!("\n### {u}\nERROR: {e}\n")),
                        }
                    }
                    out
                };
                (format!("并发读取 {} 个页面", urls.len()), obs)
            }
            "delegate" if !pause.delegate_allowed() => {
                // Not deep-research (or already inside a sub-agent) — no delegation; do it yourself.
                ("delegate".to_string(), "[此处不开子智能体委派 —— 请自己完成这个子任务]".to_string())
            }
            "delegate" => {
                // Spawn a bounded multi-agent sub-team of TOOL-USING sub-agents for ONE complex,
                // separable subtask (each sub-agent runs the full litsearch/web/python loop).
                let task = arg_str(&v, "task").to_string();
                if task.trim().is_empty() {
                    ("delegate".to_string(), "ERROR: delegate needs a `task` — a self-contained subtask worth a whole sub-team.".to_string())
                } else if delegate_calls >= 2 {
                    // Hard cap: a sub-team costs minutes; beyond two, finish the remaining branches
                    // yourself so the run converges instead of sprawling into more sequential sub-agents.
                    ("delegate".to_string(), "[已达委派上限(每次运行最多 2 个子团队)—— 请自己完成剩余的子问题,然后收尾出报告]".to_string())
                } else {
                    delegated_once = true; // satisfies the delegate nudge — don't push again
                    delegate_calls += 1;
                    let sub_budget = CodingBudget {
                        // Tighter than the parent — a delegated sub-agent attacks ONE focused sub-question,
                        // so 12 steps is ample and keeps the (sequential) sub-team from ballooning the run.
                        max_steps: 12,
                        per_call_timeout_s: budget.per_call_timeout_s,
                        cmd_timeout_s: budget.cmd_timeout_s,
                        cell_timeout_s: budget.cell_timeout_s,
                    };
                    let (obs, figs, merged) = delegate_tooled(model, sandbox, &kernel, &sub_budget, cancel, &task, src_n, &mut on_step).await;
                    step_figures = figs; // sub-agents' computed figures flow into the parent report (deduped downstream)
                    // Renumber-merge: the sub-agents' real sources come back in the parent's global [Sn]
                    // space (ids src_n+1…); adopt them so delegated findings are clickable in the report.
                    src_n += merged.len() as u32;
                    sources.extend(merged);
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
                            // Register any sources the model declared via `scidb.cite(...)` so records
                            // fetched through python+scidb become citable [Sn] (not fabricated).
                            o = capture_cited_sources(&o, &mut sources, &mut src_n);
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
            // Specialized science-database tools (PubChem / UniProt / PDB / gene / trials / EuropePMC):
            // dispatched through the registry so adding one is a registry entry, not a new match arm.
            t if is_sci_tool(t) => {
                let arg = arg_str(&v, sci_tool_arg(t)).to_string();
                let rec = run_sci_tool(t, &arg).await;
                let obs = if rec.starts_with("ERROR") || rec.starts_with('(') {
                    rec
                } else {
                    src_n += 1;
                    sources.push(Source {
                        id: src_n,
                        kind: "db".into(),
                        title: format!("{t} · {arg}"),
                        url: String::new(),
                        excerpt: rec.chars().take(1500).collect(),
                    });
                    format!("[S{src_n}] {rec}")
                };
                (arg.chars().take(60).collect::<String>(), obs)
            }
            other => (
                other.to_string(),
                format!(
                    "ERROR: 无法识别工具 '{other}' —— 多半是你漏了 \"tool\" 字段,或工具名拼错了。请重发一个**完整**的调用,\
                     形如 {{\"tool\":\"litsearch\",\"query\":\"…\"}}。可用工具:litsearch / multi_search / web_search / \
                     web_fetch / python / run / read_file / write_file / plan / load_skill / delegate,以及各原生图块\
                     (```protein / ```mermaid / ```network …)。别再重发同样的错误调用。"
                ),
            ),
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
            "multi_search" => v.get("queries").and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(" | ")).unwrap_or_default(),
            "web_fetch" => arg_str(&v, "url").to_string(),
            "multi_fetch" => v.get("urls").and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(" | ")).unwrap_or_default(),
            "delegate" => arg_str(&v, "task").to_string(),
            t if is_sci_tool(t) => arg_str(&v, sci_tool_arg(t)).to_string(),
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
    // figures (falls back to a transcript tail on failure). Same salvage the model-error path uses.
    let summary = clean_summary(&synthesize_wrapup(model, &sys, &transcript, &sources, cancel).await, sources.len());
    let status = if is_fallback_report(&summary) { "no_report" } else { "done" };
    CodingResult { status: status.into(), summary, steps: budget.max_steps, figures: all_figures, artifacts, sources }
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
                // The previous char was a backslash. Keep it as an escape ONLY for the sequences JSON
                // actually defines and a report would use (\" \\ \/ \n \r \t). Anything else — \sigma,
                // \frac, \{ etc. from LaTeX the model drops into a {"final":"…"} string — is a LONE
                // backslash that makes serde reject the whole envelope (which then gets dumped raw as
                // "乱码"). Escape it so the final parses and the LaTeX survives as literal text.
                const KEEP: &[char] = &['"', '\\', '/', 'n', 'r', 't'];
                if !KEEP.contains(&c) {
                    out.push('\\');
                }
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
/// Which publication figure engine(s) a GOAL explicitly demands: (structure, molecule, gene-context).
/// Keyword lists are deliberately SPECIFIC so a casual mention ("含突变") doesn't force a heavy engine
/// install — the gate only fires when the topic clearly names the kind of figure it wants.
fn figure_needs(goal: &str) -> (bool, bool, bool) {
    let g = goal.to_lowercase();
    let hit = |ks: &[&str]| ks.iter().any(|k| g.contains(k));
    (
        hit(&["结构图", "蛋白结构", "晶体结构", "激酶结构", "催化域结构", "活性位点", "结合口袋", "结构域结构", "cryo-em 结构"]),
        hit(&["2d 结构", "2维结构", "结构式", "分子结构", "小分子图", "药效团", "弹头", "母核", "smiles", "结构对比图"]),
        hit(&["基因背景", "遗传背景", "基因排布", "基因邻域", "基因盒", "操纵子", "质粒图", "可移动遗传", "转座子", "整合子"]),
    )
}

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

    #[test]
    fn figure_needs_matches_demanding_topics_not_casual_mentions() {
        // The two P1 test topics (verbatim shape) must each light up the right engine(s).
        let egfr = "各代 EGFR-TKI 的 2D 结构对比图,高亮共价弹头;并给 EGFR 激酶域结构图,标出 C797S 位点";
        let (s, m, g) = figure_needs(egfr);
        assert!(s && m, "EGFR topic must demand structure + molecule figures, got struct={s} mol={m}");
        assert!(!g, "EGFR topic must NOT trigger gene-context");
        let mcr = "mcr-1 的可移动遗传背景图(操纵子/质粒),MCR-1 催化域结构图标出活性位点,黏菌素的 2D 结构图";
        let (s, m, g) = figure_needs(mcr);
        assert!(s && m && g, "mcr topic must demand all three engines, got struct={s} mol={m} gene={g}");
        // A casual mention of a mutation / protein must NOT force a heavy structure-engine install.
        let casual = "分析 BRCA1 含某突变的患者预后,做一个生存曲线和风险比森林图";
        let (s, m, g) = figure_needs(casual);
        assert!(!s && !m && !g, "a casual mutation/survival topic must trigger no figure engine, got {s} {m} {g}");
    }

    #[test]
    fn sanitize_escapes_latex_so_a_latex_laden_final_parses() {
        // A {"final":…} carrying LaTeX (\sigma, \frac) is INVALID JSON as-is (\s is not a JSON escape)
        // → serde rejects it → the raw envelope used to get dumped as "乱码". Sanitize must fix it.
        let raw = r#"{"final":"参数 \sigma 增大,能量为 \frac{1}{2}mv^2"}"#;
        assert!(serde_json::from_str::<serde_json::Value>(raw).is_err(), "raw \\sigma is invalid json");
        let v: serde_json::Value =
            serde_json::from_str(&sanitize_json_strings(raw)).expect("sanitized should parse");
        let f = v["final"].as_str().unwrap();
        assert!(f.contains("\\sigma"), "LaTeX \\sigma preserved literally: {f}");
        assert!(f.contains("\\frac"), "LaTeX \\frac preserved (not formfeed): {f}");
        assert!(!f.contains('\u{0c}'), "no stray formfeed from \\f");
    }

    #[test]
    fn remap_cite_markers_offsets_ids_and_keeps_utf8() {
        // sub-agent local ids are lifted into the parent's global space; Chinese text stays intact
        assert_eq!(remap_cite_markers("下调 PD-L1 [S3]。", 10), "下调 PD-L1 [S13]。");
        assert_eq!(remap_cite_markers("a[S1]b[S2]c", 5), "a[S6]b[S7]c");
        // offset 0 is a no-op (local ids already ARE the global ids — first sub-agent, empty parent)
        assert_eq!(remap_cite_markers("keep [S2] as-is", 0), "keep [S2] as-is");
        // not-a-marker sequences are left untouched even with an offset
        assert_eq!(remap_cite_markers("[S] [SX] [Sa] array[5]", 4), "[S] [SX] [Sa] array[5]");
        // an unterminated "[S3" (no closing bracket) is preserved verbatim
        assert_eq!(remap_cite_markers("dangling [S3 tail", 7), "dangling [S3 tail");
    }

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
    async fn finalizes_via_raw_markdown_marker() {
        // NEW FINALIZE PROTOCOL: the report is RAW Markdown after ===AXIOM3_FINAL=== — real newlines,
        // unescaped quotes, no JSON. The loop must take everything after the marker as the final report
        // (this is the durable fix for the recurring broken-{"final"}-JSON envelope).
        let root = tmp("marker");
        let sb = Sandbox::new(&root).unwrap();
        let model = ScriptModel {
            replies: Mutex::new(vec![
                r#"{"tool":"list_dir","path":"."}"#.into(),
                "我已看完,现在收尾。\n===AXIOM3_FINAL===\n# 报告标题\n\n## 一、直接回答\n结论:这里有\"引号\"和\n真实换行,都不用转义,直接写。\n\n完。".into(),
            ]),
        };
        let mut steps = 0;
        let res = run_coding_agent("x", &sb, &model, &Cancel::new(), &Pause::new(), &CodingBudget::default(), None, |_| steps += 1, |_, _| {}).await;
        assert_eq!(res.status, "done");
        assert!(res.summary.starts_with("# 报告标题"), "got: {:?}", res.summary.chars().take(20).collect::<String>());
        assert!(res.summary.contains("\"引号\""), "raw quotes must survive (no JSON escaping)");
        assert!(res.summary.contains('\n'), "real newlines preserved");
        assert!(!res.summary.contains("AXIOM3_FINAL"), "the marker line must be stripped");
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
    async fn broken_tool_call_embedding_final_is_not_mistaken_for_a_report() {
        // REGRESSION (finalize-protocol audit): a write_file/python tool call whose STRING ARG contains
        // an unescaped "final":"…" makes the JSON broken AND contain the substring "final". It must NOT be
        // textually recovered as a final report (which would ship a garbage fragment AND skip the real
        // action) — it's a malformed tool call → nudge → the REAL final on the next turn wins.
        let root = tmp("embfinal");
        let sb = Sandbox::new(&root).unwrap();
        let model = ScriptModel {
            replies: Mutex::new(vec![
                // broken JSON (unescaped inner quotes) whose args embed `"final"` — tool key comes first
                r#"{"tool":"write_file","path":"c.json","content":"{"final": "v1", "n": 3}"}"#.into(),
                r#"{"final":"真正的收尾:已完成分析。"}"#.into(), // the REAL final on the next turn
            ]),
        };
        let res = run_coding_agent("x", &sb, &model, &Cancel::new(), &Pause::new(), &CodingBudget::default(), None, |_| {}, |_, _| {}).await;
        assert_eq!(res.status, "done", "summary: {}", res.summary);
        assert!(res.summary.contains("真正的收尾"), "must reach the real next-turn final, got: {}", res.summary);
        assert!(!res.summary.contains("\"n\": 3") && !res.summary.contains("v1\""), "must not ship the broken tool call's fragment, got: {}", res.summary);
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
    fn guard_rejects_leaked_thinking_and_tool_echo_as_report() {
        // The exact 无产出 failure: a truncated reply (thinking narration + a cut-off tool call) shipped
        // as a "done" report. A LONG thinking narration is >200 prose chars, so length alone won't catch
        // it — the tell-tale markers must. All of these must be judged NOT a real report → salvaged.
        assert!(!looks_like_real_report(
            "THINKING: 我已获得 SELECT 试验核心定量数据（MACE HR=0.80），但还缺几块关键拼图，让我做最后一轮针对性检索补全这些缺口，然后即可综合撰写报告。<tool_call>tool\":\"multi_search\",\"queries\":[\"SELECT trial semaglutide 9."
        ));
        assert!(!looks_like_real_report("我先整理任务清单。\n```json\n{\"tool\":\"plan\",\"todos\":[\"a\",\"b\"]}\n```"));
        assert!(!looks_like_real_report(""));
        assert!(!looks_like_real_report("{\"tool\":\"python\",\"code\":\"print(1)\"}"));
        // A genuine markdown report passes (substantial narrative, no scaffolding markers).
        let report = "# 司美格鲁肽心血管获益机制\n\n## 一、直接回答\n本报告系统整合 SELECT 试验与机制证据，判定 MACE 下降部分独立于减重 [S1]。抗炎、内皮功能改善与直接心肌 GLP-1R 作用共同贡献，权衡三类竞争假说后给出机制判定与验证实验设计。\n\n## 二、证据分析\n证据显示体重配对分析中获益仍存在，支持非减重通路。多项机制研究提示内皮功能改善独立于减重幅度。\n\n## 三、可执行方案\n设计体重配对随机对照，读出 hs-CRP 与 FMD，判据为组间差异显著。";
        assert!(looks_like_real_report(report));
    }

    #[test]
    fn final_accept_marker_gate_keeps_short_clean_finals_but_catches_leaks() {
        // The {"final"} ACCEPT path gates on markers ALONE (not the >200-char prose rule): a SHORT but
        // CLEAN final must ship as-is (write-mode / test finals), while a final that smuggled thinking or a
        // tool-call/code echo inside its value must be caught and salvaged.
        for clean in ["created hello.txt", "looked at the dir", "computed 42 then 43", "main.rs prints hi; no issues.", "could not write"] {
            assert!(!has_journal_markers(clean), "clean short final wrongly flagged: {clean}");
        }
        assert!(has_journal_markers("THINKING: 先画个图。{\"tool\":\"python\",\"code\":\"import matplotlib\"}"));
        assert!(has_journal_markers("<tool_call>tool\":\"multi_search\""));
        // markers INSIDE a ```fence``` (a legit report quoting JSON) do NOT trip the gate.
        assert!(!has_journal_markers("## 报告\n正文很充分。\n```json\n{\"tool\":\"x\"}\n```\n结论。"));
    }

    #[test]
    fn unwrap_recovers_final_wrapped_report() {
        // The salvage model wrapped its report in {"final":"…"} (obeying the main system prompt) after a
        // "我已…" preamble → the deliverable showed literal {"final":"##…\n\n…"}. unwrap must recover the
        // clean markdown (serde unescapes \n) and drop the preamble.
        let raw = "我已收集全部核心证据。\n\n{\"final\":\"## 报告标题\\n\\n### ① 直接回答\\n基于最高等级证据，判定获益部分独立于减重 [S1]。抗炎、内皮功能改善与直接心肌作用共同贡献，权衡三类竞争假说后给出机制判定与决定性验证实验设计，落到可照做的方案与判据，覆盖体系材料、方法参数、读出指标、判定判据与对照组设置。\"}";
        let out = unwrap_report_envelope(raw);
        assert!(out.starts_with("## 报告标题"), "got: {}", &out[..out.len().min(40)]);
        assert!(out.contains('\n') && !out.contains("\\n"), "newlines must be real, not literal");
        assert!(!out.contains("{\"final\""), "the {{final}} wrapper must be gone");
        // A plain prose report is returned unchanged (no false unwrap of an embedded ```json example).
        let prose = "## 真实报告\n\n正文分析很长，其中提到一个示例 ```json\n{\"final\":\"x\"}\n``` 但整体是散文报告，占绝大多数篇幅，不应被误当作信封解开处理掉正文内容内容内容内容。";
        assert_eq!(unwrap_report_envelope(prose), prose.trim());
    }

    #[test]
    fn recovers_broken_final_envelope_textually() {
        // GLM emitted {"final":"…"} for a long report but the JSON is BROKEN — a RAW newline inside the
        // value + a garbled close `」}` (no closing quote) → serde + sanitize both fail. Must recover the
        // body textually, unescape \n, drop the preamble + envelope. (Fixes the raw {"final":"…\n…"} blob.)
        let raw = "自检已通过。现在输出最终报告。\n\n{\"final\":\"# 报告标题\\n\\n## 一、直接回答\\n结论成立且有据 [S1]。表格 | A | B |\\n这里混进一个真实换行\n破坏了 JSON。\\n\\n## 参考文献\\n- [S1]」}";
        let out = unwrap_report_envelope(raw);
        assert!(out.starts_with("# 报告标题"), "got head: {:?}", out.chars().take(20).collect::<String>());
        assert!(out.contains('\n') && !out.contains("\\n"), "newlines must be real, not literal");
        assert!(!out.contains("{\"final\""), "the envelope wrapper must be gone");
        assert!(out.contains("[S1]"), "report content must be preserved");
    }

    #[test]
    fn textual_final_keeps_full_body_when_close_is_missing_despite_interior_braces() {
        // REGRESSION (finalize-protocol audit): a broken/truncated {"final":"…"} whose CLOSING `"}` is
        // absent and whose body has interior braces (LaTeX `\frac{a}{b}`, set `{x}`, a ```json example)
        // must NOT be truncated at the last INTERIOR `}` — the whole (truncated) body must survive.
        let raw = "{\"final\":\"# 结论\\n方程 \\\\frac{a}{b} 成立 [S1]。后面还有一整段很重要的分析和参考文献，绝不能被 {a}{b} 里的括号截断掉";
        let out = textual_final(raw).expect("recovers a body");
        assert!(out.contains("[S1]"), "content before interior braces kept: {out}");
        assert!(out.contains("绝不能被") && out.contains("截断掉"), "the tail AFTER an interior `}}` must survive: {out}");
        // A well-formed envelope (proper `"}` close) still trims exactly at the real end.
        let ok = textual_final("{\"final\":\"# 报告\\n正文 {x} 里有括号。\"}").expect("recovers");
        assert!(ok.contains("{x}") && !ok.contains("\"}"), "well-formed close trims correctly: {ok}");
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

    #[test]
    fn dangling_citation_helpers_detect_and_strip_out_of_range_markers() {
        // max_cited_marker: highest [Sn]; an unclosed `[S12` must NOT false-match.
        assert_eq!(max_cited_marker("结论 [S2] 与 [S7],还有 [S12 未闭合"), 7);
        assert_eq!(max_cited_marker("正文没有任何引用"), 0);
        // strip_dangling_markers: drop ONLY n > max_valid; keep valid markers + all claim text (incl. CJK).
        let r = "机制成立 [S1],证据充分 [S2];但这条 [S7] 和 [S4] 无据支撑。中文正文照旧保留。";
        let s = strip_dangling_markers(r, 2);
        assert!(s.contains("[S1]") && s.contains("[S2]"), "valid markers kept: {s}");
        assert!(!s.contains("[S7]") && !s.contains("[S4]"), "dangling markers dropped: {s}");
        assert!(s.contains("机制成立") && s.contains("中文正文照旧保留"), "claim text + CJK preserved: {s}");
        // max_valid = 0 (no sources retrieved) → every [Sn] is dangling.
        assert_eq!(strip_dangling_markers("凭空 [S1] 引用", 0), "凭空  引用");
    }

    #[test]
    fn clean_summary_strips_leaked_marker_and_dangling_together() {
        // A salvage/wrap-up writer sometimes re-prepends the marker AND cites out-of-range sources.
        // clean_summary must remove the leading ===AXIOM3_FINAL=== and drop [Sn] beyond the source count.
        let raw = "===AXIOM3_FINAL===\n# 报告\n结论 [S1] 与 [S9] 混合。";
        let out = clean_summary(raw, 2);
        assert!(!out.contains("AXIOM3_FINAL"), "marker stripped: {out}");
        assert!(out.starts_with("# 报告"), "report body kept: {out}");
        assert!(out.contains("[S1]") && !out.contains("[S9]"), "valid kept, dangling dropped: {out}");
        // Idempotent + no-op on a clean report.
        assert_eq!(clean_summary("# 干净\n正文 [S1]。", 3), "# 干净\n正文 [S1]。");
    }

    #[test]
    fn normalize_bundles_multiref_citations_so_gate_and_frontend_see_them() {
        // REGRESSION (benchmark found this): bundled `[S2, S3]` / `[S1;S16]` must become `[S2][S3]` so the
        // dangling gate + strip + the frontend's single-`[Sn]` clickable regex all see every ref.
        assert_eq!(normalize_citations("结论 [S2, S3] 与 [S1;S16] 成立"), "结论 [S2][S3] 与 [S1][S16] 成立");
        assert_eq!(normalize_citations("单个 [S4] 不变"), "单个 [S4] 不变");
        // Non-citation brackets untouched (must not eat [Section 2]/[START]/[SI]).
        assert_eq!(normalize_citations("[Section 2]、[START]、[SI] 不动"), "[Section 2]、[START]、[SI] 不动");
        // The exact T3 failure: a 9000-char report citing bundled [S1, S16] with only 3 real sources —
        // clean_summary normalizes then strips the out-of-range ref (no dead link ships), keeps valid ones.
        let cleaned = clean_summary("机制成立 [S2, S3];另见 [S1, S16]。", 3);
        assert!(cleaned.contains("[S2]") && cleaned.contains("[S3]") && cleaned.contains("[S1]"), "valid refs kept: {cleaned}");
        assert!(!cleaned.contains("[S16]"), "dangling [S16] stripped after normalization: {cleaned}");
    }

    #[test]
    fn quant_claim_count_measures_numbers_not_bibliography() {
        // Bibliographic pointers carry no evidence depth…
        let biblio = "Smith 等(2019)报道了该机制。Lopez 等(2012)提出了模型。Kortüm 等(2016)发现了耐药。";
        assert!(quant_claim_count(biblio) <= 1, "bibliographic pointers shouldn't count: {}", quant_claim_count(biblio));
        // …whereas real quantitative claims do.
        let quant = "血小板减少发生于 9/20(45%)。中位 OS 12.8 个月。DC50 为 0.130 nM。降解深度 60-72% vs ≥95%。\
                     入组 n=20 例。mPFS 5.0 vs 2.1 个月(HR 0.40,p<0.001)。";
        assert!(quant_claim_count(quant) >= 5, "quantitative claims must count: {}", quant_claim_count(quant));
        // Numbers inside a fenced code block don't inflate the count.
        assert_eq!(quant_claim_count("```python\nx = 45 # %\n```\n没有数字的正文。"), 0);
    }

    #[test]
    fn ensure_reference_list_makes_every_cited_source_resolvable() {
        // REGRESSION (benchmark found this): a report cited [S91] while its own 参考文献 listed far fewer
        // entries — the dangling gate passed (91 <= registered count) but the reader could not look it up.
        let sources = vec![
            Source { id: 3, kind: "paper".into(), title: "KT-474 首次人体研究".into(), url: "https://doi.org/10.1000/kt474".into(), excerpt: String::new() },
            Source { id: 91, kind: "db".into(), title: "ARV-471 试验记录".into(), url: "https://clinicaltrials.gov/study/NCT04072952".into(), excerpt: String::new() },
        ];
        let report = "# 报告\n结论 A [S3];结论 B [S91]。\n\n## 参考文献\n- [S3] KT-474 首次人体研究 — https://doi.org/10.1000/kt474\n";
        let out = ensure_reference_list(report, &sources);
        assert!(out.contains("[S91] ARV-471 试验记录"), "cited-but-unlisted source appended: {out}");
        assert!(out.contains("clinicaltrials.gov/study/NCT04072952"), "appended entry carries a clickable URL: {out}");
        // Already-complete bibliography is left alone (idempotent).
        let complete = "# 报告\n只引用 [S3]。\n\n## 参考文献\n- [S3] KT-474 首次人体研究 — https://doi.org/10.1000/kt474\n";
        assert_eq!(ensure_reference_list(complete, &sources), complete);
        // No reference section at all → one is created for the cited source.
        let noref = "# 报告\n结论 [S3]。";
        let made = ensure_reference_list(noref, &sources);
        assert!(made.contains("## 参考文献") && made.contains("[S3]"), "creates a reference section: {made}");
    }

    #[test]
    fn capture_registers_scidb_cited_sources_and_relabels_output() {
        // REGRESSION (benchmark root cause): records fetched via python+scidb never became citable [Sn]
        // (only native litsearch did), so the model fabricated citations. scidb.cite(...) prints a
        // #AXIOM_CITE# line that capture_cited_sources turns into real registered sources + [Sn] ids.
        let mut sources: Vec<Source> = Vec::new();
        let mut src_n = 0u32;
        let o = "PDB 7XKJ loaded\n#AXIOM_CITE#[{\"title\":\"KRAS G12D structure\",\"url\":\"https://www.rcsb.org/structure/7XKJ\",\"excerpt\":\"[PDB] ...\"}]\n分析完成";
        let out = capture_cited_sources(o, &mut sources, &mut src_n);
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].id, 1);
        assert_eq!(sources[0].url, "https://www.rcsb.org/structure/7XKJ");
        assert!(!out.contains("#AXIOM_CITE#"), "raw marker dropped from the observation: {out}");
        assert!(out.contains("[S1]"), "assigned id surfaced back to the model: {out}");
        assert!(out.contains("PDB 7XKJ loaded") && out.contains("分析完成"), "surrounding output preserved: {out}");
        // De-dup by URL: citing the same record again reuses [S1], adds no new source.
        let out2 = capture_cited_sources("#AXIOM_CITE#[{\"title\":\"x\",\"url\":\"https://www.rcsb.org/structure/7XKJ\"}]", &mut sources, &mut src_n);
        assert_eq!(sources.len(), 1, "dedup by url");
        assert!(out2.contains("[S1]"));
    }

    #[tokio::test]
    async fn last_step_final_is_accepted_and_cleaned_not_discarded() {
        // REGRESSION (self-test found this): when a nudge gate WOULD fire on the FINAL step, nudging
        // discards the model's real report and forces the budget-exhausted wrap-up (which leaked a marker
        // + a dangling cite). On the last step the gates must be SKIPPED and the report ACCEPTED, with the
        // marker + dangling-citation strip still applied.
        let root = tmp("laststep");
        let sb = Sandbox::new(&root).unwrap();
        let pause = Pause::new();
        pause.encourage_delegate(); // research mode → the dangling gate WOULD fire (0 sources, cites [S3])
        let budget = CodingBudget { max_steps: 1, per_call_timeout_s: 30, cmd_timeout_s: 10, cell_timeout_s: 30 };
        let report = "===AXIOM3_FINAL===\n# 研究简报\n\n## 一、直接回答\n结论成立且证据链完整，机制路径清晰，已权衡多条竞争假说并给出决定性验证实验设计，覆盖体系材料、方法参数、读出指标、判定判据与对照组设置，正文足够充分以通过真报告判定；为满足长度门槛这里继续补充足够多的中文叙述内容，保持自然通顺不堆砌，逐条说明关键机制、证据强度、剂量与效应关系、潜在混杂、外推边界、已知局限，以及下一步可照做的验证方案与预期读出，确保非空白字符数量明显超过二百个字符的下限要求，避免因过短被判定为非报告而走抢救路径 [S3]。\n\n## 二、机制与证据\n多条通路协同，权衡后给出主判断，并列出反证条件与证伪实验，保证结论可被后续工作检验。\n\n## 三、结论\n综上，机制与证据支持该判断，后续以所述实验证伪或坐实。";
        let model = ScriptModel { replies: Mutex::new(vec![report.into()]) };
        let res = run_coding_agent("研究课题", &sb, &model, &Cancel::new(), &pause, &budget, None, |_| {}, |_, _| {}).await;
        assert_eq!(res.status, "done", "last-step report must be accepted (not discarded to no_report): {}", res.summary.chars().take(50).collect::<String>());
        assert!(!res.summary.contains("AXIOM3_FINAL"), "marker stripped: {}", res.summary.chars().take(30).collect::<String>());
        assert!(!res.summary.contains("[S3]"), "dangling [S3] stripped (0 sources): {}", res.summary);
        assert!(res.summary.starts_with("# 研究简报"), "the model's real report ships: {}", res.summary.chars().take(20).collect::<String>());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn research_final_nudges_then_strips_a_dangling_citation() {
        // REGRESSION (self-test found this): research mode, THIN retrieval (0 sources here). The model
        // finalizes a report citing [S3] (dangling). The gate nudges ONCE; if the model re-emits the same
        // dangling cite, the backstop STRIPS it so no dead provenance link ships — the claim text stays.
        let root = tmp("dangling_loop");
        let sb = Sandbox::new(&root).unwrap();
        let pause = Pause::new();
        pause.encourage_delegate(); // research mode → the dangling-citation gate is active
        // A report long enough to pass looks_like_real_report (≥200 non-whitespace prose chars).
        let report = "===AXIOM3_FINAL===\n# 研究简报\n\n## 一、直接回答\n结论成立且证据链完整，机制路径清晰，已权衡多条竞争假说并给出决定性验证实验设计，覆盖体系材料、方法参数、读出指标、判定判据与对照组设置，正文足够充分以通过真报告判定；为满足长度门槛这里继续补充足够多的中文叙述内容，保持自然通顺不堆砌，说明关键机制、证据强度、局限与下一步可照做的验证方案，确保非空白字符数量明显超过二百的下限要求 [S3]。\n\n## 二、结论\n综上，机制与证据支持该判断，后续以所述实验证伪或坐实。";
        let model = ScriptModel {
            replies: Mutex::new(vec![
                report.into(), // 1st finalize → dangling-citation gate nudges (max_cited=3 > 0 sources)
                report.into(), // model re-emits the SAME dangling cite → backstop must strip it
            ]),
        };
        let res = run_coding_agent("研究课题", &sb, &model, &Cancel::new(), &pause, &CodingBudget::default(), None, |_| {}, |_, _| {}).await;
        assert_eq!(res.status, "done", "summary: {}", res.summary);
        assert!(!res.summary.contains("[S3]"), "dangling [S3] must be stripped (0 sources), got: {}", res.summary);
        assert!(res.summary.contains("结论成立") && res.summary.starts_with("# 研究简报"), "claim/report body preserved: {}", res.summary.chars().take(20).collect::<String>());
        let _ = std::fs::remove_dir_all(&root);
    }
}
