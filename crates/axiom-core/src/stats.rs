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
fn betacf(a: f64, b: f64, x: f64) -> f64 {
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
            break;
        }
    }
    h
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
        bt * betacf(a, b, x) / a
    } else {
        1.0 - bt * betacf(b, a, 1.0 - x) / b
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
    fn correlation_reduces_to_t() {
        // r(48) = 0.5 → t = 0.5 * sqrt(48 / 0.75) = 4.0 → small p.
        let t = r_to_t(0.5, 48.0);
        assert!(approx(t, 4.0, 1e-9), "t={t}");
        assert!(t_two_tailed_p(t, 48.0) < 0.001);
        // r=0 → t=0 → p=1.
        assert!(approx(t_two_tailed_p(r_to_t(0.0, 30.0), 30.0), 1.0, 1e-9));
    }
}
