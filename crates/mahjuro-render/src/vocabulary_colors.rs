//! Keyword RGBA tints shared by UI helpers and the archive plaque decal raster.
//!
//! Numbered suits (`Manzu`, `Souzu`, `Pinzu`) always tint — they are game
//! jargon, not ordinary English. Honor / bonus suit needles (`winds`,
//! `dragons`, …) skip tinting in [`GlossaryMode::Prose`] when the token
//! looks title-cased (`Winds` in "Wild Winds") so relic and prose names are
//! not recolored. [`GlossaryMode::Panel`] always tints table hits (guide
//! glossary rows). HUD verbs (`Chips`, `Mult`, …) behave as before.

use crate::theme::color;

/// Controls when whitespace tokens receive glossary tints.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GlossaryMode {
    #[default]
    Off,
    /// Mixed English + jargon (tooltips, hallway, modals).
    Prose,
    /// Guide/tutorial glossary rows — always tint table hits.
    Panel,
}

impl GlossaryMode {
    #[inline]
    pub fn from_legacy_glossary_tint(on: bool) -> Self {
        if on {
            Self::Prose
        } else {
            Self::Off
        }
    }

    #[inline]
    pub fn tints(self) -> bool {
        !matches!(self, Self::Off)
    }
}

/// Longest-token-first table consumed by [`color_for_token`].
pub const COLORED_KEYWORD_TABLE: &[(&str, [f32; 4])] = &[
    ("manzu", color::keyword::MANZU),
    ("souzu", color::keyword::SOUZU),
    ("pinzu", color::keyword::PINZU),
    ("dragons", color::keyword::DRAGON),
    ("flowers", color::keyword::FLOWER),
    ("seasons", color::keyword::SEASON),
    ("trigger", color::keyword::TRIGGER),
    ("honors", color::keyword::HONORS),
    ("chips", color::keyword::CHIPS),
    ("cp", color::keyword::CHIPS),
    ("winds", color::keyword::WIND),
    ("mult", color::keyword::MULT),
    ("han", color::keyword::TRIGGER),
    ("yen", color::keyword::GOLD),
    ("play", color::keyword::PLAY),
];

/// Needles that also read as ordinary English words (often title-cased in
/// proper names like "Wild Winds"). Match is still ASCII case-insensitive on
/// letters, but we **decline** to tint when the surface token looks like a
/// title-cased word (leading upper + at least one later lower) in
/// [`GlossaryMode::Prose`]. Pure lowercase, ALL CAPS, and numeric/punct-heavy
/// tokens are unaffected.
const SKIP_WHEN_TITLE_CASED: &[&str] = &["winds", "dragons", "flowers", "seasons", "honors"];

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

/// True when `s` is non-empty and every character is layout punctuation.
pub fn is_punctuation_only(s: &str) -> bool {
    !s.is_empty() && s.chars().all(is_trim_punct)
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

#[inline]
fn is_the_article(token: &str) -> bool {
    trim_token_punct(token) == "The"
}

#[inline]
fn is_house_word(token: &str) -> bool {
    let core = trim_token_punct(token);
    if core.eq_ignore_ascii_case("house") {
        return true;
    }
    core.len() >= 6
        && core.as_bytes()[5] == b'\''
        && core[..5].eq_ignore_ascii_case("house")
}

/// Marks `The` + `House` proper-noun tokens (case-sensitive article; possessive
/// `House's` counts as House). Lowercase "the house" in ordinary prose is skipped.
pub fn house_name_word_mask(words: &[&str]) -> Vec<bool> {
    let mut mask = vec![false; words.len()];
    if words.len() < 2 {
        return mask;
    }
    for i in 0..=words.len() - 2 {
        if is_the_article(words[i]) && is_house_word(words[i + 1]) {
            mask[i] = true;
            mask[i + 1] = true;
        }
    }
    mask
}

#[inline]
fn is_moon_word(token: &str) -> bool {
    let core = trim_token_punct(token);
    if core.eq_ignore_ascii_case("moon") {
        return true;
    }
    core.len() >= 6
        && core.as_bytes()[4] == b'\''
        && core[..4].eq_ignore_ascii_case("moon")
}

/// Marks `The` + `Moon` proper-noun tokens (case-sensitive article; possessive
/// `Moon's` counts as Moon).
pub fn moon_name_word_mask(words: &[&str]) -> Vec<bool> {
    let mut mask = vec![false; words.len()];
    if words.len() < 2 {
        return mask;
    }
    for i in 0..=words.len() - 2 {
        if is_the_article(words[i]) && is_moon_word(words[i + 1]) {
            mask[i] = true;
            mask[i + 1] = true;
        }
    }
    mask
}

#[inline]
pub fn is_house_keyword_color(c: [f32; 4]) -> bool {
    c == color::keyword::HOUSE
}

#[inline]
pub fn is_moon_keyword_color(c: [f32; 4]) -> bool {
    c == color::keyword::MOON
}

/// Screen-space text effect for a glossary tint (e.g. **The House** → score-pop polychrome).
#[inline]
pub fn text_effect_for_glossary_tint(c: [f32; 4]) -> crate::text_effect::TextEffectId {
    if is_house_keyword_color(c) {
        crate::text_effect::TextEffectId::Polychrome
    } else if is_moon_keyword_color(c) {
        crate::text_effect::TextEffectId::MoonPolychrome
    } else {
        crate::text_effect::TextEffectId::Flat
    }
}

/// Marks whitespace tokens that belong to a relic display name in `words`.
pub fn relic_name_word_mask(words: &[&str]) -> Vec<bool> {
    let mut mask = vec![false; words.len()];
    if words.is_empty() {
        return mask;
    }
    for def in mahjuro_core::core::relic::all_relic_defs() {
        let name_words: Vec<&str> = def.name.split_whitespace().collect();
        let n = name_words.len();
        if n == 0 || n > words.len() {
            continue;
        }
        for start in 0..=words.len() - n {
            if (0..n).all(|j| words[start + j].eq_ignore_ascii_case(name_words[j])) {
                for slot in &mut mask[start..start + n] {
                    *slot = true;
                }
            }
        }
    }
    mask
}

/// Lookup tint for a token core (punctuation stripped). `force` bypasses title-case guard.
pub fn lookup_keyword_color(core: &str, mode: GlossaryMode, force: bool) -> Option<[f32; 4]> {
    if matches!(mode, GlossaryMode::Off) {
        return None;
    }
    for &(needle, rgba) in COLORED_KEYWORD_TABLE {
        if core.eq_ignore_ascii_case(needle) {
            if !force
                && matches!(mode, GlossaryMode::Prose)
                && SKIP_WHEN_TITLE_CASED
                    .iter()
                    .any(|n| n.eq_ignore_ascii_case(needle))
                && looks_like_title_cased_word(core)
            {
                return None;
            }
            return Some(rgba);
        }
    }
    None
}

/// Like [`colored_token_segments`], but forces a fixed tint on the token core.
pub fn colored_token_segments_tinted(
    token: &str,
    tint: [f32; 4],
    default: [f32; 4],
) -> Vec<(String, [f32; 4])> {
    let (lead, core, trail) = split_token_punct(token);
    let mut out: Vec<(String, [f32; 4])> = Vec::with_capacity(3);
    if !lead.is_empty() {
        out.push((lead.to_string(), default));
    }
    if !core.is_empty() {
        if tint == default && !trail.is_empty() {
            out.push((format!("{core}{trail}"), default));
        } else {
            out.push((core.to_string(), tint));
            if !trail.is_empty() {
                out.push((trail.to_string(), default));
            }
        }
    } else if !trail.is_empty() {
        out.push((trail.to_string(), default));
    }
    if out.is_empty() {
        out.push((token.to_string(), default));
    }
    out
}

/// Colored substrings for one whitespace-delimited token. Leading/trailing
/// punctuation keeps `default`; only the core is glossary-tinted.
pub fn colored_token_segments(
    token: &str,
    default: [f32; 4],
    mode: GlossaryMode,
) -> Vec<(String, [f32; 4])> {
    colored_token_segments_forced(token, default, mode, false)
}

/// Like [`colored_token_segments`], with optional forced tint (from `{{term:…}}`).
pub fn colored_token_segments_forced(
    token: &str,
    default: [f32; 4],
    mode: GlossaryMode,
    force: bool,
) -> Vec<(String, [f32; 4])> {
    let (lead, core, trail) = split_token_punct(token);
    let core_color = color_for_token_forced(token, default, mode, force);
    let mut out: Vec<(String, [f32; 4])> = Vec::with_capacity(3);
    if !lead.is_empty() {
        out.push((lead.to_string(), default));
    }
    if !core.is_empty() {
        // Keep trailing punctuation in the same label when it is not tinted.
        // Separate labels expose the font's side-bearing and read as "word ,"
        // in the guide and other glossary paragraphs.
        if core_color == default && !trail.is_empty() {
            out.push((format!("{core}{trail}"), default));
        } else {
            out.push((core.to_string(), core_color));
            if !trail.is_empty() {
                out.push((trail.to_string(), default));
            }
        }
    } else if !trail.is_empty() {
        out.push((trail.to_string(), default));
    }
    if out.is_empty() {
        out.push((token.to_string(), default));
    }
    out
}

/// Whitespace-delimited chunk (may include leading/trailing punctuation).
pub fn color_for_token(token: &str, default: [f32; 4], mode: GlossaryMode) -> [f32; 4] {
    color_for_token_forced(token, default, mode, false)
}

/// Like [`color_for_token`], with optional forced tint (from `{{term:…}}`).
pub fn color_for_token_forced(
    token: &str,
    default: [f32; 4],
    mode: GlossaryMode,
    force: bool,
) -> [f32; 4] {
    let core = trim_token_punct(token);
    if core.is_empty() {
        return default;
    }
    lookup_keyword_color(core, mode, force).unwrap_or(default)
}

/// Segments for one word in a whitespace-split line, applying relic/house masks.
pub fn glossary_word_segments(
    words: &[&str],
    idx: usize,
    mode: GlossaryMode,
    default: [f32; 4],
) -> Vec<(String, [f32; 4])> {
    glossary_word_segments_forced(words, idx, mode, default, false)
}

/// Like [`glossary_word_segments`], with optional forced tint.
pub fn glossary_word_segments_forced(
    words: &[&str],
    idx: usize,
    mode: GlossaryMode,
    default: [f32; 4],
    force: bool,
) -> Vec<(String, [f32; 4])> {
    let Some(&word) = words.get(idx) else {
        return Vec::new();
    };
    let relic_mask = relic_name_word_mask(words);
    let house_mask = house_name_word_mask(words);
    let moon_mask = moon_name_word_mask(words);
    if relic_mask[idx] {
        vec![(word.to_string(), default)]
    } else if house_mask[idx] {
        colored_token_segments_tinted(word, color::keyword::HOUSE, default)
    } else if moon_mask[idx] {
        colored_token_segments_tinted(word, color::keyword::MOON, default)
    } else {
        colored_token_segments_forced(word, default, mode, force)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        GlossaryMode, color_for_token, color_for_token_forced, colored_token_segments,
        glossary_word_segments,
    };

    #[test]
    fn trailing_comma_stays_default() {
        let d = [0.5, 0.5, 0.5, 1.0];
        let segs = colored_token_segments("mult,", d, GlossaryMode::Prose);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].0, "mult");
        assert_ne!(segs[0].1, d);
        assert_eq!(segs[1].0, ",");
        assert_eq!(segs[1].1, d);
    }

    #[test]
    fn untinted_trailing_comma_stays_on_same_label() {
        let d = [0.5, 0.5, 0.5, 1.0];
        let segs = colored_token_segments("pairs,", d, GlossaryMode::Prose);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].0, "pairs,");
        assert_eq!(segs[0].1, d);
    }

    #[test]
    fn leading_paren_stays_default() {
        let d = [0.5, 0.5, 0.5, 1.0];
        let segs = colored_token_segments("(winds", d, GlossaryMode::Prose);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].0, "(");
        assert_eq!(segs[0].1, d);
        assert_eq!(segs[1].0, "winds");
        assert_ne!(segs[1].1, d);
    }

    #[test]
    fn trailing_paren_stays_default() {
        let d = [0.5, 0.5, 0.5, 1.0];
        let segs = colored_token_segments("dragons)", d, GlossaryMode::Prose);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].0, "dragons");
        assert_ne!(segs[0].1, d);
        assert_eq!(segs[1].0, ")");
        assert_eq!(segs[1].1, d);
    }

    #[test]
    fn title_cased_winds_in_relic_name_is_not_keyword() {
        let d = [0.5, 0.5, 0.5, 1.0];
        assert_eq!(color_for_token("Winds", d, GlossaryMode::Prose), d);
        assert_eq!(color_for_token("Wild", d, GlossaryMode::Prose), d);
        let words = ["Wild", "Winds"];
        let mask = super::relic_name_word_mask(&words);
        assert!(mask[0] && mask[1]);
        let segs = glossary_word_segments(&words, 1, GlossaryMode::Prose, d);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].0, "Winds");
        assert_eq!(segs[0].1, d);
    }

    #[test]
    fn panel_tints_title_cased_honors() {
        use crate::theme::color;
        let d = [0.5, 0.5, 0.5, 1.0];
        assert_eq!(
            color_for_token("Honors", d, GlossaryMode::Panel),
            color::keyword::HONORS
        );
        assert_eq!(
            color_for_token("Winds", d, GlossaryMode::Panel),
            color::keyword::WIND
        );
    }

    #[test]
    fn prose_suppresses_title_cased_honors() {
        let d = [0.5, 0.5, 0.5, 1.0];
        assert_eq!(color_for_token("Honors", d, GlossaryMode::Prose), d);
    }

    #[test]
    fn forced_term_tints_in_prose() {
        use crate::theme::color;
        let d = [0.5, 0.5, 0.5, 1.0];
        assert_eq!(
            color_for_token_forced("Honors", d, GlossaryMode::Prose, true),
            color::keyword::HONORS
        );
    }

    #[test]
    fn lowercase_winds_still_keywords() {
        let d = [0.5, 0.5, 0.5, 1.0];
        let c = color_for_token("winds", d, GlossaryMode::Prose);
        assert_ne!(c, d);
        let c2 = color_for_token("WINDS", d, GlossaryMode::Prose);
        assert_ne!(c2, d);
    }

    #[test]
    fn chips_still_keywords_when_title_cased() {
        let d = [0.5, 0.5, 0.5, 1.0];
        let c = color_for_token("Chips", d, GlossaryMode::Prose);
        assert_ne!(c, d);
    }

    #[test]
    fn title_cased_manzu_still_keywords() {
        use crate::theme::color;
        let d = [0.5, 0.5, 0.5, 1.0];
        assert_eq!(
            color_for_token("Manzu", d, GlossaryMode::Prose),
            color::keyword::MANZU
        );
        assert_eq!(
            color_for_token("Souzu", d, GlossaryMode::Prose),
            color::keyword::SOUZU
        );
        assert_eq!(
            color_for_token("Pinzu", d, GlossaryMode::Prose),
            color::keyword::PINZU
        );
    }

    #[test]
    fn play_keyword_is_leaf_green() {
        use crate::theme::color;
        let d = [0.5, 0.5, 0.5, 1.0];
        assert_eq!(
            color_for_token("play", d, GlossaryMode::Prose),
            color::keyword::PLAY
        );
        assert_eq!(
            color_for_token("Play", d, GlossaryMode::Prose),
            color::keyword::PLAY
        );
    }

    #[test]
    fn trigger_keyword_stays_brass() {
        use crate::theme::color;
        let d = [0.5, 0.5, 0.5, 1.0];
        assert_eq!(
            color_for_token("trigger", d, GlossaryMode::Prose),
            color::keyword::TRIGGER
        );
    }

    #[test]
    fn the_house_phrase_masks_both_tokens() {
        let words = ["Beat", "The", "House", "on", "wing", "7."];
        let mask = super::house_name_word_mask(&words);
        assert!(!mask[0]);
        assert!(mask[1] && mask[2]);
        assert!(!mask[3]);
    }

    #[test]
    fn lowercase_the_house_is_not_masked() {
        let words = ["the", "more", "the", "house", "will", "reveal"];
        let mask = super::house_name_word_mask(&words);
        assert!(mask.iter().all(|m| !*m));
    }

    #[test]
    fn the_house_possessive_masks_house_token() {
        let words = ["The", "House's", "five"];
        let mask = super::house_name_word_mask(&words);
        assert!(mask[0] && mask[1]);
        assert!(!mask[2]);
    }

    #[test]
    fn house_glossary_tint_uses_score_pop_polychrome() {
        use crate::text_effect::TextEffectId;
        assert_eq!(
            super::text_effect_for_glossary_tint(crate::theme::color::keyword::HOUSE),
            TextEffectId::Polychrome
        );
        assert_eq!(
            super::text_effect_for_glossary_tint(crate::theme::color::keyword::MOON),
            TextEffectId::MoonPolychrome
        );
        assert_eq!(
            super::text_effect_for_glossary_tint([0.5, 0.5, 0.5, 1.0]),
            TextEffectId::Flat
        );
    }

    #[test]
    fn the_moon_phrase_masks_both_tokens() {
        let words = ["The", "Moon's", "light", "welcomes", "you."];
        let mask = super::moon_name_word_mask(&words);
        assert!(mask[0] && mask[1]);
        assert!(!mask[2]);
    }
}
