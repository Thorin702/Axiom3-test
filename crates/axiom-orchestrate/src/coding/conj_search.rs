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

/// Survival axis. `Unparsed`/`Insufficient` sit at 0.5: "we don't know" must not read as "it's good".
fn survival_lens(s: Severity) -> f64 {
    match s {
        Severity::Fatal => 0.0,
        Severity::Serious => 0.35,
        Severity::Minor => 0.70,
        Severity::Unrefuted => 0.85,
        Severity::Insufficient => 0.5,
        Severity::Unparsed => 0.5,
    }
}

/// Structure axis: how complete the six required fields are.
/// The empty-block special case is essential — `conjecture_issues` returns exactly ONE issue for text
/// that parses as no conjecture at all, so the naive `1 - issues/7` would award 0.86 to a candidate
/// that is not even shaped like a hypothesis. Highest score for the worst input, exactly backwards.
fn structure_lens(text: &str, n_sources: usize) -> f64 {
    let blocks = conjecture_blocks(text);
    if blocks.is_empty() {
        return 0.0;
    }
    let issues: usize = blocks.iter().map(|(id, b)| block_issues(id, b, n_sources).len()).sum();
    let slots = (blocks.len() * 7) as f64; // 6 CONJ_FIELDS + anchor-resolves
    (1.0 - issues as f64 / slots).clamp(0.0, 1.0)
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

/// Future seam: score "现在就能算" by ACTUALLY RUNNING it (piece B's kernel slot), never by keyword
/// matching — matching `ChEMBL`/`gnomAD` in the text would be name-drop-scores-full-marks, the exact
/// shape of the retired `quant_claim_count` gate.
///
/// ⚠️ Whatever implements this: the COUNT of derived results must never become a score or a gate.
/// Some recipes almost always return a number, so the moment counting is rewarded the system degrades
/// into farming the cheapest recipe.
#[allow(dead_code)]
pub(super) trait ComputabilityProbe: Send + Sync {
    fn probe(&self, body: &str) -> Option<f64>;
}

/// The three mechanical axes, combined by `search::combine_lens` (blend of mean and MIN, so a
/// candidate cannot ride one strong axis to the top).
fn score_candidate(text: &str, sev: Severity, sources: &[Source]) -> ([f64; 3], f64) {
    let lens = [structure_lens(text, sources.len()), novelty_lens(text, sources), survival_lens(sev)];
    let s = combine_lens(&lens);
    (lens, if s.is_finite() { s.clamp(0.0, 1.0) } else { 0.5 })
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

const GEN_SYS: &str = "你在为一份科研报告生成【推想】候选。推想不需要有现成来源,但必须够得着。\n\
    严格按这个结构回一条,不要别的:\n\
    ### C: <一句话主张>\n\
    - **锚点**：<站在哪些已知之上,必须带 [Sn]>\n\
    - **缺口**：<哪部分确实未知——这正是价值所在,不许掩饰>\n\
    - **方法路径**：<怎么做>\n\
    - **现在就能算**：<用公开数据/本平台内核现在就能跑的具体一步>\n\
    - **证伪条件**：<什么结果出现就说明它错了>\n\
    - **时间尺度**：<多久能见分晓>\n\
    禁止把推想写得像已确立事实。";

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
        let text = format!("{id}\n{body}");
        let r = refute_one(model, &ev, id, body, sources, cancel).await;
        out.model_calls += 1;
        let sev = r.as_deref().map(refutation_severity).unwrap_or(Severity::Unparsed);
        let (lens, score) = score_candidate(&text, sev, sources);
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

        let gen = model.complete(GEN_SYS, &user, cancel).await.ok().map(|s| s.trim().to_string());
        out.model_calls += 1;
        let Some(cand) = gen.filter(|s| !s.is_empty()) else {
            out.failed += 1;
            nodes[target].gen_arm.update(0.0); // generating here didn't work; discourage it
            on_step("conj_gen".into(), "候选生成失败".into(), String::new());
            continue;
        };
        let blocks = conjecture_blocks(&cand);
        let (h, b) = match blocks.first() {
            Some((h, b)) => (h.clone(), b.clone()),
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
        let (lens, score) = score_candidate(&text, sev, sources);
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
    out.push_str("**排名**(分数 = 三轴的均值与最小值各半,单轴强不能拉高总分):\n\n");
    out.push_str("| # | 推想 | 来源 | 反驳裁决 | 结构 | 新颖 | 存活 | 总分 |\n|---|---|---|---|---|---|---|---|\n");
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
        assert_eq!(survival_lens(Severity::Unparsed), 0.5, "「不知道」不能当成「很好」");
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
        assert!(survival_lens(Severity::Unrefuted) > survival_lens(Severity::Insufficient));
    }

    #[test]
    fn structure_lens_is_zero_for_text_that_is_not_a_conjecture() {
        // ADVERSARIAL: `conjecture_issues` returns exactly ONE issue for unparseable text, so the
        // naive 1 - issues/7 would score the WORST possible input 0.86. Highest score for garbage.
        assert_eq!(structure_lens("这是一段普通的散文,完全没有推想结构。", 3), 0.0);
        let full = "### C1: 主张\n- **锚点**：[S1] 已知\n- **缺口**：未知部分\n- **方法路径**：做法\n\
                    - **现在就能算**：用 ChEMBL 算 IC50 分布\n- **证伪条件**：若 X 不成立\n- **时间尺度**：6 个月\n";
        assert!(structure_lens(full, 3) > 0.9);
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
        let full = "### C1: 一条与语料完全不同的全新主张涉及迷走神经与菌群代谢\n- **锚点**：[S1] 已知\n\
                    - **缺口**：未知\n- **方法路径**：做法\n- **现在就能算**：算一算\n- **证伪条件**：若 X\n- **时间尺度**：6 个月\n";
        let (_, fatal) = score_candidate(full, Severity::Fatal, &s);
        let (_, alive) = score_candidate(full, Severity::Minor, &s);
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
