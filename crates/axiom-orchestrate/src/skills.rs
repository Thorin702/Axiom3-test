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
        name: "single-cell-rnaseq",
        keywords: "scrna scRNA-seq scanpy anndata umap leiden cluster cell type marker harmony batch integration immune",
        summary: "scRNA-seq: QC → normalize → HVG → PCA → batch-correct → Leiden → UMAP → annotate.",
        card: "# Single-cell RNA-seq (scanpy)\n\
            Install: `pip_install scanpy anndata leidenalg igraph` (and `harmonypy` for batch integration).\n\
            Pipeline (build up in the kernel step by step):\n\
            1. Load into AnnData (sc.read_csv / read_10x_mtx / read_h5ad); orient cells × genes.\n\
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
        assert!(get("phylogenetics").is_some());
        assert!(get("nonexistent").is_none());
        // A miss returns nothing; the catalogue still lists everything.
        assert!(search("zzz nonsense qqq").is_empty());
        assert!(catalogue().contains("crispr-screen"));
    }
}
