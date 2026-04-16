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
    GlossaryEntry {
        term: "Flowers",
        aliases: &["flower", "bonus tile", "bonus tiles"],
        description: "Rare wildcard tiles (Plum, Orchid, Chrysanthemum, Bamboo). \
                      Each substitutes for one missing tile in a triplet or \
                      sequence (max one per meld, cannot complete pairs). Each \
                      flower grants a unique effect when scored: Plum +40 chips, \
                      Orchid +1.5 mult, Chrysanthemum +15 chips per meld, \
                      Bamboo +$4 gold.",
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
        description: "The brass plinth shows the dora tile. Each dora tile in \
                      a scored set adds +25 chips, or +35 with the Dora Crown \
                      relic. Dora Crown also reveals a second dora.",
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
    GlossaryEntry {
        term: "Chicken Hand",
        aliases: &["chicken"],
        description: "A structurally valid meld or hand that triggers no \
                      yaku. It scores base chips × 1 mult — legal, but \
                      worth very little. Build toward a yaku to multiply \
                      your score.",
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
        term: "Shanten",
        aliases: &[],
        description: "How many tile swaps your hand is away from a complete \
                      shape (4 melds + 1 pair, or 7 pairs). Shanten 0 means \
                      tenpai — one tile away. Shanten -1 means won.",
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
        description: "Yaku: the standard complete 14-tile hand shape. \
                      Build 4 melds (triplets, sequences, or kongs) plus \
                      1 pair: 4+4+4+4+2, not 2x7 seven pairs. +5 mult, \
                      +60 chips.",
    },
    GlossaryEntry {
        term: "Tanyao",
        aliases: &["all simples"],
        description: "Yaku: all simples. Use only number tiles ranked \
                      2\u{2013}8 \u{2014} no terminals (1 or 9) and no \
                      honors (e.g. \u{1f3b4}234 \u{1f38b}567 \u{1f534}88). \
                      +2 mult, +30 chips.",
    },
    GlossaryEntry {
        term: "Toitoi",
        aliases: &["all triplets"],
        description: "Yaku: every meld is a triplet (or kong) \u{2014} no \
                      sequences allowed (e.g. \u{1f3b4}222 \u{1f38b}555 \
                      \u{1f534}999). +4 mult, +50 chips.",
    },
    GlossaryEntry {
        term: "Yakuhai",
        aliases: &["value tiles"],
        description: "Yaku: a triplet (or kong) of any dragon, or of the \
                      current round wind (e.g. \u{1f409}\u{1f409}\u{1f409}). \
                      +3 mult, +40 chips.",
    },
    GlossaryEntry {
        term: "Iipeikou",
        aliases: &["pure double sequence"],
        description: "Yaku: two identical sequences in the same suit \
                      (e.g. \u{1f38b}123 \u{1f38b}123). +3 mult, +40 chips.",
    },
    GlossaryEntry {
        term: "Sanshoku",
        aliases: &["three colour straight"],
        description: "Yaku: the same number sequence in all three number \
                      suits (e.g. \u{1f3b4}456 \u{1f38b}456 \u{1f534}456). \
                      +4 mult, +50 chips.",
    },
    GlossaryEntry {
        term: "Ittsu",
        aliases: &["pure straight"],
        description: "Yaku: a 1\u{2013}9 run in one number suit, built \
                      from three sequences (e.g. \u{1f38b}123 \u{1f38b}456 \
                      \u{1f38b}789). +4 mult, +50 chips.",
    },
    GlossaryEntry {
        term: "Honitsu",
        aliases: &["half flush"],
        description: "Yaku: one number suit plus honors only \u{2014} no \
                      other number suits (e.g. \u{1f38b}234 \u{1f38b}678 \
                      \u{1f32c}\u{1f32c}\u{1f32c}). +4 mult, +50 chips.",
    },
    GlossaryEntry {
        term: "Chinitsu",
        aliases: &["full flush", "flush"],
        description: "Yaku: a single number suit with no honors. Every \
                      tile shares the same suit (e.g. \u{1f38b}123 \u{1f38b}456 \
                      \u{1f38b}789 \u{1f38b}11). +6 mult, +80 chips.",
    },
    GlossaryEntry {
        term: "Junchan",
        aliases: &["terminal in each set"],
        description: "Yaku: every meld contains a terminal (rank 1 or 9). \
                      Sequences must be 1-2-3 or 7-8-9 (e.g. \u{1f38b}123 \
                      \u{1f3b4}789 \u{1f534}111 \u{1f38b}99). +4 mult, \
                      +50 chips.",
    },
    GlossaryEntry {
        term: "Honroutou",
        aliases: &["all terminals and honors"],
        description: "Yaku: every tile is either a terminal (1 or 9) or \
                      an honor (e.g. \u{1f38b}111 \u{1f3b4}999 \
                      \u{1f32c}\u{1f32c}\u{1f32c}). +4 mult, +40 chips.",
    },
    GlossaryEntry {
        term: "Chiitoitsu",
        aliases: &["seven pairs"],
        description: "Yaku: an alternate hand shape \u{2014} seven distinct \
                      pairs, no melds (e.g. \u{1f3b4}11 \u{1f3b4}33 \
                      \u{1f38b}55 \u{1f38b}77 \u{1f534}22 \u{1f534}44 \
                      \u{1f32c}\u{1f32c}). +4 mult, +50 chips.",
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
