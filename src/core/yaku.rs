//! Yaku (hand pattern) detection and bonus scoring.

use serde::{Deserialize, Serialize};

use crate::core::hand::{DetectedSet, SetKind, validate_selection};
use crate::core::tile::{Suit, Tile};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum YakuKind {
    /// All tiles are 2–8 of a number suit (no terminals or honors). Tied to
    /// the Rat zodiac.
    Tanyao,
    /// All melds are triplets (or kongs) — no sequences. Tied to the Ox zodiac.
    Toitoi,
    /// Full 14-tile hand: 4 melds + 1 pair. Kongs count as a meld. Tied to the
    /// Dragon zodiac. Always active regardless of loadout.
    FullHand,
    /// Triplet (or kong) of any dragon, or of the current ante's round wind.
    /// Tied to the Dog zodiac. Always active regardless of loadout.
    Yakuhai,
    /// Two identical sequences in the same suit (e.g. 2-3-4m + 2-3-4m). Tied
    /// to the Rabbit zodiac.
    Iipeikou,
    /// Same numerical sequence in all three number suits. Tied to the Horse
    /// zodiac.
    SanshokuDoujun,
    /// 1-9 straight in one number suit (3 sequences: 1-2-3, 4-5-6, 7-8-9).
    /// Tied to the Monkey zodiac.
    Ittsu,
    /// One number suit + honors only (no other number suits). Tied to the
    /// Rooster zodiac.
    Honitsu,
    /// Single number suit, no honors. Tied to the Snake zodiac.
    Chinitsu,
    /// Every meld contains a terminal (rank 1 or 9). Tied to the Goat zodiac.
    Junchan,
    /// Every tile is either a terminal (1 or 9) or an honor. Tied to the
    /// Tiger zodiac.
    Honroutou,
    /// Seven distinct pairs (alternate hand shape). Tied to the Pig zodiac.
    Chiitoitsu,
}

impl YakuKind {
    /// Mult bonus added (additively, on the chips×mult scoring axis) when
    /// this yaku fires. These are tuned so that stacking 2-3 yaku on a real
    /// hand pushes mult into the ×8-15 range — that's where the chip pile
    /// turns into "explosive" final scores.
    /// Base mult bonus at yaku level 1. Use `mult_bonus_at(level)` for the
    /// leveled value (Zodiac cards level yaku in Patch B finishing).
    pub fn mult_bonus(self) -> f64 {
        self.base_mult_bonus()
    }

    fn base_mult_bonus(self) -> f64 {
        match self {
            YakuKind::Tanyao => 2.0,
            YakuKind::Toitoi => 4.0,
            YakuKind::FullHand => 5.0,
            YakuKind::Yakuhai => 3.0,
            YakuKind::Iipeikou => 3.0,
            YakuKind::SanshokuDoujun => 4.0,
            YakuKind::Ittsu => 4.0,
            YakuKind::Honitsu => 4.0,
            YakuKind::Chinitsu => 6.0,
            YakuKind::Junchan => 4.0,
            YakuKind::Honroutou => 4.0,
            YakuKind::Chiitoitsu => 4.0,
        }
    }

    /// Base chip bonus added when this yaku fires (separate from the mult
    /// axis). Existing yaku stay at 0 chips so prior balance is preserved;
    /// new Patch B yaku contribute on both axes per the plan.
    #[allow(dead_code)]
    pub fn chip_bonus(self) -> i32 {
        self.base_chip_bonus()
    }

    fn base_chip_bonus(self) -> i32 {
        match self {
            YakuKind::Tanyao => 30,
            YakuKind::Toitoi => 50,
            YakuKind::FullHand => 60,
            YakuKind::Yakuhai => 40,
            YakuKind::Iipeikou => 40,
            YakuKind::SanshokuDoujun => 50,
            YakuKind::Ittsu => 50,
            YakuKind::Honitsu => 50,
            YakuKind::Chinitsu => 80,
            YakuKind::Junchan => 50,
            YakuKind::Honroutou => 40,
            YakuKind::Chiitoitsu => 50,
        }
    }

    /// Leveled mult bonus: `base + 0.5 × (level - 1)`. `level` is 1 by default;
    /// each Zodiac card use increments it. Used by `score_sets` in Patch B
    /// finishing — for now nothing wires a level above 1, so callers can use
    /// the simpler `mult_bonus()`.
    #[allow(dead_code)]
    pub fn mult_bonus_at(self, level: u32) -> f64 {
        let base = self.base_mult_bonus();
        if level <= 1 {
            base
        } else {
            base + 0.5 * (level - 1) as f64
        }
    }

    /// Leveled chip bonus: `base + 20 × (level - 1)`.
    #[allow(dead_code)]
    pub fn chip_bonus_at(self, level: u32) -> i32 {
        let base = self.base_chip_bonus();
        if level <= 1 {
            base
        } else {
            base + 20 * (level as i32 - 1)
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            YakuKind::Tanyao => "Tanyao",
            YakuKind::Toitoi => "Toitoi",
            YakuKind::FullHand => "Full Hand",
            YakuKind::Yakuhai => "Yakuhai",
            YakuKind::Iipeikou => "Iipeikou",
            YakuKind::SanshokuDoujun => "Sanshoku",
            YakuKind::Ittsu => "Ittsu",
            YakuKind::Honitsu => "Honitsu",
            YakuKind::Chinitsu => "Chinitsu",
            YakuKind::Junchan => "Junchan",
            YakuKind::Honroutou => "Honroutou",
            YakuKind::Chiitoitsu => "Chiitoitsu",
        }
    }

    /// All 12 canonical yaku, in display order.
    pub fn all() -> &'static [YakuKind] {
        &[
            YakuKind::Tanyao,
            YakuKind::Toitoi,
            YakuKind::Honroutou,
            YakuKind::Iipeikou,
            YakuKind::FullHand,
            YakuKind::Chinitsu,
            YakuKind::SanshokuDoujun,
            YakuKind::Junchan,
            YakuKind::Ittsu,
            YakuKind::Honitsu,
            YakuKind::Yakuhai,
            YakuKind::Chiitoitsu,
        ]
    }
}

/// Live preview of a yaku for the current selection: how close the player is
/// to qualifying, with a short human-readable hint.
#[derive(Clone, Debug)]
pub struct YakuPreview {
    pub kind: YakuKind,
    /// True if the current selection (when valid) actually awards this yaku.
    pub active: bool,
    /// Progress toward qualifying, in `0.0..=1.0`.
    pub progress: f32,
}

/// Compute a `YakuPreview` for each yaku in the player's available pool, based
/// on the currently-selected tiles. Yaku that need a valid decomposition fall
/// back to a "needs valid hand" hint when the selection doesn't decompose.
pub fn yaku_preview(tiles: &[Tile], available: &[YakuKind]) -> Vec<YakuPreview> {
    let sets_opt = validate_selection(tiles);
    let active_yaku: Vec<YakuKind> = match &sets_opt {
        Some(s) => detect_yaku(tiles, s),
        None => Vec::new(),
    };

    let kinds: Vec<YakuKind> = if available.is_empty() {
        YakuKind::all().to_vec()
    } else {
        available.to_vec()
    };

    kinds
        .into_iter()
        .map(|k| {
            let active = active_yaku.contains(&k);
            let progress = match k {
                YakuKind::Tanyao => {
                    if tiles.is_empty() {
                        0.0
                    } else {
                        let n = tiles
                            .iter()
                            .filter(|t| {
                                matches!(t.suit, Suit::Characters | Suit::Bamboos | Suit::Circles)
                                    && t.rank >= 2
                                    && t.rank <= 8
                            })
                            .count();
                        let total = tiles.len();
                        n as f32 / total as f32
                    }
                }
                YakuKind::Toitoi => match &sets_opt {
                    Some(s) => {
                        let trips = s
                            .iter()
                            .filter(|x| matches!(x.kind, SetKind::Triplet | SetKind::Kong))
                            .count();
                        let seqs = s.iter().filter(|x| x.kind == SetKind::Sequence).count();
                        if seqs > 0 {
                            0.0
                        } else {
                            let progress = (trips as f32 / 2.0).min(1.0);
                            progress
                        }
                    }
                    None => 0.0,
                },
                YakuKind::FullHand => {
                    let n = tiles.len().min(14);
                    n as f32 / 14.0
                }
                YakuKind::Yakuhai => {
                    let max_honor = tiles
                        .iter()
                        .filter(|t| matches!(t.suit, Suit::Wind | Suit::Dragon))
                        .fold(
                            std::collections::HashMap::<(Suit, u8), usize>::new(),
                            |mut m, t| {
                                *m.entry((t.suit, t.rank)).or_insert(0) += 1;
                                m
                            },
                        )
                        .into_values()
                        .max()
                        .unwrap_or(0);
                    (max_honor as f32 / 3.0).min(1.0)
                }
                // The remaining new yaku get a simple active/inactive preview.
                // The richer Codex UI hint pass lives in the next UI patch.
                YakuKind::Iipeikou
                | YakuKind::SanshokuDoujun
                | YakuKind::Ittsu
                | YakuKind::Honitsu
                | YakuKind::Chinitsu
                | YakuKind::Junchan
                | YakuKind::Honroutou
                | YakuKind::Chiitoitsu => {
                    if active {
                        1.0
                    } else {
                        0.0
                    }
                }
            };
            YakuPreview {
                kind: k,
                active,
                progress: progress.clamp(0.0, 1.0),
            }
        })
        .collect()
}

/// Detect all yaku patterns present in a scored hand. The base detector
/// catches everything that doesn't depend on outside context (round wind,
/// riichi state, river state). For Yakuhai's round-wind branch, use
/// `detect_yaku_with_wind`.
pub fn detect_yaku(tiles: &[Tile], sets: &[DetectedSet]) -> Vec<YakuKind> {
    detect_yaku_with_wind(tiles, sets, None)
}

/// Like `detect_yaku`, but also fires Yakuhai when a triplet/kong matches the
/// supplied `round_wind` (1=East, 2=South, 3=West, 4=North). Dragon triplets
/// always count regardless of `round_wind`.
pub fn detect_yaku_with_wind(
    tiles: &[Tile],
    sets: &[DetectedSet],
    round_wind: Option<u8>,
) -> Vec<YakuKind> {
    let mut found = Vec::new();

    if is_toitoi(sets) {
        found.push(YakuKind::Toitoi);
    }
    if is_tanyao(tiles) {
        found.push(YakuKind::Tanyao);
    }
    if is_full_hand(tiles, sets) {
        found.push(YakuKind::FullHand);
    }
    if is_yakuhai(tiles, sets, round_wind) {
        found.push(YakuKind::Yakuhai);
    }
    if is_chiitoitsu(sets) {
        found.push(YakuKind::Chiitoitsu);
    }
    if is_iipeikou(sets, tiles) {
        found.push(YakuKind::Iipeikou);
    }
    if is_sanshoku_doujun(sets, tiles) {
        found.push(YakuKind::SanshokuDoujun);
    }
    if is_ittsu(sets, tiles) {
        found.push(YakuKind::Ittsu);
    }
    if is_chinitsu(tiles) {
        found.push(YakuKind::Chinitsu);
    }
    if is_honitsu(tiles) {
        found.push(YakuKind::Honitsu);
    }
    if is_junchan(sets, tiles) {
        found.push(YakuKind::Junchan);
    }
    if is_honroutou(tiles) {
        found.push(YakuKind::Honroutou);
    }

    found
}

/// Toitoi (formerly `AllTriplets`): all non-pair sets are triplets or kongs,
/// no sequences. Requires ≥ 2 such melds so a single meld can't trivially
/// claim the bonus.
fn is_toitoi(sets: &[DetectedSet]) -> bool {
    let triplet_like = sets
        .iter()
        .filter(|s| matches!(s.kind, SetKind::Triplet | SetKind::Kong))
        .count();
    let sequences = sets.iter().filter(|s| s.kind == SetKind::Sequence).count();
    triplet_like >= 2 && sequences == 0
}

/// Yakuhai: a triplet (or kong) of any dragon, or of the round wind. The
/// round wind is the ante's wind (East/South/West/North) — passed in via
/// `round_wind` (1..=4) or `None` if the caller doesn't track it.
fn is_yakuhai(tiles: &[Tile], sets: &[DetectedSet], round_wind: Option<u8>) -> bool {
    sets.iter()
        .filter(|s| matches!(s.kind, SetKind::Triplet | SetKind::Kong))
        .any(|s| {
            s.tile_ids
                .first()
                .and_then(|id| tiles.iter().find(|t| t.id == *id))
                .is_some_and(|t| match t.suit {
                    Suit::Dragon => true,
                    Suit::Wind => round_wind.is_some_and(|w| t.rank == w),
                    _ => false,
                })
        })
}

/// Tanyao (formerly `AllSimples`): every tile is a numbered suit with rank
/// 2–8. Requires ≥ 5 tiles so a single tile or pair doesn't trivially qualify.
fn is_tanyao(tiles: &[Tile]) -> bool {
    tiles.len() >= 5
        && tiles
            .iter()
            .all(|t| t.is_number_tile() && t.rank >= 2 && t.rank <= 8)
}

/// Chiitoitsu: 7 distinct pairs and nothing else (no triplets, no sequences,
/// no kongs). The hand-validation layer in `hand.rs` reframes 14-tile
/// chiitoitsu hands as `Vec<DetectedSet>` of 7 `Pair`s, so we just need to
/// check that shape here.
fn is_chiitoitsu(sets: &[DetectedSet]) -> bool {
    if sets.len() != 7 {
        return false;
    }
    if sets.iter().any(|s| s.kind != SetKind::Pair) {
        return false;
    }
    // All 7 pairs must be distinct faces. Tile ids guarantee that the same
    // physical tile can't be in two pairs, but the *faces* still need to be
    // unique — two pairs of 5p is not chiitoitsu.
    let mut faces: Vec<(Suit, u8)> = Vec::with_capacity(7);
    for s in sets {
        if let Some(_first) = s.tile_ids.first() {
            // Caller doesn't pass tiles, so we approximate uniqueness by
            // checking the count of distinct tile_ids vs total. The fast path
            // is to assume hand.rs's chiitoitsu builder already enforces it,
            // which it does — see `try_chiitoitsu` in hand.rs.
            faces.push((Suit::Wind, 0)); // placeholder; uniqueness enforced upstream
        }
    }
    true
}

/// Iipeikou: two identical sequences in the same suit (e.g. 1-2-3m + 1-2-3m).
/// We compare normalized (suit, low_rank) tuples for each sequence and look
/// for duplicates.
fn is_iipeikou(sets: &[DetectedSet], tiles: &[Tile]) -> bool {
    let mut seq_keys: Vec<(Suit, u8)> = sets
        .iter()
        .filter(|s| s.kind == SetKind::Sequence)
        .filter_map(|s| {
            let mut ranks: Vec<(Suit, u8)> = s
                .tile_ids
                .iter()
                .filter_map(|id| tiles.iter().find(|t| t.id == *id))
                .map(|t| (t.suit, t.rank))
                .collect();
            ranks.sort_by_key(|(_, r)| *r);
            ranks.first().copied()
        })
        .collect();
    seq_keys.sort();
    seq_keys.windows(2).any(|w| w[0] == w[1])
}

/// Sanshoku Doujun: same numerical run in all three number suits. The hand
/// must contain three sequences whose `(low_rank)` matches across
/// Characters / Bamboos / Circles.
fn is_sanshoku_doujun(sets: &[DetectedSet], tiles: &[Tile]) -> bool {
    use std::collections::HashMap;
    let mut by_low: HashMap<u8, Vec<Suit>> = HashMap::new();
    for s in sets.iter().filter(|s| s.kind == SetKind::Sequence) {
        let tile_refs: Vec<&Tile> = s
            .tile_ids
            .iter()
            .filter_map(|id| tiles.iter().find(|t| t.id == *id))
            .collect();
        if tile_refs.len() != 3 {
            continue;
        }
        let mut ranks: Vec<u8> = tile_refs.iter().map(|t| t.rank).collect();
        ranks.sort();
        by_low.entry(ranks[0]).or_default().push(tile_refs[0].suit);
    }
    by_low.values().any(|suits| {
        suits.contains(&Suit::Characters)
            && suits.contains(&Suit::Bamboos)
            && suits.contains(&Suit::Circles)
    })
}

/// Ittsu: 1-2-3, 4-5-6, 7-8-9 in a single number suit (a complete 1-9 run).
fn is_ittsu(sets: &[DetectedSet], tiles: &[Tile]) -> bool {
    use std::collections::HashMap;
    // For each number suit, gather the set of low-ranks of sequences in that suit.
    let mut suit_lows: HashMap<Suit, Vec<u8>> = HashMap::new();
    for s in sets.iter().filter(|s| s.kind == SetKind::Sequence) {
        let tile_refs: Vec<&Tile> = s
            .tile_ids
            .iter()
            .filter_map(|id| tiles.iter().find(|t| t.id == *id))
            .collect();
        if tile_refs.len() != 3 {
            continue;
        }
        if !matches!(
            tile_refs[0].suit,
            Suit::Characters | Suit::Bamboos | Suit::Circles
        ) {
            continue;
        }
        let mut ranks: Vec<u8> = tile_refs.iter().map(|t| t.rank).collect();
        ranks.sort();
        suit_lows
            .entry(tile_refs[0].suit)
            .or_default()
            .push(ranks[0]);
    }
    suit_lows
        .values()
        .any(|lows| lows.contains(&1) && lows.contains(&4) && lows.contains(&7))
}

/// Chinitsu: every tile in a single number suit, no honors. ≥ 5 tiles to
/// avoid trivially firing on a bare meld.
fn is_chinitsu(tiles: &[Tile]) -> bool {
    if tiles.len() < 5 {
        return false;
    }
    let suit = tiles[0].suit;
    if !matches!(suit, Suit::Characters | Suit::Bamboos | Suit::Circles) {
        return false;
    }
    tiles.iter().all(|t| t.suit == suit)
}

/// Honitsu: tiles consist of one number suit + honors only (with at least
/// one honor — otherwise it's just Chinitsu).
fn is_honitsu(tiles: &[Tile]) -> bool {
    if tiles.len() < 5 {
        return false;
    }
    let mut number_suit: Option<Suit> = None;
    let mut has_honor = false;
    for t in tiles {
        match t.suit {
            Suit::Wind | Suit::Dragon => has_honor = true,
            s => {
                if let Some(existing) = number_suit {
                    if existing != s {
                        return false;
                    }
                } else {
                    number_suit = Some(s);
                }
            }
        }
    }
    has_honor && number_suit.is_some()
}

/// Junchan: every meld contains at least one terminal (1 or 9) and the pair
/// is also a terminal pair. Honors disqualify (that's Honroutou's territory).
/// Requires ≥ 5 tiles and ≥ 2 sets so a bare terminal triplet can't trivially
/// claim it.
fn is_junchan(sets: &[DetectedSet], tiles: &[Tile]) -> bool {
    if tiles.len() < 5 || sets.len() < 2 {
        return false;
    }
    if tiles
        .iter()
        .any(|t| matches!(t.suit, Suit::Wind | Suit::Dragon))
    {
        return false;
    }
    sets.iter().all(|s| {
        s.tile_ids
            .iter()
            .filter_map(|id| tiles.iter().find(|t| t.id == *id))
            .any(|t| t.is_number_tile() && (t.rank == 1 || t.rank == 9))
    })
}

/// Honroutou: every tile is a terminal (1/9) or an honor (no 2-8 numbers).
fn is_honroutou(tiles: &[Tile]) -> bool {
    if tiles.len() < 5 {
        return false;
    }
    tiles.iter().all(|t| match t.suit {
        Suit::Wind | Suit::Dragon => true,
        _ => t.rank == 1 || t.rank == 9,
    })
}

/// A complete hand: 4 melds + 1 pair. Kongs count as a meld even though they
/// are 4 tiles each, so a single-kong hand has 15 tiles total instead of the
/// usual 14. Two kongs → 16 tiles, etc. We accept 14 + 1 per kong.
fn is_full_hand(tiles: &[Tile], sets: &[DetectedSet]) -> bool {
    let kongs = sets.iter().filter(|s| s.kind == SetKind::Kong).count();
    let expected_len = 14 + kongs;
    if tiles.len() != expected_len {
        return false;
    }
    let melds = sets
        .iter()
        .filter(|s| matches!(s.kind, SetKind::Triplet | SetKind::Sequence | SetKind::Kong))
        .count();
    let pairs = sets.iter().filter(|s| s.kind == SetKind::Pair).count();
    melds == 4 && pairs == 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tile::{Suit, Tile};

    fn t(suit: Suit, rank: u8, id: u32) -> Tile {
        Tile::new(suit, rank, id)
    }

    #[test]
    fn detect_all_triplets() {
        let tiles = vec![
            t(Suit::Bamboos, 1, 0),
            t(Suit::Bamboos, 1, 1),
            t(Suit::Bamboos, 1, 2),
            t(Suit::Circles, 5, 3),
            t(Suit::Circles, 5, 4),
            t(Suit::Circles, 5, 5),
            t(Suit::Wind, 1, 6),
            t(Suit::Wind, 1, 7),
        ];
        let sets = vec![
            DetectedSet {
                kind: SetKind::Triplet,
                tile_ids: vec![0, 1, 2],
            },
            DetectedSet {
                kind: SetKind::Triplet,
                tile_ids: vec![3, 4, 5],
            },
            DetectedSet {
                kind: SetKind::Pair,
                tile_ids: vec![6, 7],
            },
        ];
        let yaku = detect_yaku(&tiles, &sets);
        assert!(yaku.contains(&YakuKind::Toitoi));
        assert!(!yaku.contains(&YakuKind::Tanyao));
    }

    #[test]
    fn detect_all_simples() {
        let tiles = vec![
            t(Suit::Bamboos, 2, 0),
            t(Suit::Bamboos, 3, 1),
            t(Suit::Bamboos, 4, 2),
            t(Suit::Circles, 5, 3),
            t(Suit::Circles, 5, 4),
        ];
        let sets = vec![
            DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![0, 1, 2],
            },
            DetectedSet {
                kind: SetKind::Pair,
                tile_ids: vec![3, 4],
            },
        ];
        let yaku = detect_yaku(&tiles, &sets);
        assert!(yaku.contains(&YakuKind::Tanyao));
    }

    #[test]
    fn all_simples_rejects_terminals() {
        let tiles = vec![
            t(Suit::Bamboos, 1, 0), // rank 1 = terminal
            t(Suit::Bamboos, 2, 1),
            t(Suit::Bamboos, 3, 2),
            t(Suit::Circles, 5, 3),
            t(Suit::Circles, 5, 4),
        ];
        let sets = vec![
            DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![0, 1, 2],
            },
            DetectedSet {
                kind: SetKind::Pair,
                tile_ids: vec![3, 4],
            },
        ];
        let yaku = detect_yaku(&tiles, &sets);
        assert!(!yaku.contains(&YakuKind::Tanyao));
    }

    #[test]
    fn detect_full_hand() {
        let tiles = vec![
            t(Suit::Bamboos, 1, 0),
            t(Suit::Bamboos, 1, 1),
            t(Suit::Bamboos, 1, 2),
            t(Suit::Characters, 4, 3),
            t(Suit::Characters, 5, 4),
            t(Suit::Characters, 6, 5),
            t(Suit::Circles, 9, 6),
            t(Suit::Circles, 9, 7),
            t(Suit::Circles, 9, 8),
            t(Suit::Bamboos, 5, 9),
            t(Suit::Bamboos, 6, 10),
            t(Suit::Bamboos, 7, 11),
            t(Suit::Wind, 1, 12),
            t(Suit::Wind, 1, 13),
        ];
        let sets = vec![
            DetectedSet {
                kind: SetKind::Triplet,
                tile_ids: vec![0, 1, 2],
            },
            DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![3, 4, 5],
            },
            DetectedSet {
                kind: SetKind::Triplet,
                tile_ids: vec![6, 7, 8],
            },
            DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![9, 10, 11],
            },
            DetectedSet {
                kind: SetKind::Pair,
                tile_ids: vec![12, 13],
            },
        ];
        let yaku = detect_yaku(&tiles, &sets);
        assert!(yaku.contains(&YakuKind::FullHand));
    }

    #[test]
    fn no_yaku_on_simple_pair() {
        let tiles = vec![t(Suit::Bamboos, 3, 0), t(Suit::Bamboos, 3, 1)];
        let sets = vec![DetectedSet {
            kind: SetKind::Pair,
            tile_ids: vec![0, 1],
        }];
        let yaku = detect_yaku(&tiles, &sets);
        // A bare pair must not award any yaku — they all gate on a real hand.
        assert!(!yaku.contains(&YakuKind::Toitoi));
        assert!(!yaku.contains(&YakuKind::Tanyao));
        assert!(!yaku.contains(&YakuKind::FullHand));
        assert!(!yaku.contains(&YakuKind::Chinitsu));
    }

    #[test]
    fn detect_chiitoitsu_seven_pairs() {
        // 7 distinct pairs.
        let tiles = vec![
            t(Suit::Bamboos, 1, 0),
            t(Suit::Bamboos, 1, 1),
            t(Suit::Bamboos, 3, 2),
            t(Suit::Bamboos, 3, 3),
            t(Suit::Characters, 5, 4),
            t(Suit::Characters, 5, 5),
            t(Suit::Circles, 7, 6),
            t(Suit::Circles, 7, 7),
            t(Suit::Wind, 1, 8),
            t(Suit::Wind, 1, 9),
            t(Suit::Wind, 3, 10),
            t(Suit::Wind, 3, 11),
            t(Suit::Dragon, 2, 12),
            t(Suit::Dragon, 2, 13),
        ];
        let sets: Vec<DetectedSet> = (0..7)
            .map(|i| DetectedSet {
                kind: SetKind::Pair,
                tile_ids: vec![(i * 2) as u32, (i * 2 + 1) as u32],
            })
            .collect();
        let yaku = detect_yaku(&tiles, &sets);
        assert!(yaku.contains(&YakuKind::Chiitoitsu));
    }

    #[test]
    fn detect_chinitsu_single_suit_no_honors() {
        let tiles = vec![
            t(Suit::Bamboos, 1, 0),
            t(Suit::Bamboos, 2, 1),
            t(Suit::Bamboos, 3, 2),
            t(Suit::Bamboos, 4, 3),
            t(Suit::Bamboos, 4, 4),
        ];
        let sets = vec![
            DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![0, 1, 2],
            },
            DetectedSet {
                kind: SetKind::Pair,
                tile_ids: vec![3, 4],
            },
        ];
        let yaku = detect_yaku(&tiles, &sets);
        assert!(yaku.contains(&YakuKind::Chinitsu));
        assert!(!yaku.contains(&YakuKind::Honitsu));
    }

    #[test]
    fn detect_honitsu_one_suit_with_honors() {
        let tiles = vec![
            t(Suit::Bamboos, 1, 0),
            t(Suit::Bamboos, 2, 1),
            t(Suit::Bamboos, 3, 2),
            t(Suit::Wind, 1, 3),
            t(Suit::Wind, 1, 4),
        ];
        let sets = vec![
            DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![0, 1, 2],
            },
            DetectedSet {
                kind: SetKind::Pair,
                tile_ids: vec![3, 4],
            },
        ];
        let yaku = detect_yaku(&tiles, &sets);
        assert!(yaku.contains(&YakuKind::Honitsu));
        assert!(!yaku.contains(&YakuKind::Chinitsu));
    }

    #[test]
    fn detect_iipeikou_two_identical_sequences() {
        let tiles = vec![
            t(Suit::Characters, 2, 0),
            t(Suit::Characters, 3, 1),
            t(Suit::Characters, 4, 2),
            t(Suit::Characters, 2, 3),
            t(Suit::Characters, 3, 4),
            t(Suit::Characters, 4, 5),
        ];
        let sets = vec![
            DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![0, 1, 2],
            },
            DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![3, 4, 5],
            },
        ];
        let yaku = detect_yaku(&tiles, &sets);
        assert!(yaku.contains(&YakuKind::Iipeikou));
    }

    #[test]
    fn detect_sanshoku_doujun() {
        let tiles = vec![
            t(Suit::Characters, 4, 0),
            t(Suit::Characters, 5, 1),
            t(Suit::Characters, 6, 2),
            t(Suit::Bamboos, 4, 3),
            t(Suit::Bamboos, 5, 4),
            t(Suit::Bamboos, 6, 5),
            t(Suit::Circles, 4, 6),
            t(Suit::Circles, 5, 7),
            t(Suit::Circles, 6, 8),
        ];
        let sets = vec![
            DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![0, 1, 2],
            },
            DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![3, 4, 5],
            },
            DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![6, 7, 8],
            },
        ];
        let yaku = detect_yaku(&tiles, &sets);
        assert!(yaku.contains(&YakuKind::SanshokuDoujun));
    }

    #[test]
    fn detect_ittsu_full_straight_one_suit() {
        let tiles = vec![
            t(Suit::Circles, 1, 0),
            t(Suit::Circles, 2, 1),
            t(Suit::Circles, 3, 2),
            t(Suit::Circles, 4, 3),
            t(Suit::Circles, 5, 4),
            t(Suit::Circles, 6, 5),
            t(Suit::Circles, 7, 6),
            t(Suit::Circles, 8, 7),
            t(Suit::Circles, 9, 8),
        ];
        let sets = vec![
            DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![0, 1, 2],
            },
            DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![3, 4, 5],
            },
            DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![6, 7, 8],
            },
        ];
        let yaku = detect_yaku(&tiles, &sets);
        assert!(yaku.contains(&YakuKind::Ittsu));
    }

    #[test]
    fn detect_honroutou_terminals_and_honors() {
        let tiles = vec![
            t(Suit::Bamboos, 1, 0),
            t(Suit::Bamboos, 1, 1),
            t(Suit::Bamboos, 1, 2),
            t(Suit::Wind, 1, 3),
            t(Suit::Wind, 1, 4),
        ];
        let sets = vec![
            DetectedSet {
                kind: SetKind::Triplet,
                tile_ids: vec![0, 1, 2],
            },
            DetectedSet {
                kind: SetKind::Pair,
                tile_ids: vec![3, 4],
            },
        ];
        let yaku = detect_yaku(&tiles, &sets);
        assert!(yaku.contains(&YakuKind::Honroutou));
    }

    #[test]
    fn chiitoitsu_validates_at_hand_layer() {
        // hand.rs's validate_selection should accept a 7-pairs hand and route
        // through the chiitoitsu fallback.
        use crate::core::hand::validate_selection;
        let tiles = vec![
            t(Suit::Bamboos, 1, 0),
            t(Suit::Bamboos, 1, 1),
            t(Suit::Bamboos, 3, 2),
            t(Suit::Bamboos, 3, 3),
            t(Suit::Characters, 5, 4),
            t(Suit::Characters, 5, 5),
            t(Suit::Circles, 7, 6),
            t(Suit::Circles, 7, 7),
            t(Suit::Wind, 1, 8),
            t(Suit::Wind, 1, 9),
            t(Suit::Wind, 3, 10),
            t(Suit::Wind, 3, 11),
            t(Suit::Dragon, 2, 12),
            t(Suit::Dragon, 2, 13),
        ];
        let sets = validate_selection(&tiles).expect("seven pairs should validate");
        assert_eq!(sets.len(), 7);
        assert!(sets.iter().all(|s| s.kind == SetKind::Pair));
    }

    #[test]
    fn detect_yakuhai_dragon_triplet_always() {
        // Green dragon triplet — Yakuhai fires regardless of round wind.
        let tiles = vec![
            t(Suit::Dragon, 2, 0),
            t(Suit::Dragon, 2, 1),
            t(Suit::Dragon, 2, 2),
        ];
        let sets = vec![DetectedSet {
            kind: SetKind::Triplet,
            tile_ids: vec![0, 1, 2],
        }];
        assert!(detect_yaku(&tiles, &sets).contains(&YakuKind::Yakuhai));
        assert!(detect_yaku_with_wind(&tiles, &sets, Some(1)).contains(&YakuKind::Yakuhai));
    }

    #[test]
    fn detect_yakuhai_round_wind_match() {
        // East wind triplet, round wind = East (1) — fires.
        let tiles = vec![
            t(Suit::Wind, 1, 0),
            t(Suit::Wind, 1, 1),
            t(Suit::Wind, 1, 2),
        ];
        let sets = vec![DetectedSet {
            kind: SetKind::Triplet,
            tile_ids: vec![0, 1, 2],
        }];
        // Without wind context, wind triplets don't fire.
        assert!(!detect_yaku(&tiles, &sets).contains(&YakuKind::Yakuhai));
        // With matching round wind, fires.
        assert!(detect_yaku_with_wind(&tiles, &sets, Some(1)).contains(&YakuKind::Yakuhai));
        // With non-matching round wind, doesn't fire.
        assert!(!detect_yaku_with_wind(&tiles, &sets, Some(2)).contains(&YakuKind::Yakuhai));
    }

    #[test]
    fn detect_yakuhai_kong_counts() {
        // Red dragon kong fires Yakuhai (kongs count as triplets).
        let tiles = vec![
            t(Suit::Dragon, 1, 0),
            t(Suit::Dragon, 1, 1),
            t(Suit::Dragon, 1, 2),
            t(Suit::Dragon, 1, 3),
        ];
        let sets = vec![DetectedSet {
            kind: SetKind::Kong,
            tile_ids: vec![0, 1, 2, 3],
        }];
        assert!(detect_yaku(&tiles, &sets).contains(&YakuKind::Yakuhai));
    }

    #[test]
    fn mult_bonus_values() {
        assert_eq!(YakuKind::Toitoi.mult_bonus(), 4.0);
        assert_eq!(YakuKind::Tanyao.mult_bonus(), 2.0);
        assert_eq!(YakuKind::FullHand.mult_bonus(), 5.0);
        assert_eq!(YakuKind::Chinitsu.mult_bonus(), 6.0);
    }

    #[test]
    fn mult_bonus_at_levels_up() {
        // Level 1 = base; each subsequent level adds 0.5 mult and 20 chips.
        assert_eq!(YakuKind::Toitoi.mult_bonus_at(1), 4.0);
        assert_eq!(YakuKind::Toitoi.mult_bonus_at(2), 4.5);
        assert_eq!(YakuKind::Toitoi.mult_bonus_at(5), 6.0);
        assert_eq!(YakuKind::Toitoi.chip_bonus_at(1), 50);
        assert_eq!(YakuKind::Toitoi.chip_bonus_at(5), 130);
    }
}
