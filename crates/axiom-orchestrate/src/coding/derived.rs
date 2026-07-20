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
            RerunStatus::Match => "复算一致 ✅".into(),
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
        name: "trial_landscape",
        slots: &["query"],
        what: "某方向注册临床试验的期别分布、入组规模分布、有多少条已公布结果",
        volatile: true,
        build: |m| format!(
            "_ax_t = scidb.clinical_trials(\"{}\", limit=100)\n\
             _ax_n = [int(t[\"enrollment\"]) for t in _ax_t if str(t.get(\"enrollment\", \"\")).isdigit()]\n\
             _ax_ph = {{}}\n\
             for _ax_x in _ax_t:\n\
             \x20   _ax_k = _ax_x.get(\"phase\") or \"未标注\"\n\
             \x20   _ax_ph[_ax_k] = _ax_ph.get(_ax_k, 0) + 1\n\
             _ax_res = sum(1 for _ax_x in _ax_t if _ax_x.get(\"has_results\"))\n\
             if _ax_t:\n\
             \x20   scidb.derive(\"{} 方向的注册试验格局\", {{\"trials\": len(_ax_t), \"with_results\": _ax_res, \"median_enrollment\": statistics.median(_ax_n) if _ax_n else None, \"phase_counts\": _ax_ph}}, inputs={{\"query\": \"{}\", \"limit\": 100}}, method=\"ClinicalTrials.gov 检索结果的聚合统计\")\n\
             else:\n\
             \x20   print(\"RECIPE_NO_DATA: 该查询未返回任何注册试验\")\n",
            g(m, "query"), g(m, "query"), g(m, "query")),
    },
    Recipe {
        name: "lit_trend",
        slots: &["query"],
        what: "某主题文献的年度产出曲线与近三年占比,用于判断这个方向是新兴还是已饱和",
        volatile: true,
        build: |m| format!(
            "_ax_p = scidb.lit_search(\"{}\", limit=100)\n\
             _ax_y = {{}}\n\
             for _ax_x in _ax_p:\n\
             \x20   _ax_k = str(_ax_x.get(\"year\") or \"\")[:4]\n\
             \x20   if _ax_k.isdigit():\n\
             \x20       _ax_y[_ax_k] = _ax_y.get(_ax_k, 0) + 1\n\
             if _ax_y:\n\
             \x20   _ax_ys = sorted(_ax_y)\n\
             \x20   _ax_recent = sum(_ax_y[k] for k in _ax_ys if int(k) >= 2022)\n\
             \x20   scidb.derive(\"{} 的文献年度分布\", {{\"papers\": sum(_ax_y.values()), \"by_year\": _ax_y, \"since_2022_share\": round(_ax_recent/max(1,sum(_ax_y.values())), 3)}}, inputs={{\"query\": \"{}\", \"limit\": 100}}, method=\"Europe PMC 检索命中的发表年份计数\")\n\
             else:\n\
             \x20   print(\"RECIPE_NO_DATA: 未取到带年份的文献\")\n",
            g(m, "query"), g(m, "query"), g(m, "query")),
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
            (!v.contains('无') || v.chars().count() > 6).then_some((id, v))
        })
        .collect()
}

// ── slot execution ────────────────────────────────────────────────────────────────────────────────

pub(super) struct DerivePlan {
    pub recipe: &'static Recipe,
    pub slots: std::collections::BTreeMap<String, String>,
    pub conj_id: String,
}

/// ONE tightly-constrained model call: pick a recipe and fill its slots. The model writes no code and
/// makes no judgement — that is the model layer's legitimate job (generate a candidate), and every
/// value it returns passes `sanitize_slot` before touching a cell.
///
/// Returns `None` when nothing matches. There is deliberately NO fallback recipe: computing "median
/// trial enrollment" for a structural-biology conjecture and shipping it with a blake3 fingerprint
/// reads MORE rigorous than saying nothing, while being irrelevant. The honest output is the shortfall
/// banner.
pub(super) async fn plan_derivation(
    goal: &str,
    claims: &[(String, String)],
    model: &impl CodingModel,
    cancel: &Cancel,
) -> Result<DerivePlan, String> {
    let menu: Vec<String> = RECIPES
        .iter()
        .map(|r| format!("- {} (槽位: {}) —— {}", r.name, r.slots.join(", "), r.what))
        .collect();
    let sys = format!(
        "你要从下面这张【固定配方表】里挑一条,让平台去真算一个数。你不写代码、不做判断,只填槽位。\n{}\n\n\
         只回一个 JSON,不要别的:{{\"recipe\":\"<表中的name>\",\"conj\":\"<C几>\",\"slots\":{{...}}}}\n\
         若没有任何一条配方能服务这些推想,回 {{\"recipe\":\"none\"}} —— 宁可不算,也不要挑个不相干的配方凑数。\n\
         槽位值必须是 ASCII 标识符(如 CHEMBL4523582、TP53、ENSG00000141510、KRAS G12D inhibitor),不要写中文。",
        menu.join("\n")
    );
    let user = format!(
        "【课题】{goal}\n\n【各条推想声称「现在就能算」的内容】\n{}",
        claims.iter().map(|(id, v)| format!("{id}: {v}")).collect::<Vec<_>>().join("\n")
    );
    if cancel.is_cancelled() {
        return Err("已取消".into());
    }
    let r = model.complete(&sys, &user, cancel).await.map_err(|e| e.to_string())?;
    let js = r
        .find('{')
        .and_then(|a| r.rfind('}').map(|b| &r[a..=b]))
        .ok_or_else(|| "模型未返回 JSON".to_string())?;
    let v: Value = serde_json::from_str(js).map_err(|e| format!("JSON 解析失败:{e}"))?;
    let name = v.get("recipe").and_then(|x| x.as_str()).unwrap_or("none");
    if name == "none" {
        return Err("模型判断没有配方能服务这些推想".into());
    }
    // Exact match only — a fuzzy match would let a hallucinated name silently become a real cell.
    let recipe = RECIPES
        .iter()
        .find(|r| r.name == name)
        .ok_or_else(|| format!("模型给出的配方名 `{name}` 不在配方表中"))?;
    let mut slots = std::collections::BTreeMap::new();
    for s in recipe.slots {
        let raw = v.get("slots").and_then(|x| x.get(s)).and_then(|x| x.as_str()).unwrap_or("");
        let clean = sanitize_slot(raw).ok_or_else(|| {
            format!(
                "槽位 `{s}` 的值「{}」无法接受(实体名含非 ASCII 字符或为空,配方槽位只接受 ASCII 标识符)",
                raw.chars().take(40).collect::<String>()
            )
        })?;
        slots.insert(s.to_string(), clean);
    }
    Ok(DerivePlan {
        recipe,
        slots,
        conj_id: v.get("conj").and_then(|x| x.as_str()).unwrap_or("").to_string(),
    })
}

// ── ledger ────────────────────────────────────────────────────────────────────────────────────────

/// Write every derived result into the report. Harness-written, because registration is not use: the
/// measured escalation says the model leaves registered facts uncited about half the time.
pub(super) fn append_derived_results(summary: &str, derived: &[Derived]) -> String {
    if derived.is_empty() || summary.contains("## 本平台自算结果") {
        return summary.to_string();
    }
    let mut out = String::from("\n\n---\n\n## 本平台自算结果(DERIVED)\n\n");
    out.push_str(
        "下列数值由本平台在本次运行中**直接计算得到**,不是检索来的、也不是估计的;每条附完整代码与三段指纹,可原样复跑。\n\n\
         > ⚠️ **指纹证明的是「可复现」,不是「正确」**:一段把 IC50 当 nM(实为 µM)的代码,三段哈希全对、复算一致,\
         数值仍然错一千倍。且数据源为公开库的实时快照,复跑结果可能随上游更新而变化。\n\n",
    );
    for d in derived.iter().filter(|d| d.ok) {
        out.push_str(&format!(
            "### [S{}] {}{}\n\n- **结果**:`{}`{}\n- **方法**:{}\n- **输入**:`{}`\n- **配方**:`{}`\n\
             - **指纹(blake3)**:代码 `{}` · 输入 `{}` · 输出 `{}`\n- **复算状态**:{}\n\n```python\n{}```\n\n",
            d.source_id,
            d.label,
            if d.conj_id.is_empty() { String::new() } else { format!(" — 对应 {}", d.conj_id) },
            canon(&d.value),
            if d.unit.is_empty() { String::new() } else { format!(" {}", d.unit) },
            d.method,
            canon(&d.inputs),
            d.recipe,
            d.code_hash,
            d.input_hash,
            d.output_hash,
            d.rerun.label(d.volatile),
            d.code
        ));
    }
    for d in derived.iter().filter(|d| !d.ok) {
        out.push_str(&format!(
            "> ⚠️ 平台尝试自算但未成功{}:{}\n\n",
            if d.recipe.is_empty() { String::new() } else { format!("(配方 `{}`)", d.recipe) },
            d.error.as_deref().unwrap_or("未知原因")
        ));
    }
    format!("{summary}{out}")
}

/// Per-conjecture fulfilment banner. Deliberately NOT suppressed by one success: a report where 1 of 5
/// "现在就能算" claims was actually computed must not read as if all five were.
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
    if unmet.is_empty() {
        return summary.to_string();
    }
    format!(
        "{summary}\n\n> ⚠️ **「现在就能算」尚未兑现**:{} 条推想声称现在就能算,其中 {} 条本次并未真的算出来({})。\
         这些是**声称**,不是已完成的计算 —— 请勿把它们当作已有结果读。\n",
        claims.len(),
        unmet.len(),
        unmet.join("、")
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
        let mut m = std::collections::BTreeMap::new();
        m.insert("query".to_string(), sanitize_slot("KRAS G12D\"); evil()").unwrap());
        let cell = build_cell(&RECIPES[1], &m);
        assert!(!cell.contains("evil()") || !cell.contains("\"); evil()"), "注入片段不得原样进入 cell");
        assert!(cell.contains("scidb.derive("), "配方必须调用 derive 登记");
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
        // ADVERSARIAL: one honoured claim must NOT silence the other four.
        let rep = "### C1: 甲\n- **现在就能算**：用 ChEMBL 算效价分布\n\
                   ### C2: 乙\n- **现在就能算**：用 GWAS 算关联\n";
        let d = vec![Derived { conj_id: "C1: 甲".into(), ok: true, ..Default::default() }];
        let out = append_fulfillment_banner(rep, &d, true);
        assert!(out.contains("尚未兑现"), "只兑现一条不能压掉警告: {out}");
    }

    #[test]
    fn no_fallback_fabrication_when_nothing_matches() {
        // ADVERSARIAL, core: when no recipe applies the system must produce a banner, never a real but
        // irrelevant number wearing a fingerprint.
        let rep = "### C1: 甲\n- **现在就能算**：用某种方法算\n";
        let out = append_derived_results(rep, &[]);
        assert_eq!(out, rep, "无自算结果时不得凭空生成台账");
        assert!(append_fulfillment_banner(rep, &[], true).contains("尚未兑现"));
    }

    #[test]
    fn ledger_prints_rerun_status_verbatim_and_is_idempotent() {
        let d = vec![Derived {
            source_id: 1, label: "x".into(), value: serde_json::json!(1), ok: true,
            rerun: RerunStatus::ContextDependent, code: "c\n".into(), ..Default::default()
        }];
        let once = append_derived_results("正文", &d);
        assert!(once.contains("无法独立复算"), "复算状态必须逐字印出,不能只印 ✅");
        assert!(once.contains("不是「正确」"), "免责必须在导语里");
        assert_eq!(append_derived_results(&once, &d), once, "重复调用不得重复追加");
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
    fn computable_claims_skips_abandoned_and_empty() {
        let rep = "### C1: 甲\n- **现在就能算**：用 ChEMBL 算效价\n\
                   ### C2（已放弃）: 乙\n- **现在就能算**：用 GWAS 算\n- **反驳**：锚点不成立\n\
                   ### C3: 丙\n- **现在就能算**：无\n";
        let c = computable_claims(rep);
        assert_eq!(c.len(), 1, "已放弃与「无」都不该进兑现清单: {c:?}");
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
