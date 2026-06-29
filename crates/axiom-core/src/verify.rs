//! Deterministic verification engine — the open-core heart (SSOT tiered verification).
//!
//! This is the DETERMINISTIC tier that runs on EVERY claim with no model and no
//! external corpus. It extracts atomic claims from a draft and flags risks:
//! over-claims, uncited quantitative assertions, absolute quantifiers, and
//! internal numeric mismatches. It is bilingual (Latin + CJK) by design.
//!
//! Honest by construction: without evidence grounding the deterministic tier emits
//! ONLY [`VerdictStatus::Uncertain`] — never [`VerdictStatus::Verified`] (that
//! needs grounding) and never [`VerdictStatus::Refuted`] (a value mismatch can be
//! valid under different conditions; only evidence can refute). Every finding is a
//! risk to resolve, with a stable machine-coded reason. The LLM fleet + literature
//! cross-check (which unlock evidence-based `Verified`/`Refuted` through the
//! `Contradiction` seam) layer on top later via the same [`Checker`] trait — this
//! module is the skeleton, not the whole engine.
//!
//! Purity: extraction stamps claims with `new_id()`/`now_ms()`, but every
//! [`Checker`] is pure (same claim + context → same findings), so the checkers and
//! [`adjudicate`] are fully unit-testable without a clock or I/O.

use crate::model::{new_id, now_ms, Claim, ClaimType, VerdictStatus};

/// Whether a finding, on its own, contradicts the claim or merely flags a risk
/// that external verification must resolve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingKind {
    /// Deterministically contradicts the claim → Refuted. Reserved for
    /// evidence-based checkers; the lexical/numeric tier never emits this.
    Contradiction,
    /// A risk external verification must resolve → claim stays Uncertain.
    Risk,
}

/// One observation a checker makes about a claim.
#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    /// Stable checker code (also the i18n key root), e.g. `"overclaim"`.
    pub checker: &'static str,
    pub kind: FindingKind,
    /// Human-readable detail; the UI may re-render from `checker` + `excerpt`.
    pub message: String,
    /// The exact substring that triggered it, for UI highlighting.
    pub excerpt: Option<String>,
}

/// Read-only context a checker sees: the whole draft + every sibling claim, so a
/// checker can reason across claims (e.g. internal consistency).
pub struct DraftContext<'a> {
    pub draft: &'a str,
    pub claims: &'a [Claim],
}

/// A deterministic checker. Pure: same input → same findings (no clock, no I/O).
pub trait Checker {
    fn code(&self) -> &'static str;
    fn check(&self, claim: &Claim, ctx: &DraftContext) -> Vec<Finding>;
}

/// The per-claim result of a verification pass.
#[derive(Debug, Clone)]
pub struct ClaimReport {
    pub claim: Claim,
    pub findings: Vec<Finding>,
    pub status: VerdictStatus,
    /// Stable machine code(s) (never prose) — e.g. `"overclaim,uncited_quantitative"`
    /// or `"clean_pending"`. The UI localizes from these.
    pub reason: Option<String>,
}

// ---------------------------------------------------------------------------
// Claim extraction
// ---------------------------------------------------------------------------

/// Split a draft into atomic candidate claims (deterministic sentence split +
/// heuristic typing). Model-based extraction is a later upgrade behind this same
/// return type.
pub fn extract_claims(project_id: &str, draft: &str) -> Vec<Claim> {
    split_sentences(draft)
        .into_iter()
        .filter(|s| is_claim_like(s))
        .map(|text| Claim {
            id: new_id(),
            project_id: project_id.to_string(),
            claim_type: classify(&text),
            is_bearing: is_bearing(&text),
            text,
            output_locale: None,
            created_at: now_ms(),
        })
        .collect()
}

/// Sentence terminators in both Latin and CJK scripts, plus newlines. A `.`
/// between two digits is a decimal point, not a terminator (so `0.45`, `94.5%`
/// stay intact). Abbreviations like `Fig.`/`et al.` are a known limitation that a
/// model-based segmenter will later subsume.
fn split_sentences(draft: &str) -> Vec<String> {
    let chars: Vec<char> = draft.chars().collect();
    let mut out = Vec::new();
    let mut cur = String::new();
    for i in 0..chars.len() {
        let ch = chars[i];
        let is_term = matches!(ch, '.' | '!' | '?' | '。' | '！' | '？' | ';' | '；' | '\n');
        let is_decimal = ch == '.'
            && i > 0
            && chars[i - 1].is_ascii_digit()
            && i + 1 < chars.len()
            && chars[i + 1].is_ascii_digit();
        if is_term && !is_decimal {
            let t = cur.trim();
            if !t.is_empty() {
                out.push(t.to_string());
            }
            cur.clear();
        } else {
            cur.push(ch);
        }
    }
    let t = cur.trim();
    if !t.is_empty() {
        out.push(t.to_string());
    }
    out
}

/// A fragment is claim-like if it has enough substance to be worth judging.
/// Script-aware: CJK is morpheme-dense, so a 3-character CJK-only phrase can be a
/// full claim, while Latin text needs ~6 alphanumerics.
fn is_claim_like(s: &str) -> bool {
    let alnum = s.chars().filter(|c| c.is_alphanumeric()).count();
    let cjk = s.chars().filter(|c| is_cjk(*c)).count();
    if cjk > 0 && cjk == alnum {
        cjk >= 3
    } else {
        alnum >= 6
    }
}

fn classify(text: &str) -> ClaimType {
    if has_citation_marker(text) {
        ClaimType::Citation
    } else if !numeric_mentions(text).is_empty() {
        ClaimType::NumericStat
    } else if contains_any(text, CAUSAL_TERMS) {
        ClaimType::Causal
    } else if contains_any(text, METHOD_TERMS) {
        ClaimType::Methodological
    } else if contains_any(text, DEFINITION_TERMS) {
        ClaimType::Definitional
    } else {
        ClaimType::Descriptive
    }
}

/// A bearing (conclusion) claim triggers the LLM adversarial fleet later.
fn is_bearing(text: &str) -> bool {
    contains_any(text, BEARING_TERMS) || matches!(classify(text), ClaimType::Causal)
}

// ---------------------------------------------------------------------------
// Checkers
// ---------------------------------------------------------------------------

/// The default deterministic checker set.
pub fn default_checkers() -> Vec<Box<dyn Checker>> {
    vec![
        Box::new(OverclaimChecker),
        Box::new(AbsoluteQuantifierChecker),
        Box::new(UncitedQuantitativeChecker),
        Box::new(InternalNumericConflictChecker),
    ]
}

/// Novelty / superiority language that requires external support.
pub struct OverclaimChecker;
const OVERCLAIM_TERMS: &[&str] = &[
    "首次", "首个", "第一个", "唯一", "前所未有", "突破性", "最佳", "最优",
    "unprecedented", "first-ever", "novel approach", "state-of-the-art",
    "groundbreaking", "superior to all",
];
impl Checker for OverclaimChecker {
    fn code(&self) -> &'static str {
        "overclaim"
    }
    fn check(&self, claim: &Claim, _ctx: &DraftContext) -> Vec<Finding> {
        flag_terms(&claim.text, OVERCLAIM_TERMS, "overclaim", |t| {
            format!("novelty/superiority claim \"{t}\" needs external support")
        })
    }
}

/// Universal/absolute quantifiers that are easy to overstate.
pub struct AbsoluteQuantifierChecker;
const ABSOLUTE_TERMS: &[&str] = &[
    "所有", "全部", "总是", "从不", "绝不", "任何情况", "100%", "完全消除",
    "in all cases", "always", "never fails", "every single", "none of",
    "completely eliminat",
];
impl Checker for AbsoluteQuantifierChecker {
    fn code(&self) -> &'static str {
        "absolute_quantifier"
    }
    fn check(&self, claim: &Claim, _ctx: &DraftContext) -> Vec<Finding> {
        flag_terms(&claim.text, ABSOLUTE_TERMS, "absolute_quantifier", |t| {
            format!("absolute quantifier \"{t}\" is rarely literally true")
        })
    }
}

/// A quantitative claim (a value with a recognised unit) and no citation marker.
pub struct UncitedQuantitativeChecker;
impl Checker for UncitedQuantitativeChecker {
    fn code(&self) -> &'static str {
        "uncited_quantitative"
    }
    fn check(&self, claim: &Claim, _ctx: &DraftContext) -> Vec<Finding> {
        // Gate on a real measurement (value + unit), so a bare year like "2021"
        // is not mistaken for an uncited statistic.
        if !numeric_mentions(&claim.text).is_empty() && !has_citation_marker(&claim.text) {
            vec![Finding {
                checker: "uncited_quantitative",
                kind: FindingKind::Risk,
                message: "quantitative claim has no citation".to_string(),
                excerpt: None,
            }]
        } else {
            vec![]
        }
    }
}

/// Two claims sharing a topic keyword but stating different values for the same
/// unit. This is a RISK flag, not a refutation: different values can be perfectly
/// valid under different conditions, so the honest deterministic move is "verify
/// these conditions differ", never "one is false". Conservative: requires a shared
/// non-stopword keyword AND an identical (canonicalised) unit. To emit exactly one
/// finding per conflicting pair, only the later claim *in document order* reports.
pub struct InternalNumericConflictChecker;
impl Checker for InternalNumericConflictChecker {
    fn code(&self) -> &'static str {
        "internal_conflict"
    }
    fn check(&self, claim: &Claim, ctx: &DraftContext) -> Vec<Finding> {
        let mine = numeric_mentions(&claim.text);
        if mine.is_empty() {
            return vec![];
        }
        let my_idx = match ctx.claims.iter().position(|c| c.id == claim.id) {
            Some(i) => i,
            None => return vec![],
        };
        let my_keys = keywords(&claim.text);
        for other in &ctx.claims[..my_idx] {
            if !shares_keyword(&my_keys, &keywords(&other.text)) {
                continue;
            }
            for (v1, u1) in &mine {
                for (v2, u2) in numeric_mentions(&other.text) {
                    if *u1 == u2 && (v1 - v2).abs() > f64::EPSILON {
                        return vec![Finding {
                            checker: "internal_conflict",
                            kind: FindingKind::Risk,
                            message: format!(
                                "value {v1}{u1} differs from {v2}{u2} stated earlier for a shared topic — confirm the conditions differ"
                            ),
                            excerpt: Some(format!("{v1}{u1}")),
                        }];
                    }
                }
            }
        }
        vec![]
    }
}

// ---------------------------------------------------------------------------
// Adjudication + orchestration
// ---------------------------------------------------------------------------

/// Combine a claim's findings into the 5-state verdict + a stable machine-coded
/// reason. Deterministic tier: a Contradiction (reserved for future evidence-based
/// checkers) → Refuted; any risk flag → Uncertain (reason = the checker codes); a
/// clean claim → Uncertain (`clean_pending`). Never Verified here.
pub fn adjudicate(findings: &[Finding]) -> (VerdictStatus, Option<String>) {
    let contradictions = codes_of(findings, FindingKind::Contradiction);
    if !contradictions.is_empty() {
        return (VerdictStatus::Refuted, Some(contradictions.join(",")));
    }
    if !findings.is_empty() {
        let mut all = findings.iter().map(|f| f.checker).collect::<Vec<_>>();
        all.sort_unstable();
        all.dedup();
        (VerdictStatus::Uncertain, Some(all.join(",")))
    } else {
        (VerdictStatus::Uncertain, Some("clean_pending".to_string()))
    }
}

fn codes_of(findings: &[Finding], kind: FindingKind) -> Vec<&'static str> {
    let mut v: Vec<&'static str> = findings
        .iter()
        .filter(|f| f.kind == kind)
        .map(|f| f.checker)
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// Run the full deterministic pass over a draft.
pub fn verify_draft(project_id: &str, draft: &str) -> Vec<ClaimReport> {
    let claims = extract_claims(project_id, draft);
    let ctx = DraftContext { draft, claims: &claims };
    let checkers = default_checkers();
    claims
        .iter()
        .map(|claim| {
            let findings: Vec<Finding> =
                checkers.iter().flat_map(|c| c.check(claim, &ctx)).collect();
            let (status, reason) = adjudicate(&findings);
            ClaimReport { claim: claim.clone(), findings, status, reason }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Lexical / numeric helpers (regex-free, to keep axiom-core lean)
// ---------------------------------------------------------------------------

const CAUSAL_TERMS: &[&str] = &["导致", "因为", "因此", "使得", "leads to", "because", "causes", "results in", "due to"];
const METHOD_TERMS: &[&str] = &["方法", "采用", "我们使用", "measured", "synthesized", "we used", "protocol"];
const DEFINITION_TERMS: &[&str] = &["定义为", "是指", "is defined as", "refers to", "we define"];
const BEARING_TERMS: &[&str] = &["因此", "综上", "首次", "表明", "证明", "we show", "we demonstrate", "we conclude", "our results show"];
/// Negations that, appearing just before a flagged term, defuse it ("not always").
const NEGATIONS: &[&str] = &["not ", " no ", "n't", "neither", "without", "cannot", "不", "没", "无", "非", "未"];
/// Generic words too common to signal a shared TOPIC between two claims.
const STOPWORDS: &[&str] = &[
    "result", "results", "method", "methods", "study", "studies", "data", "value",
    "values", "analysis", "experiment", "experiments", "using", "based", "observed",
    "研究", "方法", "结果", "数据", "实验", "分析", "表明", "结论",
];

fn contains_any(text: &str, terms: &[&str]) -> bool {
    let low = text.to_lowercase();
    terms.iter().any(|t| low.contains(&t.to_lowercase()))
}

/// Flag each term present in `text` (case-insensitive), unless a negation sits
/// just before it (so "does not always" is not flagged as an absolute claim).
fn flag_terms(
    text: &str,
    terms: &[&str],
    code: &'static str,
    describe: impl Fn(&str) -> String,
) -> Vec<Finding> {
    let low = text.to_lowercase();
    terms
        .iter()
        .filter_map(|t| {
            let pos = low.find(&t.to_lowercase())?;
            if negated_before(&low, pos) {
                return None;
            }
            Some(Finding {
                checker: code,
                kind: FindingKind::Risk,
                message: describe(t),
                excerpt: Some(t.to_string()),
            })
        })
        .collect()
}

/// Is there a negation within the ~12 chars before byte offset `pos`? `pos` comes
/// from `str::find`, so it is always a valid char boundary.
fn negated_before(low: &str, pos: usize) -> bool {
    let before = &low[..pos];
    let mut tail: Vec<char> = before.chars().rev().take(12).collect();
    tail.reverse();
    let tail: String = tail.into_iter().collect();
    NEGATIONS.iter().any(|n| tail.contains(n))
}

/// A citation marker: a bracketed number (`[12]`, `〔12〕`, `【12】`, `（12）`),
/// "et al", "doi", "ref.", or a parenthesized 4-digit year.
fn has_citation_marker(s: &str) -> bool {
    let low = s.to_lowercase();
    if low.contains("et al") || low.contains("doi") || low.contains("ref.") {
        return true;
    }
    let chars: Vec<char> = s.chars().collect();
    for w in chars.windows(2) {
        if matches!(w[0], '[' | '〔' | '【' | '（') && w[1].is_ascii_digit() {
            return true;
        }
    }
    (s.contains('(') || s.contains('（')) && contains_year(s)
}

/// Any 19xx / 20xx token.
fn contains_year(s: &str) -> bool {
    let c: Vec<char> = s.chars().collect();
    c.windows(4).any(|w| {
        ((w[0] == '1' && w[1] == '9') || (w[0] == '2' && w[1] == '0'))
            && w[2].is_ascii_digit()
            && w[3].is_ascii_digit()
    })
}

/// Numeric mentions as `(value, canonical_unit)`. Only numbers followed by a
/// RECOGNISED unit are returned, so a following ordinary word (e.g. the "and" after
/// a year) is not mistaken for a unit. Handles decimals, comma thousands
/// separators, and a leading sign (ASCII `-` or Unicode `−`).
fn numeric_mentions(s: &str) -> Vec<(f64, String)> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let neg = matches!(chars[i], '-' | '−')
            && i + 1 < chars.len()
            && chars[i + 1].is_ascii_digit();
        let dstart = if neg { i + 1 } else { i };
        if dstart < chars.len() && chars[dstart].is_ascii_digit() {
            let mut j = dstart;
            while j < chars.len() && (chars[j].is_ascii_digit() || chars[j] == '.' || chars[j] == ',') {
                j += 1;
            }
            // Trim a trailing '.'/',' that is punctuation, not part of the number.
            let mut end = j;
            while end > dstart && matches!(chars[end - 1], '.' | ',') {
                end -= 1;
            }
            let raw: String = chars[i..end]
                .iter()
                .filter(|c| **c != ',')
                .map(|c| if *c == '−' { '-' } else { *c })
                .collect();
            let mut k = end;
            if k < chars.len() && chars[k] == ' ' {
                k += 1;
            }
            if let (Ok(v), Some((u, next))) = (raw.parse::<f64>(), read_unit(&chars, k)) {
                out.push((v, u));
                i = next;
            } else {
                i = j.max(i + 1);
            }
            continue;
        }
        i += 1;
    }
    out
}

/// Read a RECOGNISED unit token at `pos` (`%`, `°C`, a known Latin unit word, or a
/// known CJK unit), returning the canonical unit and the index past it. Returns
/// `None` if the token at `pos` is not a recognised unit.
fn read_unit(chars: &[char], pos: usize) -> Option<(String, usize)> {
    if pos >= chars.len() {
        return None;
    }
    if chars[pos] == '%' {
        return normalize_unit("%").map(|u| (u, pos + 1));
    }
    if chars[pos] == '°' && pos + 1 < chars.len() && chars[pos + 1].eq_ignore_ascii_case(&'C') {
        return normalize_unit("°c").map(|u| (u, pos + 2));
    }
    // Latin unit word (read up to 12 letters, then require it be recognised).
    let mut k = pos;
    while k < chars.len() && chars[k].is_ascii_alphabetic() && k - pos < 12 {
        k += 1;
    }
    if k > pos {
        let tok: String = chars[pos..k].iter().collect();
        return normalize_unit(&tok).map(|u| (u, k));
    }
    // CJK unit (try 3, then 2, then 1 chars).
    let mut m = (pos + 3).min(chars.len());
    while m > pos {
        if chars[pos..m].iter().all(|c| is_cjk(*c)) {
            let tok: String = chars[pos..m].iter().collect();
            if let Some(u) = normalize_unit(&tok) {
                return Some((u, m));
            }
        }
        m -= 1;
    }
    None
}

/// Canonicalise a unit token, or `None` if it is not a recognised measurement
/// unit. Only true synonyms collapse (percent/% ; kelvin/k); different SCALES stay
/// distinct (mg ≠ kg) so equal numbers with different scales are not seen as equal.
fn normalize_unit(raw: &str) -> Option<String> {
    let u = raw.to_lowercase();
    match u.as_str() {
        "%" | "percent" | "percentage" | "pct" => Some("%".to_string()),
        "kelvin" => Some("k".to_string()),
        "celsius" => Some("°c".to_string()),
        "k" | "°c" | "mol" | "mmol" | "umol" | "nmol" | "g" | "mg" | "kg" | "ng"
        | "ug" | "da" | "kda" | "m" | "cm" | "mm" | "nm" | "um" | "km" | "l" | "ml"
        | "ul" | "s" | "ms" | "min" | "h" | "hr" | "hz" | "khz" | "mhz" | "ghz"
        | "v" | "mv" | "kv" | "ev" | "kev" | "mev" | "pa" | "kpa" | "mpa" | "bar"
        | "atm" | "j" | "kj" | "w" | "kw" | "ppm" | "ppb" | "rpm" | "bp" | "kb"
        | "db" => Some(u),
        "摄氏度" | "度" => Some("°c".to_string()),
        "毫升" => Some("ml".to_string()),
        "升" => Some("l".to_string()),
        "毫克" => Some("mg".to_string()),
        "千克" => Some("kg".to_string()),
        "克" => Some("g".to_string()),
        "米" => Some("m".to_string()),
        "秒" => Some("s".to_string()),
        _ => None,
    }
}

/// Content keywords for topic overlap: Latin words ≥4 chars and maximal CJK runs
/// ≥2 chars, minus generic [`STOPWORDS`]. Conservative on purpose — fewer spurious
/// "shared topic" matches.
fn keywords(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut latin = String::new();
    let mut cjk = String::new();
    let push = |w: String, out: &mut Vec<String>| {
        if !STOPWORDS.contains(&w.as_str()) {
            out.push(w);
        }
    };
    let flush_latin = |latin: &mut String, out: &mut Vec<String>| {
        if latin.chars().count() >= 4 {
            push(latin.to_lowercase(), out);
        }
        latin.clear();
    };
    let flush_cjk = |cjk: &mut String, out: &mut Vec<String>| {
        if cjk.chars().count() >= 2 {
            push(cjk.clone(), out);
        }
        cjk.clear();
    };
    for ch in s.chars() {
        if ch.is_ascii_alphabetic() {
            flush_cjk(&mut cjk, &mut out);
            latin.push(ch);
        } else if is_cjk(ch) {
            flush_latin(&mut latin, &mut out);
            cjk.push(ch);
        } else {
            flush_latin(&mut latin, &mut out);
            flush_cjk(&mut cjk, &mut out);
        }
    }
    flush_latin(&mut latin, &mut out);
    flush_cjk(&mut cjk, &mut out);
    out
}

fn is_cjk(ch: char) -> bool {
    matches!(ch as u32, 0x4E00..=0x9FFF)
}

fn shares_keyword(a: &[String], b: &[String]) -> bool {
    a.iter().any(|x| b.contains(x))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_of<'a>(draft: &'a str, claims: &'a [Claim]) -> DraftContext<'a> {
        DraftContext { draft, claims }
    }

    #[test]
    fn extracts_and_filters_sentences() {
        let draft = "We synthesized a Pd-Cu catalyst. ok. The efficiency reached 94%.";
        let claims = extract_claims("p", draft);
        assert_eq!(claims.len(), 2, "the 2-char 'ok' is dropped");
        assert!(claims.iter().any(|c| c.text.contains("Pd-Cu")));
    }

    #[test]
    fn decimal_points_do_not_split_sentences() {
        let claims = extract_claims("p", "The rate constant was 0.45 per second.");
        assert_eq!(claims.len(), 1);
        assert!(claims[0].text.contains("0.45"), "decimal kept intact: {:?}", claims[0].text);
    }

    #[test]
    fn cjk_short_claims_are_extracted() {
        let claims = extract_claims("p", "已证实。结论正确。");
        assert_eq!(claims.len(), 2, "3-4 char CJK claims survive extraction");
    }

    #[test]
    fn overclaim_is_flagged() {
        let claims = extract_claims("p", "This is the first-ever room-temperature synthesis");
        let ctx = ctx_of("", &claims);
        let f = OverclaimChecker.check(&claims[0], &ctx);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].kind, FindingKind::Risk);
    }

    #[test]
    fn absolute_quantifier_is_flagged_but_negation_is_not() {
        let yes = extract_claims("p", "The reaction always succeeds");
        let no = extract_claims("p", "The reaction does not always succeed");
        let cy = ctx_of("", &yes);
        let cn = ctx_of("", &no);
        assert_eq!(AbsoluteQuantifierChecker.check(&yes[0], &cy).len(), 1);
        assert_eq!(
            AbsoluteQuantifierChecker.check(&no[0], &cn).len(),
            0,
            "a negated absolute is not flagged"
        );
    }

    #[test]
    fn uncited_number_flagged_but_cited_number_is_not() {
        let uncited = extract_claims("p", "The yield was 73 percent");
        let cited = extract_claims("p", "The yield was 73 percent [12]");
        let cited_year = extract_claims("p", "The yield was 73 percent (Smith 2021)");
        let cjk_cited = extract_claims("p", "产率为 73% 〔12〕");
        assert_eq!(UncitedQuantitativeChecker.check(&uncited[0], &ctx_of("", &uncited)).len(), 1);
        assert_eq!(UncitedQuantitativeChecker.check(&cited[0], &ctx_of("", &cited)).len(), 0);
        assert_eq!(UncitedQuantitativeChecker.check(&cited_year[0], &ctx_of("", &cited_year)).len(), 0);
        assert_eq!(
            UncitedQuantitativeChecker.check(&cjk_cited[0], &ctx_of("", &cjk_cited)).len(),
            0,
            "CJK bracket citation 〔n〕 suppresses the flag"
        );
    }

    #[test]
    fn bare_year_is_not_flagged_as_uncited() {
        for d in ["We began this study in 2021", "We studied this in 2021 and found progress"] {
            let claims = extract_claims("p", d);
            let ctx = ctx_of("", &claims);
            assert_eq!(
                UncitedQuantitativeChecker.check(&claims[0], &ctx).len(),
                0,
                "a bare year is not an uncited statistic: {d:?}"
            );
        }
    }

    #[test]
    fn comma_thousands_parse_to_the_same_value() {
        // 1,000 and 1000 are equal → no spurious conflict.
        let draft = "Sample had 1,000 mol. Sample had 1000 mol.";
        let reports = verify_draft("p", draft);
        assert!(
            reports.iter().all(|r| r.findings.iter().all(|f| f.checker != "internal_conflict")),
            "1,000 must parse as 1000"
        );
    }

    #[test]
    fn percent_synonym_unifies_with_symbol() {
        // "73 percent" and "94%" share unit → a conflict flag (not a refutation).
        let draft = "Yield was 73 percent. Yield was 94%.";
        let reports = verify_draft("p", draft);
        let flagged = reports
            .iter()
            .filter(|r| r.findings.iter().any(|f| f.checker == "internal_conflict"))
            .count();
        assert_eq!(flagged, 1);
    }

    #[test]
    fn internal_numeric_conflict_flags_but_never_refutes() {
        let draft = "The faradaic efficiency reached 94%. The faradaic efficiency reached 80%.";
        let reports = verify_draft("p", draft);
        assert_eq!(reports.len(), 2);
        let flagged = reports
            .iter()
            .filter(|r| r.findings.iter().any(|f| f.checker == "internal_conflict"))
            .count();
        assert_eq!(flagged, 1, "exactly one side carries the conflict flag");
        assert!(reports.iter().all(|r| r.status != VerdictStatus::Refuted));
    }

    #[test]
    fn unrelated_numbers_do_not_conflict() {
        let draft = "The temperature was 80 K. The pressure was 80 bar.";
        let reports = verify_draft("p", draft);
        assert!(reports
            .iter()
            .all(|r| r.findings.iter().all(|f| f.checker != "internal_conflict")));
    }

    #[test]
    fn adjudicate_maps_findings_to_status_with_coded_reasons() {
        let contradiction = vec![Finding {
            checker: "evidence_contradiction",
            kind: FindingKind::Contradiction,
            message: "x".into(),
            excerpt: None,
        }];
        let risk = vec![
            Finding { checker: "overclaim", kind: FindingKind::Risk, message: "x".into(), excerpt: None },
            Finding { checker: "absolute_quantifier", kind: FindingKind::Risk, message: "y".into(), excerpt: None },
        ];
        assert_eq!(adjudicate(&contradiction), (VerdictStatus::Refuted, Some("evidence_contradiction".into())));
        assert_eq!(adjudicate(&risk), (VerdictStatus::Uncertain, Some("absolute_quantifier,overclaim".into())));
        assert_eq!(adjudicate(&[]), (VerdictStatus::Uncertain, Some("clean_pending".into())));
    }

    #[test]
    fn deterministic_tier_emits_only_uncertain() {
        let drafts = [
            "The first-ever yield was 94%. The yield was 80%.",
            "We measured 300 K [3]. The sky is blue.",
        ];
        for d in drafts {
            for r in verify_draft("p", d) {
                assert_eq!(
                    r.status,
                    VerdictStatus::Uncertain,
                    "deterministic tier emits only Uncertain, got {:?} for {:?}",
                    r.status,
                    r.claim.text
                );
            }
        }
    }

    #[test]
    fn pass_is_deterministic() {
        let draft = "The first-ever yield was 94%. The yield was 80%.";
        let a = verify_draft("p", draft);
        let b = verify_draft("p", draft);
        let sa: Vec<_> = a.iter().map(|r| (r.findings.clone(), r.status, r.reason.clone())).collect();
        let sb: Vec<_> = b.iter().map(|r| (r.findings.clone(), r.status, r.reason.clone())).collect();
        assert_eq!(sa, sb, "findings + status + reason are stable across runs");
    }
}
