//! Comma-separated score magnitudes for HUD and Chronicle copy.

/// Whole score with thousands separators (`40,368`).
pub fn format_score(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::format_score;

    #[test]
    fn format_score_adds_thousands_separators() {
        assert_eq!(format_score(0), "0");
        assert_eq!(format_score(999), "999");
        assert_eq!(format_score(1_000), "1,000");
        assert_eq!(format_score(40_368), "40,368");
        assert_eq!(format_score(1_234_567), "1,234,567");
    }
}
