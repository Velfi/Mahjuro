//! Wilson score interval (95%) for binomial proportions, on a 0–100 percent scale.

/// Wilson 95% CI for `k` successes in `n` trials. Returns `(lo_pct, hi_pct)` in `[0, 100]`.
pub fn wilson_95_pct(k: u64, n: u64) -> Option<(f64, f64)> {
    if n == 0 {
        return None;
    }
    let z = 1.96_f64;
    let nf = n as f64;
    let kf = (k.min(n)) as f64;
    let p = kf / nf;
    let z2 = z * z;
    let denom = 1.0 + z2 / nf;
    let center = (p + z2 / (2.0 * nf)) / denom;
    let inner = p * (1.0 - p) / nf + z2 / (4.0 * nf * nf);
    let half = z * inner.max(0.0).sqrt() / denom;
    let lo = ((center - half).clamp(0.0, 1.0)) * 100.0;
    let hi = ((center + half).clamp(0.0, 1.0)) * 100.0;
    Some((lo, hi))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wilson_symmetric_mid() {
        let (lo, hi) = wilson_95_pct(50, 100).unwrap();
        assert!(35.0 < lo && lo < 45.0, "lo={lo}");
        assert!(55.0 < hi && hi < 65.0, "hi={hi}");
    }

    #[test]
    fn wilson_zero_n() {
        assert!(wilson_95_pct(0, 0).is_none());
    }
}
