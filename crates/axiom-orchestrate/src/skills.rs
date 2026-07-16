//! A registry of domain "skill cards" the analysis agent can search and load on demand (mirrors
//! Claude Science's searchable Skills). Each card is an expert procedure — what to `pip_install`,
//! the canonical pipeline, which native figure block to emit, the rigor pitfalls — so a weaker model
//! does the domain-correct thing.
//!
//! Two sources, merged into one registry:
//!   1. BUILTINS — embedded in the binary (always available, always current, no fs/network need).
//!   2. `~/.axiom3/skills/<name>/SKILL.md` — user-authored skill FILES a domain expert can add or
//!      edit WITHOUT recompiling. A disk skill with the same `name` overrides the builtin, so a
//!      power user can also tune a shipped card. On first use we materialize a README + a template
//!      into that dir so the format is self-documenting.
//! The merged registry is built once (OnceLock) and handed out as `&'static [Skill]`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Runtime skill — OWNED, so an entry can equally come from a compiled-in Builtin or a SKILL.md file.
pub struct Skill {
    pub name: String,
    pub keywords: String,
    pub summary: String,
    pub card: String,
}

/// Compile-time literal skill — the shipped built-in library, embedded in the binary.
struct Builtin {
    name: &'static str,
    /// Extra search terms (synonyms/tools) beyond the name + summary.
    keywords: &'static str,
    summary: &'static str,
    card: &'static str,
}

const BUILTINS: &[Builtin] = &[
    Builtin {
        name: "protein-structure-figure",
        keywords: "pymol protein structure figure cartoon ribbon active site catalytic residue mutation binding pocket domain fold pdb crystal cryo-em zinc metal coordination publication figure 结构图 蛋白 活性位点 突变",
        summary: "Publication-grade PROTEIN/STRUCTURE figures with headless ray-traced PyMOL (matplotlib cannot make these) + measured geometry + a composed, captioned panel.",
        card: "# Publication-grade protein STRUCTURE figures (PyMOL, headless ray-traced)\n\
            USE THIS whenever the figure is a protein / enzyme / complex: overall fold, active site, a mutation, a binding pocket, domain architecture.\n\
            matplotlib CANNOT make these (no cartoon secondary structure, no ray tracing, no atom selection language). Hand-drawing one in matplotlib always looks amateurish — use PyMOL.\n\
            \n\
            0. INSTALL once (slow conda solve → it runs as a BACKGROUND job; poll with job_status until DONE):\n\
               {\"tool\":\"conda_install\",\"packages\":[\"pymol-open-source\",\"biopython\",\"pillow\",\"matplotlib\"]}\n\
            \n\
            1. STRUCTURE. Prefer a REAL experimental structure — RCSB has 200k+. Fetch by id inside the script: `cmd.fetch('5grr', type='pdb', async_=0)` (writes 5grr.pdb into the cwd). Use the `pdb` tool first to confirm the id/method/resolution.\n\
            \n\
            2. RUN IT — from the PYTHON KERNEL by subprocess. Invoke PyMOL AS A MODULE (`python -m pymol -cq`) via the bioenv's OWN python — cross-platform (PyMOL ships as a python module; there is NO pymol.exe on Windows). Do NOT use the `run` tool (its allowlist has no pymol, and its 45 s shell timeout can cut a ray-trace off). The kernel cell timeout is generous (600 s):\n\
               ```\n\
               import os, subprocess\n\
               BIO = os.path.expanduser('~/.axiom3/bioenv')\n\
               PY = os.path.join(BIO, 'python.exe') if os.name=='nt' else os.path.join(BIO, 'bin', 'python')   # bioenv python, absolute → survives kernel restart, no PATH dependency\n\
               open('render.py','w').write(SCRIPT)\n\
               CF = 0x08000000 if os.name=='nt' else 0   # CREATE_NO_WINDOW: don't flash a black console on Windows\n\
               r = subprocess.run([PY, '-m', 'pymol', '-cq', 'render.py'], capture_output=True, text=True, creationflags=CF)\n\
               print(r.returncode, r.stdout[-2000:], r.stderr[-2000:])    # ALWAYS print rc+stderr, or a silent failure looks like success\n\
               ```\n\
               Headless: NO GPU and NO display needed (PyMOL ray-traces in software; ~2-30 s per panel).\n\
               *** GOTCHA (bit me): under `pymol -cq`, __file__ is NOT your script — so os.path.dirname(__file__) points INSIDE the pymol package and your PNGs silently vanish there. ALWAYS write ABSOLUTE output paths into the script (os.getcwd() of the KERNEL is the workspace — bake it in). ***\n\
            \n\
            3. CLEAN THE MODEL FIRST — do this immediately after loading, BEFORE styling. Skipping it is the #1 way a structure figure ends up looking amateurish: raw PDBs carry hundreds of crystallographic waters and stray ions that ray-trace into a blizzard of little red/grey stars scattered across every panel.\n\
               cmd.remove('solvent')                              # waters — ALWAYS. Non-negotiable.\n\
               cmd.remove('hydrogens')\n\
               cmd.remove('not polymer and not (%s)' % KEEP)      # KEEP = the ligand resn + the catalytic metal(s) + any bridging OH you will explicitly name; everything else (buffer ions, cryoprotectant, extra copies) goes.\n\
               If the crystal has multiple chains/copies and you only discuss one, `cmd.remove('not chain A')` too. The site panel especially must be free of floating waters.\n\
            \n\
            4. PUBLICATION STYLING — this is what separates a figure from a screenshot. Set ALL of it:\n\
               cmd.bg_color('white'); cmd.set('ray_opaque_background',1); cmd.set('antialias',2); cmd.set('ray_shadows',0)\n\
               cmd.set('ambient_occlusion_mode',1)   # AO = the depth that makes it read as 3D. Do not skip.\n\
               cmd.set('ambient',0.22); cmd.set('direct',0.58); cmd.set('reflect',0.22); cmd.set('specular',0.10)\n\
               cmd.set('cartoon_fancy_helices',1); cmd.set('cartoon_smooth_loops',1); cmd.set('stick_radius',0.18)\n\
               cmd.set('depth_cue',0); cmd.set('ray_trace_fog',0)\n\
               COLOUR muted + SEMANTIC: grey85 base; `skyblue` for 'ss H'; `wheat` for 'ss S'; `firebrick` sticks for catalytic residues; `grey65` sticks for the 5 Å shell; `purple` sphere for a metal. NEVER full-saturation primaries.\n\
            \n\
            5. PANELS (mirror a journal figure):\n\
               A) Overall fold — the WHOLE domain must fit in frame. `cmd.hide('everything'); cmd.show('cartoon','polymer'); cmd.orient('polymer')` (orient on the WHOLE chain, NOT a residue selection), then `cmd.zoom('polymer', buffer=5, complete=1)` so nothing is clipped, then ray+png. *** GOTCHA: NEVER zoom the fold panel onto a residue/site selection — that over-zooms into a tangle of helices and the 'overall fold' shows no fold at all. The site is panel B's job; panel A is the whole protein. ***\n\
               B) Site zoom: cmd.set('cartoon_transparency',0.86) AND hide the cartoon outside ~15 Å, so the CHEMISTRY reads instead of being buried by ribbons; show sticks for the catalytic residues + the 5 Å shell; label ONLY the key residues:\n\
                  cmd.label(sel+' and name CA', '\\\"%s%s\\\" % (resn.capitalize(), resi)')\n\
               *** GOTCHA: if the structure has SEVERAL metal sites, pick the CATALYTIC one by PROXIMITY to the nucleophile (e.g. the Zn within 2.5 Å of the Ser/Thr OG) — otherwise your distances are nonsense. For a BINUCLEAR site (e.g. NDM-1 two-Zn), colour the two metals DIFFERENTLY and report each metal's own coordination sphere separately. ***\n\
               *** GOTCHA: draw coordination bonds with cmd.distance(...), then cmd.hide('labels', name) — the numeric labels pile up and wreck the image. The numbers belong in the CAPTION. ***\n\
            \n\
            6. MEASURE, NEVER ASSERT. Compute every number from the coordinates:\n\
               zx = cmd.get_model(ZN).atom[0].coord; for a in cmd.get_model(CAT+' and not elem C').atom: d = ((zx[0]-a.coord[0])**2+(zx[1]-a.coord[1])**2+(zx[2]-a.coord[2])**2)**0.5\n\
               Real bond lengths, distance-to-active-site, SASA (freesasa), residue counts. A number you did not compute is a fabrication.\n\
            \n\
            7. COMPOSE + CAPTION (the last 20% of the polish) — do this in matplotlib so the kernel captures it as the report figure:\n\
               *** CLOSE STRAY FIGURES FIRST: `plt.close('all')` before you build the composite. The kernel captures EVERY open matplotlib figure via plt.get_fignums(), so a half-built or failed earlier attempt you left open gets shipped in the report next to the good one. Only the final composite should be open when the cell ends. ***\n\
               For EACH panel: `ax = fig.add_subplot(...); ax.imshow(mpimg.imread(png)); ax.axis('off')` — call ax.axis('off') on EVERY image axes (a forgotten one shows raw 0..1 ticks). Before saving, ASSERT every axes you created was actually filled: if a panel's PyMOL png is missing or failed to render, DROP that panel and rebuild the grid smaller — NEVER leave an empty white 0..1 axes in the figure (a blank panel reads as a broken deliverable).\n\
               Then: bold panel letters (A/B) → one-line sub-caption per panel → a CAPTION BAR carrying the MEASURED numbers + a legend line (\"red sticks = catalytic; grey = 5 Å shell; purple = Zn; blue = helix, wheat = strand\").\n\
               Use a GENEROUS figsize (e.g. 18×9): the kernel captures at dpi=110, so a small figsize throws away the ray-traced detail.\n\
               ALSO savefig the full-resolution PNG into the workspace — it then appears under 产物文件 and the user can download the print-quality original.\n",
    },
    Builtin {
        name: "small-molecule-figure",
        keywords: "rdkit small molecule drug ligand inhibitor compound chemical structure 2d smiles pharmacophore scaffold warhead substructure reaction descriptor logp tpsa molecular weight medicinal chemistry 分子结构 小分子 药物 配体 化合物",
        summary: "Publication-grade SMALL-MOLECULE / drug / ligand 2D structure figures with RDKit (clean stereo bonds, substructure highlight) + REAL computed descriptors + a composed panel.",
        card: "# Publication-grade small-molecule / drug figures (RDKit)\n\
            USE THIS whenever the figure is a CHEMICAL: a drug / inhibitor / metabolite / ligand 2D structure, a series comparison, a reaction, or highlighting a pharmacophore / warhead / scaffold. RDKit draws proper stereochemistry (wedge bonds) and is the right tool — a hand-drawn matplotlib molecule is never acceptable.\n\
            \n\
            1. INSTALL (pip wheels — imports in the SAME kernel right after, NO conda needed):\n\
               {\"tool\":\"pip_install\",\"packages\":[\"rdkit\"]}   (add \"pillow\" if not present)\n\
            \n\
            2. STRUCTURES come from a SOURCE, never invented. Get the SMILES from the literature / PubChem / ChEMBL (use web_fetch / the litsearch abstracts). `m = Chem.MolFromSmiles(smi)` and *** ALWAYS check `assert m is not None` — a wrong SMILES silently returns None. *** Then `AllChem.Compute2DCoords(m)` before drawing.\n\
            \n\
            3. DRAW crisp + publication-style with MolDraw2DCairo:\n\
               from rdkit.Chem.Draw import rdMolDraw2D\n\
               d = rdMolDraw2D.MolDraw2DCairo(560, 460); op = d.drawOptions(); op.bondLineWidth = 2; op.padding = 0.12\n\
               HIGHLIGHT the meaningful substructure (warhead / pharmacophore / shared scaffold) via SMARTS so the figure MAKES A POINT: `patt = Chem.MolFromSmarts(smarts); hit = m.GetSubstructMatch(patt)` then pass highlightAtoms=list(hit) (+ the bonds within hit) to `rdMolDraw2D.PrepareAndDrawMolecule`. d.FinishDrawing(); the PNG bytes are d.GetDrawingText().\n\
               For a REACTION use `rdChemReactions.ReactionFromSmarts(...)` + Draw.ReactionToImage. For a scaffold across a series, align to a common core (`AllChem.GenerateDepictionMatching2DStructure`) so the eye compares like-with-like.\n\
            \n\
            4. MEASURE, NEVER ASSERT — compute descriptors from the structure, don't state them from memory:\n\
               from rdkit.Chem import Descriptors, Crippen, rdMolDescriptors\n\
               MW=Descriptors.MolWt(m); logP=Crippen.MolLogP(m); TPSA=rdMolDescriptors.CalcTPSA(m); HBD=rdMolDescriptors.CalcNumHBD(m); HBA=rdMolDescriptors.CalcNumHBA(m)\n\
               (Lipinski/Veber context: MW, logP, TPSA, HBD/HBA, rotatable bonds — all computed.) A descriptor you did not compute is a fabrication.\n\
            \n\
            5. COMPOSE in matplotlib (so the kernel captures it): `plt.close('all')` first; imshow each molecule PNG in a grid with `ax.axis('off')`, bold titles, and a CAPTION row carrying the computed descriptor table + a one-line legend (what the highlight means). savefig the full-res PNG into the workspace too.\n",
    },
    Builtin {
        name: "gene-context-figure",
        keywords: "dna features viewer gene context operon cassette plasmid map mobile genetic element transposon integron resistance gene neighbourhood synteny locus contig arrow genbank 基因排布 操纵子 质粒图谱 耐药基因 可移动元件 基因背景",
        summary: "Publication-grade GENE-CONTEXT / operon / resistance-cassette / plasmid maps with dna_features_viewer (strand-aware gene arrows, ruler) — the figure resistance/genomics papers use.",
        card: "# Publication-grade gene-context / operon / plasmid maps (dna_features_viewer)\n\
            USE THIS whenever the figure is a LINEAR (or circular) arrangement of genes: an operon, a resistance cassette / mobile genetic element (transposon, integron, IS-flanked context), a gene neighbourhood / synteny block, or a plasmid map. Strand-aware gene arrows on a ruler — matplotlib patches look amateurish next to this.\n\
            \n\
            1. INSTALL (pip — imports in the SAME kernel right after):\n\
               {\"tool\":\"pip_install\",\"packages\":[\"dna_features_viewer\"]}   (biopython too if you will parse GenBank)\n\
            \n\
            2. COORDINATES come from a SOURCE where possible: parse a real GenBank/RefSeq record with Biopython (`SeqIO.read(...,'genbank')`, iterate `record.features` of type CDS, read `.location.start/.end/.strand` and the `/gene` or `/product` qualifier). *** If you only have an illustrative layout from the literature (no coordinates you can cite), you MAY draw a schematic — but the title/caption MUST say \"schematic, positions illustrative\" so a reader never mistakes made-up coordinates for measured ones. ***\n\
            \n\
            3. DRAW:\n\
               from dna_features_viewer import GraphicFeature, GraphicRecord\n\
               feats = [GraphicFeature(start=s, end=e, strand=+1 or -1, label='geneName', color='#c0392b'), ...]\n\
               rec = GraphicRecord(sequence_length=L, features=feats); ax = rec.plot(figure_width=13)[0]  # or rec.plot(ax=ax)\n\
               COLOUR semantically + muted: the gene of interest (e.g. the resistance gene) in a strong accent, flanking IS/mobile elements in grey, housekeeping genes in soft blues/greens. For a plasmid use CircularGraphicRecord.\n\
            \n\
            4. Keep it readable: a generous figure_width (12-14) and, if labels collide, `GraphicRecord(..., feature_level_height=...)` or shorten labels; add a ruler (`with_ruler=True`, default) and a bold title naming the element. savefig into the workspace and let the kernel capture it.\n\
            \n\
            5. HONESTY: gene lengths/positions on the axis are real only if you read them from a record — otherwise mark the figure schematic (see step 2). Never present an invented coordinate as measured.\n",
    },
    Builtin {
        name: "single-cell-rnaseq",
        keywords: "scrna scRNA-seq scanpy anndata umap leiden cluster cell type marker harmony batch integration immune",
        summary: "scRNA-seq: QC → normalize → HVG → PCA → batch-correct → Leiden → UMAP → annotate.",
        card: "# Single-cell RNA-seq (scanpy)\n\
            Install: `pip_install scanpy anndata leidenalg igraph` (and `harmonypy` for batch integration).\n\
            Pipeline (build up in the kernel step by step):\n\
            1. Load into AnnData (sc.read_csv / read_10x_mtx / read_h5ad); orient cells × genes. If >~2000 cells, SUBSAMPLE FIRST so the pipeline is fast: `adata = adata[np.random.choice(adata.n_obs, 800, replace=False)].copy()`.\n\
            2. QC: sc.pp.calculate_qc_metrics; filter cells (min_genes) and genes (min_cells); threshold pct-mito.\n\
            3. Normalize: sc.pp.normalize_total(target_sum=1e4); sc.pp.log1p.\n\
            4. HVG: sc.pp.highly_variable_genes(n_top_genes=2000); subset.\n\
            5. Scale + PCA: sc.pp.scale(max_value=10); sc.tl.pca.\n\
            6. If ≥2 donors/samples, BATCH-CORRECT (harmonypy on the PCA, by batch key) — skipping this is a common error.\n\
            7. sc.pp.neighbors; sc.tl.leiden; sc.tl.umap.\n\
            8. Annotate clusters from canonical markers via sc.tl.rank_genes_groups(method='wilcoxon').\n\
            Figures: UMAP → ```chart scatter (x/y = UMAP1/2, group = cell type); markers → ```dotplot or ```heatmap.\n\
            Rigor: for composition/DE between conditions, aggregate to the SAMPLE (pseudobulk) — never test per-cell (pseudoreplication). BH-correct across genes/clusters. Report per-sample proportions with a test + CI.",
    },
    Builtin {
        name: "phylogenetics",
        keywords: "phylogeny tree evolution ancestral reconstruction MSA alignment mafft iqtree sequence homolog newick clade bootstrap",
        summary: "Phylogenetics: homologs → MSA → trim → ML tree → trace residues → ancestral reconstruction.",
        card: "# Phylogenetics + ancestral reconstruction\n\
            Install: `pip_install biopython dendropy`. MAFFT/IQ-TREE are BINARIES (need conda/system); if unavailable, align with Bio.Align.PairwiseAligner (progressive) and build NJ/ML trees with dendropy from a distance matrix — and state the limitation.\n\
            Pipeline:\n\
            1. Gather homologs: fetch from UniProt/InterPro by accession with requests; parse FASTA with Bio.SeqIO.\n\
            2. Curate a non-redundant, taxonomically balanced set; annotate domain of life / lifestyle.\n\
            3. MSA, then trim gappy/low-information columns.\n\
            4. Tree: ML with support values (or NJ if no ML tool); root sensibly.\n\
            5. Trace key residues: map catalytic positions to alignment columns; tabulate the state per sequence and per domain.\n\
            6. Ancestral reconstruction (parsimony/ML on the tree, dendropy) at those columns; report posterior probabilities.\n\
            Figures: ```tree (Newick); ```msa; ```seqlogo (per-column base/aa frequencies); residue states → ```heatmap.\n\
            Rigor: report branch support; flag long-branch attraction; frame ancestral function as a structure/evolution HYPOTHESIS, not an assay result.",
    },
    Builtin {
        name: "differential-expression",
        keywords: "differential expression DE deseq edger volcano fold change padj fdr multiple testing omics rnaseq bulk transcriptomics",
        summary: "Bulk/omics DE: per-feature effect size + test → BH/FDR correction → volcano/heatmap.",
        card: "# Differential expression / omics statistics\n\
            Install: `statsmodels` (present); for count data `pip_install pydeseq2`.\n\
            Pipeline:\n\
            1. Per feature: effect size (log2 fold change = mean(treat) − mean(ctrl) in log space) and a test (Welch t / Mann-Whitney; for counts use a negative-binomial model / pydeseq2).\n\
            2. CORRECT for multiple testing across ALL features: statsmodels.stats.multitest.multipletests(pvals, method='fdr_bh'); report adjusted p (padj).\n\
            3. Call significant by padj<0.05 AND |lfc|>threshold.\n\
            Figures: ```volcano {\"genes\":[{\"gene\":..,\"lfc\":..,\"padj\":..}]}; top hits (z-scored) → ```heatmap.\n\
            Rigor: BH/FDR is MANDATORY — never report raw p across thousands of tests. With replicates, aggregate to the experimental unit. Report effect size + CI, not p alone.",
    },
    Builtin {
        name: "crispr-screen",
        keywords: "crispr cas9 knockout screen sgrna guide brunello off-target cfd mit library sequencing kinome",
        summary: "CRISPR-KO library: gene set → optimized guides → on/off-target QC → controls → construction + depth.",
        card: "# CRISPR knockout screen (design/analysis)\n\
            Install: pandas/numpy (present). Cas-OFFinder is a BINARY (system/conda); if absent, score candidate off-targets by mismatch-weighted CFD from a supplied genome index.\n\
            Design:\n\
            1. Define the target gene set (canonical source, e.g. KinHub for the kinome); resolve gene ALIASES (a big source of misses).\n\
            2. Take guides from a published optimized library (Brunello) matched by symbol; keep 4/gene.\n\
            3. On-target QC: Rule Set 2 score, GC 20–80%, no poly-T terminator, no internal cloning-enzyme (BsmBI) site.\n\
            4. Off-target: predict (CFD/MIT), flag high-CFD hits in CODING exons; note paralog artifacts (a guide hitting a duplicated gene is not a design flaw).\n\
            5. Add non-targeting + positive-essential controls; plan oligo construction (adapters) + sequencing depth (representation × reads/guide).\n\
            Figures: coverage → ```chart bar; specificity → ```chart scatter; hit ranking → ```manhattan or ```volcano; domains/mutations → ```track.\n\
            Rigor: ≥3 usable guides/gene; document dropped/flagged guides; don't silently lose coverage.",
    },
    Builtin {
        name: "protein-engineering",
        keywords: "protein mutation variant effect esm esmfold proteinmpnn inverse folding structure pymol stability engineering enzyme",
        summary: "Protein variant effects: characterize → PLM/structure scoring → consensus rank → structural rationale.",
        card: "# Protein variant effect prediction / engineering\n\
            Install: `pip_install biopython`. ESM scoring: `pip_install fair-esm` (CPU-slow → run a fold with job_start). ESMFold/ProteinMPNN are heavy models.\n\
            Pipeline:\n\
            1. Characterize the sequence: length, net charge, domains; pull catalytic/functional residues from UniProt features (requests). PROTECT catalytic residues (and their shell) from mutation.\n\
            2. Score single mutations with a PLM (ESM-2 masked-marginal LLR). For structure-based scores, fold (ESMFold; long → job_start + checkpoint) then score with inverse folding (ProteinMPNN / ESM-IF).\n\
            3. Consensus-rank across methods (z-normalize each, require agreement); filter by solvent accessibility and distance to the active site; diversify positions.\n\
            Figures: mutational landscape → ```heatmap; candidate positions → ```track; structure → ```protein (4-char PDB id or inline PDB).\n\
            Rigor: PLM/inverse-folding scores predict fitness/foldability, NOT turnover — frame as ranked hypotheses; cross-validate anything you fit; state that the active site was protected; recommend wet-lab validation.",
    },
    Builtin {
        name: "gwas",
        keywords: "gwas association genome-wide snp variant manhattan qq bonferroni population structure",
        summary: "GWAS: per-variant association → genome-wide correction → Manhattan + QQ.",
        card: "# GWAS / association scan\n\
            Pipeline:\n\
            1. Per variant: association test (linear/logistic on genotype dosage), get a p-value.\n\
            2. Control for POPULATION STRUCTURE (include genotype PCs as covariates) — omitting this inflates false positives.\n\
            3. Correct for multiple testing: genome-wide significance 5e-8 (Bonferroni), or BH for adjusted p.\n\
            Figures: ```manhattan {\"points\":[{\"chr\":..,\"pos\":..,\"p\":..,\"snp\":..}],\"threshold\":..}; a QQ plot via ```chart scatter (expected vs observed −log10 p).\n\
            Rigor: report the genome-wide threshold and lambda (genomic inflation); a single-cohort hit is a hypothesis until replicated.",
    },
    Builtin {
        name: "evidence-synthesis",
        keywords: "systematic review meta-analysis meta analysis prisma forest plot funnel egger heterogeneity i2 odds ratio hazard ratio pooled effect kaplan meier survival number at risk grade evidence quality clinical 综述 荟萃 系统评价 森林图 生存 循证",
        summary: "Systematic review / meta-analysis / clinical evidence synthesis: screen → pool → forest/funnel/KM, with PRISMA accounting and honest heterogeneity.",
        card: "# Evidence synthesis (systematic review / meta-analysis / clinical)\n\
            USE THIS whenever the deliverable pools MULTIPLE published studies (a review, a meta-analysis, a clinical-evidence summary) — the product's core mode. You are synthesising the literature, NOT running a new trial: every number is cited or computed from cited data, never invented.\n\
            Pipeline:\n\
            1. SELECTION accounting → ```prisma: record identified / removed (duplicates) / screened / excluded / assessed / included with reasons. Fill only the counts you actually have; leave the rest blank — NEVER fabricate a screening number.\n\
            2. POOL a common effect (log-scale for ratio measures OR/HR/RR, then back-transform); report the 95%CI (computed on the log scale), and heterogeneity I²/tau².\n\
            3. Figures: forest → ```forest {\"effectType\":\"OR\",\"pooled\":{..},\"heterogeneity\":{\"i2\":..},\"rows\":[..]}; publication bias → ```funnel (Egger p, and the caption MUST say '研究数<10 不解读不对称性'); time-to-event → ```km (step curve + number-at-risk + log-rank p + HR[95%CI]).\n\
            4. GRADE the certainty (high/moderate/low) per outcome; carry it into the report's risk section.\n\
            Rigor: whiskers are 95%CI (not SEM); ratio CIs are symmetric on the LOG axis only; an unsourced pooled number is fabrication — every estimate ties to its studies [Sn]. State this is a synthesis of published evidence, not a new experiment.",
    },
];

/// `~/.axiom3/skills` — the user-authored skill directory (created on first use). Same root the
/// kernel's conda/pip envs live under, so "all my science stuff is in ~/.axiom3" stays true.
fn skills_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(Path::new(&home).join(".axiom3").join("skills"))
}

/// Parse one SKILL.md — YAML-ish frontmatter (`name`/`keywords`/`summary` between `---` fences)
/// then the card body. Missing name falls back to the folder name. Returns None if there's no body.
fn parse_skill_md(text: &str, folder: &str) -> Option<Skill> {
    let (mut name, mut keywords, mut summary) = (folder.to_string(), String::new(), String::new());
    let body;
    let t = text.strip_prefix('\u{feff}').unwrap_or(text); // tolerate a BOM
    if let Some(rest) = t.trim_start().strip_prefix("---") {
        // frontmatter up to the next line that is exactly `---`
        if let Some(end) = rest.find("\n---") {
            for line in rest[..end].lines() {
                let line = line.trim();
                if let Some((k, v)) = line.split_once(':') {
                    let v = v.trim().to_string();
                    match k.trim().to_ascii_lowercase().as_str() {
                        "name" if !v.is_empty() => name = v,
                        "keywords" => keywords = v,
                        "summary" => summary = v,
                        _ => {}
                    }
                }
            }
            body = rest[end + 4..].trim_start_matches(['\n', '\r']).to_string();
        } else {
            body = t.to_string();
        }
    } else {
        body = t.to_string();
    }
    if body.trim().is_empty() {
        return None;
    }
    if summary.is_empty() {
        summary = body.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim_start_matches('#').trim().to_string();
    }
    Some(Skill { name: name.trim().to_ascii_lowercase(), keywords, summary, card: body })
}

/// Load every `<dir>/<skill>/SKILL.md`. Folders starting with `.` or `_` are skipped (so the
/// `_template` example isn't loaded as a real skill). Robust to junk: parse failures are ignored.
fn load_dir(dir: &Path) -> Vec<Skill> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    for e in rd.flatten() {
        if out.len() >= 200 {
            break;
        }
        let folder = e.file_name().to_string_lossy().to_string();
        if folder.starts_with('.') || folder.starts_with('_') {
            continue;
        }
        let md = e.path().join("SKILL.md");
        if let Ok(meta) = std::fs::metadata(&md) {
            if meta.len() > 256 * 1024 {
                continue; // a skill card is prose, not a dataset
            }
            if let Ok(text) = std::fs::read_to_string(&md) {
                if let Some(s) = parse_skill_md(&text, &folder) {
                    out.push(s);
                }
            }
        }
    }
    out
}

/// On first use, drop a README + a `_template/SKILL.md` into the skills dir so a domain expert can
/// see the format and copy it — WITHOUT recompiling. Never clobbers files a user already wrote.
fn materialize_docs(dir: &Path) {
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let readme = dir.join("README.md");
    if !readme.exists() {
        let _ = std::fs::write(&readme, "# Axiom3 技能库 · Skill registry\n\n\
            这里的每个子文件夹放一个 `SKILL.md`,就是一张“技能卡”——研究/分析智能体会检索并按卡执行。\n\
            **改这里的文件即时生效,无需重新编译 App**(下次运行任务时加载)。同名会覆盖内置技能。\n\n\
            格式(YAML 头 + Markdown 正文):\n\n\
            ```\n---\nname: my-skill\nkeywords: 关键词 synonyms tools 供检索命中\nsummary: 一句话说明这张卡做什么\n---\n\
            # 正文就是给智能体看的操作手册\n1. 装什么包(pip_install / conda_install)\n2. 规范流程(逐步)\n\
            3. 用哪种图块 / 引擎\n4. 严谨性red-line:数字要计算不能编造\n```\n\n\
            复制 `_template/` 改成你自己的技能即可。内置技能(蛋白结构图/小分子图/基因背景图/scRNA-seq 等)随 App 出厂,始终最新。\n")
        ;
    }
    let tmpl_dir = dir.join("_template");
    let tmpl = tmpl_dir.join("SKILL.md");
    if !tmpl.exists() && std::fs::create_dir_all(&tmpl_dir).is_ok() {
        let _ = std::fs::write(&tmpl, "---\nname: my-skill\n\
            keywords: 领域 方法 工具 别名 — 这些词会被检索命中\n\
            summary: 一句话:这张卡在什么情况下用、产出什么\n---\n\
            # 我的技能(把这段换成真正的操作手册)\n\
            USE THIS WHEN: 描述何时该用这张卡。\n\n\
            1. 安装:{\"tool\":\"pip_install\",\"packages\":[\"...\"]}\n\
            2. 规范流程:逐步写清楚,让弱一点的模型也能照做对。\n\
            3. 图:说明用哪个原生图块或引擎。\n\
            4. 严谨性:每个数字都要由代码计算得到,不能凭记忆断言;不确定就说不确定。\n")
        ;
    }
}

/// The merged, immutable skill registry (builtins + disk), built ONCE. Disk overrides builtin by name.
fn registry() -> &'static [Skill] {
    static REG: OnceLock<Vec<Skill>> = OnceLock::new();
    REG.get_or_init(|| {
        let mut v: Vec<Skill> = BUILTINS
            .iter()
            .map(|b| Skill {
                name: b.name.to_string(),
                keywords: b.keywords.to_string(),
                summary: b.summary.to_string(),
                card: b.card.to_string(),
            })
            .collect();
        if let Some(dir) = skills_dir() {
            materialize_docs(&dir);
            for s in load_dir(&dir) {
                match v.iter_mut().find(|e| e.name == s.name) {
                    Some(existing) => *existing = s, // a disk skill tunes/overrides a builtin
                    None => v.push(s),
                }
            }
        }
        v
    })
    .as_slice()
}

/// Rank skills by how many query terms appear in name/keywords/summary. Empty result → caller
/// should surface the full list so the agent can pick.
pub fn search(query: &str) -> Vec<&'static Skill> {
    let q = query.to_lowercase();
    let terms: Vec<&str> = q.split(|c: char| !c.is_alphanumeric()).filter(|t| t.len() >= 2).collect();
    let mut scored: Vec<(usize, &'static Skill)> = registry()
        .iter()
        .map(|s| {
            let hay = format!("{} {} {}", s.name, s.keywords, s.summary).to_lowercase();
            let score = terms.iter().filter(|t| hay.contains(**t)).count();
            (score, s)
        })
        .filter(|(score, _)| *score > 0)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().map(|(_, s)| s).collect()
}

pub fn get(name: &str) -> Option<&'static Skill> {
    let n = name.trim().to_lowercase();
    registry().iter().find(|s| s.name == n)
}

/// One-line catalogue of every skill (for when a search misses).
pub fn catalogue() -> String {
    registry().iter().map(|s| format!("- {} — {}", s.name, s.summary)).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_ranks_relevant_skills_and_get_roundtrips() {
        let hits = search("single cell umap clustering");
        assert!(!hits.is_empty());
        assert_eq!(hits[0].name.as_str(), "single-cell-rnaseq", "expected scRNA skill first, got {:?}", hits.iter().map(|s| s.name.as_str()).collect::<Vec<_>>());
        assert!(search("volcano fdr differential").iter().any(|s| s.name == "differential-expression"));
        // Each FIGURE-ENGINE skill must WIN the queries an agent actually types — a skill nobody
        // retrieves is a skill that doesn't exist. Each is also loadable straight by name (how the
        // system prompt routes to it).
        let want = [
            ("protein-structure-figure", ["pymol protein structure figure", "active site zinc coordination figure", "enzyme fold cartoon ribbon"]),
            ("small-molecule-figure",    ["rdkit drug molecule structure figure", "inhibitor smiles pharmacophore highlight", "ligand descriptor logp figure"]),
            ("gene-context-figure",      ["gene context operon cassette figure", "plasmid map mobile genetic element", "resistance gene neighbourhood arrows"]),
        ];
        for (name, queries) in want {
            for q in queries {
                let hits = search(q);
                assert_eq!(hits.first().map(|s| s.name.as_str()), Some(name), "query {q:?} → {:?}", hits.iter().map(|s| s.name.as_str()).collect::<Vec<_>>());
            }
        }
        // The PyMOL card must carry the two things that make it WORK: the absolute-path kernel
        // invocation (the `run` allowlist has no pymol) and measured-not-asserted geometry.
        let card = &get("protein-structure-figure").expect("loadable by name").card;
        assert!(card.contains("-m', 'pymol'") || card.contains("'-m', 'pymol'"), "card must invoke pymol as a module (cross-platform), not a per-OS binary");
        assert!(card.contains("MEASURE, NEVER ASSERT"), "card must forbid asserted numbers");
        // The new import-based engines must install via pip_install (into the KERNEL venv), NOT
        // conda_install (which lands in a different env the kernel can't import from).
        assert!(get("small-molecule-figure").unwrap().card.contains("pip_install"), "rdkit skill must pip_install into the kernel");
        assert!(get("gene-context-figure").unwrap().card.contains("pip_install"), "dna_features_viewer skill must pip_install into the kernel");
        assert!(get("phylogenetics").is_some());
        assert!(get("nonexistent").is_none());
        // A miss returns nothing; the catalogue still lists everything.
        assert!(search("zzz nonsense qqq").is_empty());
        assert!(catalogue().contains("crispr-screen"));
    }

    #[test]
    fn skill_md_frontmatter_parses_and_falls_back() {
        let s = parse_skill_md("---\nname: My-Skill\nkeywords: a b c\nsummary: does a thing\n---\n# Body\nstep 1\n", "folder").unwrap();
        assert_eq!(s.name, "my-skill"); // lowercased for name-matching
        assert_eq!(s.keywords, "a b c");
        assert_eq!(s.summary, "does a thing");
        assert!(s.card.starts_with("# Body"));
        // No frontmatter → folder name + first heading as summary; whole text is the card.
        let s2 = parse_skill_md("# Just a card\nno frontmatter", "fallback-name").unwrap();
        assert_eq!(s2.name, "fallback-name");
        assert_eq!(s2.summary, "Just a card");
        assert!(s2.card.contains("no frontmatter"));
        // Empty body is not a skill.
        assert!(parse_skill_md("---\nname: x\n---\n   \n", "f").is_none());
    }

    #[test]
    fn materialize_then_load_dir_round_trips_on_disk() {
        // Exercise the REAL file I/O path: materialize the docs, drop a user SKILL.md, load it back.
        let dir = std::env::temp_dir().join("axiom3_skills_rt_test");
        let _ = std::fs::remove_dir_all(&dir);
        materialize_docs(&dir);
        // README + a _template/SKILL.md were written…
        assert!(dir.join("README.md").exists(), "README materialized");
        assert!(dir.join("_template").join("SKILL.md").exists(), "template materialized");
        // …but the _template is NOT loaded as a real skill (underscore-prefixed dirs are skipped).
        let author = dir.join("my-figure-skill");
        std::fs::create_dir_all(&author).unwrap();
        std::fs::write(author.join("SKILL.md"), "---\nname: my-figure-skill\nkeywords: foo bar\nsummary: a user skill\n---\n# card\ndo the thing\n").unwrap();
        let loaded = load_dir(&dir);
        assert!(loaded.iter().any(|s| s.name == "my-figure-skill"), "user skill loaded from disk");
        assert!(!loaded.iter().any(|s| s.name.contains("template")), "_template dir must be skipped");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn disk_skill_overrides_builtin_by_name() {
        // A disk SKILL.md with a builtin's name REPLACES it (power-user tuning); a new name ADDS.
        let mut v: Vec<Skill> = BUILTINS
            .iter()
            .map(|b| Skill { name: b.name.into(), keywords: b.keywords.into(), summary: b.summary.into(), card: b.card.into() })
            .collect();
        let n0 = v.len();
        let disk = vec![
            parse_skill_md("---\nname: gwas\nsummary: my tuned gwas\n---\ntuned body\n", "gwas").unwrap(),
            parse_skill_md("---\nname: brand-new\nsummary: novel\n---\nnew body\n", "brand-new").unwrap(),
        ];
        for s in disk {
            match v.iter_mut().find(|e| e.name == s.name) {
                Some(existing) => *existing = s,
                None => v.push(s),
            }
        }
        assert_eq!(v.len(), n0 + 1, "override must not grow the count; only the new skill adds one");
        assert_eq!(v.iter().find(|e| e.name == "gwas").unwrap().summary, "my tuned gwas");
        assert!(v.iter().any(|e| e.name == "brand-new"));
    }
}
