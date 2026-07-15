//! A small, self-contained registry of domain "skill cards" the analysis agent can search and
//! load on demand (mirrors Claude Science's searchable Skills). Each card is an expert procedure —
//! what to `pip_install`, the canonical pipeline, which native figure block to emit, and the rigor
//! pitfalls — so a weaker model does the domain-correct thing. Cards are embedded (always available,
//! no filesystem/network dependency).

pub struct Skill {
    pub name: &'static str,
    /// Extra search terms (synonyms/tools) beyond the name + summary.
    pub keywords: &'static str,
    pub summary: &'static str,
    pub card: &'static str,
}

pub const SKILLS: &[Skill] = &[
    Skill {
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
            2. RUN IT — from the PYTHON KERNEL by subprocess, using the ABSOLUTE binary path. Do NOT use the `run` tool (its allowlist has no `pymol`, and its 45 s shell timeout can cut a ray-trace off). The kernel cell timeout is generous (600 s):\n\
               ```\n\
               import os, subprocess\n\
               PYMOL = os.path.expanduser('~/.axiom3/bioenv/bin/pymol')   # absolute → survives a kernel restart, no PATH dependency\n\
               open('render.py','w').write(SCRIPT)\n\
               r = subprocess.run([PYMOL,'-cq','render.py'], capture_output=True, text=True)\n\
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
    Skill {
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
    Skill {
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
    Skill {
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
    Skill {
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
    Skill {
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
    Skill {
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
];

/// Rank skills by how many query terms appear in name/keywords/summary. Empty result → caller
/// should surface the full list so the agent can pick.
pub fn search(query: &str) -> Vec<&'static Skill> {
    let q = query.to_lowercase();
    let terms: Vec<&str> = q.split(|c: char| !c.is_alphanumeric()).filter(|t| t.len() >= 2).collect();
    let mut scored: Vec<(usize, &'static Skill)> = SKILLS
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
    SKILLS.iter().find(|s| s.name == n)
}

/// One-line catalogue of every skill (for when a search misses).
pub fn catalogue() -> String {
    SKILLS.iter().map(|s| format!("- {} — {}", s.name, s.summary)).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_ranks_relevant_skills_and_get_roundtrips() {
        let hits = search("single cell umap clustering");
        assert!(!hits.is_empty());
        assert_eq!(hits[0].name, "single-cell-rnaseq", "expected scRNA skill first, got {:?}", hits.iter().map(|s| s.name).collect::<Vec<_>>());
        assert!(search("volcano fdr differential").iter().any(|s| s.name == "differential-expression"));
        // The structure-figure skill must WIN the queries an agent actually types when it needs a
        // protein figure — a skill nobody retrieves is a skill that doesn't exist. It is also loadable
        // straight by name, which is how the system prompt routes to it.
        for q in ["pymol protein structure figure", "active site zinc coordination figure", "enzyme fold cartoon ribbon"] {
            let hits = search(q);
            assert_eq!(hits.first().map(|s| s.name), Some("protein-structure-figure"), "query {q:?} → {:?}", hits.iter().map(|s| s.name).collect::<Vec<_>>());
        }
        let card = get("protein-structure-figure").expect("loadable by name").card;
        // The card must carry the two things that make it WORK rather than merely sound right: the
        // absolute-path kernel invocation (the `run` allowlist has no pymol) and measured-not-asserted geometry.
        assert!(card.contains("~/.axiom3/bioenv/bin/pymol"), "card must give the absolute binary path");
        assert!(card.contains("MEASURE, NEVER ASSERT"), "card must forbid asserted numbers");
        assert!(get("phylogenetics").is_some());
        assert!(get("nonexistent").is_none());
        // A miss returns nothing; the catalogue still lists everything.
        assert!(search("zzz nonsense qqq").is_empty());
        assert!(catalogue().contains("crispr-screen"));
    }
}
