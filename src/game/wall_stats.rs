//! Strategic diagnostics for the Wall Ledger — counts, probabilities, and heuristics.

use crate::core::debuff::TileDebuff;
use crate::core::tile::{Suit, Tile, TileEnhancement};
use crate::game::run::RunState;
use crate::game::wall_ledger::{WallLedgerFaceGroup, WallLedgerMode, WallLedgerReadModel};

/// Per-face tile counts by where copies live this round.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TileLocationCounts {
    pub in_wall: usize,
    pub in_hand: usize,
    pub played: usize,
    pub discarded: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AbundanceState {
    Exhausted,
    Thin,
    Normal,
    Abundant,
}

pub type WallTab = WallLedgerTab;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WallLedgerTab {
    #[default]
    Wall,
}

impl WallLedgerTab {
    pub const ALL: [Self; 1] = [Self::Wall];

    pub fn label(self) -> &'static str {
        match self {
            Self::Wall => "Wall",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FaceKey {
    pub suit: Suit,
    pub rank: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModifierBreakdown {
    pub pearl: usize,
    pub gilded: usize,
    pub polychrome: usize,
    pub debuffed: usize,
    pub plain: usize,
}

impl ModifierBreakdown {
    pub fn total(&self) -> usize {
        self.pearl + self.gilded + self.polychrome + self.debuffed + self.plain
    }
}

#[derive(Clone, Debug)]
pub struct TileLedgerEntry {
    pub suit: Suit,
    pub rank: u8,
    pub remaining: usize,
    pub seen: usize,
    pub total: usize,
    pub locations: TileLocationCounts,
    pub draw_probability: f32,
    pub wall_share: f32,
    pub abundance: AbundanceState,
    pub modifiers: ModifierBreakdown,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SuitSummary {
    pub manzu: usize,
    pub souzu: usize,
    pub pinzu: usize,
    pub honors: usize,
    pub flowers: usize,
}

#[derive(Clone, Debug)]
pub struct BestDrawHint {
    pub face: FaceKey,
    pub reason: String,
}

#[derive(Clone, Debug)]
pub struct YakuPathHint {
    pub label: String,
    pub detail: String,
}

#[derive(Clone, Debug)]
pub struct SelectedTileDetails {
    pub face: FaceKey,
    pub name: String,
    pub remaining: usize,
    pub total: usize,
    pub locations: TileLocationCounts,
    pub draw_probability: f32,
    pub wall_share: f32,
    pub modifiers: ModifierBreakdown,
    pub about: String,
}

#[derive(Clone, Debug)]
pub struct WallStats {
    pub entries: Vec<TileLedgerEntry>,
    pub suit_summary: SuitSummary,
    pub total_remaining: usize,
    pub total_wall: usize,
    pub most_common: Vec<(FaceKey, usize)>,
    pub thin_exhausted: Vec<(FaceKey, usize)>,
    pub abundant: Vec<(FaceKey, usize)>,
    pub best_draws: Vec<BestDrawHint>,
    pub yaku_hints: Vec<YakuPathHint>,
    pub global_modifiers: ModifierBreakdown,
}

/// Standard 38-face grid order (matches wall ledger scene).
pub const GRID_FACE_ORDER: [(Suit, u8); 38] = [
    (Suit::Manzu, 1),
    (Suit::Manzu, 2),
    (Suit::Manzu, 3),
    (Suit::Manzu, 4),
    (Suit::Manzu, 5),
    (Suit::Manzu, 6),
    (Suit::Manzu, 7),
    (Suit::Manzu, 8),
    (Suit::Manzu, 9),
    (Suit::Souzu, 1),
    (Suit::Souzu, 2),
    (Suit::Souzu, 3),
    (Suit::Souzu, 4),
    (Suit::Souzu, 5),
    (Suit::Souzu, 6),
    (Suit::Souzu, 7),
    (Suit::Souzu, 8),
    (Suit::Souzu, 9),
    (Suit::Pinzu, 1),
    (Suit::Pinzu, 2),
    (Suit::Pinzu, 3),
    (Suit::Pinzu, 4),
    (Suit::Pinzu, 5),
    (Suit::Pinzu, 6),
    (Suit::Pinzu, 7),
    (Suit::Pinzu, 8),
    (Suit::Pinzu, 9),
    (Suit::Wind, 1),
    (Suit::Wind, 2),
    (Suit::Wind, 3),
    (Suit::Wind, 4),
    (Suit::Dragon, 1),
    (Suit::Dragon, 2),
    (Suit::Dragon, 3),
    (Suit::Flower, 1),
    (Suit::Flower, 2),
    (Suit::Flower, 3),
    (Suit::Flower, 4),
];

pub fn face_short_name(suit: Suit, rank: u8) -> String {
    match suit {
        Suit::Manzu => format!("{rank} Manzu"),
        Suit::Souzu => format!("{rank} Souzu"),
        Suit::Pinzu => format!("{rank} Pinzu"),
        Suit::Wind => match rank {
            1 => "East".into(),
            2 => "South".into(),
            3 => "West".into(),
            4 => "North".into(),
            _ => format!("Wind {rank}"),
        },
        Suit::Dragon => match rank {
            1 => "Red Dragon".into(),
            2 => "Green Dragon".into(),
            3 => "White Dragon".into(),
            _ => format!("Dragon {rank}"),
        },
        Suit::Flower => match rank {
            1 => "Plum".into(),
            2 => "Orchid".into(),
            3 => "Bamboo".into(),
            4 => "Chrysanthemum".into(),
            _ => format!("Flower {rank}"),
        },
        Suit::Season => format!("Season {rank}"),
    }
}

fn classify_copy(tile: &Tile, debuffs: &[TileDebuff], drawn: bool) -> ModifierBreakdown {
    let mut out = ModifierBreakdown::default();
    if drawn {
        return out;
    }
    match tile.enhancement {
        Some(TileEnhancement::Pearl) => out.pearl = 1,
        Some(TileEnhancement::Gilded) => out.gilded = 1,
        Some(TileEnhancement::Polychrome) => out.polychrome = 1,
        None if debuffs.iter().any(|d| d.matches(tile)) => out.debuffed = 1,
        None => out.plain = 1,
    }
    out
}

fn merge_modifiers(acc: &mut ModifierBreakdown, add: ModifierBreakdown) {
    acc.pearl += add.pearl;
    acc.gilded += add.gilded;
    acc.polychrome += add.polychrome;
    acc.debuffed += add.debuffed;
    acc.plain += add.plain;
}

fn face_counts(
    group: Option<&WallLedgerFaceGroup>,
    debuffs: &[TileDebuff],
) -> (usize, usize, usize, ModifierBreakdown) {
    let Some(group) = group else {
        return (0, 0, 0, ModifierBreakdown::default());
    };
    let mut remaining = 0usize;
    let mut seen = 0usize;
    let mut mods = ModifierBreakdown::default();
    for copy in &group.copies {
        if copy.drawn {
            seen += 1;
        } else {
            remaining += 1;
            merge_modifiers(&mut mods, classify_copy(&copy.tile, debuffs, false));
        }
    }
    (remaining, seen, group.copies.len(), mods)
}

pub fn face_location_counts(
    suit: Suit,
    rank: u8,
    group: Option<&WallLedgerFaceGroup>,
    run: &RunState,
    mode: WallLedgerMode,
) -> TileLocationCounts {
    let in_wall = group
        .map(|g| g.copies.iter().filter(|c| !c.drawn).count())
        .unwrap_or(0);
    if !mode.shows_round_locations() {
        return TileLocationCounts {
            in_wall,
            ..Default::default()
        };
    }
    let in_hand = run
        .hand()
        .iter()
        .filter(|t| t.suit == suit && t.rank == rank)
        .count();
    let played = run
        .structure_tiles()
        .iter()
        .filter(|t| t.suit == suit && t.rank == rank)
        .count();
    let label = Tile::new(suit, rank, 0).label();
    let discarded = run
        .chronicle
        .discards_by_face
        .get(&label)
        .copied()
        .unwrap_or(0) as usize;
    TileLocationCounts {
        in_wall,
        in_hand,
        played,
        discarded,
    }
}

fn suit_bucket(summary: &mut SuitSummary, suit: Suit, count: usize) {
    match suit {
        Suit::Manzu => summary.manzu += count,
        Suit::Souzu => summary.souzu += count,
        Suit::Pinzu => summary.pinzu += count,
        Suit::Wind | Suit::Dragon => summary.honors += count,
        Suit::Flower | Suit::Season => summary.flowers += count,
    }
}

fn standard_face_cap(suit: Suit) -> usize {
    match suit {
        Suit::Flower | Suit::Season => 1,
        _ => 4,
    }
}

pub fn abundance_state(suit: Suit, remaining: usize) -> AbundanceState {
    if remaining == 0 {
        AbundanceState::Exhausted
    } else if is_strategically_thin(suit, remaining) {
        AbundanceState::Thin
    } else if remaining >= 6 {
        AbundanceState::Abundant
    } else {
        AbundanceState::Normal
    }
}

/// UI tint for grid/summary abundance emphasis.
pub fn abundance_color(state: AbundanceState) -> [f32; 4] {
    use crate::render::theme::color;
    match state {
        AbundanceState::Exhausted => color::alpha(color::RUBY, 0.58),
        AbundanceState::Thin => color::alpha(color::GOLD, 0.88),
        AbundanceState::Abundant => color::alpha(color::JADE, 0.94),
        AbundanceState::Normal => color::alpha(color::CHAMPAGNE, 0.92),
    }
}

fn is_strategically_thin(suit: Suit, remaining: usize) -> bool {
    if remaining == 0 {
        return true;
    }
    if matches!(suit, Suit::Flower | Suit::Season) {
        return false;
    }
    remaining == 1
}

fn sequence_waits(
    suit: Suit,
    rank: u8,
    remaining_of: &dyn Fn(Suit, u8) -> usize,
) -> (usize, Vec<String>) {
    if !matches!(suit, Suit::Manzu | Suit::Souzu | Suit::Pinzu) {
        return (0, Vec::new());
    }
    let mut patterns = 0usize;
    let mut hints = Vec::new();
    let lo = rank.saturating_sub(2).max(1);
    let hi = rank.min(7);
    for start in lo..=hi {
        let a = start;
        let b = start + 1;
        let c = start + 2;
        let min_left = remaining_of(suit, a)
            .min(remaining_of(suit, b))
            .min(remaining_of(suit, c));
        if min_left == 0 {
            continue;
        }
        patterns += 1;
        hints.push(format!("{a}-{b}-{c} {}", suit_name(suit)));
    }
    (patterns, hints)
}

fn suit_name(suit: Suit) -> &'static str {
    match suit {
        Suit::Manzu => "Manzu",
        Suit::Souzu => "Souzu",
        Suit::Pinzu => "Pinzu",
        _ => "",
    }
}

fn tile_about(suit: Suit, rank: u8, remaining: usize, _total: usize) -> String {
    match suit {
        Suit::Manzu | Suit::Souzu | Suit::Pinzu => {
            let suit = suit_name(suit);
            if remaining == 0 {
                format!("{rank} {suit} is exhausted from the wall.")
            } else if remaining >= 6 {
                format!(
                    "Strong supply in {suit}. High availability — good for sequences and pairs."
                )
            } else if (2..=8).contains(&rank) {
                format!("Middle tile in {suit}. Forms many sequences when neighbors remain.")
            } else {
                format!("Terminal in {suit}. Pairs and honor-adjacent builds watch rank pressure.")
            }
        }
        Suit::Wind => "Wind honor — pairs and yakuhai directions.".into(),
        Suit::Dragon => "Dragon honor — dragon yaku and pair builds.".into(),
        Suit::Flower => "Flower wildcard — one per meld when substituting.".into(),
        Suit::Season => "Season tile.".into(),
    }
}

fn compute_yaku_hints(entries: &[TileLedgerEntry], summary: &SuitSummary) -> Vec<YakuPathHint> {
    let mut hints = Vec::new();

    let numbered = summary.manzu + summary.souzu + summary.pinzu;
    if numbered > 0 {
        let entries = [
            ("Manzu", summary.manzu),
            ("Souzu", summary.souzu),
            ("Pinzu", summary.pinzu),
        ];
        if let Some((name, count)) = entries.iter().max_by_key(|(_, c)| *c) {
            if *count * 3 > numbered * 2 {
                hints.push(YakuPathHint {
                    label: "Flush".into(),
                    detail: format!("{name} favored"),
                });
            }
        }
    }

    let middle: usize = entries
        .iter()
        .filter(|e| {
            matches!(e.suit, Suit::Manzu | Suit::Souzu | Suit::Pinzu) && (2..=8).contains(&e.rank)
        })
        .map(|e| e.remaining)
        .sum();
    let terminals: usize = entries
        .iter()
        .filter(|e| {
            matches!(e.suit, Suit::Manzu | Suit::Souzu | Suit::Pinzu)
                && (e.rank == 1 || e.rank == 9)
        })
        .map(|e| e.remaining)
        .sum();
    if middle > terminals.saturating_add(8) {
        hints.push(YakuPathHint {
            label: "All Simples".into(),
            detail: "strong".into(),
        });
    } else if terminals > middle {
        hints.push(YakuPathHint {
            label: "Terminals / Honors".into(),
            detail: "favored".into(),
        });
    }

    if summary.honors >= 12 {
        hints.push(YakuPathHint {
            label: "Honor tiles".into(),
            detail: "strong".into(),
        });
    }

    let triplet_faces = entries.iter().filter(|e| e.remaining >= 3).count();
    if triplet_faces >= 8 {
        hints.push(YakuPathHint {
            label: "Triplets".into(),
            detail: "strong".into(),
        });
    } else if triplet_faces <= 3 {
        hints.push(YakuPathHint {
            label: "Triplets".into(),
            detail: "weak".into(),
        });
    }

    for suit in [Suit::Manzu, Suit::Souzu, Suit::Pinzu] {
        let covered: Vec<_> = (1..=9)
            .filter(|r| {
                entries
                    .iter()
                    .any(|e| e.suit == suit && e.rank == *r && e.remaining > 0)
            })
            .collect();
        if covered.len() == 9 {
            hints.push(YakuPathHint {
                label: "Pure Straight".into(),
                detail: format!("possible in {}", suit_name(suit)),
            });
        } else if covered.len() >= 7 {
            let missing: Vec<_> = (1..=9)
                .filter(|r| !covered.contains(r))
                .map(|r| format!("{r} {}", suit_name(suit)))
                .collect();
            if !missing.is_empty() {
                hints.push(YakuPathHint {
                    label: "Pure Straight".into(),
                    detail: format!("possible, missing {}", missing.join(", ")),
                });
            }
        }
    }

    if hints.is_empty() {
        hints.push(YakuPathHint {
            label: "Balanced wall".into(),
            detail: "no strong bias".into(),
        });
    }
    hints
}

pub fn compute_wall_stats(ledger: &WallLedgerReadModel, run: &RunState) -> WallStats {
    let debuffs = &run.tile_debuffs;
    let mut map = std::collections::HashMap::new();
    for g in ledger.standard_groups.iter().chain(&ledger.pack_groups) {
        map.insert((g.suit, g.rank), g);
    }

    let mut entries = Vec::with_capacity(GRID_FACE_ORDER.len());
    let mut global_modifiers = ModifierBreakdown::default();
    let mut suit_summary = SuitSummary::default();

    for &(suit, rank) in &GRID_FACE_ORDER {
        let group = map.get(&(suit, rank)).copied();
        let (remaining, seen, total, mods) = face_counts(group, debuffs);
        let locations = face_location_counts(suit, rank, group, run, ledger.mode);
        merge_modifiers(&mut global_modifiers, mods);
        suit_bucket(&mut suit_summary, suit, remaining);

        entries.push(TileLedgerEntry {
            suit,
            rank,
            remaining,
            seen,
            total,
            locations,
            draw_probability: 0.0,
            wall_share: 0.0,
            abundance: abundance_state(suit, remaining),
            modifiers: mods,
        });
    }

    let total_remaining = ledger.remaining;
    let total_wall = ledger.total;
    let denom = total_remaining.max(1) as f32;
    for e in &mut entries {
        e.draw_probability = e.remaining as f32 / denom;
        e.wall_share = if total_remaining > 0 {
            e.remaining as f32 / denom
        } else {
            0.0
        };
    }

    let remaining_of = |s: Suit, r: u8| -> usize {
        entries
            .iter()
            .find(|e| e.suit == s && e.rank == r)
            .map(|e| e.remaining)
            .unwrap_or(0)
    };

    let mut ranked: Vec<(FaceKey, usize)> = entries
        .iter()
        .filter(|e| e.remaining > standard_face_cap(e.suit))
        .map(|e| {
            (
                FaceKey {
                    suit: e.suit,
                    rank: e.rank,
                },
                e.remaining,
            )
        })
        .collect();
    if ranked.is_empty() {
        ranked = entries
            .iter()
            .filter(|e| e.remaining > 0)
            .map(|e| {
                (
                    FaceKey {
                        suit: e.suit,
                        rank: e.rank,
                    },
                    e.remaining,
                )
            })
            .collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.suit.cmp(&b.0.suit)));
        let top = ranked.first().map(|(_, c)| *c).unwrap_or(0);
        if ranked.iter().all(|(_, c)| *c == top) {
            ranked.clear();
        }
    } else {
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.suit.cmp(&b.0.suit)));
    }
    let most_common: Vec<_> = ranked.iter().take(3).copied().collect();

    let mut thin_exhausted: Vec<(FaceKey, usize)> = entries
        .iter()
        .filter(|e| is_strategically_thin(e.suit, e.remaining))
        .map(|e| {
            (
                FaceKey {
                    suit: e.suit,
                    rank: e.rank,
                },
                e.remaining,
            )
        })
        .collect();
    thin_exhausted.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.suit.cmp(&b.0.suit)));

    let abundant: Vec<_> = entries
        .iter()
        .filter(|e| e.remaining >= 6)
        .map(|e| {
            (
                FaceKey {
                    suit: e.suit,
                    rank: e.rank,
                },
                e.remaining,
            )
        })
        .collect();

    let mut best_scored: Vec<(FaceKey, usize, String)> = Vec::new();
    for e in &entries {
        if e.remaining == 0 {
            continue;
        }
        let (patterns, _) = sequence_waits(e.suit, e.rank, &remaining_of);
        let reason = if patterns >= 2 {
            format!("completes {patterns} waits")
        } else if patterns == 1 {
            "extends 1 sequence".into()
        } else if matches!(e.suit, Suit::Dragon) {
            "supports Dragon yaku".into()
        } else if matches!(e.suit, Suit::Wind) && e.remaining >= 2 {
            "supports wind pairs".into()
        } else if e.remaining >= 6 {
            format!("×{} abundant", e.remaining)
        } else if e.remaining > standard_face_cap(e.suit) {
            format!("×{} surplus", e.remaining)
        } else {
            continue;
        };
        let score = patterns * 100 + e.remaining;
        best_scored.push((
            FaceKey {
                suit: e.suit,
                rank: e.rank,
            },
            score,
            reason,
        ));
    }
    best_scored.sort_by(|a, b| b.1.cmp(&a.1));
    let best_draws: Vec<_> = best_scored
        .into_iter()
        .take(3)
        .map(|(face, _, reason)| BestDrawHint { face, reason })
        .collect();

    let yaku_hints = compute_yaku_hints(&entries, &suit_summary);

    WallStats {
        entries,
        suit_summary,
        total_remaining,
        total_wall,
        most_common,
        thin_exhausted,
        abundant,
        best_draws,
        yaku_hints,
        global_modifiers,
    }
}

pub fn selected_tile_details(
    stats: &WallStats,
    face: FaceKey,
    debuffs: &[TileDebuff],
    group: Option<&WallLedgerFaceGroup>,
) -> Option<SelectedTileDetails> {
    let entry = stats
        .entries
        .iter()
        .find(|e| e.suit == face.suit && e.rank == face.rank)?;
    let (_, _, _, mods) = face_counts(group, debuffs);
    Some(SelectedTileDetails {
        face,
        name: face_short_name(face.suit, face.rank),
        remaining: entry.remaining,
        total: entry.total,
        locations: entry.locations,
        draw_probability: entry.draw_probability,
        wall_share: entry.wall_share,
        modifiers: mods,
        about: tile_about(face.suit, face.rank, entry.remaining, entry.total),
    })
}

pub fn compute_wall_stats_for_run(ledger: &WallLedgerReadModel, run: &RunState) -> WallStats {
    compute_wall_stats(ledger, run)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::deck::{Wall, build_wall};
    use crate::game::wall_ledger::{WallLedgerMode, read_wall_ledger};

    #[test]
    fn stats_remaining_counts_match_ledger() {
        let mut run = RunState::new_with_material(crate::persistence::TileMaterial::Bamboo);
        run.wall = Wall::from_unshuffled(build_wall());
        run.wall.draw();
        let ledger = read_wall_ledger(&run, WallLedgerMode::Live);
        let stats = compute_wall_stats_for_run(&ledger, &run);
        assert_eq!(stats.total_remaining, 139);
        assert_eq!(stats.entries.len(), 38);
        let manzu_5 = stats
            .entries
            .iter()
            .find(|e| e.suit == Suit::Manzu && e.rank == 5)
            .unwrap();
        assert_eq!(manzu_5.remaining, 4);
        assert_eq!(manzu_5.total, 4);
    }

    #[test]
    fn sequence_waits_counts_patterns_not_tile_copies() {
        let run = RunState::new_with_material(crate::persistence::TileMaterial::Bamboo);
        let ledger = read_wall_ledger(&run, WallLedgerMode::ShopPreview);
        let stats = compute_wall_stats_for_run(&ledger, &run);
        let souzu_5 = stats
            .entries
            .iter()
            .find(|e| e.suit == Suit::Souzu && e.rank == 5)
            .unwrap();
        assert_eq!(souzu_5.remaining, 4);
        assert!(
            stats
                .best_draws
                .iter()
                .any(|h| h.reason.contains("3 waits")),
            "middle tile should report 3 sequence patterns"
        );
    }

    #[test]
    fn flowers_are_not_listed_as_thin_at_baseline() {
        let run = RunState::new_with_material(crate::persistence::TileMaterial::Bamboo);
        let ledger = read_wall_ledger(&run, WallLedgerMode::ShopPreview);
        let stats = compute_wall_stats_for_run(&ledger, &run);
        assert!(
            !stats
                .thin_exhausted
                .iter()
                .any(|(f, _)| f.suit == Suit::Flower)
        );
    }

    #[test]
    fn face_location_counts_split_wall_hand_and_discards() {
        let mut run = RunState::new_with_material(crate::persistence::TileMaterial::Bamboo);
        run.wall = Wall::from_unshuffled(build_wall());
        run.wall.draw();
        let drawn = run.wall.all_tiles()[0];
        // The randomly dealt starting hand may already contain copies of
        // `drawn`'s face; clear it so the in_hand count is deterministic.
        run.hand_mut().clear();
        run.hand_mut().push(drawn);
        run.chronicle
            .note_discarded_tile(&Tile::new(Suit::Wind, 1, 0));
        let ledger = read_wall_ledger(&run, WallLedgerMode::Live);
        let group = ledger
            .standard_groups
            .iter()
            .find(|g| g.suit == drawn.suit && g.rank == drawn.rank);
        let loc = face_location_counts(drawn.suit, drawn.rank, group, &run, WallLedgerMode::Live);
        assert_eq!(loc.in_wall, 3);
        assert_eq!(loc.in_hand, 1);
        assert_eq!(loc.discarded, 0);
        let east = face_location_counts(Suit::Wind, 1, None, &run, WallLedgerMode::Live);
        assert_eq!(east.discarded, 1);
    }

    #[test]
    fn shop_preview_hides_round_specific_locations() {
        let mut run = RunState::new_with_material(crate::persistence::TileMaterial::Bamboo);
        run.wall = Wall::from_unshuffled(build_wall());
        run.wall.draw();
        let drawn = run.wall.all_tiles()[0];
        run.hand_mut().push(drawn);
        let ledger = read_wall_ledger(&run, WallLedgerMode::ShopPreview);
        let group = ledger
            .standard_groups
            .iter()
            .find(|g| g.suit == drawn.suit && g.rank == drawn.rank);
        let loc = face_location_counts(
            drawn.suit,
            drawn.rank,
            group,
            &run,
            WallLedgerMode::ShopPreview,
        );
        assert_eq!(loc.in_wall, 4);
        assert_eq!(loc.in_hand, 0);
        assert_eq!(loc.played, 0);
        assert_eq!(loc.discarded, 0);
    }
}
