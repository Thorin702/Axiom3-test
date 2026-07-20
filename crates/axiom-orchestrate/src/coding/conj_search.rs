//! ALGORITHM LAYER for conjectures: adaptive-branching search over hypotheses, scored by MECHANICAL
//! signals only. No model is ever asked "how good is this" — the model generates and refines
//! candidates; the algorithm decides where to spend the next call, and ranks what comes back.
//!
//! ## Why this exists
//! The adversarial refuter already runs and already produces a verdict line (`致命程度: 致命 | 严重 |
//! 轻微 | 无法反驳`) for every conjecture. Until now NOTHING in the repo parsed it: we paid for up to
//! four adversarial model calls per run and threw the entire signal away. Parsing it turns a cost we
//! were already paying into the reward function of a search.
//!
//! ## Honest scope
//! The tree reuses `search::select_node` (Thompson sampling over Beta posteriors, GO-WIDE vs GO-DEEP).
//! At the default budget each arm sees 0-2 observations, where Beta(1+s,1+f) is nearly Uniform(0,1) —
//! so branch choice is close to random and this is closer to a candidate tournament than to a
//! meaningfully adaptive search. That is stated in the ledger rather than papered over, and
//! `AXIOM_CONJ_SEARCH_RANDOM=1` runs a uniform-random control so the difference can be MEASURED
//! instead of asserted.

use super::{bigram_novelty, block_issues, conjecture_blocks, is_abandoned, refute_one, CodingModel, Source};
use crate::search::{combine_lens, select_node, Arm, Branchable, SelectTrace};
use axiom_runtime::supervisor::Cancel;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// How badly the refuter says a conjecture is hurt. `Unparsed` and `Insufficient` are NOT failures to
/// be optimistic about — see `survival_lens`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Severity {
    Fatal,
    Serious,
    Minor,
    /// The refuter tried and could not land a blow — the conjecture held up.
    Unrefuted,
    /// The refuter could not attack because the EVIDENCE was too thin. Opposite meaning to
    /// `Unrefuted`, and scoring them the same would pay a premium for unfalsifiable vagueness.
    Insufficient,
    Unparsed,
}

impl Severity {
    pub(super) fn label(self) -> &'static str {
        match self {
            Severity::Fatal => "致命",
            Severity::Serious => "严重",
            Severity::Minor => "轻微",
            Severity::Unrefuted => "无法反驳",
            Severity::Insufficient => "证据不足以反驳",
            Severity::Unparsed => "裁决未能解析",
        }
    }
    fn idx(self) -> usize {
        match self {
            Severity::Fatal => 0,
            Severity::Serious => 1,
            Severity::Minor => 2,
            Severity::Unrefuted => 3,
            Severity::Insufficient => 4,
            Severity::Unparsed => 5,
        }
    }
}

/// Parse the refuter's verdict line. Deliberately narrow, in three ways that are each a real bug if
/// dropped:
///  1. The LABEL ITSELF contains 「致命」, so a naive `line.contains("致命")` marks every verdict Fatal.
///     The label must be stripped before matching.
///  2. GLM echoing the template back (`致命程度: 致命 | 严重 | 轻微 | 无法反驳`) is an observed failure
///     mode. Matching more than one enum word means we did not get an answer → `Unparsed`, never Fatal.
///     A parser that kills every candidate is worse than no parser.
///  3. Only the first 12 chars of a line may carry the key, mirroring `is_abandoned` — prose that
///     merely discusses something 致命 must not be read as a verdict.
pub(super) fn refutation_severity(r: &str) -> Severity {
    for l in r.lines() {
        let l = l.trim_start_matches(['-', '*', '#', ' ', '\t']);
        if !l.chars().take(12).collect::<String>().contains("致命程度") {
            continue;
        }
        let Some(i) = l.find("致命程度") else { continue };
        let v = l[i + "致命程度".len()..]
            .trim_start_matches([':', '：', ' ', '\t', '-', '*', '】'])
            .trim();
        let hits = ["致命", "严重", "轻微", "无法反驳"].iter().filter(|k| v.contains(*k)).count();
        if hits != 1 {
            return Severity::Unparsed;
        }
        if v.contains("无法反驳") {
            let thin = r.lines().any(|x| {
                let x = x.trim_start_matches(['-', '*', '#', ' ', '\t']);
                x.chars().take(8).collect::<String>().contains("依据") && x.contains("证据不足")
            });
            return if thin { Severity::Insufficient } else { Severity::Unrefuted };
        }
        if v.contains("致命") {
            return Severity::Fatal;
        }
        if v.contains("严重") {
            return Severity::Serious;
        }
        return Severity::Minor;
    }
    Severity::Unparsed
}

/// Survival axis.
///
/// The `Insufficient` case is where verification must NOT punish novelty. Measured across 10 real
/// reports, 6 of 10 CHAMPIONS were "证据不足以反驳" — the refuter couldn't kill them because no current
/// evidence bears on them. For a product whose value is *reachable novel hypotheses*, that is the
/// SWEET SPOT (a precise, testable claim nobody has tested yet), not a weakness — yet a flat 0.5 ranked
/// it as mediocre and the ledger called the ranking "untrustworthy", underselling the best output.
///
/// The discriminator between good-novel and vacuous is whether the conjecture carries a CONCRETE,
/// executable falsifier. The refuter's rule ④ already routes the genuinely vacuous to 致命, so anything
/// reaching `Insufficient` has passed that filter; `concrete_falsifier` is a second, mechanical check.
/// It stays a SOFT score input, never a gate (the quant_claim_count lesson).
///
/// `Unparsed` remains 0.5 — a genuinely unknown/broken verdict, distinct from a novel-but-untested one.
fn survival_lens(s: Severity, concrete_falsifier: bool) -> f64 {
    match s {
        Severity::Fatal => 0.0,
        Severity::Serious => 0.35,
        Severity::Unparsed => 0.5,
        Severity::Insufficient => {
            if concrete_falsifier {
                0.68
            } else {
                0.45
            }
        }
        Severity::Minor => 0.70,
        Severity::Unrefuted => 0.85,
    }
}

/// A SOFT falsifiability signal: does the conjecture's 证伪条件 name a measurable outcome (a
/// comparison, a statistic, a magnitude) rather than a circular "if it's wrong"? Used only to tell a
/// precise novel conjecture from a vague one inside the `Insufficient` bucket — never a gate.
fn has_concrete_falsifier(body: &str) -> bool {
    let Some(f) = super::field_value(body, "证伪条件") else { return false };
    f.contains(['≥', '≤', '<', '>', '%', '='])
        || ["AUC", "HR", "IC50", "EC50", "统计", "显著", "倍", "阈值", "下降", "升高", "降低", "提高", "减少", "增加", "p 值", "P 值", "置信"]
            .iter()
            .any(|m| f.contains(m))
}

/// Structure axis, scored from the ALREADY-PARSED (heading, body) pair.
/// ⚠️ It must NOT re-parse `heading + body`: `conjecture_blocks` strips the leading `#` when it
/// extracts a heading, so re-parsing finds no block and returns 0.0 for EVERY candidate. That shipped
/// once — with `combine_lens`'s min term, a permanently-zero axis pinned every score to ~0.3 and the
/// ranking became noise. The unit test that missed it fed a full `### C1:` document, a shape this
/// call path never sees.
fn structure_lens(heading: &str, body: &str, n_sources: usize) -> f64 {
    if body.trim().is_empty() {
        return 0.0;
    }
    let issues = block_issues(heading, body, n_sources).len();
    (1.0 - issues as f64 / 7.0).clamp(0.0, 1.0) // 6 CONJ_FIELDS + anchor-resolves
}

/// Ceiling on the lexical novelty axis. Above this, extra "unseen bigrams" almost always means odd
/// vocabulary rather than a better idea, so rewarding it further just pays for obscure wording.
const NOV_CAP: f32 = 0.6;

/// Novelty axis. Returns 0.5 (= unknown) rather than 0.0 for text too short to judge or when nothing
/// was retrieved: scoring "unknown" as "worst" would systematically punish concise conjectures.
fn novelty_lens(text: &str, sources: &[Source]) -> f64 {
    let dense = text.chars().filter(|c| !c.is_whitespace()).count();
    if dense < 24 || sources.is_empty() {
        return 0.5;
    }
    let corpus: String =
        sources.iter().map(|s| format!("{} {}", s.title, s.excerpt)).collect::<Vec<_>>().join(" ");
    (bigram_novelty(text, &corpus) / NOV_CAP).clamp(0.0, 1.0) as f64
}

/// Score a candidate. Returns (display_lens[structure, novelty, survival], score).
///
/// ⚠️ The SCORE uses only structure + survival. Novelty is computed for the ledger's display column
/// but is DELIBERATELY OUT of the ranking: the 8-topic panel showed novelty_lens saturates at 1.00 for
/// every real candidate (a Chinese conjecture's char-bigrams almost always exceed NOV_CAP vs the source
/// corpus), so including it moved zero rankings while advertising a three-axis rigour that was fiction.
/// Structure stays IN as a floor guard — it caught the all-0.0 regression and heavily penalises text
/// that isn't a conjecture — but since it too is ~1.0 for well-formed candidates, real ranking is
/// survival-driven, which the ledger now says plainly instead of claiming "three-axis selection".
fn score_candidate(heading: &str, body: &str, sev: Severity, sources: &[Source]) -> ([f64; 3], f64) {
    let text = format!("{heading}\n{body}");
    let structure = structure_lens(heading, body, sources.len());
    let novelty = novelty_lens(&text, sources); // display only
    let survival = survival_lens(sev, has_concrete_falsifier(body));
    let s = combine_lens(&[structure, survival]);
    ([structure, novelty, survival], if s.is_finite() { s.clamp(0.0, 1.0) } else { 0.5 })
}

// ── search tree ───────────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Origin {
    Seed,
    Wide,
    Deep,
}

impl Origin {
    fn label(self) -> &'static str {
        match self {
            Origin::Seed => "初稿",
            Origin::Wide => "新分支",
            Origin::Deep => "深化",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum StopReason {
    BudgetDone,
    CallCap,
    Deadline,
    Cancelled,
    NoCandidates,
}

impl StopReason {
    fn label(self) -> &'static str {
        match self {
            StopReason::BudgetDone => "扩展预算用尽",
            StopReason::CallCap => "模型调用上限",
            StopReason::Deadline => "时间上限",
            StopReason::Cancelled => "用户取消",
            StopReason::NoCandidates => "无可扩展候选",
        }
    }
}

struct ConjNode {
    heading: String,
    body: String,
    refutation: String,
    severity: Severity,
    lens: [f64; 3],
    score: f64,
    children: Vec<usize>,
    depth: usize,
    origin: Origin,
    subtree: Arm,
    gen_arm: Arm,
}

impl Branchable for ConjNode {
    fn children(&self) -> &[usize] {
        &self.children
    }
    fn depth(&self) -> usize {
        self.depth
    }
    fn subtree(&self) -> &Arm {
        &self.subtree
    }
    fn gen_arm(&self) -> &Arm {
        &self.gen_arm
    }
}

#[derive(Clone)]
pub(super) struct ConjWinner {
    pub heading: String,
    pub body: String,
    pub refutation: String,
    pub score: f64,
    pub lens: [f64; 3],
    pub severity: Severity,
    pub origin: &'static str,
    pub depth: usize,
}

pub(super) struct ConjSearchOutcome {
    pub ran: bool,
    pub seeds: usize,
    pub wide: usize,
    pub deep: usize,
    pub failed: usize,
    pub dupes: usize,
    pub model_calls: usize,
    pub elapsed_s: u64,
    pub stop: StopReason,
    pub sev_hist: [usize; 6],
    pub random_select: bool,
    pub expansions_cfg: u32,
    pub winners: Vec<ConjWinner>,
}

impl Default for ConjSearchOutcome {
    fn default() -> Self {
        Self {
            ran: false,
            seeds: 0,
            wide: 0,
            deep: 0,
            failed: 0,
            dupes: 0,
            model_calls: 0,
            elapsed_s: 0,
            stop: StopReason::NoCandidates,
            sev_hist: [0; 6],
            random_select: false,
            expansions_cfg: 0,
            winners: Vec::new(),
        }
    }
}

pub(super) struct ConjSearchCfg {
    pub expansions: u32,
    pub max_calls: usize,
    pub max_depth: usize,
    pub top_k: usize,
    pub secs: u64,
    pub seed: u64,
    pub random_select: bool,
}

impl ConjSearchCfg {
    /// Env-tunable so the control experiment and the escape hatch need no rebuild.
    /// `AXIOM_CONJ_SEARCH_EXPANSIONS=0` disables the search entirely (falls back to the plain refuter).
    pub(super) fn from_env(goal: &str) -> Self {
        let ev = |k: &str, d: u64| std::env::var(k).ok().and_then(|s| s.parse().ok()).unwrap_or(d);
        let mut h = DefaultHasher::new();
        goal.hash(&mut h);
        Self {
            expansions: ev("AXIOM_CONJ_SEARCH_EXPANSIONS", 3).min(8) as u32,
            max_calls: 12,
            max_depth: 3,
            top_k: 3,
            secs: ev("AXIOM_CONJ_SEARCH_SECS", 480),
            seed: h.finish(),
            random_select: ev("AXIOM_CONJ_SEARCH_RANDOM", 0) == 1,
        }
    }
}

/// Near-duplicate rejection: a "new branch" that merely rewords an existing node is fake breadth.
fn is_dupe(text: &str, existing: &[ConjNode]) -> bool {
    existing.iter().any(|e| {
        let other = format!("{}\n{}", e.heading, e.body);
        bigram_novelty(text, &other) < 0.35
    })
}

/// Parse ONE generated candidate. `conjecture_blocks` alone is too strict here: it requires a `C<digits>`
/// heading, so a model answering `### C:` — which the first version of GEN_SYS literally asked for —
/// had EVERY candidate discarded as malformed. 19 of 24 expansions were thrown away before this
/// fallback existed. Generation is expensive; refusing to read a well-formed answer over its heading
/// format is the worst possible reason to waste it.
fn parse_candidate(cand: &str) -> Option<(String, String)> {
    if let Some((h, b)) = conjecture_blocks(cand).into_iter().next() {
        return Some((h, b));
    }
    // Fallback: the six fields are what actually matter. If most are present, take the first
    // heading-ish line as the claim and the rest as the body.
    let present = super::CONJ_FIELDS.iter().filter(|f| cand.contains(**f)).count();
    if present < 3 {
        return None;
    }
    let mut lines = cand.lines();
    let head = lines
        .by_ref()
        .find(|l| !l.trim().is_empty())?
        .trim_start_matches(['#', ' ', '*'])
        .trim()
        .to_string();
    let body: String = lines.collect::<Vec<_>>().join("\n");
    (!body.trim().is_empty()).then(|| (head.chars().take(60).collect(), body))
}

const GEN_SYS: &str = "你在为一份科研报告生成【推想】候选。推想不需要有现成来源,但必须够得着。\n\
    严格按这个结构回一条,不要别的:\n\
    ### C9: <一句话主张>\n\
    - **锚点**：<站在哪些已知之上,必须带 [Sn]>\n\
    - **缺口**：<哪部分确实未知——这正是价值所在,不许掩饰>\n\
    - **方法路径**：<怎么做>\n\
    - **现在就能算**：<必须从下面的【本平台配方表】里选一条,写成「用 某配方名 算 某个量」;\n\
    写平台跑不了的东西等于没写,系统会真的去执行这一格;真没有一条沾边才写「无」>\n\
    - **证伪条件**：<什么结果出现就说明它错了>\n\
    - **时间尺度**：<多久能见分晓>\n\
    禁止把推想写得像已确立事实。";

/// GEN_SYS plus the live recipe menu. The menu must reach the WRITER, not just the matcher.
fn gen_sys() -> String {
    format!("{GEN_SYS}\n\n【本平台现在就能跑的配方表】\n{}", super::derived::recipe_menu())
}

/// Run the search. Returns an outcome that is ALWAYS reportable — a search that did nothing still
/// yields `ran:false`, which the ledger surfaces rather than silently omitting.
#[allow(clippy::too_many_arguments)]
pub(super) async fn search_conjectures(
    goal: &str,
    draft: &str,
    model: &impl CodingModel,
    transcript: &str,
    sources: &[Source],
    cancel: &Cancel,
    cfg: &ConjSearchCfg,
    on_step: &mut (dyn FnMut(String, String, String) + Send),
) -> ConjSearchOutcome {
    let t0 = std::time::Instant::now();
    let deadline = t0 + std::time::Duration::from_secs(cfg.secs);
    let mut out = ConjSearchOutcome {
        random_select: cfg.random_select,
        expansions_cfg: cfg.expansions,
        ..Default::default()
    };
    let seeds = conjecture_blocks(draft);
    if seeds.is_empty() {
        return out;
    }
    out.ran = true;
    let mut rng = StdRng::seed_from_u64(cfg.seed);
    let ev: String = transcript.chars().rev().take(12000).collect::<String>().chars().rev().collect();
    let mut nodes: Vec<ConjNode> = Vec::new();

    // Root: the draft itself. Never a winner; only a branch point.
    nodes.push(ConjNode {
        heading: String::new(),
        body: String::new(),
        refutation: String::new(),
        severity: Severity::Unparsed,
        lens: [0.0; 3],
        score: 0.0,
        children: Vec::new(),
        depth: 0,
        origin: Origin::Seed,
        subtree: Arm::default(),
        gen_arm: Arm::default(),
    });

    // ── seeds: refute what the model already wrote. NOT new cost — the refuter gate spent these
    // calls anyway; the difference is that the verdict is now parsed instead of discarded.
    for (id, body) in seeds.iter().take(3) {
        if cancel.is_cancelled() {
            out.stop = StopReason::Cancelled;
            return finish(out, nodes, t0, cfg);
        }
        if is_abandoned(id, body) {
            continue;
        }
        let r = refute_one(model, &ev, id, body, sources, cancel).await;
        out.model_calls += 1;
        let sev = r.as_deref().map(refutation_severity).unwrap_or(Severity::Unparsed);
        let (lens, score) = score_candidate(id, body, sev, sources);
        commit(&mut nodes, 0, id.clone(), body.clone(), r.unwrap_or_default(), sev, lens, score, Origin::Seed);
        out.seeds += 1;
        out.sev_hist[sev.idx()] += 1;
        on_step(
            "conj_seed".into(),
            format!("{id} · {} · {:.2}", sev.label(), score),
            format!("结构 {:.2} / 新颖 {:.2} / 存活 {:.2}", lens[0], lens[1], lens[2]),
        );
    }
    if nodes.len() == 1 {
        out.stop = StopReason::NoCandidates;
        return finish(out, nodes, t0, cfg);
    }

    // ── expansion loop ────────────────────────────────────────────────────────────────────────
    out.stop = StopReason::BudgetDone;
    let dig: String = sources
        .iter()
        .take(20)
        .map(|s| format!("[S{}] {}", s.id, s.title.chars().take(60).collect::<String>()))
        .collect::<Vec<_>>()
        .join("\n");

    for _ in 0..cfg.expansions {
        if cancel.is_cancelled() {
            out.stop = StopReason::Cancelled;
            break;
        }
        if std::time::Instant::now() >= deadline {
            out.stop = StopReason::Deadline;
            break;
        }
        // Two calls per expansion (generate + refute); never start one we cannot finish.
        if out.model_calls + 2 > cfg.max_calls {
            out.stop = StopReason::CallCap;
            break;
        }

        let (target, trace) = if cfg.random_select {
            (uniform_expandable(&nodes, cfg.max_depth, &mut rng), SelectTrace { target: 0, went_deep: false, hops: Vec::new() })
        } else {
            select_node(&nodes, cfg.max_depth, &mut rng)
        };
        let deep = target != 0;
        let parent_txt = if deep {
            format!("{}\n{}", nodes[target].heading, nodes[target].body)
        } else {
            String::new()
        };
        let user = if deep {
            format!(
                "【课题】{goal}\n\n【已检索来源】\n{dig}\n\n【证据节选】\n{ev}\n\n\
                 【要深化的推想】\n{parent_txt}\n\n【它遭到的反驳】\n{}\n\n\
                 针对这条反驳改进它:要么修正主张使其能扛住,要么把缺口说得更准。不要换题目,输出一条完整推想:",
                nodes[target].refutation
            )
        } else {
            let taken: Vec<String> =
                nodes.iter().skip(1).map(|n| n.heading.chars().take(40).collect()).collect();
            format!(
                "【课题】{goal}\n\n【已检索来源】\n{dig}\n\n【证据节选】\n{ev}\n\n\
                 【已有推想(不要重复)】\n{}\n\n\
                 提出一条与上述都不同角度的新推想,大胆但要够得着:",
                taken.join("\n")
            )
        };

        let gen = model.complete(&gen_sys(), &user, cancel).await.ok().map(|s| s.trim().to_string());
        out.model_calls += 1;
        let Some(cand) = gen.filter(|s| !s.is_empty()) else {
            out.failed += 1;
            nodes[target].gen_arm.update(0.0); // generating here didn't work; discourage it
            on_step("conj_gen".into(), "候选生成失败".into(), String::new());
            continue;
        };
        let (h, b) = match parse_candidate(&cand) {
            Some(hb) => hb,
            None => {
                out.failed += 1;
                nodes[target].gen_arm.update(0.0);
                on_step("conj_gen".into(), "候选不成推想块".into(), cand.chars().take(80).collect());
                continue;
            }
        };
        let text = format!("{h}\n{b}");
        if is_dupe(&text, &nodes[1..]) {
            out.dupes += 1;
            nodes[target].gen_arm.update(0.0); // fake breadth is a failed expansion
            on_step("conj_dupe".into(), format!("{h} 与已有候选近似,丢弃"), String::new());
            continue;
        }

        let r = refute_one(model, &ev, &h, &b, sources, cancel).await;
        out.model_calls += 1;
        let sev = r.as_deref().map(refutation_severity).unwrap_or(Severity::Unparsed);
        let (lens, score) = score_candidate(&h, &b, sev, sources);
        let origin = if deep { Origin::Deep } else { Origin::Wide };
        commit(&mut nodes, target, h.clone(), b, r.unwrap_or_default(), sev, lens, score, origin);
        if deep {
            out.deep += 1;
        } else {
            out.wide += 1;
        }
        out.sev_hist[sev.idx()] += 1;
        let hops = trace.hops.len();
        on_step(
            "conj_expand".into(),
            format!("{} {h} · {} · {:.2}", origin.label(), sev.label(), score),
            format!(
                "结构 {:.2} / 新颖 {:.2} / 存活 {:.2}(下降 {hops} 跳{})",
                lens[0], lens[1], lens[2],
                if cfg.random_select { ",随机对照" } else { "" }
            ),
        );
    }
    finish(out, nodes, t0, cfg)
}

fn uniform_expandable(nodes: &[ConjNode], max_depth: usize, rng: &mut StdRng) -> usize {
    let cands: Vec<usize> =
        (0..nodes.len()).filter(|&i| nodes[i].depth < max_depth).collect();
    if cands.is_empty() {
        return 0;
    }
    cands[rng.gen_range(0..cands.len())]
}

/// Attach a scored candidate and backpropagate. The asymmetry is deliberate and mirrors `search.rs`:
/// `gen_arm` credits ONLY the node we generated at (did generating HERE pay off?), while `subtree`
/// propagates to the root (is this whole branch worth descending into?). ⚠️ Do not merge them.
#[allow(clippy::too_many_arguments)]
fn commit(
    nodes: &mut Vec<ConjNode>,
    parent: usize,
    heading: String,
    body: String,
    refutation: String,
    severity: Severity,
    lens: [f64; 3],
    score: f64,
    origin: Origin,
) {
    let depth = nodes[parent].depth + 1;
    nodes.push(ConjNode {
        heading,
        body,
        refutation,
        severity,
        lens,
        score,
        children: Vec::new(),
        depth,
        origin,
        subtree: Arm::default(),
        gen_arm: Arm::default(),
    });
    let idx = nodes.len() - 1;
    nodes[parent].children.push(idx);
    nodes[parent].gen_arm.update(score);
    let mut cur = idx;
    loop {
        nodes[cur].subtree.update(score);
        if cur == 0 {
            break;
        }
        // walk up via the first parent that lists `cur` as a child
        match (0..nodes.len()).find(|&i| nodes[i].children.contains(&cur)) {
            Some(p) => cur = p,
            None => break,
        }
    }
}

fn finish(
    mut out: ConjSearchOutcome,
    nodes: Vec<ConjNode>,
    t0: std::time::Instant,
    cfg: &ConjSearchCfg,
) -> ConjSearchOutcome {
    out.elapsed_s = t0.elapsed().as_secs();
    let mut ranked: Vec<&ConjNode> = nodes.iter().skip(1).collect();
    ranked.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    out.winners = ranked
        .into_iter()
        .take(cfg.top_k)
        .map(|n| ConjWinner {
            heading: n.heading.clone(),
            body: n.body.clone(),
            refutation: n.refutation.clone(),
            score: n.score,
            lens: n.lens,
            severity: n.severity,
            origin: n.origin.label(),
            depth: n.depth,
        })
        .collect();
    out
}

// ── ledger ────────────────────────────────────────────────────────────────────────────────────────

/// Overlap of a winner's content words with the report body, used only to warn that the top-ranked
/// conjecture never made it into the prose. Heuristic by construction, and labelled as such.
fn adopted(w: &ConjWinner, summary: &str) -> bool {
    let toks: Vec<String> = w
        .heading
        .chars()
        .filter(|c| !c.is_whitespace() && !c.is_ascii_punctuation())
        .collect::<Vec<_>>()
        .windows(3)
        .map(|w| w.iter().collect())
        .collect();
    if toks.len() < 3 {
        return true; // too short to judge — don't cry wolf
    }
    let hit = toks.iter().filter(|t| summary.contains(*t)).count();
    hit * 3 >= toks.len()
}

/// Write the search's own audit trail into the report. This is the whole point of the ASK → EXECUTE
/// spectrum: the harness does not ask the model to report what the search found, it writes it in.
/// A search that never ran produces a VISIBLE notice, never silence.
pub(super) fn append_conjecture_search(summary: &str, oc: Option<&ConjSearchOutcome>, research: bool) -> String {
    if !research || summary.contains("## 推想搜索台账") {
        return summary.to_string();
    }
    let has_conj = !conjecture_blocks(summary).is_empty();
    let Some(oc) = oc else {
        return if has_conj {
            format!("{summary}\n\n---\n\n## 推想搜索台账\n\n⚠️ **本次未执行推想搜索**(本轮预算/配置下未触发)。下方推想只经过了结构校验,未经过对抗裁决与候选比较。\n")
        } else {
            summary.to_string()
        };
    };
    if !oc.ran {
        return format!("{summary}\n\n---\n\n## 推想搜索台账\n\n⚠️ **推想搜索启动但无可用初稿**(报告中未解析到推想块),因此没有候选被裁决。\n");
    }

    let mut out = String::from("\n\n---\n\n## 推想搜索台账\n\n");
    out.push_str(
        "本节由系统自动写入,记录推想是怎么被挑出来的。评分全部来自机械信号(结构完整度 / 词汇新颖度 / 抗反驳存活度),\
         **没有任何一步是问模型「这个好不好」**。\n\n",
    );
    // The single most important honesty clause: at small budgets this is a tournament, not a search.
    if oc.expansions_cfg < 6 {
        out.push_str(&format!(
            "> ⚠️ **本次扩展预算为 {} 次,每个分支只有 0-2 个观测**,此时 Beta 后验与均匀分布几乎无差别 —— \
             即分支选择接近随机。本次实际起作用的是「多候选 + 机械排序」,请勿理解为预算被智能分配。\n\n",
            oc.expansions_cfg
        ));
    }
    if oc.random_select {
        out.push_str("> 🔬 本次运行开启了 **均匀随机对照**(AXIOM_CONJ_SEARCH_RANDOM=1),分支不由 Thompson 采样决定。\n\n");
    }
    out.push_str(&format!(
        "**过程**:初稿 {} 条 → 新分支 {} 条 / 深化 {} 条;丢弃重复 {} 条、生成失败 {} 条;\
         模型调用 {} 次,耗时 {}s,终止原因:{}。\n\n",
        oc.seeds, oc.wide, oc.deep, oc.dupes, oc.failed, oc.model_calls, oc.elapsed_s, oc.stop.label()
    ));
    let names = ["致命", "严重", "轻微", "无法反驳", "证据不足以反驳", "裁决未能解析"];
    let hist: Vec<String> = names
        .iter()
        .zip(oc.sev_hist.iter())
        .filter(|(_, c)| **c > 0)
        .map(|(n, c)| format!("{n}×{c}"))
        .collect();
    let total: usize = oc.sev_hist.iter().sum();
    out.push_str(&format!("**反驳裁决分布**:{}", if hist.is_empty() { "无".into() } else { hist.join(" · ") }));
    if total > 0 && oc.sev_hist[5] * 2 >= total {
        out.push_str(&format!(
            " ← ⚠️ 半数以上裁决无法解析({}/{}),本次存活度评分基本无判别力,排序结果请谨慎对待",
            oc.sev_hist[5], total
        ));
    }
    out.push_str("\n\n");

    if oc.winners.is_empty() {
        out.push_str("**结果**:没有候选通过裁决。\n");
        return format!("{summary}{out}");
    }
    // Name what actually ranks. The 8-topic panel proved structure+novelty saturate at 1.00 for every
    // real candidate, so ranking is survival-driven; claiming "three-axis selection" was fiction.
    out.push_str("**排名**(实际排序由「抗反驳存活度」决定;结构分只作地板守卫;新颖分仅供参考、不参与排序):\n\n");
    out.push_str("| # | 推想 | 来源 | 反驳裁决 | 结构 | 新颖(仅参考) | 存活 | 总分 |\n|---|---|---|---|---|---|---|---|\n");
    for (i, w) in oc.winners.iter().enumerate() {
        out.push_str(&format!(
            "| {} | {} | {}(深度 {}) | {} | {:.2} | {:.2} | {:.2} | **{:.2}** |\n",
            i + 1,
            w.heading.chars().take(46).collect::<String>().replace('|', "/"),
            w.origin, w.depth, w.severity.label(), w.lens[0], w.lens[1], w.lens[2], w.score
        ));
    }
    out.push('\n');
    let top = &oc.winners[0];
    // Two very different "the champion didn't win by surviving an attack" cases, told apart:
    //  · Unparsed — the verdict could not be read (broken/unknown). The ranking really is untrustworthy.
    //  · Insufficient — the refuter found NO current evidence to refute it. For a novel, testable
    //    hypothesis that is the point, not a defect; framing it as "untrustworthy" undersells the exact
    //    output this product exists to produce. Reframe it honestly as an exploratory frontier claim.
    match top.severity {
        Severity::Unparsed => out.push_str(
            "> ⚠️ **排名第一的推想其反驳裁决无法解析**,本次排序不可信 —— 冠军是靠中性分顶上来的,\
             不是靠扛住了攻击。请把下方候选都当作「未经有效裁决」看待。\n\n",
        ),
        Severity::Insufficient => out.push_str(
            "> 🔬 **排名第一的推想是一条新颖的探索性假说**:反驳者用现有已检索证据【既不能证实也不能证伪】它\
             (而非它站不住)。这正是本平台想要的产出——够得着的新方向,但请按【待前瞻验证的假说】看待,\
             不是已确立结论。它的证伪条件写在推想里,就是验证它的下一步。\n\n",
        ),
        _ => {}
    }
    if !adopted(top, summary) {
        out.push_str(&format!(
            "> ⚠️ **正文未采用排名第一的推想**(启发式判断,可能误判)。冠军原文:\n>\n> **{}**\n>\n> {}\n\n",
            top.heading,
            top.body.lines().take(8).collect::<Vec<_>>().join("\n> ")
        ));
    }
    out.push_str(&format!(
        "**排名第一的推想遭到的最强反驳**({}):\n\n> {}\n",
        top.severity.label(),
        top.refutation.lines().take(6).collect::<Vec<_>>().join("\n> ")
    ));
    format!("{summary}{out}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src(t: &str, e: &str) -> Source {
        Source { id: 1, kind: "paper".into(), title: t.into(), url: String::new(), excerpt: e.into() }
    }

    #[test]
    fn severity_strips_its_own_label() {
        // The label CONTAINS 「致命」. A naive contains() on the whole line marks every verdict Fatal —
        // the single easiest way to silently kill every candidate.
        assert_eq!(refutation_severity("致命程度: 无法反驳"), Severity::Unrefuted);
        assert_eq!(refutation_severity("致命程度：轻微"), Severity::Minor);
        assert_eq!(refutation_severity("- **致命程度**：致命"), Severity::Fatal);
    }

    #[test]
    fn severity_rejects_template_echo() {
        // ADVERSARIAL: GLM echoing the template back must NOT be read as a verdict. A parser that
        // returns Fatal here would kill every candidate in every run where the model gets lazy.
        let echo = "最强反驳: 略\n致命程度: 致命 | 严重 | 轻微 | 无法反驳";
        assert_eq!(refutation_severity(echo), Severity::Unparsed);
        assert_eq!(survival_lens(Severity::Unparsed, true), 0.5, "「不知道」不能当成「很好」");
    }

    #[test]
    fn severity_ignores_prose_and_needs_the_verdict_line() {
        // ADVERSARIAL (false-negative direction): prose discussing something 致命 must not override
        // the actual verdict line. Same narrow-match discipline as `is_abandoned`.
        let r = "最强反驳: 这条通路的抑制在小鼠里是致命的,该推想忽略了这点\n致命程度: 轻微";
        assert_eq!(refutation_severity(r), Severity::Minor);
        assert_eq!(refutation_severity("这条推想是致命错误"), Severity::Unparsed);
    }

    #[test]
    fn severity_splits_unrefuted_from_insufficient_evidence() {
        // Same word 「无法反驳」, opposite meanings: the conjecture held up vs we had nothing to
        // attack it with. Scoring them alike pays a premium for unfalsifiable vagueness.
        let held = "最强反驳: 找不到破绽\n依据: [S1] 的数据与之一致\n致命程度: 无法反驳";
        let thin = "最强反驳: 无从下手\n依据: 现有证据不足以反驳\n致命程度: 无法反驳";
        assert_eq!(refutation_severity(held), Severity::Unrefuted);
        assert_eq!(refutation_severity(thin), Severity::Insufficient);
        assert!(survival_lens(Severity::Unrefuted, true) > survival_lens(Severity::Insufficient, true));
    }

    #[test]
    fn structure_lens_scores_the_parsed_pair_not_a_reparsed_document() {
        // REGRESSION (found by the 8-topic panel, not by unit tests): the previous version re-parsed
        // `heading + body`, but `conjecture_blocks` already stripped the leading `#`, so re-parsing
        // found no block and returned 0.0 for EVERY candidate in EVERY run. With combine_lens's min
        // term that pinned all scores to ~0.3 and made the ranking pure noise. The old test passed a
        // full `### C1:` document — a shape this call path never receives.
        let body = "- **锚点**：[S1] 已知\n- **缺口**：未知部分\n- **方法路径**：做法\n\
                    - **现在就能算**：用 ChEMBL 算 IC50 分布\n- **证伪条件**：若 X 不成立\n- **时间尺度**：6 个月\n";
        let full = structure_lens("C1: 主张", body, 3);
        assert!(full > 0.9, "六字段齐全的候选结构分必须接近 1.0,实得 {full:.2}");
        // A candidate missing most fields scores low, and an empty one scores 0.
        assert!(structure_lens("C1: 主张", "- **锚点**：[S1] 已知\n", 3) < 0.4);
        assert_eq!(structure_lens("C1: 主张", "  ", 3), 0.0);
    }

    #[test]
    fn generated_candidates_survive_heading_format_drift() {
        // REGRESSION: GEN_SYS asked for `### C:` while conjecture_blocks demands `C<digits>`, so 19 of
        // 24 real expansions were discarded as malformed. Generation is the expensive part — refusing
        // a well-formed answer over its heading format is the worst way to waste it.
        let drifted = "### C: 某个主张\n- **锚点**：[S1]\n- **缺口**：未知\n- **方法路径**：做法\n";
        let (h, b) = parse_candidate(drifted).expect("`### C:` 形式的候选必须能被读进来");
        assert!(h.contains("某个主张"));
        assert!(b.contains("锚点"));
        // The canonical form still parses through the normal path…
        assert!(parse_candidate("### C2: 主张\n- **锚点**：[S1]\n- **缺口**：未知\n- **方法路径**：X\n").is_some());
        // …but genuine garbage is still rejected: the fallback needs real fields, not any prose.
        assert!(parse_candidate("这是一段完全没有结构的散文,模型跑题了。").is_none());
        assert!(parse_candidate("").is_none());
    }

    #[test]
    fn gen_template_matches_the_parser() {
        // The template and the parser must never drift apart again — assert on the REAL constant.
        let head = GEN_SYS.lines().find(|l| l.contains("###")).expect("模板必须给出标题格式");
        let sample = format!("{}\n- **锚点**：[S1]\n- **缺口**：未知\n- **方法路径**：X\n",
            head.trim().replace("<一句话主张>", "某主张"));
        assert!(parse_candidate(&sample).is_some(), "GEN_SYS 要求的格式必须能被 parse_candidate 读出来: {sample}");
    }

    #[test]
    fn novelty_lens_treats_short_text_as_unknown_not_worst() {
        let s = vec![src("衰老细胞", "衰老细胞上调 BCL-2 抗凋亡蛋白")];
        assert_eq!(novelty_lens("太短", &s), 0.5, "短文本是「未知」不是「最差」");
        assert_eq!(novelty_lens("一段足够长的推想正文用来测试新颖度轴的行为表现如何", &[]), 0.5, "无来源时无从判断");
    }

    #[test]
    fn combined_score_cannot_ride_one_strong_axis() {
        let s = vec![src("衰老细胞", "衰老细胞上调 BCL-2 抗凋亡蛋白从而抵抗凋亡")];
        // Structurally perfect + novel, but the refuter判致命 → must not rank high.
        let full = "- **锚点**：[S1] 已知\n- **缺口**：未知\n- **方法路径**：做法\n\
                    - **现在就能算**：算一算\n- **证伪条件**：若 X\n- **时间尺度**：6 个月\n";
        let (_, fatal) = score_candidate("C1: 一条与语料完全不同的全新主张涉及迷走神经与菌群代谢", full, Severity::Fatal, &s);
        let (_, alive) = score_candidate("C1: 一条与语料完全不同的全新主张涉及迷走神经与菌群代谢", full, Severity::Minor, &s);
        assert!(fatal < 0.5, "被判致命的推想不能拿高分: {fatal:.2}");
        assert!(alive > fatal);
    }

    #[test]
    fn ledger_fires_when_search_never_ran() {
        // ADVERSARIAL: None must NOT mean "print nothing". Silent degradation is the failure mode
        // this whole layer exists to prevent — the reader would assume adjudication happened.
        let rep = "## 推想\n### C1: 某主张\n- **锚点**：[S1]\n";
        let out = append_conjecture_search(rep, None, true);
        assert!(out.contains("未执行推想搜索"), "搜索没跑必须写出来");
        // …but a non-research run stays untouched.
        assert_eq!(append_conjecture_search(rep, None, false), rep);
    }

    #[test]
    fn ledger_is_idempotent_and_never_drops_the_report() {
        let oc = ConjSearchOutcome { ran: true, seeds: 1, expansions_cfg: 3, ..Default::default() };
        let rep = "## 正文\n内容内容内容\n";
        let once = append_conjecture_search(rep, Some(&oc), true);
        assert!(once.starts_with(rep), "台账绝不能吃掉正文");
        assert_eq!(append_conjecture_search(&once, Some(&oc), true), once, "重复调用不得重复追加");
    }

    #[test]
    fn ledger_confesses_when_the_bandit_is_effectively_random() {
        let oc = ConjSearchOutcome { ran: true, seeds: 2, expansions_cfg: 3, ..Default::default() };
        let out = append_conjecture_search("正文", Some(&oc), true);
        assert!(out.contains("接近随机"), "小预算下必须承认分支选择接近随机,不得吹成智能分配");
    }

    #[test]
    fn novelty_is_out_of_scoring_but_kept_for_display() {
        // REGRESSION (red-team, algebraic): novelty saturates at 1.00 for real candidates, so it moved
        // ZERO rankings while advertising 3-axis rigour. Score must depend ONLY on structure+survival.
        let s = vec![src("衰老细胞", "衰老细胞上调 BCL-2 抗凋亡蛋白从而抵抗凋亡")];
        let body = "- **锚点**：[S1]\n- **缺口**：未知\n- **方法路径**：X\n- **现在就能算**：算\n- **证伪条件**：Y\n- **时间尺度**：6月\n";
        // Two candidates identical except wording novelty; same severity → identical SCORE despite
        // different novelty display values.
        let (lens_a, sc_a) = score_candidate("C1: 一个措辞极其常见的复述型主张", body, Severity::Minor, &s);
        let (lens_b, sc_b) = score_candidate("C1: 迷走神经菌群代谢跨器官轴一个生僻组合主张", body, Severity::Minor, &s);
        assert!((sc_a - sc_b).abs() < 1e-9, "新颖度不得影响总分: {sc_a} vs {sc_b}");
        // …but the novelty column is still POPULATED (display), not zeroed out.
        assert!(lens_a[1] >= 0.0 && lens_b[1] >= 0.0);
        // Fatal still tanks the score (survival IS in).
        let (_, sc_fatal) = score_candidate("C1: 主张", body, Severity::Fatal, &s);
        assert!(sc_fatal < sc_a, "存活轴必须仍然主导");
    }

    #[test]
    fn ledger_separates_broken_verdict_from_novel_frontier() {
        let mk = |sev| {
            let w = ConjWinner {
                heading: "C1: 冠军推想".into(), body: "- **锚点**：[S1]".into(),
                refutation: "最强反驳: 无\n致命程度: 无法反驳".into(),
                score: 0.5, lens: [1.0, 1.0, 0.5], severity: sev, origin: "深化", depth: 2,
            };
            let oc = ConjSearchOutcome { ran: true, seeds: 1, expansions_cfg: 3, winners: vec![w], ..Default::default() };
            append_conjecture_search("正文内容", Some(&oc), true)
        };
        // Unparsed = broken verdict → genuinely untrustworthy ranking.
        assert!(mk(Severity::Unparsed).contains("排序不可信"), "裁决无法解析的冠军必须标不可信");
        // Insufficient = novel testable frontier → must NOT be called untrustworthy; reframed as
        // an exploratory hypothesis. This is the verification-must-not-punish-novelty fix.
        let ins = mk(Severity::Insufficient);
        assert!(!ins.contains("排序不可信"), "新颖前沿推想不该被贬为排序不可信: {ins}");
        assert!(ins.contains("探索性假说"), "证据不足的冠军应重构为探索性假说: {ins}");
    }

    #[test]
    fn insufficient_with_concrete_falsifier_outranks_vague_and_serious() {
        // The core novelty fix: a novel conjecture the refuter couldn't refute for lack of evidence,
        // but which carries a CONCRETE falsifier (AUC comparison, statistic), must not be pinned to a
        // mediocre 0.5 — it is the product's sweet spot. A vague one stays low; a 严重-flawed one lower.
        let concrete = has_concrete_falsifier("- **证伪条件**：若前瞻比较中 HLA 亲和力排名的 AUC ≥ TCR 匹配度 AUC 且差异显著,则证伪");
        let vague = has_concrete_falsifier("- **证伪条件**：若机制被证明不成立则本推想错误");
        assert!(concrete && !vague, "有可测证伪条件才算 concrete: concrete={concrete} vague={vague}");
        assert!(survival_lens(Severity::Insufficient, true) > survival_lens(Severity::Insufficient, false),
            "有具体证伪条件的新颖推想,存活分必须高于空泛的");
        assert!(survival_lens(Severity::Insufficient, true) > survival_lens(Severity::Serious, true),
            "新颖可测的推想应排在有严重缺陷的推想之上");
        assert!(survival_lens(Severity::Insufficient, false) > survival_lens(Severity::Fatal, true));
        // …but a genuinely survived attack (Unrefuted/Minor) still ranks above a merely-novel one.
        assert!(survival_lens(Severity::Minor, true) > survival_lens(Severity::Insufficient, true));
    }

    #[test]
    fn ledger_flags_mostly_unparsed_verdicts() {
        let mut oc = ConjSearchOutcome { ran: true, seeds: 3, expansions_cfg: 3, ..Default::default() };
        oc.sev_hist[5] = 3; // all unparsed
        let out = append_conjecture_search("正文", Some(&oc), true);
        assert!(out.contains("无法解析"), "解析失败率高必须暴露,否则排序看起来仍然可信");
    }

    #[test]
    fn ledger_warns_when_the_winner_is_not_in_the_prose() {
        let w = ConjWinner {
            heading: "C1: 肠道菌群代谢产物经迷走神经调控衰老细胞清除".into(),
            body: "- **锚点**：[S1]".into(),
            refutation: "最强反驳: 无\n致命程度: 轻微".into(),
            score: 0.8, lens: [1.0, 0.6, 0.7], severity: Severity::Minor, origin: "新分支", depth: 1,
        };
        let oc = ConjSearchOutcome { ran: true, seeds: 1, expansions_cfg: 3, winners: vec![w], ..Default::default() };
        let out = append_conjecture_search("## 正文\n完全没有提到那条推想的内容。", Some(&oc), true);
        assert!(out.contains("正文未采用"), "冠军没进正文必须提示");
    }

    #[test]
    fn dupe_detection_rejects_rewordings() {
        let node = ConjNode {
            heading: "C1: 肠道菌群代谢产物经迷走神经调控衰老细胞的免疫清除效率".into(),
            body: "- **锚点**：[S1] 已知内容".into(),
            refutation: String::new(), severity: Severity::Minor, lens: [0.0; 3], score: 0.5,
            children: vec![], depth: 1, origin: Origin::Seed,
            subtree: Arm::default(), gen_arm: Arm::default(),
        };
        let nodes = vec![node];
        // ADVERSARIAL: a near-reword is fake breadth and must be rejected…
        assert!(is_dupe("C2: 肠道菌群代谢产物经迷走神经调控衰老细胞的免疫清除效率\n- **锚点**：[S1] 已知内容", &nodes));
        // …while a genuinely different angle passes.
        assert!(!is_dupe("C2: BCL-xL PROTAC 的血小板毒性可通过组织选择性递送规避\n- **锚点**：[S2] 临床前数据", &nodes));
    }
}
