//! Keyword RGBA tints shared by UI helpers and the archive plaque decal raster.
//!
//! Honor / bonus suit needles (`winds`, `dragons`, …) skip tinting when the
//! token looks title-cased (`Winds` in "Wild Winds") so proper names are not
//! recolored. Numbered suits (`Manzu`, `Souzu`, `Pinzu`) always tint — they
//! are game jargon, not ordinary English. HUD verbs (`Chips`, `Mult`, …)
//! behave as before.

use crate::theme::color;

/// Longest-token-first table consumed by [`color_for_token`].
pub const COLORED_KEYWORD_TABLE: &[(&str, [f32; 4])] = &[
    ("characters", color::keyword::MANZU),
    ("bamboos", color::keyword::SOUZU),
    ("manzu", color::keyword::MANZU),
    ("souzu", color::keyword::SOUZU),
    ("pinzu", color::keyword::PINZU),
    ("dragons", color::keyword::DRAGON),
    ("flowers", color::keyword::FLOWER),
    ("seasons", color::keyword::SEASON),
    ("trigger", color::keyword::TRIGGER),
    ("bamboo", color::keyword::SOUZU),
    ("honors", color::keyword::HONORS),
    ("chips", color::keyword::CHIPS),
    ("winds", color::keyword::WIND),
    ("mult", color::keyword::MULT),
    ("dots", color::keyword::PINZU),
    ("yen", color::keyword::GOLD),
    ("play", color::keyword::PLAY),
];

/// Needles that also read as ordinary English words (often title-cased in
/// proper names like "Wild Winds"). Match is still ASCII case-insensitive on
/// letters, but we **decline** to tint when the surface token looks like a
/// title-cased word (leading upper + at least one later lower). Pure
/// lowercase, ALL CAPS, and numeric/punct-heavy tokens are unaffected.
const SKIP_WHEN_TITLE_CASED: &[&str] = &[
    "characters",
    "bamboos",
    "bamboo",
    "dots",
    "winds",
    "dragons",
    "flowers",
    "seasons",
    "honors",
];

#[inline]
fn looks_like_title_cased_word(s: &str) -> bool {
    let mut it = s.chars();
    let Some(first) = it.next() else {
        return false;
    };
    if !first.is_uppercase() {
        return false;
    }
    // Reject ALL CAPS ("WINDS", "CPU") so those still tint as jargon.
    it.any(|c| c.is_lowercase())
}

#[inline]
fn is_trim_punct(c: char) -> bool {
    matches!(
        c,
        ',' | '.' | ';' | ':' | '!' | '?' | '\'' | '"' | ')' | '(' | ']' | '[' | '}' | '{' | '…'
    ) || c == '\u{201c}'
        || c == '\u{201d}'
        || c == '\u{2019}'
        || c == '\u{2014}'
        || c == '\u{00d7}'
}

#[inline]
fn trim_token_punct(token: &str) -> &str {
    token.trim_matches(is_trim_punct)
}

/// Split a whitespace token into `(leading_punct, core, trailing_punct)`.
pub fn split_token_punct(token: &str) -> (&str, &str, &str) {
    let mut start = 0usize;
    let mut end = token.len();
    while start < end {
        let ch = token[start..].chars().next().unwrap();
        if is_trim_punct(ch) {
            start += ch.len_utf8();
        } else {
            break;
        }
    }
    while end > start {
        let ch = token[..end].chars().last().unwrap();
        if is_trim_punct(ch) {
            end -= ch.len_utf8();
        } else {
            break;
        }
    }
    (&token[..start], &token[start..end], &token[end..])
}

/// Colored substrings for one whitespace-delimited token. Leading/trailing
/// punctuation keeps `default`; only the core is glossary-tinted.
pub fn colored_token_segments(token: &str, default: [f32; 4]) -> Vec<(String, [f32; 4])> {
    let (lead, core, trail) = split_token_punct(token);
    let core_color = color_for_token(token, default);
    let mut out: Vec<(String, [f32; 4])> = Vec::with_capacity(3);
    if !lead.is_empty() {
        out.push((lead.to_string(), default));
    }
    if !core.is_empty() {
        out.push((core.to_string(), core_color));
    }
    if !trail.is_empty() {
        out.push((trail.to_string(), default));
    }
    if out.is_empty() {
        out.push((token.to_string(), default));
    }
    out
}

/// Whitespace-delimited chunk (may include leading/trailing punctuation).
pub fn color_for_token(token: &str, default: [f32; 4]) -> [f32; 4] {
    let core = trim_token_punct(token);
    if core.is_empty() {
        return default;
    }
    for &(needle, rgba) in COLORED_KEYWORD_TABLE {
        if core.eq_ignore_ascii_case(needle) {
            if SKIP_WHEN_TITLE_CASED
                .iter()
                .any(|n| n.eq_ignore_ascii_case(needle))
                && looks_like_title_cased_word(core)
            {
                return default;
            }
            return rgba;
        }
    }
    default
}

#[cfg(test)]
mod tests {
    use super::{color_for_token, colored_token_segments};

    #[test]
    fn trailing_comma_stays_default() {
        let d = [0.5, 0.5, 0.5, 1.0];
        let segs = colored_token_segments("mult,", d);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].0, "mult");
        assert_ne!(segs[0].1, d);
        assert_eq!(segs[1].0, ",");
        assert_eq!(segs[1].1, d);
    }

    #[test]
    fn leading_paren_stays_default() {
        let d = [0.5, 0.5, 0.5, 1.0];
        let segs = colored_token_segments("(winds", d);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].0, "(");
        assert_eq!(segs[0].1, d);
        assert_eq!(segs[1].0, "winds");
        assert_ne!(segs[1].1, d);
    }

    #[test]
    fn trailing_paren_stays_default() {
        let d = [0.5, 0.5, 0.5, 1.0];
        let segs = colored_token_segments("dragons)", d);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].0, "dragons");
        assert_ne!(segs[0].1, d);
        assert_eq!(segs[1].0, ")");
        assert_eq!(segs[1].1, d);
    }

    #[test]
    fn title_cased_winds_is_not_keyword() {
        let d = [0.5, 0.5, 0.5, 1.0];
        assert_eq!(color_for_token("Winds", d), d);
        assert_eq!(color_for_token("Wild", d), d);
    }

    #[test]
    fn lowercase_winds_still_keywords() {
        let d = [0.5, 0.5, 0.5, 1.0];
        let c = color_for_token("winds", d);
        assert_ne!(c, d);
        let c2 = color_for_token("WINDS", d);
        assert_ne!(c2, d);
    }

    #[test]
    fn chips_still_keywords_when_title_cased() {
        let d = [0.5, 0.5, 0.5, 1.0];
        let c = color_for_token("Chips", d);
        assert_ne!(c, d);
    }

    #[test]
    fn title_cased_manzu_still_keywords() {
        use crate::theme::color;
        let d = [0.5, 0.5, 0.5, 1.0];
        assert_eq!(color_for_token("Manzu", d), color::keyword::MANZU);
        assert_eq!(color_for_token("Souzu", d), color::keyword::SOUZU);
        assert_eq!(color_for_token("Pinzu", d), color::keyword::PINZU);
    }

    #[test]
    fn play_keyword_is_leaf_green() {
        use crate::theme::color;
        let d = [0.5, 0.5, 0.5, 1.0];
        assert_eq!(color_for_token("play", d), color::keyword::PLAY);
        assert_eq!(color_for_token("Play", d), color::keyword::PLAY);
    }

    #[test]
    fn trigger_keyword_stays_brass() {
        use crate::theme::color;
        let d = [0.5, 0.5, 0.5, 1.0];
        assert_eq!(color_for_token("trigger", d), color::keyword::TRIGGER);
    }
}
