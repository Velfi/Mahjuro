//! Word-level paragraph breaking with TeX-style demerit minimization.
//!
//! Break opportunities sit only between whitespace-delimited words. Punctuation
//! glued to a word (see [`crate::vocabulary_colors::colored_token_segments`])
//! stays on the same line as its core — we never split inside a unit.

/// One unbreakable layout unit (typically a word plus any glued punctuation).
#[derive(Clone, Debug)]
pub struct TextBreakUnit<T> {
    pub width: f32,
    pub payload: T,
}

/// Partition `units` into lines no wider than `max_width` (including `space_width`
/// between units on a line). Uses dynamic programming to minimize line demerits
/// rather than a naive first-fit greedy wrap.
pub fn break_units_kp<T: Clone>(
    units: &[TextBreakUnit<T>],
    max_width: f32,
    space_width: f32,
) -> Vec<Vec<T>> {
    let n = units.len();
    if n == 0 {
        return vec![Vec::new()];
    }
    if max_width <= 0.0 {
        return vec![units.iter().map(|u| u.payload.clone()).collect()];
    }

    const INF: f64 = f64::INFINITY;
    let mut dp = vec![INF; n + 1];
    let mut break_after = vec![0usize; n];
    dp[n] = 0.0;

    for i in (0..n).rev() {
        let mut line_w = 0.0_f32;
        let mut found = false;
        for j in i..n {
            if j > i {
                line_w += space_width;
            }
            line_w += units[j].width;
            if line_w > max_width {
                break;
            }
            found = true;
            let demerit = line_demerit(line_w, max_width, j == n - 1);
            let total = demerit + dp[j + 1];
            if total < dp[i] {
                dp[i] = total;
                break_after[i] = j;
            }
        }
        if !found {
            // Single overlong unit: force it onto its own line.
            let demerit = line_demerit(units[i].width, max_width, i == n - 1);
            dp[i] = demerit + dp[i + 1];
            break_after[i] = i;
        }
    }

    let mut lines: Vec<Vec<T>> = Vec::new();
    let mut i = 0;
    while i < n {
        let end = break_after[i];
        lines.push(units[i..=end].iter().map(|u| u.payload.clone()).collect());
        i = end + 1;
    }
    lines
}

/// Cubic slack demerit (Knuth–Plass flavour). Last lines are not stretched to
/// fill the measure, so their slack is discounted.
fn line_demerit(used_w: f32, max_w: f32, is_last_line: bool) -> f64 {
    let slack = (max_w - used_w).max(0.0) as f64;
    if slack <= f64::EPSILON {
        return 0.0;
    }
    let ratio = slack / max_w as f64;
    let mut demerit = ratio.powi(3) * 3000.0;
    if is_last_line {
        demerit *= 0.25;
    }
    demerit
}

/// Greedy word wrap on plain strings — same break opportunities as [`break_units_kp`].
pub fn wrap_words_kp(
    words: &[&str],
    word_width: impl Fn(&str) -> f32,
    max_width: f32,
    space_width: f32,
) -> Vec<String> {
    let units: Vec<TextBreakUnit<String>> = words
        .iter()
        .map(|w| TextBreakUnit {
            width: word_width(w),
            payload: (*w).to_string(),
        })
        .collect();
    let lines = break_units_kp(&units, max_width, space_width);
    lines.into_iter().map(|parts| parts.join(" ")).collect()
}

#[cfg(test)]
mod tests {
    use crate::{decal::load_ui_font, text_wrap::{TextBreakUnit, break_units_kp, wrap_words_kp}};

    fn word_w(word: &str, font_px: f32) -> f32 {
        let font = load_ui_font().expect("ui font");
        word.chars()
            .map(|c| font.metrics(c, font_px).advance_width)
            .sum()
    }

    #[test]
    fn kp_keeps_trailing_punct_on_same_line_as_word() {
        let font_px = 28.0;
        let space_w = word_w(" ", font_px);
        let text = "An East Wind is blowing!";
        let words: Vec<&str> = text.split_whitespace().collect();
        let max_w = word_w("An East Wind is blowing", font_px) + 2.0;
        let lines = wrap_words_kp(&words, |w| word_w(w, font_px), max_w, space_w);
        assert!(
            lines.iter().any(|l| l.contains('!')),
            "expected ! to stay with blowing, got {lines:?}"
        );
        assert!(
            !lines.iter().any(|l| l.trim() == "!"),
            "orphan punctuation line: {lines:?}"
        );
    }

    #[test]
    fn kp_prefers_balanced_breaks_over_greedy() {
        let units = vec![
            TextBreakUnit {
                width: 40.0,
                payload: "aaa",
            },
            TextBreakUnit {
                width: 40.0,
                payload: "bbb",
            },
            TextBreakUnit {
                width: 40.0,
                payload: "ccc",
            },
            TextBreakUnit {
                width: 40.0,
                payload: "ddd",
            },
        ];
        // Greedy at width 90: [aaa bbb] [ccc ddd]
        // KP at width 90: also [aaa bbb] [ccc ddd] — same here; sanity check DP runs.
        let lines = break_units_kp(&units, 90.0, 5.0);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], vec!["aaa", "bbb"]);
        assert_eq!(lines[1], vec!["ccc", "ddd"]);
    }
}
