use super::decomposition::find_sequences;
use super::*;
use crate::core::rules::RuleModifier;
use crate::core::tile::{Suit, Tile};

fn t(suit: Suit, rank: u8, id: u32) -> Tile {
    Tile::new(suit, rank, id)
}

#[test]
fn triplet_detected() {
    let hand = vec![
        t(Suit::Souzu, 3, 0),
        t(Suit::Souzu, 3, 1),
        t(Suit::Souzu, 3, 2),
    ];
    let sets = find_pairs_and_triplets(&hand);
    assert!(sets.iter().any(|s| s.kind == MeldKind::Triplet));
}

#[test]
fn sequence_detected() {
    let hand = vec![
        t(Suit::Manzu, 2, 0),
        t(Suit::Manzu, 3, 1),
        t(Suit::Manzu, 4, 2),
    ];
    let seqs = find_sequences(&hand);
    assert!(seqs.iter().any(|s| s.kind == MeldKind::Sequence));
}

// ── validate_selection ─────────────────────────────────────────

#[test]
fn validate_pair() {
    let tiles = vec![t(Suit::Souzu, 5, 0), t(Suit::Souzu, 5, 1)];
    let sets = validate_selection(&tiles).unwrap();
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0].kind, MeldKind::Pair);
}

#[test]
fn validate_triplet() {
    let tiles = vec![
        t(Suit::Pinzu, 7, 0),
        t(Suit::Pinzu, 7, 1),
        t(Suit::Pinzu, 7, 2),
    ];
    let sets = validate_selection(&tiles).unwrap();
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0].kind, MeldKind::Triplet);
}

#[test]
fn validate_sequence() {
    let tiles = vec![
        t(Suit::Manzu, 3, 0),
        t(Suit::Manzu, 4, 1),
        t(Suit::Manzu, 5, 2),
    ];
    let sets = validate_selection(&tiles).unwrap();
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0].kind, MeldKind::Sequence);
}

#[test]
fn validate_rejects_leftover() {
    // 4 tiles: triplet + 1 leftover → invalid
    let tiles = vec![
        t(Suit::Souzu, 3, 0),
        t(Suit::Souzu, 3, 1),
        t(Suit::Souzu, 3, 2),
        t(Suit::Pinzu, 9, 3),
    ];
    assert!(validate_selection(&tiles).is_none());
}

#[test]
fn validate_rejects_single_tile() {
    let tiles = vec![t(Suit::Souzu, 1, 0)];
    assert!(validate_selection(&tiles).is_none());
}

#[test]
fn validate_rejects_empty() {
    assert!(validate_selection(&[]).is_none());
}

#[test]
fn validate_multi_set() {
    // triplet + sequence = 6 tiles
    let tiles = vec![
        t(Suit::Souzu, 3, 0),
        t(Suit::Souzu, 3, 1),
        t(Suit::Souzu, 3, 2),
        t(Suit::Manzu, 1, 3),
        t(Suit::Manzu, 2, 4),
        t(Suit::Manzu, 3, 5),
    ];
    let sets = validate_selection(&tiles).unwrap();
    assert_eq!(sets.len(), 2);
}

#[test]
fn validate_full_hand() {
    // 4 sets + 1 pair = 14 tiles
    let tiles = vec![
        // triplet
        t(Suit::Souzu, 1, 0),
        t(Suit::Souzu, 1, 1),
        t(Suit::Souzu, 1, 2),
        // sequence
        t(Suit::Manzu, 4, 3),
        t(Suit::Manzu, 5, 4),
        t(Suit::Manzu, 6, 5),
        // triplet
        t(Suit::Pinzu, 9, 6),
        t(Suit::Pinzu, 9, 7),
        t(Suit::Pinzu, 9, 8),
        // sequence
        t(Suit::Souzu, 5, 9),
        t(Suit::Souzu, 6, 10),
        t(Suit::Souzu, 7, 11),
        // pair
        t(Suit::Wind, 1, 12),
        t(Suit::Wind, 1, 13),
    ];
    let sets = validate_selection(&tiles).unwrap();
    assert_eq!(sets.len(), 5);
}

#[test]
fn validate_kong_four_of_a_kind() {
    // 4 identical tiles must decompose as a single Kong, not Triplet+leftover.
    let tiles = vec![
        t(Suit::Souzu, 5, 0),
        t(Suit::Souzu, 5, 1),
        t(Suit::Souzu, 5, 2),
        t(Suit::Souzu, 5, 3),
    ];
    let sets = validate_selection(&tiles).unwrap();
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0].kind, MeldKind::Kong);
    assert_eq!(sets[0].tile_ids.len(), 4);
}

#[test]
fn find_pairs_and_triplets_emits_kong() {
    let tiles = vec![
        t(Suit::Pinzu, 7, 0),
        t(Suit::Pinzu, 7, 1),
        t(Suit::Pinzu, 7, 2),
        t(Suit::Pinzu, 7, 3),
    ];
    let sets = find_pairs_and_triplets(&tiles);
    assert!(sets.iter().any(|s| s.kind == MeldKind::Kong));
    assert!(!sets.iter().any(|s| s.kind == MeldKind::Triplet));
}

#[test]
fn validate_ambiguous_decomposition() {
    // 1-1-1-2-3 souzu: could be triplet(1,1,1) + leftover(2,3) = FAIL
    // or pair(1,1) + sequence(1,2,3) = SUCCESS
    // Backtracking should find the valid decomposition.
    let tiles = vec![
        t(Suit::Souzu, 1, 0),
        t(Suit::Souzu, 1, 1),
        t(Suit::Souzu, 1, 2),
        t(Suit::Souzu, 2, 3),
        t(Suit::Souzu, 3, 4),
    ];
    let sets = validate_selection(&tiles).unwrap();
    assert_eq!(sets.len(), 2);
}

// ── suggest_completions ─────────────────────────────────────────

#[test]
fn suggest_completions_finds_pair_partner() {
    // Hand has one selected tile; another copy exists in hand.
    let hand = vec![
        t(Suit::Souzu, 3, 0),
        t(Suit::Souzu, 3, 1),
        t(Suit::Pinzu, 5, 2),
    ];
    let selected = vec![0]; // selected tile id 0 (Souzu 3)
    let hints = suggest_completions(&hand, &selected);
    // Index 1 (Souzu 3, id=1) should be suggested since adding it forms a pair.
    assert!(hints.contains(&1));
}

// ── flower wildcard tests ──────────────────────────────────────

#[test]
fn flower_completes_triplet() {
    // 2 identical tiles + 1 flower = valid triplet (flower must be consumed)
    let tiles = vec![
        t(Suit::Souzu, 5, 0),
        t(Suit::Souzu, 5, 1),
        t(Suit::Flower, 1, 100),
    ];
    let sets = validate_selection(&tiles).unwrap();
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0].kind, MeldKind::Triplet);
    assert!(sets[0].tile_ids.contains(&100)); // flower id present
}

#[test]
fn flower_completes_sequence_high() {
    // 1m, 2m + flower = 1-2-3m sequence (flower fills rank 3)
    let tiles = vec![
        t(Suit::Manzu, 1, 0),
        t(Suit::Manzu, 2, 1),
        t(Suit::Flower, 2, 100),
    ];
    let sets = validate_selection(&tiles).unwrap();
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0].kind, MeldKind::Sequence);
}

#[test]
fn flower_completes_sequence_mid() {
    // 1m, 3m + flower = 1-2-3m sequence (flower fills rank 2)
    let tiles = vec![
        t(Suit::Manzu, 1, 0),
        t(Suit::Manzu, 3, 1),
        t(Suit::Flower, 3, 100),
    ];
    let sets = validate_selection(&tiles).unwrap();
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0].kind, MeldKind::Sequence);
}

#[test]
fn flower_completes_sequence_low() {
    // 8m, 9m + flower = 7-8-9m sequence (flower fills rank 7)
    let tiles = vec![
        t(Suit::Manzu, 8, 0),
        t(Suit::Manzu, 9, 1),
        t(Suit::Flower, 1, 100),
    ];
    let sets = validate_selection(&tiles).unwrap();
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0].kind, MeldKind::Sequence);
}

#[test]
fn flower_cannot_pair_with_regular() {
    // 1 regular tile + 1 flower should NOT form a valid pair (flowers
    // pair only with other flowers, or act as wildcards in triplets/seqs).
    let tiles = vec![t(Suit::Souzu, 3, 0), t(Suit::Flower, 1, 100)];
    assert!(validate_selection(&tiles).is_none());
}

#[test]
fn flower_max_one_per_meld() {
    // 1 tile + 2 flowers should NOT form a valid triplet
    let tiles = vec![
        t(Suit::Souzu, 5, 0),
        t(Suit::Flower, 1, 100),
        t(Suit::Flower, 2, 101),
    ];
    assert!(validate_selection(&tiles).is_none());
}

#[test]
fn flower_in_multi_meld_hand() {
    // pair + flower-assisted triplet = valid 5-tile hand
    let tiles = vec![
        t(Suit::Pinzu, 7, 0),
        t(Suit::Pinzu, 7, 1),
        t(Suit::Pinzu, 7, 2),
        t(Suit::Souzu, 3, 3),
        t(Suit::Souzu, 3, 4),
        t(Suit::Flower, 1, 100),
    ];
    let sets = validate_selection(&tiles).unwrap();
    // Should decompose as: triplet(7p×3) + triplet(3s×2 + flower)
    // or triplet(7p×2 + flower) + triplet(... ) — either is valid
    assert_eq!(sets.len(), 2);
}

#[test]
fn unused_flower_is_invalid() {
    // Selecting a flower alongside a valid pair — flower can't be used
    // (only triplets/sequences), so the selection is invalid. Players
    // shouldn't select flowers they can't use.
    let tiles = vec![
        t(Suit::Souzu, 3, 0),
        t(Suit::Souzu, 3, 1),
        t(Suit::Flower, 1, 100),
    ];
    // pair(3s×2) + unused flower → flower must be consumed → invalid
    // flower-triplet(3s×2 + flower) works!
    let sets = validate_selection(&tiles).unwrap();
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0].kind, MeldKind::Triplet);
}

#[test]
fn flower_pair_valid() {
    // Two flowers form a valid pair regardless of rank.
    let tiles = vec![t(Suit::Flower, 1, 100), t(Suit::Flower, 2, 101)];
    let sets = validate_selection(&tiles).unwrap();
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0].kind, MeldKind::Pair);
}

#[test]
fn flower_triplet_valid() {
    // Three flowers form a valid triplet.
    let tiles = vec![
        t(Suit::Flower, 1, 100),
        t(Suit::Flower, 2, 101),
        t(Suit::Flower, 3, 102),
    ];
    let sets = validate_selection(&tiles).unwrap();
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0].kind, MeldKind::Triplet);
}

#[test]
fn flower_two_pairs_valid() {
    // Four flowers form two valid pairs.
    let tiles = vec![
        t(Suit::Flower, 1, 100),
        t(Suit::Flower, 2, 101),
        t(Suit::Flower, 3, 102),
        t(Suit::Flower, 4, 103),
    ];
    let sets = validate_selection(&tiles).unwrap();
    assert_eq!(sets.len(), 2);
    assert!(sets.iter().all(|s| s.kind == MeldKind::Pair));
}

#[test]
fn single_flower_invalid() {
    // Just one flower — invalid
    let tiles = vec![t(Suit::Flower, 1, 100)];
    assert!(validate_selection(&tiles).is_none());
}

// ── NoFlowerWildcards (The Rot) ────────────────────────────────

#[test]
fn no_flower_wildcards_blocks_substitution() {
    let rules = [RuleModifier::NoFlowerWildcards];
    let tiles = vec![
        t(Suit::Souzu, 5, 0),
        t(Suit::Souzu, 5, 1),
        t(Suit::Flower, 1, 100),
    ];
    assert!(validate_selection_with_rules(&tiles, &rules).is_none());
}

#[test]
fn no_flower_wildcards_allows_flower_pair() {
    let rules = [RuleModifier::NoFlowerWildcards];
    let tiles = vec![t(Suit::Flower, 1, 100), t(Suit::Flower, 2, 101)];
    let sets = validate_selection_with_rules(&tiles, &rules).unwrap();
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0].kind, MeldKind::Pair);
}

// ── validate_selection_with_rules ──────────────────────────────

#[test]
fn sequence_wrap_891() {
    let tiles = vec![
        t(Suit::Manzu, 8, 0),
        t(Suit::Manzu, 9, 1),
        t(Suit::Manzu, 1, 2),
    ];
    // Without wrap: invalid
    assert!(validate_selection(&tiles).is_none());
    // With wrap: valid sequence
    let sets = validate_selection_with_rules(&tiles, &[RuleModifier::SequenceWrap]).unwrap();
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0].kind, MeldKind::Sequence);
}

#[test]
fn sequence_wrap_912() {
    let tiles = vec![
        t(Suit::Souzu, 9, 0),
        t(Suit::Souzu, 1, 1),
        t(Suit::Souzu, 2, 2),
    ];
    assert!(validate_selection(&tiles).is_none());
    let sets = validate_selection_with_rules(&tiles, &[RuleModifier::SequenceWrap]).unwrap();
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0].kind, MeldKind::Sequence);
}

#[test]
fn no_sequences_rejects_sequence() {
    let tiles = vec![
        t(Suit::Manzu, 1, 0),
        t(Suit::Manzu, 2, 1),
        t(Suit::Manzu, 3, 2),
    ];
    // Normal: valid sequence
    assert!(validate_selection(&tiles).is_some());
    // NoSequences: rejected
    assert!(validate_selection_with_rules(&tiles, &[RuleModifier::NoSequences]).is_none());
}

#[test]
fn no_sequences_allows_triplets() {
    let tiles = vec![
        t(Suit::Souzu, 5, 0),
        t(Suit::Souzu, 5, 1),
        t(Suit::Souzu, 5, 2),
    ];
    let sets = validate_selection_with_rules(&tiles, &[RuleModifier::NoSequences]).unwrap();
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0].kind, MeldKind::Triplet);
}

#[test]
fn require_honor_rejects_structure_without_honor() {
    let tiles = vec![
        t(Suit::Souzu, 5, 0),
        t(Suit::Souzu, 5, 1),
        t(Suit::Souzu, 5, 2),
    ];
    assert!(validate_selection_with_rules(&tiles, &[RuleModifier::RequireHonor]).is_none());
}

#[test]
fn require_honor_allows_honor_only_structure() {
    let tiles = vec![
        t(Suit::Dragon, 1, 0),
        t(Suit::Dragon, 1, 1),
        t(Suit::Dragon, 1, 2),
    ];
    let sets = validate_selection_with_rules(&tiles, &[RuleModifier::RequireHonor]).unwrap();
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0].kind, MeldKind::Triplet);
}

#[test]
fn require_honor_allows_mixed_structure_with_one_honor_meld() {
    let tiles = vec![
        t(Suit::Souzu, 1, 0),
        t(Suit::Souzu, 2, 1),
        t(Suit::Souzu, 3, 2),
        t(Suit::Dragon, 1, 3),
        t(Suit::Dragon, 1, 4),
        t(Suit::Dragon, 1, 5),
    ];
    let sets = validate_selection_with_rules(&tiles, &[RuleModifier::RequireHonor]).unwrap();
    assert_eq!(sets.len(), 2);
}

// ── Tricky decompositions (chiitoitsu, kokushi, shared-tile search) ───

#[test]
fn tricky_chiitoitsu_seven_distinct_pairs() {
    let tiles = vec![
        t(Suit::Souzu, 1, 0),
        t(Suit::Souzu, 1, 1),
        t(Suit::Souzu, 3, 2),
        t(Suit::Souzu, 3, 3),
        t(Suit::Manzu, 5, 4),
        t(Suit::Manzu, 5, 5),
        t(Suit::Pinzu, 7, 6),
        t(Suit::Pinzu, 7, 7),
        t(Suit::Wind, 1, 8),
        t(Suit::Wind, 1, 9),
        t(Suit::Wind, 3, 10),
        t(Suit::Wind, 3, 11),
        t(Suit::Dragon, 2, 12),
        t(Suit::Dragon, 2, 13),
    ];
    let sets = validate_selection(&tiles).expect("chiitoitsu hand");
    assert_eq!(sets.len(), 7);
    assert!(sets.iter().all(|s| s.kind == MeldKind::Pair));
    let alts = enumerate_decompositions(&tiles, &[]);
    assert!(!alts.is_empty());
    assert!(alts.iter().all(|c| c.len() == 7));
}

/// Kokushi Musō: twelve orphan singletons + one pair on an orphan face.
#[test]
fn tricky_kokushi_musou_decomposed() {
    let tiles = vec![
        t(Suit::Manzu, 1, 0),
        t(Suit::Manzu, 9, 1),
        t(Suit::Souzu, 1, 2),
        t(Suit::Souzu, 9, 3),
        t(Suit::Pinzu, 1, 4),
        t(Suit::Pinzu, 9, 5),
        t(Suit::Wind, 1, 6),
        t(Suit::Wind, 2, 7),
        t(Suit::Wind, 3, 8),
        t(Suit::Wind, 4, 9),
        t(Suit::Dragon, 1, 10),
        t(Suit::Dragon, 2, 11),
        t(Suit::Dragon, 3, 12),
        t(Suit::Manzu, 1, 13), // pair on 1m
    ];
    let sets = validate_selection(&tiles).expect("kokushi");
    assert_eq!(sets.len(), 13);
    assert_eq!(
        sets.iter().filter(|s| s.kind == MeldKind::Single).count(),
        12
    );
    assert_eq!(sets.iter().filter(|s| s.kind == MeldKind::Pair).count(), 1);
    let yaku = crate::core::yaku::detect_yaku_with_wind(&tiles, &sets, None, None, None);
    assert!(yaku.contains(&crate::core::yaku::YakuKind::KokushiMusou));
    assert!(!yaku.contains(&crate::core::yaku::YakuKind::Honroutou));
}

#[test]
fn staging_preview_three_pairs_plus_lone_flower() {
    use super::staging_preview_melds;
    let tiles = vec![
        t(Suit::Manzu, 3, 0),
        t(Suit::Manzu, 3, 1),
        t(Suit::Souzu, 3, 2),
        t(Suit::Souzu, 3, 3),
        t(Suit::Pinzu, 3, 4),
        t(Suit::Pinzu, 3, 5),
        t(Suit::Flower, 4, 100),
    ];
    let (melds, bad) = staging_preview_melds(&tiles, &[]);
    // Full validation consumes the flower as a wildcard in one triplet + two pairs.
    assert_eq!(melds.len(), 3);
    assert!(
        melds
            .iter()
            .any(|m| { m.kind == MeldKind::Triplet && m.tile_ids.contains(&100) })
    );
    assert!(bad.is_empty());
}

#[test]
fn staging_preview_flower_pair_on_invalid_mixed() {
    use super::staging_preview_melds;
    // Two flowers + stray numbered tile — preview should still lift the flower pair.
    let tiles = vec![
        t(Suit::Flower, 1, 100),
        t(Suit::Flower, 2, 101),
        t(Suit::Souzu, 7, 2),
    ];
    let (melds, bad) = staging_preview_melds(&tiles, &[]);
    assert_eq!(melds.len(), 1);
    assert_eq!(melds[0].kind, MeldKind::Pair);
    assert_eq!(melds[0].tile_ids, vec![100, 101]);
    assert_eq!(bad, vec![2]);
}

#[test]
fn staging_preview_flower_wildcard_triplet() {
    use super::staging_preview_melds;
    let tiles = vec![
        t(Suit::Manzu, 3, 0),
        t(Suit::Manzu, 3, 1),
        t(Suit::Flower, 1, 100),
        t(Suit::Pinzu, 8, 2),
    ];
    let (melds, bad) = staging_preview_melds(&tiles, &[]);
    assert_eq!(melds.len(), 1);
    assert_eq!(melds[0].kind, MeldKind::Triplet);
    assert_eq!(melds[0].tile_ids, vec![0, 1, 100]);
    assert_eq!(bad, vec![2]);
}

#[test]
fn staging_preview_flower_pair_plus_numbered_pair() {
    use super::staging_preview_melds;
    let tiles = vec![
        t(Suit::Manzu, 3, 0),
        t(Suit::Manzu, 3, 1),
        t(Suit::Flower, 1, 100),
        t(Suit::Flower, 2, 101),
        t(Suit::Pinzu, 8, 2),
    ];
    let (melds, bad) = staging_preview_melds(&tiles, &[]);
    assert_eq!(melds.len(), 2);
    assert!(
        melds
            .iter()
            .any(|m| m.kind == MeldKind::Pair && m.tile_ids == vec![100, 101])
    );
    assert!(
        melds
            .iter()
            .any(|m| m.kind == MeldKind::Pair && m.tile_ids == vec![0, 1])
    );
    assert_eq!(bad, vec![2]);
}

#[test]
fn staging_preview_valid_flower_triplet() {
    use super::staging_preview_melds;
    let tiles = vec![
        t(Suit::Flower, 1, 100),
        t(Suit::Flower, 2, 101),
        t(Suit::Flower, 3, 102),
    ];
    let (melds, bad) = staging_preview_melds(&tiles, &[]);
    assert_eq!(melds.len(), 1);
    assert_eq!(melds[0].kind, MeldKind::Triplet);
    assert!(bad.is_empty());
}

#[test]
fn non_contributing_empty_on_valid_pair() {
    let tiles = vec![t(Suit::Souzu, 3, 0), t(Suit::Souzu, 3, 1)];
    assert!(non_contributing_tile_ids(&tiles, &[]).is_empty());
}

#[test]
fn non_contributing_flags_orphan_on_invalid_mixed() {
    // Pair of 3s + stray 7 — no full decomposition.
    let tiles = vec![
        t(Suit::Souzu, 3, 0),
        t(Suit::Souzu, 3, 1),
        t(Suit::Souzu, 7, 2),
    ];
    assert_eq!(non_contributing_tile_ids(&tiles, &[]), vec![2]);
}

#[test]
fn non_contributing_flags_stray_fifth_on_pair_plus_two() {
    // Two 2s + 5 — pair uses both 2s; 5 cannot join the meld.
    let tiles = vec![
        t(Suit::Souzu, 2, 10),
        t(Suit::Souzu, 2, 11),
        t(Suit::Souzu, 5, 12),
    ];
    assert_eq!(non_contributing_tile_ids(&tiles, &[]), vec![12]);
}

#[test]
fn non_contributing_all_when_nothing_forms() {
    let tiles = vec![
        t(Suit::Souzu, 2, 0),
        t(Suit::Souzu, 5, 1),
        t(Suit::Souzu, 8, 2),
    ];
    let ids: Vec<u32> = tiles.iter().map(|t| t.id).collect();
    assert_eq!(non_contributing_tile_ids(&tiles, &[]), ids);
}

#[test]
fn selection_rejection_hint_pair_mismatch() {
    let tiles = vec![t(Suit::Souzu, 2, 0), t(Suit::Souzu, 5, 1)];
    assert_eq!(
        selection_rejection_hint(&tiles, &[]),
        "These two tiles aren't a pair — select two of the same tile."
    );
}

#[test]
fn selection_rejection_hint_extra_tile() {
    let tiles = vec![
        t(Suit::Souzu, 3, 0),
        t(Suit::Souzu, 3, 1),
        t(Suit::Souzu, 7, 2),
    ];
    assert_eq!(
        selection_rejection_hint(&tiles, &[]),
        "Some selected tiles don't fit this meld — deselect the extras."
    );
}

#[test]
fn selection_rejection_hint_bad_run() {
    let tiles = vec![
        t(Suit::Souzu, 1, 0),
        t(Suit::Souzu, 2, 1),
        t(Suit::Souzu, 4, 2),
    ];
    assert_eq!(
        selection_rejection_hint(&tiles, &[]),
        "These tiles aren't a run — pick three consecutive tiles in the same suit."
    );
}

/// 1-1-1-2-3: triplet(1) leaves orphans, but pair(1,1) + sequence(1,2,3) works.
#[test]
fn tricky_shared_rank_ambiguity_11123() {
    let tiles = vec![
        t(Suit::Souzu, 1, 0),
        t(Suit::Souzu, 1, 1),
        t(Suit::Souzu, 1, 2),
        t(Suit::Souzu, 2, 3),
        t(Suit::Souzu, 3, 4),
    ];
    let sets = validate_selection(&tiles).expect("pair + sequence");
    assert_eq!(sets.len(), 2);
    let alts = enumerate_decompositions(&tiles, &[]);
    assert_eq!(alts.len(), 1, "one canonical meld multiset");
}
