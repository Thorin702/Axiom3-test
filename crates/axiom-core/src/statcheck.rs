//! statcheck — a deterministic RECOMPUTE checker (open-core moat).
//!
//! Parses reported null-hypothesis tests (`t(df)=…, p=…`, `F(df1,df2)=…, p=…`,
//! `r(df)=…, p=…`) out of a claim and recomputes the p-value from the test statistic
//! and degrees of freedom ([`crate::stats`]). A mismatch between the reported and the
//! recomputed p-value is a DETERMINISTIC fact — no model, no network — and is exactly
//! the kind of self-inconsistency reviewers miss and statcheck famously surfaces.
//!
//! HONESTY: this stays a [`FindingKind::Risk`] (→ `Uncertain`), not a refutation. A
//! mismatch can have benign causes (a one-tailed test, a multiple-comparison
//! correction, a rounding/typo), so the honest output is "recompute disagrees —
//! verify", not "this is false". The deterministic tier still emits only `Uncertain`.

use crate::model::Claim;
use crate::stats::{f_upper_p, r_to_t, t_two_tailed_p};
use crate::verify::{Checker, DraftContext, Finding, FindingKind};

/// A parsed null-hypothesis test report.
#[derive(Debug, Clone, PartialEq)]
struct Nhst {
    kind: char, // 't' | 'F' | 'r'
    df1: f64,
    df2: f64, // F only; else 0
    stat: f64,
    p_op: char, // '=' | '<' | '>'
    p_reported: f64,
    p_decimals: usize, // reported precision, sets the `=`-comparison tolerance
    excerpt: String,
}

/// The deterministic statcheck checker. Registered in `verify::default_checkers`.
pub struct StatcheckChecker;

impl Checker for StatcheckChecker {
    fn code(&self) -> &'static str {
        "statcheck"
    }

    fn check(&self, claim: &Claim, _ctx: &DraftContext) -> Vec<Finding> {
        parse_nhst(&claim.text)
            .into_iter()
            .filter_map(|n| {
                let recomputed = match n.kind {
                    't' => t_two_tailed_p(n.stat, n.df1),
                    'F' => f_upper_p(n.stat, n.df1, n.df2),
                    'r' => t_two_tailed_p(r_to_t(n.stat, n.df1), n.df1),
                    _ => return None,
                };
                if !recomputed.is_finite() {
                    return None;
                }
                inconsistency(&n, recomputed).map(|message| Finding {
                    checker: "statcheck",
                    kind: FindingKind::Risk,
                    message,
                    excerpt: Some(n.excerpt.clone()),
                })
            })
            .collect()
    }
}

const ALPHA: f64 = 0.05;

/// Compare a reported test to its recomputed p-value; return a message iff they are
/// inconsistent under EVERY benign interpretation.
///
/// The recompute is two-tailed; a correctly-reported ONE-TAILED test (common in
/// directional work) reports half that. To avoid calling a correct paper wrong, the
/// reported value is checked against BOTH the two-tailed p and the one-tailed (half)
/// p (for t/r; F is intrinsically one-tailed), and is flagged only when it fits
/// NEITHER. A "decision FLIPS" message is reserved for the gross case where, even
/// two-tailed, the significance verdict at .05 changes.
fn inconsistency(n: &Nhst, p_two: f64) -> Option<String> {
    let candidates = if n.kind == 'F' { [p_two, p_two] } else { [p_two, p_two / 2.0] };
    if candidates.iter().any(|&p| satisfies(n, p)) {
        return None;
    }
    let test = describe_test(n);
    if n.p_op == '=' && decision_flips(n.p_reported, p_two) {
        Some(format!(
            "statcheck: reported p={} for {test} recomputes to p≈{p_two:.4} (two-tailed) — the significance \
             decision at .05 FLIPS; verify the statistic/df/p (not explained by a one-tailed test)",
            fmt_p(n.p_reported)
        ))
    } else {
        Some(format!(
            "statcheck: reported p{}{} for {test} recomputes to p≈{p_two:.4} (two-tailed) — inconsistent; \
             verify the statistic/df/p (e.g. a typo or a multiple-comparison correction)",
            n.p_op,
            fmt_p(n.p_reported)
        ))
    }
}

/// Is a reported p consistent with the candidate p-value `p` at the reported precision?
fn satisfies(n: &Nhst, p: f64) -> bool {
    match n.p_op {
        '=' => {
            let d = n.p_decimals.max(2);
            let factor = 10f64.powi(d as i32);
            ((p * factor).round() / factor - n.p_reported).abs() <= 1e-12
        }
        '<' => p < n.p_reported,
        '>' => p > n.p_reported,
        _ => true,
    }
}

/// Does the significance decision at α=.05 differ between reported and recomputed?
fn decision_flips(reported: f64, recomputed: f64) -> bool {
    // Ignore the boundary itself to avoid spurious flips on p == .05.
    (reported - ALPHA).abs() > 1e-9
        && (recomputed - ALPHA).abs() > 1e-9
        && (reported < ALPHA) != (recomputed < ALPHA)
}

fn describe_test(n: &Nhst) -> String {
    match n.kind {
        'F' => format!("F({},{})={}", fmt_num(n.df1), fmt_num(n.df2), fmt_num(n.stat)),
        k => format!("{k}({})={}", fmt_num(n.df1), fmt_num(n.stat)),
    }
}

fn fmt_num(x: f64) -> String {
    if x.fract() == 0.0 {
        format!("{}", x as i64)
    } else {
        format!("{x}")
    }
}

fn fmt_p(p: f64) -> String {
    let s = format!("{p}");
    s.strip_prefix('0').map(|r| r.to_string()).unwrap_or(s) // .04 not 0.04, statcheck style
}

// ---------------------------------------------------------------------------
// Parser (regex-free)
// ---------------------------------------------------------------------------

/// Plausibility ceiling on degrees of freedom: above this it isn't a real df (it's a
/// citation year, an index, or a parser glitch), and it keeps the recompute inside the
/// range where the incomplete beta is accurate.
const MAX_DF: f64 = 1.0e7;

/// Parse every reported NHST test in `text`.
fn parse_nhst(text: &str) -> Vec<Nhst> {
    let c: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        // A test symbol must stand alone: preceded by a non-ALPHANUMERIC, so neither
        // "first(" (letter before) nor "3r(" (digit before) matches.
        let prev_is_alnum = i > 0 && c[i - 1].is_alphanumeric();
        if !prev_is_alnum && matches!(c[i], 't' | 'F' | 'r') && c.get(i + 1) == Some(&'(') {
            if let Some((nhst, next)) = parse_one(&c, i) {
                out.push(nhst);
                i = next;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Parse a single test starting at the symbol index `start` (`c[start]` ∈ t/F/r).
fn parse_one(c: &[char], start: usize) -> Option<(Nhst, usize)> {
    let kind = c[start];
    let mut i = start + 2; // past "t("
    let (df1, _, after) = read_num(c, i)?;
    if !(1.0..=MAX_DF).contains(&df1) {
        return None; // not a plausible df (citation year, index, glitch)
    }
    i = after;
    let mut df2 = 0.0;
    if kind == 'F' {
        i = skip_ws(c, i);
        if c.get(i) != Some(&',') {
            return None;
        }
        i = skip_ws(c, i + 1);
        let (d2, _, after2) = read_num(c, i)?;
        if !(1.0..=MAX_DF).contains(&d2) {
            return None;
        }
        df2 = d2;
        i = after2;
    }
    i = skip_ws(c, i);
    if c.get(i) != Some(&')') {
        return None;
    }
    i = skip_ws(c, i + 1);
    if c.get(i) != Some(&'=') {
        return None;
    }
    i = skip_ws(c, i + 1);
    let (stat, _, after_stat) = read_num(c, i)?;
    // A correlation must satisfy |r| < 1.
    if kind == 'r' && stat.abs() >= 1.0 {
        return None;
    }
    i = after_stat;

    // The p-value MUST be reported ADJACENT to the statistic — only whitespace / a
    // comma / semicolon / an opening paren may sit between them. This refuses to
    // scavenge an unrelated later p across a citation or a second test (the false
    // positives a 40-char free-scan produced).
    let mut k = i;
    while matches!(c.get(k), Some(' ') | Some(',') | Some(';') | Some('(')) {
        k += 1;
    }
    if c.get(k) != Some(&'p') {
        return None;
    }
    let mut m = skip_ws(c, k + 1);
    let op = match c.get(m) {
        Some('=') => '=',
        Some('<') | Some('≤') => '<',
        Some('>') | Some('≥') => '>',
        _ => return None,
    };
    m = skip_ws(c, m + 1);
    let (p_reported, p_decimals, after_p) = read_num(c, m)?;
    let excerpt: String = c[start..after_p].iter().collect();
    Some((Nhst { kind, df1, df2, stat, p_op: op, p_reported, p_decimals, excerpt }, after_p))
}

fn skip_ws(c: &[char], mut i: usize) -> usize {
    while i < c.len() && c[i] == ' ' {
        i += 1;
    }
    i
}

/// Is there an exponent (`e`/`E` + optional sign + digit) starting at `i`?
fn is_exponent_start(c: &[char], i: usize) -> bool {
    if !matches!(c.get(i), Some('e') | Some('E')) {
        return false;
    }
    let mut j = i + 1;
    if matches!(c.get(j), Some('-') | Some('+') | Some('−')) {
        j += 1;
    }
    matches!(c.get(j), Some(d) if d.is_ascii_digit())
}

/// Read a number at `start`: optional sign, digits, optional `.` + digits, incl. a
/// leading-dot form like `.04`. Returns (value, decimal_count, index_past_it).
fn read_num(c: &[char], start: usize) -> Option<(f64, usize, usize)> {
    let mut i = start;
    let mut s = String::new();
    if matches!(c.get(i), Some('-') | Some('−') | Some('+')) {
        s.push(if c[i] == '−' { '-' } else { c[i] });
        i += 1;
    }
    let mut digits = 0;
    while i < c.len() && c[i].is_ascii_digit() {
        s.push(c[i]);
        i += 1;
        digits += 1;
    }
    let mut decimals = 0;
    if c.get(i) == Some(&'.') {
        s.push('.');
        i += 1;
        while i < c.len() && c[i].is_ascii_digit() {
            s.push(c[i]);
            i += 1;
            digits += 1;
            decimals += 1;
        }
    }
    if digits == 0 {
        return None;
    }
    // Refuse scientific notation rather than silently truncating to the mantissa
    // ("2.05e3" must not be read as 2.05). The caller then skips the test.
    if is_exponent_start(c, i) {
        return None;
    }
    // Normalise a leading dot ("-.04" / ".04") so the parse accepts it.
    let norm = if let Some(rest) = s.strip_prefix("-.") {
        format!("-0.{rest}")
    } else if let Some(rest) = s.strip_prefix('.') {
        format!("0.{rest}")
    } else {
        s
    };
    norm.parse::<f64>().ok().map(|v| (v, decimals, i))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Claim, ClaimType};

    fn claim(text: &str) -> Claim {
        Claim {
            id: "c".into(),
            project_id: "p".into(),
            text: text.into(),
            claim_type: ClaimType::NumericStat,
            is_bearing: false,
            output_locale: None,
            created_at: 0,
        }
    }

    fn flags(text: &str) -> Vec<Finding> {
        let claims = [claim(text)];
        let ctx = DraftContext { draft: text, claims: &claims };
        StatcheckChecker.check(&claims[0], &ctx)
    }

    #[test]
    fn parses_the_three_test_forms() {
        let p = parse_nhst("results: t(28) = 2.05, p = .05; F(2, 27) = 3.35, p = .049; r(48) = .50, p < .001");
        assert_eq!(p.len(), 3, "{p:#?}");
        assert_eq!(p[0].kind, 't');
        assert_eq!(p[0].df1, 28.0);
        assert_eq!(p[1].kind, 'F');
        assert_eq!((p[1].df1, p[1].df2), (2.0, 27.0));
        assert_eq!(p[2].kind, 'r');
        assert_eq!(p[2].p_op, '<');
    }

    #[test]
    fn consistent_stats_are_not_flagged() {
        // t(28)=2.05 → two-tailed p ≈ .0498 → rounds to .05.
        assert!(flags("The effect held, t(28) = 2.05, p = .05.").is_empty());
        // r(48)=.50 → p ≈ .0002 < .001.
        assert!(flags("A strong link, r(48) = .50, p < .001.").is_empty());
        // F(2,27)=3.35 → p ≈ .0501 → rounds to .05.
        assert!(flags("ANOVA was significant, F(2, 27) = 3.35, p = .05.").is_empty());
    }

    #[test]
    fn a_grossly_wrong_p_value_is_flagged_with_decision_flip() {
        // t(28)=2.05 recomputes to p≈.05 but is reported as p=.50 → decision flips.
        let f = flags("The effect was reliable, t(28) = 2.05, p = .50.");
        assert_eq!(f.len(), 1);
        assert!(f[0].message.contains("FLIPS"), "{}", f[0].message);
        assert_eq!(f[0].kind, FindingKind::Risk, "deterministic tier stays Uncertain");
    }

    #[test]
    fn a_false_significance_claim_is_flagged() {
        // r(48)=.10 → t≈.70 → two-tailed p≈.49, one-tailed≈.245 — neither is < .01.
        let f = flags("There was a correlation, r(48) = .10, p < .01.");
        assert_eq!(f.len(), 1, "{f:#?}");
        assert!(f[0].message.contains("inconsistent"), "{}", f[0].message);
    }

    #[test]
    fn a_minor_inconsistency_is_flagged_without_flip() {
        // t(10)=2.0 → two-tailed p≈.073, one-tailed≈.037; reported .30 fits NEITHER.
        // Both reported and recomputed are non-significant, so no decision flip.
        let f = flags("t(10) = 2.0, p = .30.");
        assert_eq!(f.len(), 1, "{f:#?}");
        assert!(f[0].message.contains("inconsistent"), "{}", f[0].message);
        assert!(!f[0].message.contains("FLIPS"), "{}", f[0].message);
    }

    #[test]
    fn a_one_tailed_result_is_not_falsely_flagged() {
        // t(28)=2.05 → two-tailed p≈.0498; a directional test reports the one-tailed
        // half ≈ .025, so "p = .02" must NOT be flagged (it's a valid one-tailed report).
        assert!(flags("Directional test held, t(28) = 2.05, p = .02.").is_empty());
    }

    #[test]
    fn loose_parsing_false_positives_are_gone() {
        // A citation between the statistic and an unrelated later p — must NOT pair them.
        assert!(flags("the function F(2, 27) = 3.35 (Smith, 1999) was used; later p = .80").is_empty());
        // Two tests near one p: the first has no adjacent p; the second (t(30)=3.0,
        // p=.01) is consistent — so no false flag and the real test isn't dropped.
        assert!(flags("t(28) = 2.05 and t(30) = 3.0, p = .01").is_empty());
        // Sequence/index notation with a non-adjacent p (a word sits between).
        assert!(flags("The sequence a(n), t(5) = 3.0, threshold p = .5.").is_empty());
        // A digit before the symbol ("3r(") must not parse as a test.
        assert!(flags("Model 3r(48) = .5, p = .01").is_empty());
        // Scientific notation is skipped, not truncated to the mantissa.
        assert!(flags("t(28) = 2.05e3, p = .04").is_empty());
        // An implausibly large df (above the ceiling) is not a real test.
        assert!(flags("t(20000000) = 2.0, p = .04").is_empty());
    }

    #[test]
    fn non_test_text_and_word_boundaries_do_not_false_match() {
        // "first(" must not parse as t(...); plain prose yields nothing.
        assert!(parse_nhst("We first(ly) note that the rate was 5 percent.").is_empty());
        assert!(parse_nhst("The treatment improved recovery overall.").is_empty());
    }
}
