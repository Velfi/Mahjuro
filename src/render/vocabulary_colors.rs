//! Keyword RGBA tints shared by UI helpers and the archive plaque decal raster.
//!
//! Suit-family needles (`winds`, `dragons`, …) skip tinting when the token
//! looks title-cased (`Winds` in "Wild Winds") so proper names are not
//! recolored; `winds`, `WINDS`, and game jargon (`Chips`, `Mult`, …) behave as
//! before.

use crate::core::tile::Suit;
use crate::render::theme::color;

/// Longest-token-first table consumed by [`color_for_token`].
pub const COLORED_KEYWORD_TABLE: &[(&str, [f32; 4])] = &[
    ("characters", Suit::Characters.keyword_color()),
    ("bamboos", Suit::Bamboos.keyword_color()),
    ("dragons", Suit::Dragon.keyword_color()),
    ("flowers", Suit::Flower.keyword_color()),
    ("seasons", Suit::Season.keyword_color()),
    ("trigger", color::BRASS),
    ("bamboo", Suit::Bamboos.keyword_color()),
    ("honors", color::CHAMPAGNE),
    ("chips", color::LAPIS),
    ("winds", Suit::Wind.keyword_color()),
    ("mult", color::RUBY),
    ("dots", Suit::Dots.keyword_color()),
    ("gold", color::RELIC_GOLD),
    ("play", color::BRASS),
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
fn trim_token_punct(token: &str) -> &str {
    token.trim_matches(|c: char| {
        matches!(
            c,
            ',' | '.' | ';' | ':' | '!' | '?' | '\'' | '"' | ')' | '(' | ']' | '[' | '}'
                | '{' | '…'
        ) || c == '\u{201c}'
            || c == '\u{201d}'
            || c == '\u{2019}'
            || c == '\u{2014}'
            || c == '\u{00d7}'
    })
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
    use super::color_for_token;

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
}
