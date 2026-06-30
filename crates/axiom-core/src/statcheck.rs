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
/// inconsistent. Flags a "decision flips" case (the significance verdict changes)
/// distinctly, since that is the gross, reviewer-relevant error.
fn inconsistency(n: &Nhst, recomputed: f64) -> Option<String> {
    let test = describe_test(n);
    match n.p_op {
        '=' => {
            let d = n.p_decimals.max(2);
            let factor = 10f64.powi(d as i32);
            let rounded = (recomputed * factor).round() / factor;
            if (rounded - n.p_reported).abs() <= 1e-12 {
                return None; // consistent at the reported precision
            }
            if decision_flips(n.p_reported, recomputed) {
                Some(format!(
                    "statcheck: reported p={} for {test} recomputes to p≈{recomputed:.4} — the significance \
                     decision at .05 FLIPS (likely a reporting error; confirm it isn't one-tailed/corrected)",
                    fmt_p(n.p_reported)
                ))
            } else {
                Some(format!(
                    "statcheck: reported p={} for {test} recomputes to p≈{recomputed:.4} — inconsistent; \
                     confirm the statistic, df, or p-value",
                    fmt_p(n.p_reported)
                ))
            }
        }
        '<' => {
            if recomputed < n.p_reported {
                return None;
            }
            Some(format!(
                "statcheck: reported p<{} for {test} but the recomputed p≈{recomputed:.4} is NOT below {} \
                 — inconsistent significance claim",
                fmt_p(n.p_reported),
                fmt_p(n.p_reported)
            ))
        }
        '>' => {
            if recomputed > n.p_reported {
                return None;
            }
            Some(format!(
                "statcheck: reported p>{} for {test} but the recomputed p≈{recomputed:.4} is not above it \
                 — inconsistent",
                fmt_p(n.p_reported)
            ))
        }
        _ => None,
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

/// Parse every reported NHST test in `text`.
fn parse_nhst(text: &str) -> Vec<Nhst> {
    let c: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        // A test symbol must stand alone: preceded by a non-letter so "first(" etc.
        // don't match the trailing 't('.
        let prev_is_letter = i > 0 && c[i - 1].is_alphabetic();
        if !prev_is_letter && matches!(c[i], 't' | 'F' | 'r') && c.get(i + 1) == Some(&'(') {
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
    i = after;
    let mut df2 = 0.0;
    if kind == 'F' {
        i = skip_ws(c, i);
        if c.get(i) != Some(&',') {
            return None;
        }
        i = skip_ws(c, i + 1);
        let (d2, _, after2) = read_num(c, i)?;
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
    i = after_stat;

    // Look ahead a bounded window for the p-value report.
    let p_start = i;
    let limit = (i + 40).min(c.len());
    let mut j = p_start;
    while j < limit {
        let prev_letter = j > 0 && c[j - 1].is_alphabetic();
        if !prev_letter && c[j] == 'p' {
            let mut k = skip_ws(c, j + 1);
            let op = match c.get(k) {
                Some('=') => '=',
                Some('<') | Some('≤') => '<',
                Some('>') | Some('≥') => '>',
                _ => {
                    j += 1;
                    continue;
                }
            };
            k = skip_ws(c, k + 1);
            if let Some((p_reported, p_decimals, after_p)) = read_num(c, k) {
                let excerpt: String = c[start..after_p].iter().collect();
                return Some((
                    Nhst { kind, df1, df2, stat, p_op: op, p_reported, p_decimals, excerpt },
                    after_p,
                ));
            }
        }
        j += 1;
    }
    None
}

fn skip_ws(c: &[char], mut i: usize) -> usize {
    while i < c.len() && c[i] == ' ' {
        i += 1;
    }
    i
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
        // r(48)=.10 → t≈.70 → p≈.49, but reported p<.01 → inconsistent.
        let f = flags("There was a correlation, r(48) = .10, p < .01.");
        assert_eq!(f.len(), 1, "{f:#?}");
        assert!(f[0].message.contains("NOT below"), "{}", f[0].message);
    }

    #[test]
    fn a_minor_inconsistency_is_flagged_without_flip() {
        // t(28)=2.50 → p≈.0185 reported as p=.01 → inconsistent, but BOTH are
        // significant (< .05) so the decision does NOT flip.
        let f = flags("t(28) = 2.50, p = .01.");
        assert_eq!(f.len(), 1, "{f:#?}");
        assert!(f[0].message.contains("inconsistent"), "{}", f[0].message);
        assert!(!f[0].message.contains("FLIPS"), "{}", f[0].message);
    }

    #[test]
    fn non_test_text_and_word_boundaries_do_not_false_match() {
        // "first(" must not parse as t(...); plain prose yields nothing.
        assert!(parse_nhst("We first(ly) note that the rate was 5 percent.").is_empty());
        assert!(parse_nhst("The treatment improved recovery overall.").is_empty());
    }
}
