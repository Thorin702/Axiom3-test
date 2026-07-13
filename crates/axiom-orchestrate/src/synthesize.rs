//! Fold completed step outputs into one result. Free-form synthesis ONLY — never an
//! adjudication or verdict (that lives in axiom-core, off-limits here). Falls back to
//! concatenation under role headers if the provider is unavailable or the call fails.
use crate::model::StepState;
use axiom_providers::message::{ContentBlock, Message, Request};
use axiom_runtime::supervisor::{is_retryable_error, retry_async, Cancel, RetryPolicy};
use axiom_runtime::{complete_with, Provider};
use std::time::Duration;

/// Adversarial verify stage: one critic reviews the sub-agent outputs against the goal
/// and names gaps / errors / unsupported claims. Free-form critique — never a verdict.
/// Returns "" when there's nothing to critique or the provider is unavailable (the
/// synthesis still runs without it).
pub async fn critique(goal: &str, steps: &[StepState], provider: &str, model: &str, cancel: &Cancel) -> String {
    if steps.is_empty() || cancel.is_cancelled() {
        return String::new();
    }
    let Some(p) = Provider::parse(provider) else {
        return String::new();
    };
    let mut body = String::new();
    for s in steps {
        if let Some(out) = &s.output {
            body.push_str(&format!("\n## {} ({})\n{}\n", s.spec.role, s.spec.id, out));
        }
    }
    if body.is_empty() {
        return String::new();
    }
    let system = "You are a critical reviewer of a multi-agent run. Given the goal and the \
                  sub-agent outputs, list concrete gaps, errors, contradictions, or unsupported \
                  claims — what a careful reviewer would flag. Be specific and brief; bullet \
                  points. If the outputs are solid, say so in one line. Do not rewrite them.";
    let user = format!("GOAL:\n{goal}\n\nSUB-AGENT OUTPUTS:{body}");
    let req = Request {
        system: Some(system.into()),
        messages: vec![Message::user(vec![ContentBlock::text(user)])],
        model: model.to_string(),
        max_tokens: None,
        reasoning_effort: None,
        tools: Vec::new(),
    };
    let r = retry_async(&RetryPolicy::default(), is_retryable_error, || async {
        tokio::time::timeout(Duration::from_secs(90), complete_with(p, &req))
            .await
            .map_err(|_| anyhow::anyhow!("critique timed out after 90s"))?
    })
    .await;
    r.unwrap_or_default()
}

pub async fn synthesize(goal: &str, steps: &[StepState], critique: &str, provider: &str, model: &str, cancel: &Cancel) -> String {
    let concat = concat_fallback(steps);
    if steps.is_empty() || cancel.is_cancelled() {
        return concat;
    }
    let Some(p) = Provider::parse(provider) else {
        return concat;
    };
    let mut body = String::new();
    for s in steps {
        if let Some(out) = &s.output {
            body.push_str(&format!("\n## {} ({})\n{}\n", s.spec.role, s.spec.id, out));
        }
    }
    if body.is_empty() {
        return concat;
    }
    let system = "You fold sub-agent outputs into one coherent result for the user's goal. Be \
                  concise and faithful to the parts; do not invent. A reviewer's critique is \
                  provided — address its valid points (fix gaps, drop unsupported claims) while \
                  staying faithful to the work. Write in Markdown. When the content calls for \
                  it, emit science figures that render natively: a chemical structure as a \
                  ```smiles block (one SMILES string); an architecture/flow diagram as a \
                  ```mermaid block; a key equation as a ```math block (LaTeX); a data figure \
                  as a ```chart block (JSON with type scatter|bar|line, title, xlabel, ylabel, \
                  and points/data/series); a gene/feature expression matrix as a ```dotplot \
                  block (JSON with rows, cols, and cells of {r,c,size,color}); and a 3D protein \
                  structure as a ```protein block (a 4-char PDB id like 1CRN, or inline PDB \
                  text). More native scientific figures, each its own fenced block of JSON: \
                  ```forest (ranked estimates with CIs — {rows:[{label,estimate,lo,hi}], reference?}); \
                  ```heatmap (a value matrix — {rows, cols, values:[[..]], scale:\"sequential|diverging\"}); \
                  ```confusion (a classifier's matrix — {labels, matrix:[[counts]], normalize:\"row|col|none\"}); \
                  ```dist (distributions to compare — {groups:[{label, values:[..]}], show:[\"box\",\"points\"]}); \
                  ```volcano (differential expression — {genes:[{gene,lfc,padj}], lfcThreshold, pThreshold}); \
                  ```manhattan (GWAS — {points:[{chr,pos,p,snp?,gene?}], threshold}); \
                  ```band (a curve with an uncertainty band — {series:[{name, points:[[x,y] or [x,y,lo,hi] or [x,y,err]]}], xlabel, ylabel}); \
                  ```msa (a sequence alignment — {type:\"protein|dna\", seqs:[{id,seq}], color:\"clustal|identity\"}); \
                  ```rxn (a reaction — {reactants:[SMILES], products:[SMILES], above?, below?}); \
                  ```field (a 2D scalar field — {x:[..], y:[..], z:[[..]], contours?}); \
                  ```network (a node-link graph — {nodes:[{id,group,value}], edges:[{source,target,weight?,sign?}], directed?}); \
                  ```track (protein domains + mutations — {length, domains:[{name,from,to}], markers:[{pos,label,count,class}]}); \
                  ```roc (classifier ROC/PR curves — {kind:\"roc|pr\", curves:[{name, auc?, points:[[x,y]]}]}); \
                  ```tree (phylogeny/dendrogram — a Newick string {newick:\"((A:.1,B:.2):.3,C:.4);\"}); \
                  ```seqlogo (a sequence motif — {type:\"dna|protein\", columns:[{A,C,G,T frequencies}]}); \
                  ```sankey (flows — {nodes:[..], links:[{source,target,value}]}); \
                  ```spectrum (MS/NMR/IR — {kind:\"ms|nmr|ir\", peaks:[[x,intensity]], invertX?}); \
                  ```dose (dose-response — {curves:[{name, points:[[log10dose,resp]], ic50?, hill?, top?, bottom?}]}); \
                  ```slope (paired before/after — {left, right, rows:[{label, from, to}]}); \
                  ```quiver (2D vector field — {vectors:[{x,y,u,v}]}). \
                  Only use these when genuinely \
                  relevant, and only with real values — never fabricate a structure, id, or \
                  dataset. Return only the result.";
    let crit = if critique.trim().is_empty() {
        String::new()
    } else {
        format!("\n\nREVIEWER CRITIQUE (address valid points):\n{critique}")
    };
    let user = format!("GOAL:\n{goal}\n\nSUB-AGENT OUTPUTS:{body}{crit}");
    let req = Request {
        system: Some(system.into()),
        messages: vec![Message::user(vec![ContentBlock::text(user)])],
        model: model.to_string(),
        max_tokens: None,
        reasoning_effort: None,
        tools: Vec::new(),
    };
    let r = retry_async(&RetryPolicy::default(), is_retryable_error, || async {
        tokio::time::timeout(Duration::from_secs(45), complete_with(p, &req))
            .await
            .map_err(|_| anyhow::anyhow!("synthesis timed out after 45s"))?
    })
    .await;
    r.unwrap_or(concat)
}

fn concat_fallback(steps: &[StepState]) -> String {
    let mut s = String::new();
    for st in steps {
        if let Some(out) = &st.output {
            s.push_str(&format!("## {}\n{}\n\n", st.spec.role, out));
        }
    }
    if s.is_empty() {
        s.push_str("(no output)");
    }
    s
}
