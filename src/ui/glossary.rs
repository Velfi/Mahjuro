//! Game glossary: definitions of mahjong terms for interactive tooltips.
//!
//! Each entry maps a canonical display name (plus optional aliases) to a short
//! description.  Descriptions may reference other glossary terms, enabling the
//! recursive Paradox-style tooltip chain.

/// A single glossary entry.
pub struct GlossaryEntry {
    /// Canonical display name shown as the tooltip title.
    pub term: &'static str,
    /// Alternative surface forms to match in running text (lowercase).
    pub aliases: &'static [&'static str],
    /// Description text.  May contain other glossary terms.
    pub description: &'static str,
}

pub static GLOSSARY: &[GlossaryEntry] = &[
    // ── Tile categories ──────────────────────────────────────────────
    GlossaryEntry {
        term: "Honors",
        aliases: &["honor"],
        description: "Wind and Dragon tiles. Honors cannot form sequences \
                      — only pairs and triplets.",
    },
    GlossaryEntry {
        term: "Terminals",
        aliases: &["terminal"],
        description: "Number tiles with rank 1 or 9.",
    },
    GlossaryEntry {
        term: "Simples",
        aliases: &["simple"],
        description: "Number tiles with rank 2–8. No terminals or honors.",
    },
    // ── Suits ────────────────────────────────────────────────────────
    GlossaryEntry {
        term: "Characters",
        aliases: &[],
        description: "Red numbered suit, ranks 1–9.",
    },
    GlossaryEntry {
        term: "Bamboos",
        aliases: &["bamboo"],
        description: "Green numbered suit, ranks 1–9.",
    },
    GlossaryEntry {
        term: "Circles",
        aliases: &["circle"],
        description: "Blue numbered suit, ranks 1–9.",
    },
    GlossaryEntry {
        term: "Winds",
        aliases: &["wind"],
        description: "Honor tiles: East, South, West, North.",
    },
    GlossaryEntry {
        term: "Dragons",
        aliases: &["dragon"],
        description: "Honor tiles: Red (Chun), Green (Hatsu), White (Haku).",
    },
    // ── Melds / set types ────────────────────────────────────────────
    GlossaryEntry {
        term: "Meld",
        aliases: &["melds"],
        description: "A valid tile group: a pair, triplet, or sequence.",
    },
    GlossaryEntry {
        term: "Pair",
        aliases: &["pairs"],
        description: "Two identical tiles. Base score: 10 points.",
    },
    GlossaryEntry {
        term: "Triplet",
        aliases: &["triplets"],
        description: "Three identical tiles. Base score: 40 points.",
    },
    GlossaryEntry {
        term: "Sequence",
        aliases: &["sequences"],
        description: "Three consecutive tiles of the same suit. Base score: \
                      30 points. Only number tiles can form sequences.",
    },
    // ── Core mechanics ───────────────────────────────────────────────
    GlossaryEntry {
        term: "Wall",
        aliases: &[],
        description: "The deck of remaining tiles to draw from. Standard \
                      wall: 136 tiles (4 copies of each face).",
    },
    GlossaryEntry {
        term: "Dora",
        aliases: &[],
        description: "Bonus tiles worth +30 points each when scored in a meld.",
    },
    GlossaryEntry {
        term: "Hand",
        aliases: &[],
        description: "Your current collection of tiles. Maximum 14.",
    },
    GlossaryEntry {
        term: "Discard",
        aliases: &["discards"],
        description: "Remove unwanted tiles from your hand. 3 discards per round.",
    },
    GlossaryEntry {
        term: "Play",
        aliases: &["plays"],
        description: "Score a valid meld from your hand. 4 plays per round.",
    },
    // ── Meta-game ────────────────────────────────────────────────────
    GlossaryEntry {
        term: "Blind",
        aliases: &["blinds"],
        description: "A scoring challenge with a target score you must reach. \
                      Small, Big, or Boss.",
    },
    GlossaryEntry {
        term: "Relic",
        aliases: &["relics"],
        description: "A passive bonus that modifies how scoring works. \
                      Earned by clearing blinds.",
    },
    GlossaryEntry {
        term: "Gold",
        aliases: &[],
        description: "Currency earned from blinds. Spend gold in the shop \
                      to buy relics.",
    },
    GlossaryEntry {
        term: "Yaku",
        aliases: &[],
        description: "Special hand patterns that award bonus points when \
                      completed.",
    },
    GlossaryEntry {
        term: "Round",
        aliases: &["rounds"],
        description: "One complete scoring phase: play melds and discard \
                      until out of plays.",
    },
    // ── Yaku names ───────────────────────────────────────────────────
    GlossaryEntry {
        term: "Full Hand",
        aliases: &[],
        description: "Yaku: 14 tiles forming 4 melds + 1 pair. \
                      Bonus: 200 points.",
    },
    GlossaryEntry {
        term: "All Triplets",
        aliases: &[],
        description: "Yaku: every meld is a triplet. Bonus: 100 points.",
    },
    GlossaryEntry {
        term: "All Simples",
        aliases: &[],
        description: "Yaku: only tiles rank 2–8, no terminals or honors. \
                      Bonus: 60 points.",
    },
    GlossaryEntry {
        term: "Mixed Sets",
        aliases: &[],
        description: "Yaku: hand contains at least one pair, one triplet, \
                      and one sequence. Bonus: 50 points.",
    },
    GlossaryEntry {
        term: "Flush",
        aliases: &[],
        description: "Yaku: all tiles share one suit. Bonus: 120 points.",
    },
];

// ── Term matching ────────────────────────────────────────────────────────

/// A matched glossary term within a text string.
pub struct TermMatch {
    /// Character index (not byte) where the match starts.
    pub char_start: usize,
    /// Character index (not byte) past the end of the match.
    pub char_end: usize,
    /// The glossary entry that matched.
    pub entry: &'static GlossaryEntry,
}

/// Find all glossary terms in `text`, respecting word boundaries.
///
/// Longer terms match first to prevent partial overlaps (e.g. "Full Hand"
/// matches before "Hand").  Returns matches sorted by position.
pub fn find_terms_in_text(text: &str) -> Vec<TermMatch> {
    let lower: Vec<char> = text.to_lowercase().chars().collect();
    let n = lower.len();

    // Build (lowercase_form, entry) pairs, longest first.
    let mut forms: Vec<(Vec<char>, &'static GlossaryEntry)> = Vec::new();
    for entry in GLOSSARY {
        forms.push((entry.term.to_lowercase().chars().collect(), entry));
        for alias in entry.aliases {
            forms.push((alias.to_lowercase().chars().collect(), entry));
        }
    }
    forms.sort_by(|a, b| b.0.len().cmp(&a.0.len()));

    let mut used = vec![false; n];
    let mut matches = Vec::new();

    for (form, entry) in &forms {
        let flen = form.len();
        if flen > n {
            continue;
        }
        for start in 0..=(n - flen) {
            if used[start] {
                continue;
            }
            if lower[start..start + flen] != form[..] {
                continue;
            }
            // Word-boundary check.
            if start > 0 && lower[start - 1].is_alphanumeric() {
                continue;
            }
            if start + flen < n && lower[start + flen].is_alphanumeric() {
                continue;
            }
            for i in start..start + flen {
                used[i] = true;
            }
            matches.push(TermMatch {
                char_start: start,
                char_end: start + flen,
                entry,
            });
        }
    }

    matches.sort_by_key(|m| m.char_start);
    matches
}
