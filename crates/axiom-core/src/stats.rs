//! Minimal statistical distribution functions for the deterministic RECOMPUTE
//! checkers (statcheck-style). Hand-rolled to keep `axiom-core` dependency-light: a
//! single regularized incomplete beta (Numerical-Recipes continued fraction) covers
//! the p-values for Student's t, Fisher's F, and Pearson's r — the bulk of the
//! null-hypothesis tests reported in empirical papers. No model, no network: a p-value
//! is recomputed from the reported test statistic + degrees of freedom and that is a
//! deterministic fact, the heart of the moat.

/// Natural log of the gamma function (Lanczos approximation, g = 7).
fn ln_gamma(x: f64) -> f64 {
    const C: [f64; 9] = [
        0.999_999_999_999_809_9,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];
    if x < 0.5 {
        // Reflection formula keeps the series in its convergent range.
        std::f64::consts::PI.ln()
            - (std::f64::consts::PI * x).sin().ln()
            - ln_gamma(1.0 - x)
    } else {
        let x = x - 1.0;
        let mut a = C[0];
        let t = x + 7.5;
        for (i, &c) in C.iter().enumerate().skip(1) {
            a += c / (x + i as f64);
        }
        0.5 * (2.0 * std::f64::consts::PI).ln() + (x + 0.5) * t.ln() - t + a.ln()
    }
}

/// Continued-fraction evaluation for the incomplete beta (Numerical Recipes `betacf`).
fn betacf(a: f64, b: f64, x: f64) -> Option<f64> {
    const MAXIT: usize = 300;
    const EPS: f64 = 3.0e-12;
    const FPMIN: f64 = 1.0e-300;
    let qab = a + b;
    let qap = a + 1.0;
    let qam = a - 1.0;
    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    if d.abs() < FPMIN {
        d = FPMIN;
    }
    d = 1.0 / d;
    let mut h = d;
    for m in 1..=MAXIT {
        let m = m as f64;
        let m2 = 2.0 * m;
        let aa = m * (b - m) * x / ((qam + m2) * (a + m2));
        d = 1.0 + aa * d;
        if d.abs() < FPMIN {
            d = FPMIN;
        }
        c = 1.0 + aa / c;
        if c.abs() < FPMIN {
            c = FPMIN;
        }
        d = 1.0 / d;
        h *= d * c;
        let aa = -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2));
        d = 1.0 + aa * d;
        if d.abs() < FPMIN {
            d = FPMIN;
        }
        c = 1.0 + aa / c;
        if c.abs() < FPMIN {
            c = FPMIN;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < EPS {
            return Some(h);
        }
    }
    None // did not converge within MAXIT — caller yields NaN, statcheck then reports "inconclusive"
}

/// Regularized incomplete beta function `I_x(a, b)` ∈ [0, 1].
fn betai(a: f64, b: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let bt = (ln_gamma(a + b) - ln_gamma(a) - ln_gamma(b) + a * x.ln() + b * (1.0 - x).ln()).exp();
    if x < (a + 1.0) / (a + b + 2.0) {
        betacf(a, b, x).map(|cf| bt * cf / a).unwrap_or(f64::NAN)
    } else {
        betacf(b, a, 1.0 - x).map(|cf| 1.0 - bt * cf / b).unwrap_or(f64::NAN)
    }
}

/// Two-tailed p-value for a Student's t statistic with `df` degrees of freedom:
/// `P(|T| >= |t|)`. Returns NaN for non-positive df.
pub fn t_two_tailed_p(t: f64, df: f64) -> f64 {
    if df <= 0.0 || !t.is_finite() {
        return f64::NAN;
    }
    betai(df / 2.0, 0.5, df / (df + t * t)).clamp(0.0, 1.0)
}

/// Upper-tail p-value for an F statistic with `(df1, df2)` degrees of freedom:
/// `P(F >= f)`. ANOVA / regression F-tests are upper-tailed. NaN for invalid input.
pub fn f_upper_p(f: f64, df1: f64, df2: f64) -> f64 {
    if f < 0.0 || df1 <= 0.0 || df2 <= 0.0 || !f.is_finite() {
        return f64::NAN;
    }
    betai(df2 / 2.0, df1 / 2.0, df2 / (df2 + df1 * f)).clamp(0.0, 1.0)
}

/// Convert a Pearson correlation `r` (with `df = n - 2`) to its Student's t statistic.
/// `|r| >= 1` yields an infinite t (p → 0).
pub fn r_to_t(r: f64, df: f64) -> f64 {
    let denom = 1.0 - r * r;
    if denom <= 0.0 {
        return f64::INFINITY * r.signum();
    }
    r * (df / denom).sqrt()
}

/// Lower regularized incomplete gamma `P(a, x)` via the series (good for `x < a+1`).
fn gser(a: f64, x: f64) -> Option<f64> {
    let gln = ln_gamma(a);
    let mut ap = a;
    let mut sum = 1.0 / a;
    let mut del = sum;
    for _ in 0..300 {
        ap += 1.0;
        del *= x / ap;
        sum += del;
        if del.abs() < sum.abs() * 1e-15 {
            return Some(sum * (-x + a * x.ln() - gln).exp());
        }
    }
    None // series did not converge (e.g. very large chi-square df) → NaN → inconclusive
}

/// Upper regularized incomplete gamma `Q(a, x)` via the continued fraction (`x >= a+1`).
fn gcf(a: f64, x: f64) -> Option<f64> {
    const FPMIN: f64 = 1.0e-300;
    let gln = ln_gamma(a);
    let mut b = x + 1.0 - a;
    let mut c = 1.0 / FPMIN;
    let mut d = 1.0 / b;
    let mut h = d;
    for i in 1..300 {
        let an = -(i as f64) * (i as f64 - a);
        b += 2.0;
        d = an * d + b;
        if d.abs() < FPMIN {
            d = FPMIN;
        }
        c = b + an / c;
        if c.abs() < FPMIN {
            c = FPMIN;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < 1e-15 {
            return Some((-x + a * x.ln() - gln).exp() * h);
        }
    }
    None // continued fraction did not converge → NaN → inconclusive
}

/// Upper regularized incomplete gamma `Q(a, x) = P(X >= x)` for a Gamma(a). Both the
/// chi-square and normal p-values reduce to this single primitive.
fn gammq(a: f64, x: f64) -> f64 {
    if x < 0.0 || a <= 0.0 {
        return f64::NAN;
    }
    if x == 0.0 {
        return 1.0;
    }
    if x < a + 1.0 {
        gser(a, x).map(|s| 1.0 - s).unwrap_or(f64::NAN)
    } else {
        gcf(a, x).unwrap_or(f64::NAN)
    }
}

/// Upper-tail p-value for a chi-square statistic with `df` degrees of freedom:
/// `P(χ² >= x)`. Pearson/likelihood-ratio chi-square tests are upper-tailed.
pub fn chi2_upper_p(chi2: f64, df: f64) -> f64 {
    if chi2 < 0.0 || df <= 0.0 || !chi2.is_finite() {
        return f64::NAN;
    }
    gammq(df / 2.0, chi2 / 2.0).clamp(0.0, 1.0)
}

/// Two-tailed p-value for a standard-normal z statistic: `P(|Z| >= |z|)`. Uses the
/// identity `erfc(|z|/√2) = Q(1/2, z²/2)`.
pub fn z_two_tailed_p(z: f64) -> f64 {
    if !z.is_finite() {
        return f64::NAN;
    }
    gammq(0.5, z * z / 2.0).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn ln_gamma_matches_known_values() {
        // Γ(5) = 24 → ln = 3.178; Γ(0.5) = √π → ln = 0.5723649.
        assert!(approx(ln_gamma(5.0), 24f64.ln(), 1e-9));
        assert!(approx(ln_gamma(0.5), std::f64::consts::PI.sqrt().ln(), 1e-9));
        assert!(approx(ln_gamma(1.0), 0.0, 1e-12));
    }

    #[test]
    fn student_t_p_values_match_tables() {
        // Critical t for df=28, two-tailed α=0.05 is 2.0484 → p ≈ 0.05.
        assert!(approx(t_two_tailed_p(2.0484, 28.0), 0.05, 5e-4), "{}", t_two_tailed_p(2.0484, 28.0));
        // t=0 → p=1.
        assert!(approx(t_two_tailed_p(0.0, 10.0), 1.0, 1e-9));
        // Large t → p→0.
        assert!(t_two_tailed_p(8.0, 20.0) < 1e-6);
        // df=1 (Cauchy): t=1 → p = 0.5 exactly.
        assert!(approx(t_two_tailed_p(1.0, 1.0), 0.5, 1e-6));
    }

    #[test]
    fn f_p_values_match_tables() {
        // F(1,28) critical for α=0.05 is 4.196 → p ≈ 0.05.
        assert!(approx(f_upper_p(4.196, 1.0, 28.0), 0.05, 5e-4), "{}", f_upper_p(4.196, 1.0, 28.0));
        // F(2,27) critical for α=0.05 is 3.354 → p ≈ 0.05.
        assert!(approx(f_upper_p(3.354, 2.0, 27.0), 0.05, 5e-4), "{}", f_upper_p(3.354, 2.0, 27.0));
    }

    #[test]
    fn chi_square_p_values_match_tables() {
        // χ²(1) crit α=.05 is 3.841 → p ≈ .05; χ²(2)=5.991 → .05; χ²(10)=18.307 → .05.
        assert!(approx(chi2_upper_p(3.841, 1.0), 0.05, 5e-4), "{}", chi2_upper_p(3.841, 1.0));
        assert!(approx(chi2_upper_p(5.991, 2.0), 0.05, 5e-4), "{}", chi2_upper_p(5.991, 2.0));
        assert!(approx(chi2_upper_p(18.307, 10.0), 0.05, 5e-4), "{}", chi2_upper_p(18.307, 10.0));
        // χ²(2) at x=0 → p=1; large χ² → p→0.
        assert!(approx(chi2_upper_p(0.0, 2.0), 1.0, 1e-9));
        assert!(chi2_upper_p(50.0, 3.0) < 1e-6);
    }

    #[test]
    fn normal_two_tailed_p_matches_tables() {
        // z=1.96 → p≈.05; z=2.576 → .01; z=0 → 1; z=1.645 → .10.
        assert!(approx(z_two_tailed_p(1.96), 0.05, 5e-4), "{}", z_two_tailed_p(1.96));
        assert!(approx(z_two_tailed_p(2.576), 0.01, 5e-4), "{}", z_two_tailed_p(2.576));
        assert!(approx(z_two_tailed_p(0.0), 1.0, 1e-9));
        assert!(approx(z_two_tailed_p(1.645), 0.10, 1e-3), "{}", z_two_tailed_p(1.645));
    }

    #[test]
    fn correlation_reduces_to_t() {
        // r(48) = 0.5 → t = 0.5 * sqrt(48 / 0.75) = 4.0 → small p.
        let t = r_to_t(0.5, 48.0);
        assert!(approx(t, 4.0, 1e-9), "t={t}");
        assert!(t_two_tailed_p(t, 48.0) < 0.001);
        // r=0 → t=0 → p=1.
        assert!(approx(t_two_tailed_p(r_to_t(0.0, 30.0), 30.0), 1.0, 1e-9));
    }
}
