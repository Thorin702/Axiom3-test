//! Multi-LLM AB-MCTS — Adaptive Branching Monte Carlo Tree Search over LLM candidates.
//!
//! A faithful, training-free reproduction (AB-MCTS-A style) of Sakana AI's inference-time
//! scaling method — "Wider or Deeper? Scaling LLM Inference-Time Compute with Adaptive
//! Branching Tree Search" (arXiv 2503.04412; OSS `SakanaAI/treequest`, Apache-2.0). The
//! engineering idea: instead of a fixed-width fan-out ("generate N candidates once"), the
//! search decides ADAPTIVELY at each step whether to "go WIDER" (spawn a brand-new
//! candidate) or "go DEEPER" (refine an existing candidate), and — in the MULTI-LLM variant
//! — WHICH model to sample from, all chosen by Thompson sampling over Beta posteriors, so
//! the budget concentrates where quality is paying off.
//!
//! This is the engine's quality core. It composes with what we already have: the SCORER is
//! our adversarial critique (a quality rubric), the final fold reuses `synthesize`, and
//! every node streams a `run_event` so the search tree is visible + auditable.
//!
//! RED LINE: the score is a WORK-QUALITY rubric (completeness / coherence / task-fit),
//! never a truth verdict. This crate does not depend on axiom-core and never adjudicates
//! whether a claim is true — it only ranks candidate *work* by craftsmanship.
//!
//! Posterior model: we use Beta-Bernoulli arms (reward in [0,1] treated as a success
//! probability) rather than AB-MCTS-M's PyMC mixed model, which isn't practical in Rust.
//! This is the AB-MCTS-A "node aggregation" formulation and is documented as such.
use crate::events::{now_ms, EventSink, RunEventBody, SeqEmitter};
use crate::model::{Kind, Run, RunStatus, StepSpec, StepState, StepStatus};
use crate::synthesize;
use crate::OrchestrateOptions;
use axiom_providers::message::{ContentBlock, Message, Request};
use axiom_runtime::supervisor::{is_retryable_error, retry_async, Cancel, RetryPolicy};
use axiom_runtime::{complete_with, Provider};
use rand::rngs::StdRng;
use rand::SeedableRng;
use rand_distr::{Beta, Distribution};
use std::sync::Arc;
use std::time::Duration;

/// One arm of the multi-LLM axis: which provider/model to sample a candidate from.
#[derive(Clone, Debug)]
pub struct ModelArm {
    pub provider: String,
    pub model: String,
}

/// Search budget + shape. `budget` = number of candidate generations (each is one agent);
/// `max_depth` bounds refine-chain length; `models` are the multi-LLM arms (≥1); `seed`
/// makes the Thompson sampling reproducible (tests) — vary it in production for diversity.
pub struct SearchConfig {
    pub budget: usize,
    pub max_depth: usize,
    pub models: Vec<ModelArm>,
    pub seed: u64,
}

impl SearchConfig {
    /// Single-model default from the run options (still a real AB-MCTS search, just with
    /// one arm on the model axis).
    pub fn single(opts: &OrchestrateOptions, budget: usize, seed: u64) -> Self {
        Self {
            budget,
            max_depth: 3,
            models: vec![ModelArm { provider: opts.provider.clone(), model: opts.model.clone() }],
            seed,
        }
    }
}

/// Produce a candidate solution. `refine_of = None` → a fresh candidate ("go wider");
/// `Some(draft)` → an improved version of `draft` ("go deeper"). Model-agnostic: the
/// chosen `ModelArm` selects the provider/model.
pub trait Generator: Send + Sync {
    fn generate(
        &self,
        task: &str,
        refine_of: Option<&str>,
        arm: &ModelArm,
        cancel: &Cancel,
    ) -> impl std::future::Future<Output = anyhow::Result<String>> + Send;
}

/// Score a candidate's WORK QUALITY in [0,1] — completeness, coherence, task-fit — NOT its
/// factual truth (that would breach the red line). Returns a neutral 0.5 on failure so a
/// transient scoring hiccup doesn't distort the search.
pub trait Scorer: Send + Sync {
    fn score(
        &self,
        task: &str,
        candidate: &str,
        cancel: &Cancel,
    ) -> impl std::future::Future<Output = f64> + Send;
}

// ── Beta-Bernoulli posterior arm (Thompson sampling) ─────────────────────────
#[derive(Clone, Default)]
struct Arm {
    sum: f64, // Σ reward (pseudo-successes)
    n: f64,   // # observations
}
impl Arm {
    fn update(&mut self, reward: f64) {
        if !reward.is_finite() {
            return; // never let a non-finite reward poison sum (defense-in-depth)
        }
        self.sum += reward.clamp(0.0, 1.0);
        self.n += 1.0;
    }
    /// Draw a value from Beta(1+successes, 1+failures). A never-tried arm draws from
    /// Beta(1,1) = Uniform(0,1), i.e. optimistic exploration.
    fn sample(&self, rng: &mut StdRng) -> f64 {
        let a = 1.0 + self.sum;
        let b = 1.0 + (self.n - self.sum).max(0.0);
        Beta::new(a, b).map(|d| d.sample(rng)).unwrap_or(0.5)
    }
}

struct Node {
    id: String,
    parent: Option<usize>,
    children: Vec<usize>,
    candidate: Option<String>, // None only for the root (which represents the raw task)
    score: f64,                // this node's own quality in [0,1] (0 for the root)
    depth: usize,
    model: String,
    provider: String,
    subtree: Arm, // posterior over "value reachable by descending into this node"
    gen_arm: Arm, // posterior over "value of GENERATING a new child here" (node aggregation)
}

/// Public entry: run AB-MCTS with the real provider generator + critique scorer.
pub async fn orchestrate_search(
    goal: &str,
    opts: &OrchestrateOptions,
    cfg: SearchConfig,
    sink: Arc<dyn EventSink>,
    cancel: &Cancel,
) -> Run {
    let generator = ProviderGenerator;
    let scorer = ProviderScorer { provider: opts.provider.clone(), model: opts.model.clone() };
    search_core(goal, opts, cfg, &generator, &scorer, sink, cancel).await
}

/// Generic AB-MCTS core, parameterized over the generator + scorer so the whole search
/// (adaptive wider/deeper, multi-model Thompson sampling, backprop, budgeting) is testable
/// offline with mocks and a fixed seed.
pub async fn search_core<G: Generator, S: Scorer>(
    goal: &str,
    opts: &OrchestrateOptions,
    cfg: SearchConfig,
    generator: &G,
    scorer: &S,
    sink: Arc<dyn EventSink>,
    cancel: &Cancel,
) -> Run {
    let em = Arc::new(SeqEmitter::new(sink, opts.run_id.clone()));
    let mut run = Run {
        id: opts.run_id.clone(),
        kind: Kind::Workflow,
        goal: goal.to_string(),
        status: RunStatus::Planning,
        steps: Vec::new(),
        synthesis: None,
        review: None,
        created_ms: now_ms(),
    };
    em.emit(RunEventBody::RunStarted {
        task_kind: "workflow".into(),
        title: truncate(goal.trim(), 60),
        cancellable: true,
    });

    let arms: Vec<ModelArm> = if cfg.models.is_empty() {
        vec![ModelArm { provider: opts.provider.clone(), model: opts.model.clone() }]
    } else {
        cfg.models.clone()
    };
    let budget = cfg.budget.max(1);
    let max_depth = cfg.max_depth.max(1);

    em.emit(RunEventBody::PlanReady {
        phases: vec!["planning".into(), "searching".into(), "synthesizing".into(), "done".into()],
        planned_agents: budget,
        notes: vec![format!(
            "AB-MCTS · budget {budget} · depth ≤{max_depth} · {} model(s)",
            arms.len()
        )],
    });
    if cancel.is_cancelled() {
        run.status = RunStatus::Cancelled;
        em.emit(RunEventBody::RunFinished { status: "cancelled".into(), agents_total: budget, agents_ok: 0, agents_err: 0, error: None });
        return run;
    }

    run.status = RunStatus::Running;
    em.emit(RunEventBody::PhaseChanged { phase: "searching".into(), index: 1, total: 4 });

    // root = the raw task (no candidate). depth 0 < max_depth, so it can always go wider.
    let mut nodes: Vec<Node> = vec![Node {
        id: format!("{}-root", opts.run_id),
        parent: None,
        children: Vec::new(),
        candidate: None,
        score: 0.0,
        depth: 0,
        model: String::new(),
        provider: String::new(),
        subtree: Arm::default(),
        gen_arm: Arm::default(),
    }];
    let mut model_arms: Vec<Arm> = vec![Arm::default(); arms.len()];
    let mut arm_attempts: Vec<usize> = vec![0; arms.len()];
    let mut rng = StdRng::seed_from_u64(cfg.seed);

    let mut ok = 0usize;
    let mut err = 0usize;
    for _ in 0..budget {
        if cancel.is_cancelled() {
            break;
        }
        crate::wait_while_paused(&opts.pause, cancel).await; // hold between candidate generations while paused
        // ── SELECT: descend from the root via Thompson sampling until "generate here" wins.
        let sel = select(&nodes, max_depth, &mut rng);

        // ── which model to sample from (multi-LLM axis) ──
        // Warm-start on ATTEMPTS (not the loop index): try any un-attempted model first, so
        // every model gets one real try even if an earlier warm-start generation failed —
        // then Thompson-sample. Keying on attempts also means a permanently-failing arm is
        // tried once and then dropped, rather than capturing the whole budget.
        let arm_idx = match arm_attempts.iter().position(|&a| a == 0) {
            Some(k) => k,
            None => pick_arm(&model_arms, &mut rng),
        };
        arm_attempts[arm_idx] += 1;
        let arm = &arms[arm_idx];
        let refine_of = nodes[sel].candidate.clone();
        let going_deeper = refine_of.is_some();
        let node_id = format!("{}-n{}", opts.run_id, ok + err + 1); // 1-based attempt number
        em.emit(RunEventBody::AgentSpawned {
            agent_id: node_id.clone(),
            role: format!("{} · {}", if going_deeper { "refine" } else { "explore" }, arm.model),
            model: arm.model.clone(),
            vendor: arm.provider.clone(),
            phase: "searching".into(),
        });

        // ── EXPAND: generate (or refine) a candidate ──
        let gen = generator.generate(goal, refine_of.as_deref(), arm, cancel).await;
        let candidate = match gen {
            Ok(c) if !c.trim().is_empty() => c,
            Ok(_) | Err(_) => {
                err += 1;
                let msg = gen.err().map(|e| e.to_string()).unwrap_or_else(|| "empty candidate".into());
                em.emit(RunEventBody::AgentFinished { agent_id: node_id, outcome: "err".into(), tokens_in: 0, tokens_out: 0, error: Some(msg), summary: None });
                // Advance progress on the error path too, so the bar doesn't stall (mirrors
                // the success path + the flat orchestrate, which emits on both outcomes).
                let done = ok + err;
                em.emit(RunEventBody::RunProgress { agents_done: done, agents_total: budget, agents_running: 0, agents_queued: budget.saturating_sub(done) });
                continue;
            }
        };
        // ── SCORE: the adversarial-verify critique, as a quality rubric in [0,1] ──
        // Sanitize at the boundary: a non-finite score (e.g. a buggy external Scorer) must
        // not enter the tree/posteriors — fall back to neutral rather than poison the search.
        let raw = scorer.score(goal, &candidate, cancel).await;
        let score = if raw.is_finite() { raw.clamp(0.0, 1.0) } else { 0.5 };
        ok += 1;

        let new_idx = nodes.len();
        let depth = nodes[sel].depth + 1;
        nodes.push(Node {
            id: node_id.clone(),
            parent: Some(sel),
            children: Vec::new(),
            candidate: Some(candidate.clone()),
            score,
            depth,
            model: arm.model.clone(),
            provider: arm.provider.clone(),
            subtree: Arm::default(),
            gen_arm: Arm::default(),
        });
        nodes[sel].children.push(new_idx);
        // ── BACKPROP: the new score informs every ancestor's subtree posterior, the parent's
        // GEN posterior (node aggregation), and the model arm.
        nodes[sel].gen_arm.update(score);
        model_arms[arm_idx].update(score);
        let mut cur = Some(new_idx);
        while let Some(c) = cur {
            nodes[c].subtree.update(score);
            cur = nodes[c].parent;
        }

        em.emit(RunEventBody::AgentFinished {
            agent_id: node_id,
            outcome: "ok".into(),
            tokens_in: 0,
            tokens_out: 0,
            error: None,
            summary: Some(format!("score {score:.2} · {}", truncate(&candidate, 80))),
        });
        let done = ok + err;
        em.emit(RunEventBody::RunProgress {
            agents_done: done,
            agents_total: budget,
            agents_running: 0,
            agents_queued: budget.saturating_sub(done),
        });
    }

    let cancelled = cancel.is_cancelled();

    // ── best-of: the top-scoring candidates become the run's steps; fold via synthesize ──
    let mut ranked: Vec<&Node> = nodes.iter().filter(|n| n.candidate.is_some()).collect();
    ranked.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    let top: Vec<StepState> = ranked
        .iter()
        .take(6)
        .map(|n| {
            let mut st = StepState::new(StepSpec {
                id: n.id.clone(),
                role: format!("candidate · {:.2} · {}", n.score, n.model),
                goal: String::new(),
                provider: n.provider.clone(),
                model: n.model.clone(),
                deps: Vec::new(),
                max_tokens: None,
            });
            st.status = StepStatus::Ok;
            st.output = n.candidate.clone();
            st
        })
        .collect();
    run.steps = top;

    em.emit(RunEventBody::PhaseChanged { phase: "synthesizing".into(), index: 2, total: 4 });
    run.status = RunStatus::Synthesizing;
    if run.steps.is_empty() {
        run.synthesis = Some("(no candidate produced)".into());
    } else if run.steps.len() == 1 || cancelled {
        run.synthesis = run.steps[0].output.clone();
    } else {
        // fold the top candidates into one result (reuses the verify+synthesize seam).
        let critique = synthesize::critique(goal, &run.steps, &opts.provider, &opts.model, cancel).await;
        run.review = if critique.trim().is_empty() { None } else { Some(critique.clone()) };
        run.synthesis = Some(synthesize::synthesize(goal, &run.steps, &critique, &opts.provider, &opts.model, cancel).await);
    }

    run.status = if cancelled { RunStatus::Cancelled } else { RunStatus::Done };
    em.emit(RunEventBody::PhaseChanged { phase: "done".into(), index: 3, total: 4 });
    em.emit(RunEventBody::RunFinished {
        status: if cancelled { "cancelled".into() } else { "done".into() },
        agents_total: ok + err,
        agents_ok: ok,
        agents_err: err,
        error: None,
    });
    run
}

/// Descend from the root, at each node Thompson-sampling between "descend into an
/// expandable child" and "generate a new child here", until GEN wins. Returns the node to
/// expand. Invariant: the returned node has depth < max_depth (we only ever descend into
/// children with depth < max_depth, and the root has depth 0), so its new child stays
/// within the depth cap.
fn select(nodes: &[Node], max_depth: usize, rng: &mut StdRng) -> usize {
    let mut cur = 0usize;
    loop {
        let node = &nodes[cur];
        let mut best_score = f64::MIN;
        let mut best_child: Option<usize> = None;
        for &c in &node.children {
            if nodes[c].depth < max_depth {
                let s = nodes[c].subtree.sample(rng);
                if s > best_score {
                    best_score = s;
                    best_child = Some(c);
                }
            }
        }
        // "generate here" is always a valid action at `cur` (depth < max_depth by invariant).
        let gen = node.gen_arm.sample(rng);
        match best_child {
            Some(c) if best_score >= gen => cur = c, // go deeper
            _ => return cur,                         // go wider (expand at cur)
        }
    }
}

fn pick_arm(arms: &[Arm], rng: &mut StdRng) -> usize {
    let mut best = 0usize;
    let mut best_s = f64::MIN;
    for (i, a) in arms.iter().enumerate() {
        let s = a.sample(rng);
        if s > best_s {
            best_s = s;
            best = i;
        }
    }
    best
}

fn truncate(s: &str, n: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}

// ── production generator + scorer (LLM-backed) ───────────────────────────────

/// Real generator: a fresh candidate, or an improvement of an existing draft, from the
/// chosen provider/model.
pub struct ProviderGenerator;

impl Generator for ProviderGenerator {
    fn generate(
        &self,
        task: &str,
        refine_of: Option<&str>,
        arm: &ModelArm,
        cancel: &Cancel,
    ) -> impl std::future::Future<Output = anyhow::Result<String>> + Send {
        let (system, user) = match refine_of {
            None => (
                "You produce ONE strong candidate solution to the task. Be complete, \
                 specific, well-structured, and self-contained. Return only the solution."
                    .to_string(),
                format!("TASK:\n{task}"),
            ),
            Some(draft) => (
                "You improve an existing draft so it better solves the task — fix gaps, add \
                 specificity, tighten structure and remove weak parts. Keep what works. \
                 Return only the improved version."
                    .to_string(),
                format!("TASK:\n{task}\n\nCURRENT DRAFT:\n{draft}"),
            ),
        };
        let provider = arm.provider.clone();
        let model = arm.model.clone();
        let cancelled = cancel.is_cancelled();
        async move {
            if cancelled {
                anyhow::bail!("cancelled");
            }
            let p = Provider::parse(&provider).ok_or_else(|| anyhow::anyhow!("unknown provider: {provider}"))?;
            let req = Request {
                system: Some(system),
                messages: vec![Message::user(vec![ContentBlock::text(user)])],
                model,
                max_tokens: None,
                reasoning_effort: None,
                tools: Vec::new(),
            };
            let reply = retry_async(&RetryPolicy::default(), is_retryable_error, || async {
                tokio::time::timeout(Duration::from_secs(60), complete_with(p, &req))
                    .await
                    .map_err(|_| anyhow::anyhow!("generation timed out after 60s"))?
            })
            .await?;
            Ok(reply)
        }
    }
}

/// Real scorer: the adversarial critique reduced to a quality rubric in [0,1]. It rates
/// WORK QUALITY, explicitly not factual truth (red line). Failure → neutral 0.5.
pub struct ProviderScorer {
    pub provider: String,
    pub model: String,
}

impl Scorer for ProviderScorer {
    fn score(
        &self,
        task: &str,
        candidate: &str,
        cancel: &Cancel,
    ) -> impl std::future::Future<Output = f64> + Send {
        let provider = self.provider.clone();
        let model = self.model.clone();
        let task = task.to_string();
        let candidate = candidate.to_string();
        let cancelled = cancel.is_cancelled();
        async move {
            if cancelled {
                return 0.5;
            }
            let Some(p) = Provider::parse(&provider) else { return 0.5 };
            // Multi-lens rubric in ONE call (no extra latency): rate three craftsmanship
            // axes so a fluent-but-shallow draft can't win on coherence alone. This is a
            // work-quality judgment, NOT fact-checking (red line).
            let system = "You rate a draft answer's QUALITY AS WORK on three axes, each 0-100: \
                          (1) COMPLETENESS — does it fully address every part of the task; \
                          (2) RIGOR — is it specific, well-supported, and internally consistent; \
                          (3) CLARITY — is it well-structured and coherent. This is a \
                          craftsmanship judgment, NOT a judgment of whether any claim is true — \
                          do not fact-check. Reply with EXACTLY three integers 0-100 separated \
                          by spaces, e.g. `85 70 90`. No other text.";
            let user = format!("TASK:\n{task}\n\nDRAFT:\n{candidate}\n\nScores (completeness rigor clarity):");
            let req = Request {
                system: Some(system.into()),
                messages: vec![Message::user(vec![ContentBlock::text(user)])],
                model,
                max_tokens: None,
                reasoning_effort: None,
                tools: Vec::new(),
            };
            let reply = retry_async(&RetryPolicy::default(), is_retryable_error, || async {
                tokio::time::timeout(Duration::from_secs(45), complete_with(p, &req))
                    .await
                    .map_err(|_| anyhow::anyhow!("scoring timed out after 45s"))?
            })
            .await;
            match reply {
                Ok(text) => {
                    let lenses = parse_lens(&text);
                    if lenses.len() >= 2 {
                        combine_lens(&lenses)
                    } else {
                        // a model that ignored the format and gave one holistic score
                        parse_score(&text).unwrap_or(0.5)
                    }
                }
                Err(_) => 0.5,
            }
        }
    }
}

/// Parse a model's score reply → [0,1]. Handles the common LLM rating formats: an explicit
/// fraction ("9/10", "9 out of 10", "85/100") → num/denom; a bare [0,1] fraction ("0.85") →
/// 0.85; a 0-100 integer/decimal ("87") → 0.87. Out-of-range / no number (e.g. a leading
/// year "2026") → None → the neutral 0.5 fallback. The explicit-denominator case prevents a
/// "9/10" reply being mis-scaled to 0.09.
fn parse_score(text: &str) -> Option<f64> {
    // leading numeric token + where it ends
    let mut tok = String::new();
    let mut end = text.len();
    let mut started = false;
    for (idx, ch) in text.char_indices() {
        if ch.is_ascii_digit() || (ch == '.' && !tok.is_empty() && !tok.contains('.')) {
            tok.push(ch);
            started = true;
        } else if started {
            end = idx;
            break;
        }
    }
    let num: f64 = tok.trim_end_matches('.').parse().ok()?;

    // explicit denominator? "9/10" or "9 out of 10"
    let rest = text[end..].trim_start();
    let after_denom = rest
        .strip_prefix('/')
        .or_else(|| rest.get(..6).filter(|p| p.eq_ignore_ascii_case("out of")).map(|_| &rest[6..]));
    if let Some(a) = after_denom {
        let ds: String = a.trim_start().chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
        if let Ok(d) = ds.trim_end_matches('.').parse::<f64>() {
            if d > 0.0 && num <= d {
                return Some((num / d).clamp(0.0, 1.0));
            }
        }
    }

    if tok.contains('.') && (0.0..=1.0).contains(&num) {
        Some(num) // already a [0,1] fraction, e.g. "0.85"
    } else if (0.0..=100.0).contains(&num) {
        Some(num / 100.0) // a 0-100 score, e.g. "87"
    } else {
        None // out of range → neutral fallback
    }
}

/// Extract up to 3 integer axis scores 0-100 (→ [0,1]) from a multi-lens reply like
/// "85 70 90". Ignores tokens > 100 (a stray year/id) rather than mis-scaling them.
fn parse_lens(text: &str) -> Vec<f64> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let flush = |cur: &mut String, out: &mut Vec<f64>| {
        if !cur.is_empty() {
            if let Ok(v) = cur.parse::<u32>() {
                if v <= 100 {
                    out.push(v as f64 / 100.0);
                }
            }
            cur.clear();
        }
    };
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            cur.push(ch);
        } else {
            flush(&mut cur, &mut out);
            if out.len() >= 3 {
                return out;
            }
        }
    }
    flush(&mut cur, &mut out);
    out.truncate(3);
    out
}

/// Combine axis scores into one [0,1] quality signal. A blend of mean and MIN so a draft
/// that's strong on one axis but weak on another (e.g. fluent but incomplete) can't ride
/// its best axis to the top — the weakest axis pulls it down.
fn combine_lens(v: &[f64]) -> f64 {
    if v.is_empty() {
        return 0.5;
    }
    let mean = v.iter().sum::<f64>() / v.len() as f64;
    let min = v.iter().copied().fold(f64::INFINITY, f64::min);
    (0.5 * mean + 0.5 * min).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{RunEventBody as B, VecSink};
    use std::collections::HashSet;

    fn opts() -> OrchestrateOptions {
        OrchestrateOptions { run_id: "s".into(), provider: "mock".into(), model: "mock".into(), concurrency: 2, pause: None }
    }
    fn cfg(budget: usize, max_depth: usize, seed: u64) -> SearchConfig {
        SearchConfig { budget, max_depth, models: Vec::new(), seed }
    }

    /// Generator that makes "deeper" candidates longer (each refine appends a token), so a
    /// length-based scorer rewards depth.
    struct GrowGen;
    impl Generator for GrowGen {
        fn generate(&self, _task: &str, refine_of: Option<&str>, _arm: &ModelArm, _c: &Cancel)
            -> impl std::future::Future<Output = anyhow::Result<String>> + Send {
            let out = match refine_of {
                None => "x".to_string(),
                Some(d) => format!("{d}x"),
            };
            async move { Ok(out) }
        }
    }
    /// Score ∝ length (longer = better), capped — rewards going deeper.
    struct LenScorer;
    impl Scorer for LenScorer {
        fn score(&self, _t: &str, cand: &str, _c: &Cancel) -> impl std::future::Future<Output = f64> + Send {
            let s = (cand.trim().chars().count() as f64 / 6.0).min(1.0);
            async move { s }
        }
    }
    /// Score rewards SHORT candidates — rewards staying wide (fresh depth-1 nodes).
    struct ShortScorer;
    impl Scorer for ShortScorer {
        fn score(&self, _t: &str, cand: &str, _c: &Cancel) -> impl std::future::Future<Output = f64> + Send {
            let n = cand.trim().chars().count() as f64;
            let s = (1.0 / n).min(1.0);
            async move { s }
        }
    }

    #[tokio::test]
    async fn respects_budget_and_streams_balanced_gapfree_events() {
        let sink = Arc::new(VecSink::default());
        let run = search_core("goal", &opts(), cfg(8, 3, 42), &GrowGen, &LenScorer, sink.clone(), &Cancel::new()).await;
        assert_eq!(run.status, RunStatus::Done);
        let evs = sink.events();
        for (i, e) in evs.iter().enumerate() {
            assert_eq!(e.seq, i as u64, "seq must be gap-free");
        }
        // exactly `budget` expansions were spawned, each with a terminal event
        let spawned: HashSet<_> = evs.iter().filter_map(|e| match &e.body {
            B::AgentSpawned { agent_id, .. } => Some(agent_id.clone()),
            _ => None,
        }).collect();
        let finished: HashSet<_> = evs.iter().filter_map(|e| match &e.body {
            B::AgentFinished { agent_id, .. } => Some(agent_id.clone()),
            _ => None,
        }).collect();
        assert_eq!(spawned.len(), 8, "one agent per budget unit");
        assert_eq!(spawned, finished, "every spawn has a finish");
        assert!(run.synthesis.is_some());
        assert!(!run.steps.is_empty());
    }

    #[tokio::test]
    async fn same_seed_is_deterministic() {
        let a = search_core("g", &opts(), cfg(10, 3, 7), &GrowGen, &LenScorer, Arc::new(VecSink::default()), &Cancel::new()).await;
        let b = search_core("g", &opts(), cfg(10, 3, 7), &GrowGen, &LenScorer, Arc::new(VecSink::default()), &Cancel::new()).await;
        assert_eq!(a.synthesis, b.synthesis, "fixed seed ⇒ identical search");
    }

    #[tokio::test]
    async fn depth_reward_drives_deeper_search() {
        // With a scorer that rewards length (depth), the best candidate should be refined
        // beyond depth 1 — i.e. the search chose to go deeper.
        let sink = Arc::new(VecSink::default());
        let _ = search_core("g", &opts(), cfg(14, 4, 3), &GrowGen, &LenScorer, sink.clone(), &Cancel::new()).await;
        // "refine" spawns appear once the search goes deeper (depth ≥ 2).
        let refines = sink.events().iter().filter(|e| matches!(&e.body, B::AgentSpawned { role, .. } if role.starts_with("refine"))).count();
        assert!(refines > 0, "a depth-rewarding scorer must trigger refinement (go deeper)");
    }

    #[tokio::test]
    async fn breadth_reward_stays_wide() {
        // A scorer rewarding short (fresh) candidates should keep exploring wider — most
        // expansions are depth-1 "explore", not "refine".
        let sink = Arc::new(VecSink::default());
        let _ = search_core("g", &opts(), cfg(14, 4, 3), &GrowGen, &ShortScorer, sink.clone(), &Cancel::new()).await;
        let evs = sink.events();
        let explores = evs.iter().filter(|e| matches!(&e.body, B::AgentSpawned { role, .. } if role.starts_with("explore"))).count();
        let refines = evs.iter().filter(|e| matches!(&e.body, B::AgentSpawned { role, .. } if role.starts_with("refine"))).count();
        assert!(explores > refines, "a breadth-rewarding scorer should stay wide ({explores} explore vs {refines} refine)");
    }

    #[tokio::test]
    async fn multi_model_uses_every_arm() {
        // Two model arms, uniform scorer → over enough budget both arms get sampled.
        struct FlatScorer;
        impl Scorer for FlatScorer {
            #[allow(clippy::manual_async_fn)]
            fn score(&self, _t: &str, _c: &str, _cn: &Cancel) -> impl std::future::Future<Output = f64> + Send {
                async { 0.5 }
            }
        }
        let sink = Arc::new(VecSink::default());
        let mut c = cfg(20, 2, 11);
        c.models = vec![
            ModelArm { provider: "p0".into(), model: "m0".into() },
            ModelArm { provider: "p1".into(), model: "m1".into() },
        ];
        let _ = search_core("g", &opts(), c, &GrowGen, &FlatScorer, sink.clone(), &Cancel::new()).await;
        let mut m0 = 0;
        let mut m1 = 0;
        for e in sink.events() {
            if let B::AgentSpawned { model, .. } = &e.body {
                if model == "m0" { m0 += 1; }
                if model == "m1" { m1 += 1; }
            }
        }
        assert!(m0 > 0 && m1 > 0, "both model arms should be exercised (m0={m0}, m1={m1})");
    }

    #[test]
    fn parse_score_bounds() {
        assert_eq!(parse_score("87"), Some(0.87));
        assert_eq!(parse_score("Score: 42/100"), Some(0.42));
        assert_eq!(parse_score("100"), Some(1.0));
        assert_eq!(parse_score("0"), Some(0.0));
        // a model that replies with a [0,1] fraction is honored, not scored 0
        assert_eq!(parse_score("0.85"), Some(0.85));
        assert_eq!(parse_score("quality: 0.9"), Some(0.9));
        // explicit fractions must NOT be mis-scaled to /100 (the review's med finding)
        assert_eq!(parse_score("9/10"), Some(0.9));
        assert_eq!(parse_score("It's a 9 out of 10"), Some(0.9));
        assert_eq!(parse_score("85/100"), Some(0.85));
        // a leading out-of-range number (a year) → rejected → neutral fallback
        assert_eq!(parse_score("in 2026 the score is 55"), None);
        assert_eq!(parse_score("no number here"), None);
    }

    #[test]
    fn parse_lens_extracts_axes() {
        assert_eq!(parse_lens("85 70 90"), vec![0.85, 0.7, 0.9]);
        assert_eq!(parse_lens("C:85 R:70 L:90"), vec![0.85, 0.7, 0.9]);
        assert_eq!(parse_lens("85"), vec![0.85]);
        // a stray out-of-range number (a year) is ignored, the three axes still parse
        assert_eq!(parse_lens("as of 2026: 85 70 90"), vec![0.85, 0.7, 0.9]);
        assert!(parse_lens("no numbers").is_empty());
    }

    #[test]
    fn combine_lens_penalizes_the_weakest_axis() {
        // balanced → ~the level; lopsided → pulled toward the min (can't ride one axis)
        assert!((combine_lens(&[0.9, 0.9, 0.9]) - 0.9).abs() < 1e-9);
        let lopsided = combine_lens(&[0.9, 0.9, 0.1]);
        let balanced = combine_lens(&[0.63, 0.63, 0.63]);
        assert!(lopsided < balanced, "a weak axis must drag the score down ({lopsided} vs {balanced})");
        assert_eq!(combine_lens(&[]), 0.5);
    }

    #[tokio::test]
    async fn nan_score_does_not_poison_the_search() {
        // A buggy external Scorer returning NaN must degrade to neutral, not silently kill
        // the search (Beta posteriors stuck at 0.5 forever).
        struct NanScorer;
        impl Scorer for NanScorer {
            #[allow(clippy::manual_async_fn)]
            fn score(&self, _t: &str, _c: &str, _cn: &Cancel) -> impl std::future::Future<Output = f64> + Send {
                async { f64::NAN }
            }
        }
        let run = search_core("g", &opts(), cfg(6, 3, 5), &GrowGen, &NanScorer, Arc::new(VecSink::default()), &Cancel::new()).await;
        assert_eq!(run.status, RunStatus::Done);
        assert!(!run.steps.is_empty(), "candidates are still produced");
        // the run completes cleanly; no panic, no hang.
    }

    #[tokio::test]
    async fn warm_start_survives_a_failing_arm() {
        // Arm m0 always errors; the warm-start (keyed on attempts) must still give m1 a real
        // try, and m0's failure must not capture the whole budget.
        struct PickyGen;
        impl Generator for PickyGen {
            #[allow(clippy::manual_async_fn)]
            fn generate(&self, _t: &str, _r: Option<&str>, arm: &ModelArm, _c: &Cancel)
                -> impl std::future::Future<Output = anyhow::Result<String>> + Send {
                let bad = arm.model == "m0";
                async move {
                    if bad { anyhow::bail!("m0 down") } else { Ok("ok".to_string()) }
                }
            }
        }
        struct Flat;
        impl Scorer for Flat {
            #[allow(clippy::manual_async_fn)]
            fn score(&self, _t: &str, _c: &str, _cn: &Cancel) -> impl std::future::Future<Output = f64> + Send {
                async { 0.7 }
            }
        }
        let mut c = cfg(8, 2, 3);
        c.models = vec![
            ModelArm { provider: "p0".into(), model: "m0".into() },
            ModelArm { provider: "p1".into(), model: "m1".into() },
        ];
        let run = search_core("g", &opts(), c, &PickyGen, &Flat, Arc::new(VecSink::default()), &Cancel::new()).await;
        assert_eq!(run.status, RunStatus::Done);
        // m1 produced usable candidates despite m0 failing every time
        assert!(run.steps.iter().any(|s| s.spec.model == "m1"), "the healthy arm still contributes");
    }
}
