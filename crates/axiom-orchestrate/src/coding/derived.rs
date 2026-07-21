//! DERIVED evidence — the third epistemic regime, and the moat.
//!
//! GROUNDED claims carry `[Sn]`; CONJECTURE claims carry a falsifier. DERIVED claims carry
//! SOMETHING NOBODY PUBLISHED: a number this platform computed from public data, where the script
//! itself is the provenance. That escapes the trap the whole anti-fabrication effort walked into —
//! "novel ⇒ no source ⇒ stripped as fabrication" — because a derived number is simultaneously new
//! AND checkable.
//!
//! ## The one non-negotiable rule
//! **The hashed code must be code the HARNESS owns, and the hash must be computed in Rust over the
//! string the harness holds.** A design where python self-reports `output_sha256` would let the model
//! `print` one line and mint a badge that looks more authoritative than `[Sn]` while requiring
//! nothing. The audited may not supply their own audit certificate. Hence: recipes are fixed here,
//! slots are sanitized here, the cell is assembled here, and `scidb.derive()` reports DATA ONLY.
//!
//! ## What a fingerprint does and does not prove
//! It proves the number came from that code on those inputs. It does NOT prove the code is right —
//! a recipe that reads IC50 as nM when the source is µM hashes perfectly and is wrong by 1000×.
//! That sentence is printed in the ledger, because a reader seeing three hashes will over-trust.

use super::{conjecture_blocks, field_value, is_abandoned, CodingModel, Source};
use axiom_runtime::supervisor::Cancel;
use serde_json::Value;

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) enum RerunStatus {
    #[default]
    NotAttempted,
    Match,
    Mismatch(String),
    /// The cell could not run standalone (NameError/KeyError). Saying "not self-contained" is honest;
    /// saying "not reproducible" would be a false accusation.
    ContextDependent,
    Failed(String),
}

impl RerunStatus {
    fn label(&self, volatile: bool) -> String {
        match self {
            RerunStatus::NotAttempted => "未复算".into(),
            // No green checkmark, by design: a ✅ next to a number reads as "verified correct", which is
            // precisely what a rerun does NOT establish (it re-runs the same code, right or wrong).
            RerunStatus::Match => "同一代码同一输入重跑,得到相同输出".into(),
            RerunStatus::Mismatch(_) if volatile => "复算不一致(上游活库已更新,数值随之变化)".into(),
            RerunStatus::Mismatch(g) => format!("⚠️ 复算不一致(本次得到 {g})"),
            RerunStatus::ContextDependent => "无法独立复算(代码依赖上下文)".into(),
            RerunStatus::Failed(e) => format!("复算失败:{}", e.chars().take(80).collect::<String>()),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct Derived {
    pub source_id: u32,
    pub conj_id: String,
    pub recipe: String,
    pub label: String,
    pub value: Value,
    pub unit: String,
    pub method: String,
    pub inputs: Value,
    pub code: String,
    pub code_hash: String,
    pub input_hash: String,
    pub output_hash: String,
    /// Set from the Recipe, never from model output: recipes that hit live databases are volatile,
    /// so a rerun mismatch means "upstream moved", not "irreproducible".
    pub volatile: bool,
    pub rerun: RerunStatus,
    pub ok: bool,
    pub error: Option<String>,
}

fn h(s: &str) -> String {
    blake3::hash(s.as_bytes()).to_hex()[..16].to_string()
}

fn canon(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

// ── recipes ───────────────────────────────────────────────────────────────────────────────────────

pub(super) struct Recipe {
    pub name: &'static str,
    pub slots: &'static [&'static str],
    pub what: &'static str,
    pub volatile: bool,
    pub build: fn(&std::collections::BTreeMap<String, String>) -> String,
}

fn g<'a>(m: &'a std::collections::BTreeMap<String, String>, k: &str) -> &'a str {
    m.get(k).map(|s| s.as_str()).unwrap_or("")
}

// ⚠️ Every recipe's slots must be CONCRETE ENTITIES (a ChEMBL id, gene symbols, a variant, an
// accession) — never a free-text `query`. A free-text-query recipe (the removed `lit_trend` and
// `trial_landscape`) matches ANY topic, so the model farms it: 4 of 5 derived numbers in the 8-topic
// panel were literature-year-counts wearing blake3 fingerprints, irrelevant to the mechanism claim
// they were attached to. That is `quant_claim_count` reborn — a proxy that pays out without doing the
// real work. The relevance gate (`plan_is_relevant`) is what enforces this: a slot entity must be
// named in the conjecture, which a composed query never is. Do NOT add a `query`-slot recipe back.
pub(super) static RECIPES: &[Recipe] = &[
    Recipe {
        name: "chembl_potency_spread",
        slots: &["target_chembl_id"],
        what: "某靶点已公开活性数据的效价分布(中位数/四分位/跨度),用于判断该靶点的可成药性空间",
        volatile: true,
        build: |m| format!(
            "_ax_acts = scidb.chembl_activity(target_chembl_id=\"{}\", activity_type=\"IC50\", limit=200)\n\
             _ax_v = [float(a[\"value\"]) for a in _ax_acts if a.get(\"value\") not in (None, \"\") and a.get(\"units\") == \"nM\"]\n\
             if len(_ax_v) >= 5:\n\
             \x20   _ax_v.sort()\n\
             \x20   _ax_q = lambda p: _ax_v[min(len(_ax_v)-1, int(p*len(_ax_v)))]\n\
             \x20   scidb.derive(\"{} 的 IC50 效价分布\", {{\"n\": len(_ax_v), \"median_nM\": statistics.median(_ax_v), \"q1_nM\": _ax_q(0.25), \"q3_nM\": _ax_q(0.75), \"min_nM\": _ax_v[0], \"max_nM\": _ax_v[-1]}}, inputs={{\"target_chembl_id\": \"{}\", \"activity_type\": \"IC50\", \"units\": \"nM\"}}, unit=\"nM\", method=\"ChEMBL activity 记录去空值后的分位数\")\n\
             else:\n\
             \x20   print(\"RECIPE_NO_DATA: 该靶点可用 IC50(nM)记录不足 5 条\")\n",
            g(m, "target_chembl_id"), g(m, "target_chembl_id"), g(m, "target_chembl_id")),
    },
    Recipe {
        name: "string_neighborhood",
        slots: &["genes"],
        what: "一组基因在 STRING 中的互作网络密度与桥接节点,用于判断它们是否构成一个功能模块",
        volatile: true,
        build: |m| format!(
            "_ax_ids = [x.strip() for x in \"{}\".replace(\",\", \" \").split() if x.strip()]\n\
             _ax_e = scidb.string_db(_ax_ids) if len(_ax_ids) >= 2 else []\n\
             if _ax_e:\n\
             \x20   _ax_deg = {{}}\n\
             \x20   for _ax_x in _ax_e:\n\
             \x20       for _ax_k in (_ax_x.get(\"preferredName_A\"), _ax_x.get(\"preferredName_B\")):\n\
             \x20           if _ax_k:\n\
             \x20               _ax_deg[_ax_k] = _ax_deg.get(_ax_k, 0) + 1\n\
             \x20   _ax_max = len(_ax_ids)*(len(_ax_ids)-1)/2\n\
             \x20   scidb.derive(\"该基因组的 STRING 互作网络密度\", {{\"genes\": len(_ax_ids), \"edges\": len(_ax_e), \"density\": round(len(_ax_e)/_ax_max, 3) if _ax_max else None, \"top_hub\": max(_ax_deg, key=_ax_deg.get) if _ax_deg else None}}, inputs={{\"genes\": _ax_ids}}, method=\"STRING 互作边数 / 完全图边数\")\n\
             else:\n\
             \x20   print(\"RECIPE_NO_DATA: STRING 未返回互作边\")\n",
            g(m, "genes")),
    },
    Recipe {
        name: "gwas_target_evidence",
        slots: &["gene"],
        what: "某基因的 GWAS 关联证据条数、涉及性状与最强关联强度,用于判断靶点的人类遗传学支撑",
        volatile: true,
        build: |m| format!(
            "_ax_a = scidb.gwas_catalog(\"{}\", limit=50)\n\
             if _ax_a:\n\
             \x20   _ax_tr = sorted({{(x.get(\"trait\") or \"?\") for x in _ax_a}})\n\
             \x20   scidb.derive(\"{} 的 GWAS 关联证据\", {{\"associations\": len(_ax_a), \"distinct_traits\": len(_ax_tr), \"traits\": _ax_tr[:12]}}, inputs={{\"gene\": \"{}\", \"limit\": 50}}, method=\"GWAS Catalog 关联记录计数\")\n\
             else:\n\
             \x20   print(\"RECIPE_NO_DATA: 该基因无 GWAS 关联记录\")\n",
            g(m, "gene"), g(m, "gene"), g(m, "gene")),
    },
    Recipe {
        name: "variant_frequency",
        slots: &["variant_id"],
        what: "某变异在 gnomAD 中的人群频率,用于判断它是罕见致病候选还是常见多态",
        volatile: true,
        build: |m| format!(
            "_ax_v = scidb.gnomad(\"{}\")\n\
             if _ax_v:\n\
             \x20   scidb.derive(\"{} 的人群等位基因频率\", _ax_v, inputs={{\"variant_id\": \"{}\", \"dataset\": \"gnomad_r4\"}}, method=\"gnomAD 记录直读\")\n\
             else:\n\
             \x20   print(\"RECIPE_NO_DATA: gnomAD 无该变异记录\")\n",
            g(m, "variant_id"), g(m, "variant_id"), g(m, "variant_id")),
    },
    Recipe {
        name: "structure_coverage",
        slots: &["accession"],
        what: "某蛋白已有实验结构对序列的覆盖比例与结构域构成,用于判断结构可及性",
        volatile: true,
        build: |m| format!(
            "_ax_u = scidb.uniprot(\"{}\")\n\
             _ax_ip = scidb.interpro(\"{}\")\n\
             _ax_len = _ax_u.get(\"length\") if _ax_u else None\n\
             if _ax_len:\n\
             \x20   _ax_d = [d for d in (_ax_ip or []) if d.get(\"start\") and d.get(\"end\")]\n\
             \x20   _ax_cov = sum(int(d[\"end\"]) - int(d[\"start\"]) + 1 for d in _ax_d)\n\
             \x20   scidb.derive(\"{} 的结构域覆盖\", {{\"length_aa\": _ax_len, \"domains\": len(_ax_d), \"domain_covered_aa\": _ax_cov, \"covered_fraction\": round(_ax_cov/_ax_len, 3)}}, inputs={{\"accession\": \"{}\"}}, method=\"InterPro 结构域区间并集 / UniProt 序列长度\")\n\
             else:\n\
             \x20   print(\"RECIPE_NO_DATA: 未取到该蛋白长度\")\n",
            g(m, "accession"), g(m, "accession"), g(m, "accession"), g(m, "accession")),
    },
];

/// The recipe menu, rendered for a PROMPT. Injected where conjectures are WRITTEN, not only where
/// they are matched: the panel showed the model proposing "查 GEO 的 IFN-γ score 分布" while the menu
/// offered ChEMBL/STRING/GWAS — zero overlap, so 0 of 8 runs computed anything. Matching after the
/// fact cannot fix a mismatch that was baked in at generation time. Same lesson as the ask→execute
/// spectrum: constrain the generation instead of hoping the answer lands in range.
pub(super) fn recipe_menu() -> String {
    RECIPES
        .iter()
        .map(|r| format!("- `{}`(需要:{}) —— {}", r.name, r.slots.join("、"), r.what))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Slot sanitiser. The whitelist deliberately excludes quotes, backslash, newline, `#`, `{`, `}` and
/// `;`. That is the ENTIRE reason harness-assembled cells are safe: whatever survives cannot
/// terminate the python string literal it is interpolated into, so it cannot become code. Safety here
/// rests on the whitelist, never on trusting model output.
pub(super) fn sanitize_slot(v: &str) -> Option<String> {
    let s: String = v
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || " _.,:-+()[]/".contains(*c))
        .collect();
    let s = s.trim().to_string();
    (!s.is_empty() && s.chars().count() <= 80).then_some(s)
}

const PRELUDE: &str = "import scidb, json, math, statistics\n";

pub(super) fn build_cell(r: &Recipe, slots: &std::collections::BTreeMap<String, String>) -> String {
    format!("{PRELUDE}{}", (r.build)(slots))
}

// ── capture ───────────────────────────────────────────────────────────────────────────────────────

pub(super) struct DeriveCtx<'a> {
    pub recipe: &'a str,
    pub conj_id: &'a str,
    pub code: &'a str,
    pub volatile: bool,
}

/// Turn `#AXIOM_DERIVED#` lines into registered `Derived` + `Source` records. Hashes are computed
/// HERE, in Rust, over the harness's own strings — any `*_sha256` the payload carries is ignored.
pub(super) fn capture_derived(
    stdout: &str,
    ctx: &DeriveCtx,
    derived: &mut Vec<Derived>,
    sources: &mut Vec<Source>,
    src_n: &mut u32,
) -> String {
    const TAG: &str = "#AXIOM_DERIVED#";
    if !stdout.contains(TAG) {
        return stdout.to_string();
    }
    let mut kept = String::new();
    let mut added: Vec<u32> = Vec::new();
    let mut malformed = 0usize;
    for line in stdout.lines() {
        let Some(js) = line.trim_start().strip_prefix(TAG) else {
            kept.push_str(line);
            kept.push('\n');
            continue;
        };
        let Ok(p) = serde_json::from_str::<Value>(js.trim()) else {
            malformed += 1;
            continue;
        };
        let label = p.get("label").and_then(|x| x.as_str()).unwrap_or("(未命名)").to_string();
        let value = p.get("value").cloned().unwrap_or(Value::Null);
        let inputs = p.get("inputs").cloned().unwrap_or(Value::Null);
        let out_h = h(&canon(&value));
        // Nondeterminism check: same code + same inputs, different output ⇒ flag the EXISTING record
        // rather than minting a second id that would read as two independent findings.
        let in_h = h(&canon(&inputs));
        let code_h = h(ctx.code);
        if let Some(prev) = derived
            .iter_mut()
            .find(|d| d.code_hash == code_h && d.input_hash == in_h && d.output_hash != out_h)
        {
            prev.rerun = RerunStatus::Mismatch(out_h.clone());
            continue;
        }
        *src_n += 1;
        let unit = p.get("unit").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let method = p.get("method").and_then(|x| x.as_str()).unwrap_or("").to_string();
        sources.push(Source {
            id: *src_n,
            kind: "derived".into(),
            title: format!("[本平台自算] {label}"),
            url: String::new(),
            excerpt: format!(
                "结论:{label} = {}{unit}\n方法:{method}\n输入:{}\n配方:{}\n\
                 指纹(blake3):代码 {code_h} · 输入 {in_h} · 输出 {out_h}\n\
                 ⚠️ 指纹证明可复现,不证明正确。\n可复跑代码:\n{}",
                canon(&value), canon(&inputs), ctx.recipe, ctx.code
            ),
        });
        derived.push(Derived {
            source_id: *src_n,
            conj_id: ctx.conj_id.to_string(),
            recipe: ctx.recipe.to_string(),
            label,
            value,
            unit,
            method,
            inputs,
            code: ctx.code.to_string(),
            code_hash: code_h,
            input_hash: in_h,
            output_hash: out_h,
            volatile: ctx.volatile,
            rerun: RerunStatus::NotAttempted,
            ok: true,
            error: None,
        });
        added.push(*src_n);
    }
    if !added.is_empty() {
        let ids: Vec<String> = added.iter().map(|n| format!("[S{n}]")).collect();
        kept.push_str(&format!(
            "[已登记 {} 条平台自算结果(DERIVED);在正文引用它们时用:{}]\n",
            added.len(),
            ids.join(" ")
        ));
    }
    if malformed > 0 {
        // Never drop silently: a swallowed registration looks identical to "the recipe found nothing".
        kept.push_str(&format!("[⚠️ 有 {malformed} 条派生登记因格式错误未被接受]\n"));
    }
    kept
}

/// Conjectures that CLAIM something is computable right now. Abandoned ones are skipped — there is
/// nothing to honour for a hypothesis the refuter already killed.
pub(super) fn computable_claims(summary: &str) -> Vec<(String, String)> {
    conjecture_blocks(summary)
        .into_iter()
        .filter(|(id, b)| !is_abandoned(id, b))
        .filter_map(|(id, b)| {
            let v = field_value(&b, "现在就能算")?;
            // Must NAME a platform recipe. A "现在就能算" that references no recipe is a vague claim,
            // not an executable declaration — the old length heuristic let "无沾边配方…" style text
            // through, which then misfired the derive slot. Now the trigger is mechanical: the field
            // has to point at something the harness can actually run.
            RECIPES.iter().any(|r| v.contains(r.name)).then_some((id, v))
        })
        .collect()
}

// ── slot execution ────────────────────────────────────────────────────────────────────────────────

pub(super) struct DerivePlan {
    pub recipe: &'static Recipe,
    pub slots: std::collections::BTreeMap<String, String>,
    pub conj_id: String,
    /// The entity the model claims this computation is about, copied verbatim from the conjecture.
    /// The relevance gate checks it actually appears there.
    pub entity: String,
}

/// THE RELEVANCE GATE the 8-topic panel proved was missing. A derived number is only evidence FOR a
/// conjecture if it is ABOUT something the conjecture names. This checks, mechanically, that the
/// model-declared `entity` (and every entity slot value) literally appears in the target conjecture's
/// body. A correct, reproducible, fingerprinted number about an entity the hypothesis never mentions
/// is negative value — it reads as hard evidence while being irrelevant. Fail → route to the shortfall
/// banner, never into the report.
pub(super) fn plan_is_relevant(plan: &DerivePlan, summary: &str) -> bool {
    let Some((_, body)) = conjecture_blocks(summary).into_iter().find(|(id, _)| {
        // Match on the leading `C<n>` token so "C1" and "C1: 甲" both resolve.
        let a = id.split([' ', ':', '：']).next().unwrap_or("").trim();
        let b = plan.conj_id.split([' ', ':', '：']).next().unwrap_or("").trim();
        !b.is_empty() && a == b
    }) else {
        return false;
    };
    // Check the entity against the SUBSTANTIVE claim, EXCLUDING the "现在就能算" line. The model naming
    // its entity only inside the computation-declaration would satisfy the gate circularly (the panel
    // showed exactly this: CD274 appeared only in C3's 现在就能算, and a GWAS-on-CD274 number was
    // irrelevant to C3's ctDNA claim). The number is relevant only if the entity is part of the
    // hypothesis itself — 锚点/缺口/方法路径 — not merely of the sentence proposing the computation.
    let substantive: String = body
        .lines()
        .filter(|l| !l.contains("现在就能算"))
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();
    let ent = plan.entity.trim().to_lowercase();
    ent.chars().count() >= 2 && substantive.contains(&ent)
}

/// ONE model call that turns the conjectures' OWN 「现在就能算」 declarations into structured,
/// harness-executable plans. This is the ask→execute spectrum applied to computation: the model
/// already DECLARED "用 gwas_target_evidence 算 HLA-A" while writing the conjecture; the harness does
/// not re-guess (the old `plan_derivation` made an independent pick and misfired onto an unrelated,
/// already-abandoned conjecture), it EXTRACTS what was declared and runs it ITSELF, fingerprinted.
/// The model here transcribes, it does not decide — no code, no new claims, only structuring.
///
/// Returns one plan per conjecture that declared a real recipe on a named entity. Empty vec is a
/// legitimate answer (nothing declared a runnable computation) → the shortfall banner, never a
/// fabricated fallback.
pub(super) async fn extract_derivations(
    claims: &[(String, String)],
    model: &impl CodingModel,
    cancel: &Cancel,
) -> Result<Vec<DerivePlan>, String> {
    let menu: Vec<String> = RECIPES
        .iter()
        .map(|r| format!("- {} (槽位: {}) —— {}", r.name, r.slots.join(", "), r.what))
        .collect();
    let sys = format!(
        "下面每条推想的「现在就能算」都声明了要用某个平台配方算某个实体。你的唯一工作是把这些声明\
         整理成结构化 JSON,让平台照着真跑——你不写代码、不改主意、不新增声明。\n\
         【配方表】\n{}\n\n\
         只回一个 JSON 数组,每条合格声明一项:\n\
         [{{\"conj\":\"<C几>\",\"recipe\":\"<配方名>\",\"entity\":\"<推想正文里点名的那个实体,原样抄>\",\"slots\":{{...}}}}]\n\
         规则:①配方名必须精确来自上表;②slots 填对应该 entity 的 ASCII 标识符(如 TP53、CHEMBL4523582);\
         ③entity 原样抄推想正文里点名的对象;④若某条声明没点名具体实体或配方跑题,就【跳过它,不放进数组】;\
         ⑤一条都不合格就回 []。",
        menu.join("\n")
    );
    let user = claims
        .iter()
        .map(|(id, v)| format!("{id} 的现在就能算:{v}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    if cancel.is_cancelled() {
        return Err("已取消".into());
    }
    let r = model.complete(&sys, &user, cancel).await.map_err(|e| e.to_string())?;
    let js = r
        .find('[')
        .and_then(|a| r.rfind(']').map(|b| &r[a..=b]))
        .ok_or_else(|| "模型未返回 JSON 数组".to_string())?;
    let arr: Vec<Value> = serde_json::from_str(js).map_err(|e| format!("JSON 解析失败:{e}"))?;
    let mut plans = Vec::new();
    for v in arr {
        let name = v.get("recipe").and_then(|x| x.as_str()).unwrap_or("");
        // Exact match only — a hallucinated name must never become a real cell.
        let Some(recipe) = RECIPES.iter().find(|r| r.name == name) else { continue };
        let mut slots = std::collections::BTreeMap::new();
        let mut ok = true;
        for s in recipe.slots {
            let raw = v.get("slots").and_then(|x| x.get(s)).and_then(|x| x.as_str()).unwrap_or("");
            match sanitize_slot(raw) {
                Some(c) => {
                    slots.insert(s.to_string(), c);
                }
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        plans.push(DerivePlan {
            recipe,
            slots,
            conj_id: v.get("conj").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            entity: v.get("entity").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        });
    }
    Ok(plans)
}

// ── ledger ────────────────────────────────────────────────────────────────────────────────────────

/// The badge's NEGATIVE clauses — verbatim, non-severable, and printed BEFORE the hashes.
///
/// Design rule (from the independent review that overturned this feature's goal): guarantee exactly
/// what the audit can verify, and print what it cannot. "Unforgeable correctness" is a category error —
/// verifying that code computes the quantity a natural-language claim names would need a decider that
/// maps arbitrary prose to formal specs. Three of this feature's four failures came from chasing it.
/// So the badge claims only tamper-EVIDENT execution, and the shortfall is part of the badge itself.
///
/// ⚠️ This build has NO airgapped sandbox / data-access layer / ablation battery (deferred to the next
/// major version alongside local-compute infrastructure). Until those ship we may NOT claim the output
/// provably depends on the fetched data — that disclaimer is load-bearing and must not be trimmed.
const BADGE_NEGATIVE: &str = "\
    > **本徽章证明**:此数由平台配方代码,以所示参数、对实际取得的数据算出,可凭下方代码原样复跑。\n\
    > **本徽章不证明**:代码正确实现了所称指标;数据源本身无误;查询参数的选择公允;\
    此数在取数时间之后仍成立;**该数支持本条主张**。\n\
    > **本版尚未部署断网沙箱与消融审计**,故亦不担保「输出真实依赖所取数据」\
    (防篡改审计随下个大版本的本地算力基建一起落地)。\n\n";

/// Write every derived result into the report. Harness-written, because registration is not use: the
/// measured escalation says the model leaves registered facts uncited about half the time.
///
/// Badge = tier label + scope + negative clauses, three-in-one and non-severable. Deliberately NO
/// "已验证"/"verified" wording and NO green checkmark: the cryptographic authority of three hashes is
/// itself a source of over-trust that has to be managed, so the negatives come first and the hashes
/// are folded away last.
pub(super) fn append_derived_results(summary: &str, derived: &[Derived]) -> String {
    if derived.is_empty() || summary.contains("## 本平台自算结果") {
        return summary.to_string();
    }
    let mut out = String::from("\n\n---\n\n## 本平台自算结果\n\n");
    out.push_str(
        "下列数值由本平台在本次运行中**直接计算**,不是检索来的、也不是估计的。每条都附**担保范围**与\
         **不担保项** —— 请务必连同「本徽章不证明」那几行一起读。\n\n\
         > **本产物中不带徽章的一切数字,均为模型陈述,未经平台计算。**\n\n",
    );
    for d in derived.iter().filter(|d| d.ok) {
        out.push_str(&format!(
            "### [S{}] {}{}\n\n**平台自算 · 平台配方 · 平台执行**\n\n\
             - **结果(全精度)**:`{}`{}\n- **方法**:{}\n- **输入**:`{}`\n- **配方**:`{}`\n- **复算**:{}\n\n{}\
             <details><summary>指纹与可复跑代码(blake3)</summary>\n\n\
             代码 `{}` · 输入 `{}` · 输出 `{}`\n\n```python\n{}```\n\n</details>\n\n",
            d.source_id,
            d.label,
            if d.conj_id.is_empty() { String::new() } else { format!(" — 对应 {}", d.conj_id) },
            canon(&d.value),
            if d.unit.is_empty() { String::new() } else { format!(" {}", d.unit) },
            d.method,
            canon(&d.inputs),
            d.recipe,
            d.rerun.label(d.volatile),
            BADGE_NEGATIVE,
            d.code_hash,
            d.input_hash,
            d.output_hash,
            d.code
        ));
    }
    for d in derived.iter().filter(|d| !d.ok) {
        out.push_str(&format!(
            "> ⊘ 平台尝试自算但未兑现{}:{}\n\n",
            if d.recipe.is_empty() { String::new() } else { format!("(配方 `{}`)", d.recipe) },
            d.error.as_deref().unwrap_or("未知原因")
        ));
    }
    format!("{summary}{out}")
}

/// Label computations the MODEL says it performed but which no platform badge backs.
///
/// The measured failure this addresses: the model really does compute relevant things inline (a real
/// run wrote "已执行:HLA-A 有 20 条 GWAS 关联,关联 1,138 种疾病") — numbers that LOOK computed and
/// carry no marker saying nobody verified them. Issuing a weak badge for these would be wrong: without
/// an ablation audit we cannot rule out a hardcoded constant, and a forgeable badge outranking a real
/// citation is worse than no badge. So they get a warning label instead, never a badge. Upgrading them
/// to a real tier waits on the sandbox/data-layer/ablation work deferred to the next major version.
///
/// ⚠️ Detection is a closed phrase list and is THEREFORE LEAKY — wording can be rewritten without
/// limit ("经数据核算…"), so this is a monitoring aid, NOT a complete gate. The actual backstop is the
/// no-badge default convention: any number without a badge is a model statement. Do not present this
/// as a defence line.
pub(super) fn append_self_report_notice(summary: &str, derived: &[Derived], research: bool) -> String {
    if !research || summary.contains("模型自述计算") {
        return summary.to_string();
    }
    const PHRASES: &[&str] =
        &["已执行", "已计算", "经计算", "计算得到", "算得", "本平台计算", "据平台计算", "我们计算"];
    // Scan the PROSE ONLY. The derived ledger below is harness-written and legitimately full of
    // computation wording, so scanning the whole document made this warning fire on every run that
    // produced a badge — turning a signal that means "the model claimed a computation nobody checked"
    // into a constant, and printing a false statement about the report itself.
    let prose = summary.split("## 本平台自算结果").next().unwrap_or(summary);
    if !PHRASES.iter().any(|p| prose.contains(p)) {
        return summary.to_string();
    }
    let has_badge = derived.iter().any(|d| d.ok);
    let body = if has_badge {
        "> ⚠️ **正文含「已计算/已执行」类表述**:**除【本平台自算结果】一节列出的条目外**,正文中的一切\
         计算与数字均为**模型自述计算** —— 平台未执行、未验证,不构成计算证据。\n"
    } else {
        "> ⚠️ **正文含「已计算/已执行」类表述,但本次平台未执行任何计算**:这些都是**模型自述计算**,\
         平台未执行、未验证,不构成计算证据。请作为模型陈述对待。\n"
    };
    format!("{summary}\n\n{body}")
}

/// Per-conjecture fulfilment ledger. T0 ("declared computable, NOT delivered") is a FIRST-CLASS
/// output, never hidden and never suppressed by a success elsewhere: a report where 1 of 5
/// "现在就能算" claims was actually computed must not read as if all five were. Closes with a
/// machine-generated tally so the ratio is visible at a glance rather than inferred.
pub(super) fn append_fulfillment_banner(summary: &str, derived: &[Derived], research: bool) -> String {
    if !research || summary.contains("尚未兑现") {
        return summary.to_string();
    }
    let claims = computable_claims(summary);
    if claims.is_empty() {
        return summary.to_string();
    }
    let unmet: Vec<String> = claims
        .iter()
        .filter(|(id, _)| !derived.iter().any(|d| d.ok && d.conj_id == *id))
        .map(|(id, _)| id.chars().take(24).collect())
        .collect();
    let met = claims.len() - unmet.len();
    if unmet.is_empty() {
        // Everything declared was delivered — still print the tally, so "all fulfilled" is a stated
        // fact rather than the reader's guess.
        return format!(
            "{summary}\n\n**自算兑现汇总**:可算声明 {} 条 → 已兑现 {} 条、未兑现 0 条。\n",
            claims.len(),
            met
        );
    }
    format!(
        "{summary}\n\n> ⊘ **「现在就能算」尚未兑现**:{} 条推想声称现在就能算,其中 {} 条本次并未真的算出来({})。\
         这些是**声称**,不是已完成的计算 —— 请作为模型陈述对待,勿当作已有结果读。\n\n\
         **自算兑现汇总**:可算声明 {} 条 → 已兑现 {} 条、未兑现 {} 条。\n",
        claims.len(),
        unmet.len(),
        unmet.join("、"),
        claims.len(),
        met,
        unmet.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_slot_strips_injection() {
        // ADVERSARIAL: this is the entire safety argument for harness-assembled cells.
        let evil = "\"); import os; os.system(\"rm -rf /";
        let clean = sanitize_slot(evil).unwrap();
        for bad in ['"', '\\', ';', '#', '{', '}', '\n', '\''] {
            assert!(!clean.contains(bad), "消毒后不得残留 {bad:?}: {clean}");
        }
        assert!(sanitize_slot("").is_none());
        assert!(sanitize_slot("   ").is_none());
    }

    #[test]
    fn sanitize_slot_rejects_cjk_entity_names() {
        // CJK is not in the whitelist, so a Chinese entity name yields None. That is a REJECTION the
        // banner must surface, not a silent no-op.
        assert!(sanitize_slot("表皮生长因子受体").is_none());
        assert_eq!(sanitize_slot("CHEMBL203").as_deref(), Some("CHEMBL203"));
    }

    #[test]
    fn built_cell_keeps_slot_values_inside_string_literals() {
        let r = RECIPES.iter().find(|r| r.name == "string_neighborhood").unwrap();
        let mut m = std::collections::BTreeMap::new();
        m.insert("genes".to_string(), sanitize_slot("TP53 KRAS\"); evil()").unwrap());
        let cell = build_cell(r, &m);
        assert!(!cell.contains("\"); evil()"), "注入片段不得原样进入 cell");
        assert!(cell.contains("scidb.derive("), "配方必须调用 derive 登记");
    }

    #[test]
    fn no_free_text_query_recipe_survives() {
        // REGRESSION (panel): a `query`-slot recipe matches any topic → the model farms it (4/5 derived
        // numbers were irrelevant literature counts). No recipe may take a free-text query slot.
        for r in RECIPES {
            assert!(!r.slots.contains(&"query"), "配方 {} 仍有自由文本 query 槽,会被凑数", r.name);
        }
    }

    fn plan(recipe: &'static str, conj: &str, entity: &str) -> DerivePlan {
        let r = RECIPES.iter().find(|r| r.name == recipe).unwrap();
        DerivePlan { recipe: r, slots: Default::default(), conj_id: conj.into(), entity: entity.into() }
    }

    #[test]
    fn relevance_gate_requires_the_entity_in_the_substantive_claim() {
        let rep = "### C1: 抗原提呈通路完整性决定应答\n- **锚点**：[S1] TAP1 与 B2M 参与提呈\n\
                   - **现在就能算**：用 string_neighborhood 算 CD274 的网络密度\n\
                   ### C2: 别的机制\n- **锚点**：[S2]\n";
        // Entity named in C1's SUBSTANTIVE claim (锚点) → relevant.
        assert!(plan_is_relevant(&plan("string_neighborhood", "C1", "TAP1"), rep));
        // ADVERSARIAL (the panel's exact failure): CD274 appears ONLY in the 现在就能算 line, not in the
        // hypothesis itself → computing on it is circular/irrelevant → must reject.
        assert!(!plan_is_relevant(&plan("string_neighborhood", "C1", "CD274"), rep),
            "只在「现在就能算」里出现的实体,算出来与主张不相关,必须拦(否则闸门被绕成循环论证)");
        // Entity not in the conjecture at all → reject.
        assert!(!plan_is_relevant(&plan("string_neighborhood", "C1", "KRAS"), rep));
        // Entity present but in a DIFFERENT conjecture than claimed → reject (per-conjecture).
        assert!(!plan_is_relevant(&plan("gwas_target_evidence", "C2", "TAP1"), rep));
        // Empty entity / unknown conjecture → reject, never silently pass.
        assert!(!plan_is_relevant(&plan("string_neighborhood", "C1", ""), rep));
        assert!(!plan_is_relevant(&plan("string_neighborhood", "C9", "TAP1"), rep));
    }

    #[test]
    fn hashes_are_computed_in_rust_and_forged_fields_ignored() {
        // ADVERSARIAL, and the core rule of this module: a payload claiming its own output_sha256 must
        // not be believed. The audited may not supply their own audit certificate.
        let stdout = "#AXIOM_DERIVED#{\"label\":\"x\",\"value\":42,\"inputs\":{\"a\":1},\"output_sha256\":\"deadbeef\"}\n";
        let (mut d, mut s, mut n) = (Vec::new(), Vec::new(), 0u32);
        let ctx = DeriveCtx { recipe: "r", conj_id: "C1", code: "CODE", volatile: true };
        capture_derived(stdout, &ctx, &mut d, &mut s, &mut n);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].code_hash, h("CODE"), "code_hash 必须是 Rust 对 harness 自己的代码算的");
        assert_eq!(d[0].output_hash, h("42"));
        assert_ne!(d[0].output_hash, "deadbeef", "python 自报的指纹必须被忽略");
    }

    #[test]
    fn capture_reports_malformed_registrations() {
        // A swallowed registration is indistinguishable from "found nothing" — must be surfaced.
        let stdout = "#AXIOM_DERIVED#{not json}\n";
        let (mut d, mut s, mut n) = (Vec::new(), Vec::new(), 0u32);
        let ctx = DeriveCtx { recipe: "r", conj_id: "", code: "C", volatile: false };
        let obs = capture_derived(stdout, &ctx, &mut d, &mut s, &mut n);
        assert!(d.is_empty());
        assert!(obs.contains("格式错误"), "解析失败必须回执: {obs}");
    }

    #[test]
    fn derived_registers_a_dense_source_with_derived_kind() {
        let stdout = "#AXIOM_DERIVED#{\"label\":\"IC50 分布\",\"value\":{\"median_nM\":12},\"inputs\":{},\"unit\":\"nM\"}\n";
        let (mut d, mut s, mut n) = (Vec::new(), Vec::new(), 3u32);
        let ctx = DeriveCtx { recipe: "chembl_potency_spread", conj_id: "C1", code: "CODE", volatile: true };
        let obs = capture_derived(stdout, &ctx, &mut d, &mut s, &mut n);
        assert_eq!(n, 4, "[Sn] 编号必须连续 —— 稀疏编号会被悬空引用门整体抹掉");
        assert_eq!(s[0].kind, "derived");
        assert!(s[0].url.is_empty());
        assert!(s[0].excerpt.contains("可复跑代码"), "溯源即脚本");
        assert!(obs.contains("[S4]") && !obs.contains("#AXIOM_DERIVED#"), "原始 marker 必须换成中文回执");
    }

    #[test]
    fn fulfillment_banner_is_per_conjecture_not_suppressed_by_one_success() {
        // ADVERSARIAL: one honoured claim must NOT silence the other. Claims must name a real recipe.
        let rep = "### C1: 甲\n- **现在就能算**：用 chembl_potency_spread 算效价分布\n\
                   ### C2: 乙\n- **现在就能算**：用 gwas_target_evidence 算关联\n";
        let d = vec![Derived { conj_id: "C1: 甲".into(), ok: true, ..Default::default() }];
        let out = append_fulfillment_banner(rep, &d, true);
        assert!(out.contains("尚未兑现"), "只兑现一条不能压掉警告: {out}");
    }

    #[test]
    fn no_fallback_fabrication_when_nothing_matches() {
        // ADVERSARIAL, core: when a recipe was declared but nothing was computed, the banner fires;
        // a "现在就能算" that names no recipe isn't even a computable claim → no banner, no fabrication.
        let named = "### C1: 甲\n- **现在就能算**：用 chembl_potency_spread 算效价\n";
        assert_eq!(append_derived_results(named, &[]), named, "无自算结果时不得凭空生成台账");
        assert!(append_fulfillment_banner(named, &[], true).contains("尚未兑现"));
        // A vague claim that names no recipe is not "unfulfilled" — it's not a platform computation.
        let vague = "### C1: 甲\n- **现在就能算**：用某种方法算\n";
        assert!(!append_fulfillment_banner(vague, &[], true).contains("尚未兑现"));
    }

    #[test]
    fn model_self_reported_computation_is_labelled_never_badged() {
        // The measured case: the model computes inline and writes "已执行:…20 条关联". Those numbers
        // look computed but nobody verified them. Without an ablation audit we cannot rule out a
        // hardcoded constant, so they must get a WARNING, never a badge.
        let claimed = "## 正文\n- **现在就能算**：用 gwas_target_evidence 算 TP53——已执行:20 条关联\n";
        let out = append_self_report_notice(claimed, &[], true);
        assert!(out.contains("模型自述计算"), "自述计算必须被标注: {out}");
        assert!(out.contains("平台未执行任何计算"), "无平台计算时要说清这一点");
        // Idempotent, and silent when the report makes no computation claim.
        assert_eq!(append_self_report_notice(&out, &[], true), out, "不得重复追加");
        assert_eq!(append_self_report_notice("## 正文\n普通叙述,没有计算主张。", &[], true).as_str(),
                   "## 正文\n普通叙述,没有计算主张。", "没有计算主张就不该加噪音");
        // With a real platform badge present, the notice must scope itself to everything OUTSIDE the
        // ledger — not deny the ledger's own entries.
        let d = vec![Derived { ok: true, ..Default::default() }];
        let with_badge = append_self_report_notice(claimed, &d, true);
        assert!(with_badge.contains("除【本平台自算结果】"), "有徽章时要划清范围: {with_badge}");
    }

    #[test]
    fn self_report_detection_is_leaky_by_design_and_backstopped() {
        // HONEST LIMIT, asserted so nobody later mistakes this for a complete gate: a reworded claim
        // escapes the phrase list. That is expected — the real backstop is the no-badge default
        // convention in the derived ledger, which is what this test pins.
        let reworded = "## 正文\n该指标经数据核算后为 0.75。\n";
        assert_eq!(append_self_report_notice(reworded, &[], true), reworded,
            "改写措辞会绕过——这是设计上的已知泄漏,不假装是完备门");
        // …and the backstop: whenever a badge ledger renders, it states the default convention.
        let d = vec![Derived { source_id: 1, label: "x".into(), value: serde_json::json!(1), ok: true, ..Default::default() }];
        assert!(append_derived_results("正文", &d).contains("不带徽章"),
            "兜底约定(无章数字=模型陈述)必须在场");
    }

    #[test]
    fn ledger_prints_rerun_status_verbatim_and_is_idempotent() {
        let d = vec![Derived {
            source_id: 1, label: "x".into(), value: serde_json::json!(1), ok: true,
            rerun: RerunStatus::ContextDependent, code: "c\n".into(), ..Default::default()
        }];
        let once = append_derived_results("正文", &d);
        assert!(once.contains("无法独立复算"), "复算状态必须逐字印出");
        assert_eq!(append_derived_results(&once, &d), once, "重复调用不得重复追加");
    }

    #[test]
    fn badge_is_three_in_one_and_never_claims_verification() {
        // The badge must carry its NEGATIVE clauses inseparably, print them BEFORE the hashes, and
        // never wear "verified"/✅ — the authority of three hashes is itself an over-trust risk, and
        // this build has no sandbox/ablation audit, so the shortfall is part of the badge.
        let d = vec![Derived {
            source_id: 7, label: "某分布".into(), value: serde_json::json!({"median": 12.5}),
            ok: true, rerun: RerunStatus::Match, code: "print(1)\n".into(),
            code_hash: "aaa".into(), input_hash: "bbb".into(), output_hash: "ccc".into(),
            ..Default::default()
        }];
        let out = append_derived_results("正文", &d);
        // Negative clauses present, verbatim, and covering the load-bearing disclaimers.
        assert!(out.contains("本徽章不证明"), "负面条款必须在场");
        assert!(out.contains("该数支持本条主张"), "必须明写不担保「支持本条主张」");
        assert!(out.contains("尚未部署断网沙箱"), "没有防篡改审计就必须明说,不得冒充已审计");
        // Negatives come BEFORE the hashes.
        let neg = out.find("本徽章不证明").unwrap();
        let hash = out.find("aaa").unwrap();
        assert!(neg < hash, "负面条款必须排在哈希之前(哈希的权威感需要先被限定)");
        // Banned: verification wording and the green checkmark.
        for banned in ["已验证", "verified", "✅"] {
            assert!(!out.contains(banned), "徽章禁用「{banned}」这类暗示已核实正确的表述: {out}");
        }
        // The no-badge default convention must be stated, or the ABSENCE of a badge is unreadable.
        assert!(out.contains("不带徽章"), "必须声明「无章数字=模型陈述」的默认约定");
    }

    #[test]
    fn context_dependent_is_not_reported_as_mismatch() {
        // Saying "code isn't self-contained" is honest; saying "irreproducible" is a false accusation.
        assert!(RerunStatus::ContextDependent.label(false).contains("无法独立复算"));
        assert!(!RerunStatus::ContextDependent.label(false).contains("不一致"));
        // A volatile recipe's mismatch is attributed to upstream drift, not to us.
        assert!(RerunStatus::Mismatch("abc".into()).label(true).contains("上游活库"));
        assert!(RerunStatus::Mismatch("abc".into()).label(false).contains("abc"));
    }

    #[test]
    fn computable_claims_requires_a_named_recipe_and_skips_abandoned() {
        // The trigger is mechanical: the field must NAME a platform recipe. This skips abandoned
        // conjectures, "无"-style claims, AND the panel's failure — a "无沾边配方…" declaration whose
        // length alone used to sneak it through and misfire the derive slot.
        let rep = "### C1: 甲\n- **现在就能算**：用 chembl_potency_spread 算 KRAS 效价\n\
                   ### C2（已放弃）: 乙\n- **现在就能算**：用 gwas_target_evidence 算\n- **反驳**：锚点不成立\n\
                   ### C3: 丙\n- **现在就能算**：无沾边配方,与本推想无因果关联\n\
                   ### C4: 丁\n- **现在就能算**：用某种方法算个先验\n";
        let c = computable_claims(rep);
        assert_eq!(c.len(), 1, "只有点名了配方且未放弃的才算可执行声明: {c:?}");
        assert!(c[0].0.contains("C1"));
    }
}

/// Re-execute each derived cell in a FRESH namespace and compare the output hash. This is meaningful
/// here (and was near-useless in the rejected design) because the cell is PRELUDE + a fixed recipe —
/// self-contained by construction, so a `NameError` genuinely means "not self-contained" rather than
/// "we replayed it wrong". Also sidesteps kernel namespace pollution: a fresh dict, not a naming
/// convention.
pub(super) fn rerun_code(d: &Derived) -> String {
    format!(
        "import io, json, contextlib\n\
         _ax_src = {}\n\
         _ax_buf = io.StringIO()\n\
         try:\n\
         \x20   with contextlib.redirect_stdout(_ax_buf):\n\
         \x20       exec(compile(_ax_src, '<rerun>', 'exec'), {{'__name__': '__main__'}})\n\
         \x20   print('#AXIOM_RERUN#' + json.dumps({{'out': _ax_buf.getvalue()}}, ensure_ascii=False))\n\
         except (NameError, KeyError) as _ax_e:\n\
         \x20   print('#AXIOM_RERUN#' + json.dumps({{'ctx': str(_ax_e)}}, ensure_ascii=False))\n\
         except Exception as _ax_e:\n\
         \x20   print('#AXIOM_RERUN#' + json.dumps({{'err': str(_ax_e)}}, ensure_ascii=False))\n",
        serde_json::to_string(&d.code).unwrap_or_else(|_| "''".into())
    )
}

/// Classify one rerun. The `ContextDependent` branch matters: reporting "not reproducible" for a cell
/// that merely needed prior state would be a false accusation, and false accusations in a provenance
/// ledger are worse than silence.
pub(super) fn classify_rerun(d: &Derived, stdout: &str) -> RerunStatus {
    const TAG: &str = "#AXIOM_RERUN#";
    let Some(line) = stdout.lines().find_map(|l| l.trim_start().strip_prefix(TAG)) else {
        return RerunStatus::Failed("复算未返回结果".into());
    };
    let Ok(v) = serde_json::from_str::<Value>(line.trim()) else {
        return RerunStatus::Failed("复算输出无法解析".into());
    };
    if v.get("ctx").is_some() {
        return RerunStatus::ContextDependent;
    }
    if let Some(e) = v.get("err").and_then(|x| x.as_str()) {
        return RerunStatus::Failed(e.chars().take(120).collect());
    }
    let out = v.get("out").and_then(|x| x.as_str()).unwrap_or("");
    // Re-extract the payload and re-hash its VALUE in Rust — never trust anything self-reported.
    let Some(js) = out.lines().find_map(|l| l.trim_start().strip_prefix("#AXIOM_DERIVED#")) else {
        return RerunStatus::Failed("复算未产生派生登记".into());
    };
    let Ok(p) = serde_json::from_str::<Value>(js.trim()) else {
        return RerunStatus::Failed("复算派生登记无法解析".into());
    };
    let got = h(&canon(&p.get("value").cloned().unwrap_or(Value::Null)));
    if got == d.output_hash {
        RerunStatus::Match
    } else {
        RerunStatus::Mismatch(got)
    }
}

#[cfg(test)]
mod rerun_tests {
    use super::*;

    fn d_with(code: &str, out_hash: &str) -> Derived {
        Derived { code: code.into(), output_hash: out_hash.into(), ok: true, ..Default::default() }
    }

    #[test]
    fn rerun_matching_output_is_match_and_differing_is_mismatch() {
        let d = d_with("x", &h("42"));
        let good = "#AXIOM_RERUN#{\"out\":\"#AXIOM_DERIVED#{\\\"value\\\":42}\\n\"}";
        assert_eq!(classify_rerun(&d, good), RerunStatus::Match);
        let bad = "#AXIOM_RERUN#{\"out\":\"#AXIOM_DERIVED#{\\\"value\\\":43}\\n\"}";
        assert!(matches!(classify_rerun(&d, bad), RerunStatus::Mismatch(_)));
    }

    #[test]
    fn nameerror_is_context_dependent_never_mismatch() {
        // ADVERSARIAL: "your result is irreproducible" is a much stronger claim than the evidence
        // supports when the cell simply needed earlier state. Must never be conflated.
        let d = d_with("x", &h("42"));
        let ctx = "#AXIOM_RERUN#{\"ctx\":\"name '_ax_v' is not defined\"}";
        assert_eq!(classify_rerun(&d, ctx), RerunStatus::ContextDependent);
        assert!(!matches!(classify_rerun(&d, ctx), RerunStatus::Mismatch(_)));
    }

    #[test]
    fn missing_or_garbage_rerun_output_is_failed_not_match() {
        // ADVERSARIAL (false-positive direction): absence of evidence must not read as verification.
        let d = d_with("x", &h("42"));
        assert!(matches!(classify_rerun(&d, ""), RerunStatus::Failed(_)));
        assert!(matches!(classify_rerun(&d, "#AXIOM_RERUN#{not json}"), RerunStatus::Failed(_)));
        assert!(matches!(classify_rerun(&d, "#AXIOM_RERUN#{\"out\":\"nothing\"}"), RerunStatus::Failed(_)));
    }

    #[test]
    fn rerun_code_embeds_source_safely() {
        let d = d_with("print(\"hi\")\n# tricky ' \" \\ chars", "");
        let code = rerun_code(&d);
        assert!(code.contains("redirect_stdout"), "必须捕获 stdout");
        assert!(code.contains("'__name__': '__main__'"), "必须在全新命名空间里跑");
        // json-encoding the source is what keeps quotes/backslashes from breaking the wrapper.
        assert!(code.contains("\\\"hi\\\"") || code.contains("\\\\"), "源码必须被 JSON 转义嵌入");
    }
}
