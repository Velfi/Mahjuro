#[cfg(test)]
mod cases {
    use std::collections::BTreeMap;

    use crate::OrdealKindExt;
    use crate::core::consumable::{Consumable, ConsumableInventory};
    use crate::core::debuff::{TileDebuff, TileDebuffClass};
    use crate::core::deck::Wall;
    use crate::core::deck::build_wall;
    use crate::core::hand::{DetectedMeld, MeldKind};
    use crate::core::memorial_talisman::MemorialTalismanKind;
    use crate::core::ordeal::{self, OrdealKind};
    use crate::core::relic::RelicState;
    use crate::core::relic::{
        RelicId, ScoreContext, ScoreEconomyBundle, ScorePatternBundle, ScoreRelicBundle,
        ScoreRoundBundle, ScoreTileBundle,
    };
    use crate::core::rules::RuleModifier;
    use crate::core::tile::{Suit, Tile};
    use crate::game::event_bus::{EventBus, GameEvent, GameOverReason};
    use crate::game::game_mode::{GameMode, HAND_SIZE};
    use crate::game::run::{ChamberKind, OrdealState, RunState, default_available_relics};

    /// Standard mode starting plays (Bamboo: 4 base + 1 bonus).
    const STARTING_PLAYS: u32 = 5;
    /// Standard mode starting discards (Bamboo: 4 base + 0 bonus).
    const STARTING_DISCARDS: u32 = 4;

    // Create a RunState with a deterministic (unshuffled) wall for predictable tests.
    fn test_run() -> RunState {
        let tiles = build_wall(); // deterministic order: Char 1-9, Bam 1-9, Cir 1-9, Winds, Dragons
        let mut wall = Wall::from_unshuffled(tiles);
        let mut hand = Vec::with_capacity(HAND_SIZE);
        for _ in 0..HAND_SIZE {
            if let Some(t) = wall.draw() {
                hand.push(t);
            }
        }
        let selected = vec![false; hand.len()];
        let mode = GameMode {
            starting_yen: 0,
            starting_yaku: vec![],
            ..GameMode::standard()
        };
        RunState {
            wing: 1,
            available_yaku: vec![],
            available_relics: default_available_relics(),
            base_target: mode.base_target,
            chamber: ChamberKind::Small,
            ordeal: OrdealState::default(),
            consumables: crate::core::consumable::ConsumableInventory::default(),
            discards_remaining: mode.starting_discards,
            discards_max: mode.starting_discards,
            full_hand_played_this_round: false,
            yen: mode.starting_yen as i32,
            hand,
            structure_sets: vec![],
            structure_tiles: vec![],
            joker_extra_faces: vec![],
            last_breakdown: None,
            mode: mode.clone(),
            auto_cash_in_on_full_structure: true,
            played_yaku_this_round: vec![],
            tile_debuffs: vec![],
            honors_scored_this_round: false,
            windreader_bonus_wind: None,
            yaku_times_played: rustc_hash::FxHashMap::default(),
            profile_yaku_scored: rustc_hash::FxHashSet::default(),
            tiles_played: 0,
            tiles_discarded: 0,
            times_restocked: 0,
            best_structure_score: 0,
            best_structure_name: String::new(),
            best_hand_tiles: Vec::new(),
            score_after_wing: Vec::new(),
            plays_remaining: mode.starting_plays,
            plays_max: mode.starting_plays,
            relics: RelicState::default(),
            round_rules: vec![],
            round_score: 0,
            run_number: 1,
            scored_last_turn: false,
            selected,
            target_score: mode.base_target,
            tile_enhancements: BTreeMap::new(),
            transformed_tiles: BTreeMap::new(),
            global_buff_enhancement: None,
            removed_tile_ids: rustc_hash::FxHashSet::default(),
            decimations_used: 0,
            upcoming_chamber: ChamberKind::Small,
            wall,
            yaku_levels: crate::core::zodiac::YakuLevels::default(),
            tile_packs: vec![],
            total_score_earned: 0,
            paper_lantern_extinct: false,
            silk_thread_extinct: false,
            melting_ice_extinct: false,
            xxxl_egg_extinct: false,
            tea_ceremony_extinct: false,
            chrysalis_extinct: false,
            small_chamber_tag: None,
            big_chamber_tag: None,
            tag_free_restock: 0,
            tag_patron_gift: 0,
            tag_rich_stock: 0,
            tag_bonus_plays: 0,
            tag_bonus_discards: 0,
            tag_bonus_hand_size: 0,
            pending_zodiac_celebrations: Vec::new(),
            finished_zodiac_celebration: None,
            pending_shop_focus_snap_after_celebration: false,
            relic_counters: BTreeMap::new(),
            onboarding: None,
            relic_activations: Vec::new(),
            defeat_journal: crate::core::memorial_talisman::RunDefeatJournal::default(),
            memorial_granted: false,
            memorial_snapshot: None,
            memorial_round: crate::core::memorial_talisman::MemorialRoundState::default(),
            defeat_memorial_kind: None,
            chronicle: crate::core::run_chronicle::RunChronicle::default(),
            suppress_chamber_resolution: false,
            round_end_queued: false,
            discard_refill_pending: false,
        }
    }

    fn bus() -> EventBus {
        EventBus::default()
    }

    fn winning_structure() -> (Vec<Tile>, Vec<DetectedMeld>) {
        let tiles = vec![
            Tile::new(Suit::Manzu, 1, 1),
            Tile::new(Suit::Manzu, 1, 2),
            Tile::new(Suit::Manzu, 2, 3),
            Tile::new(Suit::Manzu, 3, 4),
            Tile::new(Suit::Manzu, 4, 5),
            Tile::new(Suit::Pinzu, 2, 6),
            Tile::new(Suit::Pinzu, 3, 7),
            Tile::new(Suit::Pinzu, 4, 8),
            Tile::new(Suit::Souzu, 5, 9),
            Tile::new(Suit::Souzu, 6, 10),
            Tile::new(Suit::Souzu, 7, 11),
            Tile::new(Suit::Wind, 1, 12),
            Tile::new(Suit::Wind, 1, 13),
            Tile::new(Suit::Wind, 1, 14),
        ];
        let sets = vec![
            DetectedMeld {
                kind: MeldKind::Pair,
                tile_ids: vec![1, 2],
            },
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![3, 4, 5],
            },
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![6, 7, 8],
            },
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![9, 10, 11],
            },
            DetectedMeld {
                kind: MeldKind::Triplet,
                tile_ids: vec![12, 13, 14],
            },
        ];
        (tiles, sets)
    }

    // ── toggle_select ───────────────────────────────────────────────

    #[test]
    fn toggle_select_marks_tile() {
        let mut run = test_run();
        assert!(!run.selected[0]);
        run.toggle_select(0);
        assert!(run.selected[0]);
    }

    #[test]
    fn toggle_select_unmarks_tile() {
        let mut run = test_run();
        run.toggle_select(3);
        assert!(run.selected[3]);
        run.toggle_select(3);
        assert!(!run.selected[3]);
    }

    #[test]
    fn toggle_select_out_of_bounds_is_noop() {
        let mut run = test_run();
        run.toggle_select(999); // should not panic
        assert_eq!(run.selected_count(), 0);
    }

    #[test]
    fn toggle_select_multiple_tiles() {
        let mut run = test_run();
        run.toggle_select(0);
        run.toggle_select(5);
        run.toggle_select(13);
        assert_eq!(run.selected_count(), 3);
    }

    // ── clear_selection ─────────────────────────────────────────────

    #[test]
    fn clear_selection_resets_all() {
        let mut run = test_run();
        run.toggle_select(0);
        run.toggle_select(7);
        run.toggle_select(12);
        assert_eq!(run.selected_count(), 3);
        run.clear_selection();
        assert_eq!(run.selected_count(), 0);
        assert!(run.selected.iter().all(|&s| !s));
    }

    #[test]
    fn clear_selection_on_empty_is_noop() {
        let mut run = test_run();
        run.clear_selection(); // should not panic
        assert_eq!(run.selected_count(), 0);
    }

    // ── selected_count ──────────────────────────────────────────────

    #[test]
    fn selected_count_starts_at_zero() {
        let run = test_run();
        assert_eq!(run.selected_count(), 0);
    }

    #[test]
    fn selected_count_tracks_toggles() {
        let mut run = test_run();
        run.toggle_select(0);
        assert_eq!(run.selected_count(), 1);
        run.toggle_select(1);
        assert_eq!(run.selected_count(), 2);
        run.toggle_select(0);
        assert_eq!(run.selected_count(), 1);
    }

    // ── discard_selected ────────────────────────────────────────────

    #[test]
    fn discard_selected_removes_tiles_and_redraws() {
        let mut run = test_run();
        let mut bus = bus();
        let original_hand = run.hand.clone();

        run.toggle_select(0);
        run.toggle_select(1);
        let discarded = run.discard_selected(&mut bus);

        assert_eq!(discarded, 2);
        assert_eq!(run.hand.len(), HAND_SIZE); // auto-drew back to full
        // The first two tiles should be gone.
        assert!(!run.hand.contains(&original_hand[0]));
        assert!(!run.hand.contains(&original_hand[1]));
    }

    #[test]
    fn discard_selected_costs_one_discard() {
        let mut run = test_run();
        let mut bus = bus();
        assert_eq!(run.discards_remaining, STARTING_DISCARDS);

        run.toggle_select(0);
        run.toggle_select(1);
        run.toggle_select(2);
        run.discard_selected(&mut bus);

        assert_eq!(run.discards_remaining, STARTING_DISCARDS - 1);
    }

    #[test]
    fn discard_selected_clears_selection_after() {
        let mut run = test_run();
        let mut bus = bus();

        run.toggle_select(0);
        run.toggle_select(5);
        run.discard_selected(&mut bus);

        assert_eq!(run.selected_count(), 0);
        assert_eq!(run.selected.len(), run.hand.len());
    }

    #[test]
    fn discard_selected_returns_zero_when_none_selected() {
        let mut run = test_run();
        let mut bus = bus();
        let discarded = run.discard_selected(&mut bus);
        assert_eq!(discarded, 0);
        assert_eq!(run.discards_remaining, STARTING_DISCARDS); // not decremented
    }

    #[test]
    fn discard_selected_returns_zero_when_no_discards_left() {
        let mut run = test_run();
        let mut bus = bus();
        run.discards_remaining = 0;

        run.toggle_select(0);
        let discarded = run.discard_selected(&mut bus);

        assert_eq!(discarded, 0);
        assert_eq!(run.hand.len(), HAND_SIZE); // hand unchanged
    }

    #[test]
    fn discard_selected_emits_events() {
        let mut run = test_run();
        let mut bus = bus();

        run.toggle_select(2);
        run.toggle_select(4);
        run.discard_selected(&mut bus);

        let events: Vec<_> = bus.drain().collect();
        // Should have TileDiscarded events + TileDrawn events for the redraws.
        let discards: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, GameEvent::TileDiscarded))
            .collect();
        let draws: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, GameEvent::TileDrawn))
            .collect();
        assert_eq!(discards.len(), 2);
        assert_eq!(draws.len(), 2); // drew 2 to replace the 2 discarded
    }

    #[test]
    fn discard_selected_preserves_non_selected_tiles() {
        let mut run = test_run();
        let mut bus = bus();

        // Remember non-selected tile ids.
        let kept_ids: Vec<u32> = run
            .hand
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != 3 && *i != 7)
            .map(|(_, t)| t.id)
            .collect();

        run.toggle_select(3);
        run.toggle_select(7);
        run.discard_selected(&mut bus);

        // All originally-kept tiles should still be in hand.
        for id in &kept_ids {
            assert!(
                run.hand.iter().any(|t| t.id == *id),
                "tile id {} was lost",
                id
            );
        }
    }

    #[test]
    fn multiple_discard_rounds() {
        let mut run = test_run();
        let mut bus = bus();

        // First discard: remove 3 tiles.
        run.toggle_select(0);
        run.toggle_select(1);
        run.toggle_select(2);
        run.discard_selected(&mut bus);
        assert_eq!(run.hand.len(), HAND_SIZE);
        assert_eq!(run.discards_remaining, STARTING_DISCARDS - 1);

        // Second discard: remove 1 tile.
        run.toggle_select(0);
        run.discard_selected(&mut bus);
        assert_eq!(run.hand.len(), HAND_SIZE);
        assert_eq!(run.discards_remaining, STARTING_DISCARDS - 2);

        // Third discard: remove 5 tiles.
        for i in 0..5 {
            run.toggle_select(i);
        }
        run.discard_selected(&mut bus);
        assert_eq!(run.hand.len(), HAND_SIZE);
        assert_eq!(run.discards_remaining, STARTING_DISCARDS - 3);

        // Fourth discard: removes the last allowance.
        run.toggle_select(0);
        run.discard_selected(&mut bus);
        assert_eq!(run.discards_remaining, 0);

        // Fifth attempt: should fail (no discards left).
        run.toggle_select(0);
        let result = run.discard_selected(&mut bus);
        assert_eq!(result, 0);
        assert_eq!(run.discards_remaining, 0);
    }

    #[test]
    fn discard_all_14_tiles_redraws_full_hand() {
        let mut run = test_run();
        let mut bus = bus();

        for i in 0..HAND_SIZE {
            run.toggle_select(i);
        }
        let discarded = run.discard_selected(&mut bus);
        assert_eq!(discarded, HAND_SIZE);
        assert_eq!(run.hand.len(), HAND_SIZE); // wall has 136 - 14 = 122 tiles, plenty to redraw
    }

    // ── auto-draw with depleted wall ────────────────────────────────

    #[test]
    fn discard_with_depleted_wall_draws_what_it_can() {
        let mut run = test_run();
        let mut bus = bus();

        // Drain the wall almost completely: wall started with 140, 14 already drawn.
        // Draw remaining 126 tiles to exhaust the wall.
        for _ in 0..126 {
            run.wall.draw();
        }
        assert!(run.wall.draw().is_none()); // wall is empty

        run.toggle_select(0);
        run.toggle_select(1);
        run.discard_selected(&mut bus);

        // Can't redraw, so hand is now 12.
        assert_eq!(run.hand.len(), HAND_SIZE - 2);
        assert_eq!(run.selected.len(), run.hand.len());
    }

    // ── selected vec stays in sync ──────────────────────────────────

    #[test]
    fn selected_vec_length_matches_hand_after_discard() {
        let mut run = test_run();
        let mut bus = bus();

        run.toggle_select(5);
        run.discard_selected(&mut bus);

        assert_eq!(run.selected.len(), run.hand.len());
        // All should be false after discard.
        assert!(run.selected.iter().all(|&s| !s));
    }

    #[test]
    fn selected_vec_length_matches_hand_at_init() {
        let run = test_run();
        assert_eq!(run.selected.len(), run.hand.len());
        assert_eq!(run.selected.len(), HAND_SIZE);
    }

    // ── advance_round resets selection ───────────────────────────────

    #[test]
    fn advance_round_resets_selection() {
        let mut run = test_run();
        let mut bus = bus();
        run.toggle_select(0);
        run.toggle_select(5);
        assert_eq!(run.selected_count(), 2);

        run.advance_round(&mut bus);

        assert_eq!(run.selected_count(), 0);
        assert_eq!(run.selected.len(), run.hand.len());
        assert_eq!(run.discards_remaining, STARTING_DISCARDS);
    }

    #[test]
    fn apply_chamber_feeds_hungry_ghost_the_next_relic() {
        use crate::core::relic::relic_sell_price;

        let mut run = test_run();
        run.relics.active.push(RelicId::HungryGhost);
        run.relics.active.push(RelicId::PairPower);

        run.apply_chamber(ChamberKind::Small, None);

        assert_eq!(run.relics.active, vec![RelicId::HungryGhost]);
        let expected = relic_sell_price(RelicId::PairPower) as i32 * 10;
        assert_eq!(
            run.relic_counters.get(&RelicId::HungryGhost),
            Some(&expected)
        );
        assert!(run.relic_activations.contains(&RelicId::HungryGhost));
    }

    #[test]
    fn apply_chamber_rebuilds_round_resources_from_current_bonuses() {
        let mut run = test_run();
        run.plays_remaining = 1;
        run.discards_remaining = 0;
        run.tag_bonus_plays = 1;
        run.tag_bonus_discards = 1;

        run.apply_chamber(ChamberKind::Small, None);

        assert_eq!(run.plays_remaining, STARTING_PLAYS + 1);
        assert_eq!(run.plays_max, STARTING_PLAYS + 1);
        assert_eq!(run.discards_remaining, STARTING_DISCARDS + 1);
        assert_eq!(run.discards_max, STARTING_DISCARDS + 1);
        assert_eq!(run.tag_bonus_plays, 0);
        assert_eq!(run.tag_bonus_discards, 0);
    }

    fn dead_hand_no_actions_fixture() -> (RunState, EventBus) {
        let mut run = test_run();
        let bus = bus();
        run.hand = vec![
            Tile::new(Suit::Manzu, 1, 1),
            Tile::new(Suit::Manzu, 3, 2),
            Tile::new(Suit::Manzu, 5, 3),
            Tile::new(Suit::Manzu, 7, 4),
            Tile::new(Suit::Manzu, 9, 5),
            Tile::new(Suit::Souzu, 2, 6),
            Tile::new(Suit::Souzu, 4, 7),
            Tile::new(Suit::Souzu, 6, 8),
            Tile::new(Suit::Souzu, 8, 9),
            Tile::new(Suit::Pinzu, 1, 10),
            Tile::new(Suit::Pinzu, 3, 11),
            Tile::new(Suit::Pinzu, 5, 12),
            Tile::new(Suit::Wind, 1, 13),
            Tile::new(Suit::Dragon, 1, 14),
        ];
        run.selected = vec![false; run.hand.len()];
        run.discards_remaining = 0;
        run.plays_remaining = 3;
        run.structure_sets.clear();
        run.structure_tiles.clear();
        (run, bus)
    }

    #[test]
    fn second_wind_salvages_round_instead_of_game_over() {
        let mut run = test_run();
        let mut bus = bus();
        run.relics.active.push(RelicId::SecondWind);
        run.hand = vec![
            Tile::new(Suit::Manzu, 1, 1),
            Tile::new(Suit::Manzu, 3, 2),
            Tile::new(Suit::Manzu, 5, 3),
            Tile::new(Suit::Manzu, 7, 4),
            Tile::new(Suit::Manzu, 9, 5),
            Tile::new(Suit::Souzu, 2, 6),
            Tile::new(Suit::Souzu, 4, 7),
            Tile::new(Suit::Souzu, 6, 8),
            Tile::new(Suit::Souzu, 8, 9),
            Tile::new(Suit::Pinzu, 1, 10),
            Tile::new(Suit::Pinzu, 3, 11),
            Tile::new(Suit::Pinzu, 5, 12),
            Tile::new(Suit::Wind, 1, 13),
            Tile::new(Suit::Dragon, 1, 14),
        ];
        run.selected = vec![false; run.hand.len()];
        run.discards_remaining = 0;
        run.plays_remaining = 3;
        run.structure_sets.clear();
        run.structure_tiles.clear();

        run.refill_hand(&mut bus);

        assert!(
            !bus.queue
                .iter()
                .any(|ev| matches!(ev, GameEvent::GameOver { .. })),
            "Second Wind should prevent GameOver"
        );
        assert!(
            bus.queue.iter().any(|ev| {
                matches!(
                    ev,
                    GameEvent::RoundComplete {
                        reached_target: false,
                        ..
                    }
                )
            }),
            "Second Wind should enqueue a zero-payout RoundComplete"
        );
        assert!(
            !run.relics.has(RelicId::SecondWind),
            "Second Wind should be destroyed"
        );
        run.forfeit_current_chamber_second_wind(&mut bus);
        assert_eq!(run.upcoming_chamber, ChamberKind::Big);
        assert_eq!(run.run_number, 2);
    }

    #[test]
    fn boss_mark_salvages_out_of_plays_instead_of_game_over() {
        let mut run = test_run();
        let mut bus = bus();
        run.plays_remaining = 0;
        run.consumables = ConsumableInventory {
            items: vec![Consumable::Memorial(MemorialTalismanKind::BossMark)],
            capacity: 2,
        };

        run.refill_hand(&mut bus);

        assert!(
            !bus.queue
                .iter()
                .any(|ev| matches!(ev, GameEvent::GameOver { .. })),
            "Boss Mark should prevent GameOver when out of plays"
        );
        assert_eq!(run.plays_remaining, 1);
        assert!(run.consumables.items.is_empty());
        assert!(bus.queue.iter().any(|ev| matches!(
            ev,
            GameEvent::MemorialTalismanUsed(MemorialTalismanKind::BossMark)
        )));
    }

    #[test]
    fn exhausted_preferred_over_boss_mark_for_out_of_plays() {
        let mut run = test_run();
        let mut bus = bus();
        run.plays_remaining = 0;
        run.consumables = ConsumableInventory {
            items: vec![
                Consumable::Memorial(MemorialTalismanKind::BossMark),
                Consumable::Memorial(MemorialTalismanKind::Exhausted),
            ],
            capacity: 2,
        };

        run.refill_hand(&mut bus);

        assert_eq!(run.plays_remaining, 2);
        assert_eq!(
            run.consumables.items,
            vec![Consumable::Memorial(MemorialTalismanKind::BossMark)]
        );
        assert!(bus.queue.iter().any(|ev| matches!(
            ev,
            GameEvent::MemorialTalismanUsed(MemorialTalismanKind::Exhausted)
        )));
    }

    #[test]
    fn frozen_hand_salvages_no_actions_remaining_instead_of_game_over() {
        let (mut run, mut bus) = dead_hand_no_actions_fixture();
        run.consumables = ConsumableInventory {
            items: vec![Consumable::Memorial(MemorialTalismanKind::FrozenHand)],
            capacity: 2,
        };

        run.refill_hand(&mut bus);

        assert!(
            !bus.queue
                .iter()
                .any(|ev| matches!(ev, GameEvent::GameOver { .. })),
            "Frozen Hand should prevent GameOver when no actions remain"
        );
        assert_eq!(run.discards_remaining, 1);
        assert!(run.consumables.items.is_empty());
        assert!(!run.hand.is_empty());
    }

    #[test]
    fn second_wind_plays_used_uses_effective_round_cap() {
        let mut run = test_run();
        run.apply_chamber(ChamberKind::Small, None);
        run.plays_remaining -= 2;

        let rw = Some(ChamberKind::round_wind_for_wing(run.wing));
        let ctx = ScoreContext {
            relic: ScoreRelicBundle {
                roster: &run.relics,
                counters: run.relic_counters.clone(),
            },
            tiles: ScoreTileBundle {
                debuffs: &[],
                hand_for_ghost: run.hand(),
            },
            round: ScoreRoundBundle {
                scored_last_turn: run.scored_last_turn,
                plays_used: run.round_play_cap().saturating_sub(run.plays_remaining),
                round_wind: rw,
                bonus_round_wind: run.bonus_round_wind_for_yaku(),
                played_yaku_this_round: run.played_yaku_this_round.clone(),
                is_final_play: run.plays_remaining == 0,
            },
            pattern: ScorePatternBundle {
                dora_faces: run.wall.dora_faces(),
                available_yaku: run.available_yaku.clone(),
                yaku_levels: Some(run.yaku_levels.clone()),
            },
            economy: ScoreEconomyBundle {
                yen: run.yen,
                total_score: run.total_score_earned,
            },
            structure: None,
        };

        assert_eq!(ctx.round.plays_used, 2);
    }

    #[test]
    fn apply_chamber_uses_material_starting_discards_before_skip_bonus() {
        let mut run = RunState::new(GameMode::with_material(
            crate::persistence::TileMaterial::Plastic,
        ));
        run.discards_remaining = 0;
        run.tag_bonus_discards = 1;

        run.apply_chamber(ChamberKind::Small, None);

        assert_eq!(run.discards_remaining, 6);
        assert_eq!(run.discards_max, 6);
        assert_eq!(run.tag_bonus_discards, 0);
    }

    #[test]
    fn apply_chamber_tracks_reduced_round_caps_for_boss_taxes() {
        let mut run = test_run();
        run.ordeal.upcoming = Some(OrdealKind::Drought);

        run.apply_chamber(ChamberKind::Ordeal, None);

        assert_eq!(run.discards_remaining, STARTING_DISCARDS / 2);
        assert_eq!(run.discards_max, STARTING_DISCARDS / 2);
    }

    #[test]
    fn big_hands_increases_effective_hand_and_reduces_discard_cap() {
        let mut run = test_run();
        run.relics.active.push(RelicId::BigHands);
        assert_eq!(ordeal::effective_hand_size(&run), HAND_SIZE + 2);
        run.reset_round_resources();
        assert_eq!(run.discards_remaining, STARTING_DISCARDS - 1);
        assert_eq!(run.discards_max, STARTING_DISCARDS - 1);
    }

    #[test]
    fn tiny_hands_decreases_effective_hand_and_adds_discard_cap() {
        let mut run = test_run();
        run.relics.active.push(RelicId::TinyHands);
        assert_eq!(ordeal::effective_hand_size(&run), HAND_SIZE - 2);
        run.reset_round_resources();
        assert_eq!(run.discards_remaining, STARTING_DISCARDS + 2);
        assert_eq!(run.discards_max, STARTING_DISCARDS + 2);
    }

    #[test]
    fn kindness_adds_one_discard_cap() {
        let mut run = test_run();
        run.relics.active.push(RelicId::Kindness);
        run.reset_round_resources();
        assert_eq!(run.discards_remaining, STARTING_DISCARDS + 1);
        assert_eq!(run.discards_max, STARTING_DISCARDS + 1);
    }

    #[test]
    fn diligence_adds_one_play_cap() {
        let mut run = test_run();
        run.relics.active.push(RelicId::Diligence);
        run.reset_round_resources();
        assert_eq!(run.plays_remaining, STARTING_PLAYS + 1);
        assert_eq!(run.plays_max, STARTING_PLAYS + 1);
    }

    #[test]
    fn big_hands_and_tiny_hands_cancel_hand_delta() {
        let mut run = test_run();
        run.relics.active.push(RelicId::BigHands);
        run.relics.active.push(RelicId::TinyHands);
        assert_eq!(ordeal::effective_hand_size(&run), HAND_SIZE);
    }

    #[test]
    fn refill_hand_reaches_big_hands_target_from_undersized_hand() {
        let mut run = test_run();
        run.relics.active.push(RelicId::BigHands);
        assert_eq!(run.hand.len(), HAND_SIZE);
        let mut bus = bus();
        run.refill_hand(&mut bus);
        assert_eq!(run.hand.len(), HAND_SIZE + 2);
    }

    #[test]
    fn duplicate_temptations_stack() {
        use crate::core::tag::TagKind;

        let mut run = test_run();
        run.apply_tag(TagKind::FreeRestock, None);
        run.apply_tag(TagKind::FreeRestock, None);
        run.apply_tag(TagKind::PatronGift, None);
        run.apply_tag(TagKind::PatronGift, None);
        run.apply_tag(TagKind::RichStock, None);
        run.apply_tag(TagKind::RichStock, None);
        run.apply_tag(TagKind::BonusPlay, None);
        run.apply_tag(TagKind::BonusPlay, None);
        run.apply_tag(TagKind::WideHand, None);
        run.apply_tag(TagKind::WideHand, None);

        assert_eq!(run.tag_free_restock, 2);
        assert_eq!(run.tag_patron_gift, 2);
        assert_eq!(run.tag_rich_stock, 2);
        assert_eq!(run.tag_bonus_plays, 2);
        assert_eq!(run.tag_bonus_discards, 0);
        assert_eq!(run.tag_bonus_hand_size, 4);
    }

    #[test]
    fn apply_chamber_promotes_wide_hand_bonus_to_round_hand_size() {
        let mut run = test_run();
        run.apply_tag(crate::core::tag::TagKind::WideHand, None);

        run.apply_chamber(ChamberKind::Small, None);

        assert_eq!(run.hand.len(), HAND_SIZE + 2);
        assert_eq!(ordeal::effective_hand_size(&run), HAND_SIZE + 2);
        assert_eq!(run.tag_bonus_hand_size, 0);
    }

    #[test]
    fn skipping_with_wide_hand_carries_bonus_into_next_chamber() {
        let mut run = test_run();

        run.apply_tag(crate::core::tag::TagKind::WideHand, None);
        run.skip_to_next_chamber();

        assert_eq!(run.tag_bonus_hand_size, 2);

        run.apply_chamber(ChamberKind::Big, None);

        assert_eq!(run.hand.len(), HAND_SIZE + 2);
        assert_eq!(ordeal::effective_hand_size(&run), HAND_SIZE + 2);
        assert_eq!(run.tag_bonus_hand_size, 0);
    }

    #[test]
    fn advance_round_after_big_keeps_cleared_chamber_until_apply() {
        let mut run = test_run();
        let mut bus = bus();
        run.chamber = ChamberKind::Big;
        run.upcoming_chamber = ChamberKind::Big;
        run.apply_chamber(ChamberKind::Big, None);
        let cleared_target = run.target_score;

        run.advance_round(&mut bus);

        assert_eq!(run.chamber, ChamberKind::Big);
        assert_eq!(run.upcoming_chamber, ChamberKind::Ordeal);
        assert_eq!(run.target_score, cleared_target);
    }

    #[test]
    fn advance_round_after_boss_clears_upcoming_ordeal_until_reveal() {
        let mut run = test_run();
        let mut bus = bus();
        run.chamber = ChamberKind::Ordeal;
        run.upcoming_chamber = ChamberKind::Ordeal;
        run.ordeal.upcoming = Some(OrdealKind::Drought);
        run.resolve_upcoming_ordeal();

        run.advance_round(&mut bus);

        assert_eq!(run.wing, 2);
        assert_eq!(run.upcoming_chamber, ChamberKind::Small);
        assert_eq!(run.chamber, ChamberKind::Ordeal);
        assert!(run.ordeal.upcoming.is_none());
        assert!(run.ordeal.effect.is_none());
    }

    #[test]
    fn advance_round_after_boss_clears_ordeal_tile_debuffs() {
        use crate::core::debuff::{TileDebuff, TileDebuffClass};
        use crate::core::tile::Suit;

        let mut run = test_run();
        let mut bus = bus();
        run.chamber = ChamberKind::Ordeal;
        run.upcoming_chamber = ChamberKind::Ordeal;
        run.tile_debuffs = vec![
            TileDebuff::Suit(Suit::Souzu),
            TileDebuff::Class(TileDebuffClass::Honors),
        ];
        run.relics
            .set_debuffed([crate::core::relic::RelicId::MirrorTile]);

        run.advance_round(&mut bus);

        assert!(run.tile_debuffs.is_empty());
        assert!(
            !run.relics
                .is_debuffed(crate::core::relic::RelicId::MirrorTile)
        );
    }

    #[test]
    fn ensure_ordeal_revealed_rolls_for_current_wing() {
        let mut run = RunState::new_demo();
        run.wing = 2;
        run.ordeal.upcoming = None;
        run.ordeal.effect = None;

        run.ensure_ordeal_revealed();

        assert!(run.ordeal.upcoming.is_some());
        assert!(run.ordeal.effect.is_some());
    }

    #[test]
    fn advance_round_after_boss_preserves_pending_shop_skip_rewards() {
        let mut run = test_run();
        let mut bus = bus();
        run.chamber = ChamberKind::Ordeal;
        run.upcoming_chamber = ChamberKind::Ordeal;
        run.tag_free_restock = 1;
        run.tag_patron_gift = 1;
        run.tag_rich_stock = 1;

        run.advance_round(&mut bus);

        assert_eq!(run.tag_free_restock, 1);
        assert_eq!(run.tag_patron_gift, 1);
        assert_eq!(run.tag_rich_stock, 1);
    }

    #[test]
    fn advance_round_after_boss_records_score_after_wing() {
        let mut run = test_run();
        let mut bus = bus();
        run.wing = 3;
        run.total_score_earned = 12_500;
        run.chamber = ChamberKind::Ordeal;
        run.upcoming_chamber = ChamberKind::Ordeal;

        run.advance_round(&mut bus);

        assert_eq!(run.wing, 4);
        assert_eq!(
            run.score_after_wing.last(),
            Some(&(3, 12_500)),
            "snapshot taken before ante increment"
        );
    }

    #[test]
    fn advance_round_after_boss_clears_unconsumed_next_chamber_skip_bonuses() {
        let mut run = test_run();
        let mut bus = bus();
        run.chamber = ChamberKind::Ordeal;
        run.upcoming_chamber = ChamberKind::Ordeal;
        run.tag_bonus_plays = 1;
        run.tag_bonus_discards = 1;
        run.tag_bonus_hand_size = 2;

        run.advance_round(&mut bus);

        assert_eq!(run.tag_bonus_plays, 0);
        assert_eq!(run.tag_bonus_discards, 0);
        assert_eq!(run.tag_bonus_hand_size, 0);
    }

    // ── commit_selection_to_structure ───────────────────────────

    #[test]
    fn quick_draw_draws_extra_tile_after_play() {
        let mut run = test_run();
        run.relics.active.push(RelicId::QuickDraw);
        // Same deterministic opening hand as `commit_selection_valid_triplet`.
        run.toggle_select(0);
        run.toggle_select(1);
        run.toggle_select(2);
        let mut bus = bus();

        let committed = run.commit_selection_to_structure(&mut bus);
        assert!(committed > 0, "triplet should commit");
        assert!(run.relic_activations.contains(&RelicId::QuickDraw));
        assert_eq!(
            run.hand.len(),
            ordeal::effective_hand_size(&run) + 2,
            "Quick Draw refills to hand size + 2"
        );
    }

    #[test]
    fn commit_selection_valid_triplet() {
        let mut run = test_run();
        let mut bus = bus();
        // Deterministic hand (sorted): 1m×4, 2m×4, 3m×4, 4m×2
        // Select first 3 tiles (1m, 1m, 1m) — a triplet.
        run.toggle_select(0);
        run.toggle_select(1);
        run.toggle_select(2);
        let pts = run.commit_selection_to_structure(&mut bus);
        assert!(pts > 0, "valid triplet should commit");
        assert_eq!(run.plays_remaining, STARTING_PLAYS - 1);
        // Scored tiles removed and redrawn.
        assert_eq!(run.hand.len(), HAND_SIZE);
        assert_eq!(run.selected_count(), 0);
    }

    #[test]
    fn glass_cannon_destroys_after_first_scoring_hand() {
        let mut run = test_run();
        run.relics.active.push(RelicId::GlassCannon);
        let mut bus = bus();
        let (tiles, sets) = winning_structure();
        run.structure_tiles = tiles;
        run.structure_sets = sets;
        let _ = run.trigger_structure_manual(&mut bus);
        assert!(!run.relics.active.contains(&RelicId::GlassCannon));
    }

    #[test]
    fn glass_cannon_does_not_reduce_starting_plays_cap() {
        let mut run = test_run();
        assert_eq!(run.round_play_cap(), STARTING_PLAYS);
        run.relics.active.push(RelicId::GlassCannon);
        assert_eq!(run.round_play_cap(), STARTING_PLAYS);
    }

    #[test]
    fn green_luck_committing_honors_does_not_count_until_cash_in() {
        let mut run = test_run();
        run.auto_cash_in_on_full_structure = false;
        let winds = [
            Tile::new(Suit::Wind, 1, 901),
            Tile::new(Suit::Wind, 1, 902),
            Tile::new(Suit::Wind, 1, 903),
        ];
        let mut hand: Vec<Tile> = (0..HAND_SIZE - 3)
            .map(|i| Tile::new(Suit::Manzu, 1, 100 + i as u32))
            .collect();
        hand.extend(winds);
        run.hand = hand;
        run.selected = std::iter::repeat_n(false, HAND_SIZE - 3)
            .chain(std::iter::repeat_n(true, 3))
            .collect();
        let mut bus = bus();
        assert!(run.commit_selection_to_structure(&mut bus) > 0);
        assert!(
            !run.honors_scored_this_round,
            "playing honors must not count toward Green Luck until cash-in scores them"
        );
    }

    #[test]
    fn green_luck_ignores_debuffed_honors_at_cash_in() {
        let mut run = test_run();
        run.tile_debuffs = vec![TileDebuff::Class(TileDebuffClass::Honors)];
        let (tiles, sets) = winning_structure();
        run.structure_tiles = tiles;
        run.structure_sets = sets;
        let mut bus = bus();
        let _ = run.trigger_structure_manual(&mut bus);
        assert!(
            !run.honors_scored_this_round,
            "debuffed honor tiles in a cash-in must not count as scored honors"
        );
    }

    #[test]
    fn green_luck_counts_non_debuffed_honors_at_cash_in() {
        let mut run = test_run();
        let (tiles, sets) = winning_structure();
        run.structure_tiles = tiles;
        run.structure_sets = sets;
        let mut bus = bus();
        let _ = run.trigger_structure_manual(&mut bus);
        assert!(
            run.honors_scored_this_round,
            "cash-in that scores live honor tiles should block Green Luck"
        );
    }

    #[test]
    fn dragon_allows_non_honor_structure_but_debuffs_its_score() {
        let tiles = vec![
            Tile::new(Suit::Manzu, 1, 1),
            Tile::new(Suit::Manzu, 1, 2),
            Tile::new(Suit::Manzu, 1, 3),
        ];
        let sets = vec![DetectedMeld {
            kind: MeldKind::Triplet,
            tile_ids: vec![1, 2, 3],
        }];

        let mut baseline = test_run();
        baseline.chamber = ChamberKind::Small;
        baseline.structure_tiles = tiles.clone();
        baseline.structure_sets = sets.clone();
        let mut baseline_bus = bus();
        let baseline_earned = baseline.trigger_structure_manual(&mut baseline_bus);
        assert!(baseline_earned > 0);
        let baseline_score = baseline.round_score;

        let mut dragon = test_run();
        dragon.chamber = ChamberKind::Ordeal;
        dragon.upcoming_chamber = ChamberKind::Ordeal;
        dragon.ordeal.upcoming = Some(OrdealKind::Dragon);
        dragon.structure_tiles = tiles;
        dragon.structure_sets = sets;
        let mut dragon_bus = bus();
        let dragon_earned = dragon.trigger_structure_manual(&mut dragon_bus);
        assert_eq!(
            dragon_earned, 0,
            "Dragon debuffs non-honor tile chips; with no meld flat bonuses the cash-in scores zero"
        );
        assert_eq!(dragon.round_score, 0);
        assert!(
            dragon.structure_sets.is_empty(),
            "Dragon should still complete structure cash-in"
        );
        assert!(
            baseline_score > 0,
            "non-Dragon blind should still score tile chips from the same structure"
        );
    }

    #[test]
    fn dead_air_destroys_scored_wind_tiles() {
        let (tiles, sets) = winning_structure();
        let wind_ids: Vec<u32> = tiles
            .iter()
            .filter(|t| t.suit == Suit::Wind)
            .map(|t| t.id)
            .collect();

        let mut run = test_run();
        run.chamber = ChamberKind::Ordeal;
        run.upcoming_chamber = ChamberKind::Ordeal;
        run.ordeal.upcoming = Some(OrdealKind::DeadAir);
        run.structure_tiles = tiles;
        run.structure_sets = sets;
        let mut bus = bus();
        let _ = run.trigger_structure_manual(&mut bus);

        for id in wind_ids {
            assert!(
                run.removed_tile_ids.contains(&id),
                "Dead Air should permanently remove scored wind tile {id}"
            );
        }
        assert!(
            bus.queue
                .iter()
                .any(|e| matches!(e, GameEvent::TilesDestroyed)),
            "Dead Air destruction should emit TilesDestroyed"
        );
    }

    #[test]
    fn st_george_destroys_scored_dragon_tiles() {
        let tiles = vec![
            Tile::new(Suit::Manzu, 1, 1),
            Tile::new(Suit::Manzu, 1, 2),
            Tile::new(Suit::Manzu, 2, 3),
            Tile::new(Suit::Manzu, 3, 4),
            Tile::new(Suit::Manzu, 4, 5),
            Tile::new(Suit::Pinzu, 2, 6),
            Tile::new(Suit::Pinzu, 3, 7),
            Tile::new(Suit::Pinzu, 4, 8),
            Tile::new(Suit::Souzu, 5, 9),
            Tile::new(Suit::Souzu, 6, 10),
            Tile::new(Suit::Souzu, 7, 11),
            Tile::new(Suit::Dragon, 1, 12),
            Tile::new(Suit::Dragon, 1, 13),
            Tile::new(Suit::Dragon, 1, 14),
        ];
        let sets = vec![
            DetectedMeld {
                kind: MeldKind::Pair,
                tile_ids: vec![1, 2],
            },
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![3, 4, 5],
            },
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![6, 7, 8],
            },
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![9, 10, 11],
            },
            DetectedMeld {
                kind: MeldKind::Triplet,
                tile_ids: vec![12, 13, 14],
            },
        ];
        let dragon_ids = [12u32, 13, 14];

        let mut run = test_run();
        run.chamber = ChamberKind::Ordeal;
        run.upcoming_chamber = ChamberKind::Ordeal;
        run.ordeal.upcoming = Some(OrdealKind::StGeorge);
        run.structure_tiles = tiles;
        run.structure_sets = sets;
        let mut bus = bus();
        let _ = run.trigger_structure_manual(&mut bus);

        for id in dragon_ids {
            assert!(
                run.removed_tile_ids.contains(&id),
                "St. George should permanently remove scored dragon tile {id}"
            );
        }
        assert!(
            bus.queue
                .iter()
                .any(|e| matches!(e, GameEvent::TilesDestroyed)),
            "St. George destruction should emit TilesDestroyed"
        );
    }

    #[test]
    fn decimation_removes_ten_tiles_from_shop_preview() {
        use crate::game::decimation::{
            HOUSE_PICKS, PLAYER_PICKS, apply_decimation, decimation_eligible_tiles,
            decimation_house_pool, pick_house_tiles,
        };
        use crate::game::wall_ledger::shop_wall_hud_count;

        let mut run = test_run();
        let mut bus = bus();
        let before = shop_wall_hud_count(&run);
        let eligible = decimation_eligible_tiles(&run);
        assert!(eligible.len() >= 10);
        let player: [u32; PLAYER_PICKS] = eligible
            .iter()
            .take(PLAYER_PICKS)
            .map(|t| t.id)
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
        let pool = decimation_house_pool(&run, &player);
        let house: [u32; HOUSE_PICKS] = pick_house_tiles(&pool, &mut rand::rng())
            .try_into()
            .unwrap();
        apply_decimation(&mut run, player, house, &mut bus, true);
        assert_eq!(shop_wall_hud_count(&run), before - 10);
        assert_eq!(run.decimations_used, 1);
        assert!(
            bus.queue
                .iter()
                .any(|e| matches!(e, GameEvent::TilesDestroyed))
        );
    }

    #[test]
    fn full_structure_autocash_can_be_disabled() {
        let mut run = test_run();
        run.set_auto_cash_in_on_full_structure(false);
        let mut bus = bus();
        let (tiles, sets) = winning_structure();
        run.structure_tiles = tiles;
        run.structure_sets = sets;

        run.try_autotrigger_structure_full(&mut bus);

        assert_eq!(run.structure_sets.len(), 5);
        assert_eq!(run.structure_tiles.len(), 14);
        assert_eq!(run.round_score, 0);
    }

    #[test]
    fn full_structure_autocash_defaults_on() {
        let mut run = test_run();
        let mut bus = bus();
        let (tiles, sets) = winning_structure();
        run.structure_tiles = tiles;
        run.structure_sets = sets;

        run.try_autotrigger_structure_full(&mut bus);

        assert!(run.structure_sets.is_empty());
        assert!(run.structure_tiles.is_empty());
        assert!(run.round_score > 0);
    }

    #[test]
    fn capacity_full_non_winning_shape_autocashes() {
        let mut run = test_run();
        let mut bus = bus();
        let (tiles, sets) = capacity_full_non_winning_structure();
        assert!(!crate::core::structure::is_winning_structure_shape(
            &tiles, &sets
        ));
        assert!(crate::core::structure::structure_cannot_grow_further(
            &tiles, &sets, HAND_SIZE
        ));
        run.structure_tiles = tiles;
        run.structure_sets = sets;

        run.try_autotrigger_structure_full(&mut bus);

        assert!(run.structure_sets.is_empty());
        assert!(run.structure_tiles.is_empty());
        assert!(run.round_score > 0);
    }

    fn capacity_full_non_winning_structure() -> (Vec<Tile>, Vec<DetectedMeld>) {
        let tiles = vec![
            Tile::new(Suit::Manzu, 1, 1),
            Tile::new(Suit::Manzu, 1, 2),
            Tile::new(Suit::Manzu, 1, 3),
            Tile::new(Suit::Manzu, 2, 4),
            Tile::new(Suit::Manzu, 2, 5),
            Tile::new(Suit::Manzu, 2, 6),
            Tile::new(Suit::Manzu, 3, 7),
            Tile::new(Suit::Manzu, 3, 8),
            Tile::new(Suit::Manzu, 4, 9),
            Tile::new(Suit::Manzu, 4, 10),
            Tile::new(Suit::Manzu, 5, 11),
            Tile::new(Suit::Manzu, 5, 12),
            Tile::new(Suit::Manzu, 6, 13),
            Tile::new(Suit::Manzu, 6, 14),
        ];
        let sets = vec![
            DetectedMeld {
                kind: MeldKind::Triplet,
                tile_ids: vec![1, 2, 3],
            },
            DetectedMeld {
                kind: MeldKind::Triplet,
                tile_ids: vec![4, 5, 6],
            },
            DetectedMeld {
                kind: MeldKind::Pair,
                tile_ids: vec![7, 8],
            },
            DetectedMeld {
                kind: MeldKind::Pair,
                tile_ids: vec![9, 10],
            },
            DetectedMeld {
                kind: MeldKind::Pair,
                tile_ids: vec![11, 12],
            },
            DetectedMeld {
                kind: MeldKind::Pair,
                tile_ids: vec![13, 14],
            },
        ];
        (tiles, sets)
    }

    #[test]
    fn house_rule_blocks_autocash_until_discards_are_spent() {
        let mut run = test_run();
        let mut bus = bus();
        run.round_rules.push(RuleModifier::CashInRequiresNoDiscards);
        run.discards_remaining = 2;
        let (tiles, sets) = winning_structure();
        run.structure_tiles = tiles;
        run.structure_sets = sets;

        run.try_autotrigger_structure_full(&mut bus);

        assert_eq!(run.structure_sets.len(), 5);
        assert!(!run.can_trigger_structure_now());
        assert_eq!(run.round_score, 0);

        run.discards_remaining = 0;
        assert!(run.can_trigger_structure_now());
        run.try_autotrigger_structure_full(&mut bus);
        assert!(run.structure_sets.is_empty());
        assert!(run.round_score > 0);
    }

    #[test]
    fn commit_selection_invalid_returns_zero() {
        let mut run = test_run();
        let mut bus = bus();
        // Select 4 tiles: triplet + 1 leftover → invalid.
        run.toggle_select(0);
        run.toggle_select(1);
        run.toggle_select(2);
        run.toggle_select(4); // 2m — leftover
        let pts = run.commit_selection_to_structure(&mut bus);
        assert_eq!(pts, 0, "invalid selection should score 0");
        assert_eq!(run.plays_remaining, STARTING_PLAYS, "no play consumed");
        assert_eq!(run.hand.len(), HAND_SIZE, "hand unchanged");
    }

    #[test]
    fn commit_selection_nothing_returns_zero() {
        let mut run = test_run();
        let mut bus = bus();
        let pts = run.commit_selection_to_structure(&mut bus);
        assert_eq!(pts, 0);
        assert_eq!(run.plays_remaining, STARTING_PLAYS);
    }

    #[test]
    fn refill_hand_ends_round_when_no_actions_remain() {
        let (mut run, mut bus) = dead_hand_no_actions_fixture();

        run.refill_hand(&mut bus);

        assert!(matches!(
            bus.queue.last(),
            Some(GameEvent::GameOver {
                reason: GameOverReason::NoActionsRemaining,
            })
        ));
    }

    fn game_over_queued(bus: &EventBus) -> bool {
        bus.queue
            .iter()
            .any(|ev| matches!(ev, GameEvent::GameOver { .. }))
    }

    /// Regression: dead hand with discards spent but plays left (UI soft-lock) should
    /// end the round via [`RunState::resolve_round_end`] — not only after `refill_hand`.
    #[test]
    fn dead_hand_idle_resolve_queues_game_over() {
        let (mut run, mut bus) = dead_hand_no_actions_fixture();
        run.plays_remaining = 1;
        assert!(!game_over_queued(&bus));
        run.resolve_round_end(&mut bus);
        assert!(game_over_queued(&bus));
    }

    #[test]
    fn dead_hand_with_plays_left_queues_game_over_without_refill() {
        let (mut run, mut bus) = dead_hand_no_actions_fixture();
        run.plays_remaining = 1;

        assert_eq!(
            run.round_failure_reason(),
            Some(GameOverReason::NoActionsRemaining)
        );
        assert!(!game_over_queued(&bus));

        // Empty Play tap (or any commit attempt while stuck).
        assert_eq!(run.commit_selection_to_structure(&mut bus), 0);

        assert!(
            game_over_queued(&bus),
            "dead round should resolve without waiting for discard refill"
        );
        assert!(matches!(
            bus.queue.last(),
            Some(GameEvent::GameOver {
                reason: GameOverReason::NoActionsRemaining,
            })
        ));
    }

    #[test]
    fn out_of_plays_loss_takes_precedence_over_dead_round_reason() {
        let mut run = test_run();
        let mut bus = bus();
        run.hand = vec![
            Tile::new(Suit::Manzu, 1, 1),
            Tile::new(Suit::Manzu, 3, 2),
            Tile::new(Suit::Manzu, 5, 3),
            Tile::new(Suit::Manzu, 7, 4),
            Tile::new(Suit::Manzu, 9, 5),
            Tile::new(Suit::Souzu, 2, 6),
            Tile::new(Suit::Souzu, 4, 7),
            Tile::new(Suit::Souzu, 6, 8),
            Tile::new(Suit::Souzu, 8, 9),
            Tile::new(Suit::Pinzu, 1, 10),
            Tile::new(Suit::Pinzu, 3, 11),
            Tile::new(Suit::Pinzu, 5, 12),
            Tile::new(Suit::Wind, 1, 13),
            Tile::new(Suit::Dragon, 1, 14),
        ];
        run.selected = vec![false; run.hand.len()];
        run.discards_remaining = 0;
        run.plays_remaining = 0;

        run.refill_hand(&mut bus);

        assert!(matches!(
            bus.queue.last(),
            Some(GameEvent::GameOver {
                reason: GameOverReason::OutOfPlays,
            })
        ));
    }

    #[test]
    fn commit_selection_removes_tiles_from_hand() {
        let mut run = test_run();
        let mut bus = bus();
        // Select a pair: indices 0 and 1 (1m, 1m).
        let tile0 = run.hand[0];
        let tile1 = run.hand[1];
        run.toggle_select(0);
        run.toggle_select(1);
        run.commit_selection_to_structure(&mut bus);
        // Those specific tiles should be gone.
        assert!(!run.hand.iter().any(|t| t.id == tile0.id));
        assert!(!run.hand.iter().any(|t| t.id == tile1.id));
    }

    #[test]
    fn is_selection_valid_reflects_state() {
        let mut run = test_run();
        assert!(!run.is_selection_valid(), "empty selection is invalid");
        // Select a triplet.
        run.toggle_select(0);
        run.toggle_select(1);
        run.toggle_select(2);
        assert!(run.is_selection_valid(), "triplet should be valid");
        // Add a leftover.
        run.toggle_select(4);
        assert!(!run.is_selection_valid(), "triplet + leftover is invalid");
    }

    #[test]
    fn sixteen_identical_tiles_play_validity_matches_capacity_fitting_pick() {
        use crate::core::hand::decomposition_canonical_key;
        use crate::core::yaku::YakuKind;

        let selected: Vec<Tile> = (0..16).map(|i| Tile::new(Suit::Pinzu, 9, 100 + i)).collect();
        let mut run = test_run();
        run.available_yaku = vec![
            YakuKind::Toitoi,
            YakuKind::Honroutou,
            YakuKind::Chanta,
            YakuKind::FullHand,
            YakuKind::Chinitsu,
            YakuKind::Junchan,
        ];
        run.hand = selected.clone();
        run.selected = vec![true; 16];
        run.plays_remaining = 1;

        assert!(
            run.is_selection_valid(),
            "16 identical tiles should be playable on an empty structure"
        );

        let (sets, scoring) = run.try_validate_with_wildcards(&selected).expect("valid");
        let best = run.pick_best_decomposition(sets, &scoring, &selected);
        assert!(
            run.selection_commit_capacity_ok(&best, scoring.len()),
            "pick_best must choose a capacity-fitting split"
        );

        let mut pick_results = Vec::new();
        let mut validity = Vec::new();
        for _ in 0..32 {
            let (sets, scoring) = run.try_validate_with_wildcards(&selected).expect("valid");
            pick_results.push(run.pick_best_decomposition(sets, &scoring, &selected));
            validity.push(run.is_selection_valid());
        }
        let first_key = decomposition_canonical_key(&selected, &pick_results[0]);
        assert!(
            pick_results.iter().all(|sets| {
                decomposition_canonical_key(&selected, sets) == first_key
            }),
            "pick_best must not flicker across frames"
        );
        assert!(
            validity.iter().all(|&v| v),
            "play validity must stay enabled: {validity:?}"
        );

        // One tile already in structure: triplet-heavy splits no longer fit, but kongs still do.
        run.structure_tiles = vec![Tile::new(Suit::Manzu, 1, 9_999)];
        let (sets, scoring) = run.try_validate_with_wildcards(&selected).expect("valid");
        let best = run.pick_best_decomposition(sets, &scoring, &selected);
        assert!(
            run.selection_commit_capacity_ok(&best, scoring.len()),
            "pick_best must stay within structure capacity"
        );
        assert!(run.is_selection_valid());
    }

    #[test]
    fn sixteen_identical_tiles_rejected_when_structure_cannot_fit_any_split() {
        use crate::core::yaku::YakuKind;

        let selected: Vec<Tile> = (0..16).map(|i| Tile::new(Suit::Pinzu, 9, 100 + i)).collect();
        let mut run = test_run();
        run.available_yaku = vec![YakuKind::Toitoi, YakuKind::Chinitsu];
        run.structure_tiles = (0..12)
            .map(|i| Tile::new(Suit::Manzu, 1, 10_000 + i))
            .collect();
        run.hand = selected.clone();
        run.selected = vec![true; 16];
        run.plays_remaining = 1;

        assert!(
            run.try_validate_with_wildcards(&selected).is_some(),
            "melds still validate"
        );
        assert!(
            !run.is_selection_valid(),
            "no decomposition should fit 12 + 16 tiles in structure"
        );
    }

    #[test]
    fn is_selection_valid_rejects_structure_capacity_overflow() {
        let mut run = test_run();
        run.structure_tiles = (0..12)
            .map(|i| Tile::new(Suit::Pinzu, 1, 10_000 + i))
            .collect();
        run.toggle_select(0);
        run.toggle_select(1);
        run.toggle_select(2);
        assert!(
            run.try_validate_with_wildcards(
                &run.hand.iter().zip(run.selected.iter()).filter(|&(_, &s)| s).map(|(t, _)| *t).collect::<Vec<_>>()
            )
            .is_some(),
            "triplet melds should still validate"
        );
        assert!(
            !run.is_selection_valid(),
            "valid melds that overflow structure capacity are not playable"
        );
        assert_eq!(run.play_rejection_callout(), Some("Too many melds"));
    }

    #[test]
    fn selection_blocked_by_ordeal_rules_detects_rot_and_bureaucrat() {
        use crate::core::ordeal::{OrdealKind, ResolvedOrdealEffect};
        use crate::core::rules::{ChamberKind, RuleModifier};
        use crate::core::tile::{Suit, Tile};

        let tiles = vec![
            Tile::new(Suit::Manzu, 1, 1),
            Tile::new(Suit::Manzu, 2, 2),
            Tile::new(Suit::Flower, 1, 3),
        ];

        let mut run = test_run();
        run.chamber = ChamberKind::Ordeal;
        run.ordeal.effect = Some(ResolvedOrdealEffect::from_static(
            &OrdealKind::Rot.def().effect,
        ));
        run.round_rules.push(RuleModifier::NoFlowerWildcards);
        assert!(run.selection_blocked_by_ordeal_rules(&tiles));

        let mut run = test_run();
        run.chamber = ChamberKind::Ordeal;
        run.ordeal.effect = Some(ResolvedOrdealEffect::from_static(
            &OrdealKind::Bureaucrat.def().effect,
        ));
        run.round_rules.push(RuleModifier::MustPlayFive);
        assert!(run.selection_blocked_by_ordeal_rules(&tiles));

        let mut run = test_run();
        run.chamber = ChamberKind::Ordeal;
        run.ordeal.effect = Some(ResolvedOrdealEffect::from_static(
            &OrdealKind::Gate.def().effect,
        ));
        assert!(!run.selection_blocked_by_ordeal_rules(&tiles));
    }

    // ── discard indices are correct (reverse removal) ───────────────

    #[test]
    fn discard_removes_correct_tiles_by_index() {
        let mut run = test_run();
        let mut bus = bus();

        let tile_at_2 = run.hand[2];
        let tile_at_10 = run.hand[10];

        run.toggle_select(2);
        run.toggle_select(10);
        run.discard_selected(&mut bus);

        // These specific tiles should no longer be in hand.
        assert!(!run.hand.iter().any(|t| t.id == tile_at_2.id));
        assert!(!run.hand.iter().any(|t| t.id == tile_at_10.id));
    }

    // ── Brocade Pouch: global-buff enhancement ──────────────────────────

    #[test]
    fn souzu_talisman_transform_persists_across_round() {
        use crate::core::consumable::Consumable;
        use crate::core::deck::Wall;
        use crate::core::talisman::TalismanKind;

        let mut run = test_run();
        let mut bus = bus();

        let number_tile = run
            .hand
            .iter()
            .find(|t| t.is_number_tile())
            .copied()
            .expect("opening hand should include a number tile");
        let original_rank = number_tile.rank;

        run.consumables
            .try_push(Consumable::Talisman(TalismanKind::Souzu));
        run.use_consumable(0, &mut bus);

        assert!(run.removed_tile_ids.contains(&number_tile.id));
        assert_eq!(
            run.transformed_tiles.get(&number_tile.id).map(|t| t.suit),
            Some(crate::core::tile::Suit::Souzu)
        );
        assert_eq!(
            run.hand
                .iter()
                .find(|t| t.id == number_tile.id)
                .map(|t| t.suit),
            Some(crate::core::tile::Suit::Souzu)
        );

        run.advance_round(&mut bus);

        let preview = Wall::preview_composition(
            &run.removed_tile_ids,
            &run.tile_packs,
            &run.tile_enhancements,
            &run.transformed_tiles,
            false,
            &run.joker_extra_faces,
        );
        let baked = preview
            .all_tiles()
            .iter()
            .find(|t| t.id == number_tile.id)
            .expect("transformed tile should remain in next-round wall");
        assert_eq!(baked.suit, crate::core::tile::Suit::Souzu);
        assert_eq!(baked.rank, original_rank);

        let wall = Wall::from_filtered_with_packs(
            &run.removed_tile_ids,
            &run.tile_packs,
            &run.tile_enhancements,
            &run.transformed_tiles,
            false,
            &run.joker_extra_faces,
        );
        let drawn = wall
            .all_tiles()
            .iter()
            .find(|t| t.id == number_tile.id)
            .copied()
            .expect("transformed tile should be injectable into a fresh wall");
        assert_eq!(drawn.suit, crate::core::tile::Suit::Souzu);
        assert_eq!(drawn.rank, original_rank);
    }

    #[test]
    fn brocade_pouch_stamps_tiles_drawn_after_talisman_use() {
        use crate::core::consumable::Consumable;
        use crate::core::talisman::TalismanKind;
        use crate::core::tile::TileEnhancement;

        let mut run = test_run();
        let mut bus = bus();

        run.relics.active.push(RelicId::BrocadePouch);
        run.recompute_capacities();
        run.consumables
            .try_push(Consumable::Talisman(TalismanKind::Pearl));

        // Remember which tile ids are in hand *before* use; ids drawn later
        // should still pick up the enhancement via the global fallback.
        let original_ids: rustc_hash::FxHashSet<u32> = run.hand.iter().map(|t| t.id).collect();
        run.use_consumable(0, &mut bus);
        assert_eq!(run.global_buff_enhancement, Some(TileEnhancement::Pearl));

        // Discard all original-hand tiles to force the wall to hand out new ids.
        for i in 0..run.hand.len() {
            run.toggle_select(i);
        }
        run.discard_selected(&mut bus);

        // Freshly-drawn tiles (different ids) should now carry Pearl via the
        // global fallback in restamp_hand_enhancements.
        let replaced = run
            .hand
            .iter()
            .filter(|t| !original_ids.contains(&t.id))
            .count();
        assert!(replaced > 0, "wall should have handed out new ids");
        assert!(
            run.hand
                .iter()
                .filter(|t| !original_ids.contains(&t.id))
                .all(|t| t.enhancement == Some(TileEnhancement::Pearl)),
            "new tiles should inherit Pearl from global buff"
        );
    }

    #[test]
    fn brocade_pouch_does_not_apply_without_talisman_use() {
        let mut run = test_run();
        run.relics.active.push(RelicId::BrocadePouch);
        run.recompute_capacities();

        assert_eq!(run.global_buff_enhancement, None);
        assert!(run.hand.iter().all(|t| t.enhancement.is_none()));
    }

    #[test]
    fn brocade_pouch_adds_consumable_slot() {
        let mut run = test_run();
        let base = run.consumables.capacity;
        run.relics.active.push(RelicId::BrocadePouch);
        run.recompute_capacities();
        assert_eq!(run.consumables.capacity, base + 1);
    }

    #[test]
    fn buff_talisman_without_pouch_does_not_set_global() {
        use crate::core::consumable::Consumable;
        use crate::core::talisman::TalismanKind;

        let mut run = test_run();
        let mut bus = bus();
        run.consumables
            .try_push(Consumable::Talisman(TalismanKind::Pearl));
        run.use_consumable(0, &mut bus);
        assert_eq!(run.global_buff_enhancement, None);
    }

    #[test]
    fn melds_for_yaku_preview_matches_pick_best_decomposition() {
        use crate::core::hand::validate_selection;
        use crate::core::yaku::{YakuKind, yaku_after_pool_filter};

        let selected: Vec<Tile> = (0..6).map(|i| Tile::new(Suit::Souzu, 6, i)).collect();
        let mut run = test_run();
        run.available_yaku = vec![YakuKind::Toitoi];
        run.hand = selected.clone();
        run.selected = vec![true; 6];

        let (preview_sets, preview_effective, preview_original) =
            run.melds_for_yaku_preview(&selected);

        let (validator_sets, scoring_tiles) = run
            .try_validate_with_wildcards(&selected)
            .expect("valid six 6s");
        let best_sets = run.pick_best_decomposition(validator_sets, &scoring_tiles, &selected);
        let mut expected_sets = run.structure_sets.clone();
        expected_sets.extend(best_sets.clone());
        assert_eq!(preview_sets, expected_sets);

        let preview_yaku = yaku_after_pool_filter(
            &preview_effective,
            &preview_sets,
            Some(1),
            None,
            Some(preview_original.as_slice()),
            &run.available_yaku,
        );
        let mut commit_sets = run.structure_sets.clone();
        commit_sets.extend(best_sets.clone());
        let commit_yaku = yaku_after_pool_filter(
            &preview_effective,
            &commit_sets,
            Some(1),
            None,
            Some(preview_original.as_slice()),
            &run.available_yaku,
        );
        assert_eq!(preview_yaku, commit_yaku);

        // When multiple splits exist, the validator's first split can miss Toitoi.
        let naive_sets = validate_selection(&selected).expect("valid six 6s");
        if naive_sets != best_sets && preview_yaku.contains(&YakuKind::Toitoi) {
            let mut naive_merged = run.structure_sets.clone();
            naive_merged.extend(naive_sets);
            let naive_yaku = yaku_after_pool_filter(
                &preview_effective,
                &naive_merged,
                Some(1),
                None,
                Some(preview_original.as_slice()),
                &run.available_yaku,
            );
            assert!(
                !naive_yaku.contains(&YakuKind::Toitoi),
                "regression guard: preview must not use validator-first split"
            );
        }
    }

    #[test]
    fn yaku_preview_stable_with_many_flowers() {
        use crate::core::yaku::{YakuKind, yaku_after_pool_filter};

        let mut selected: Vec<Tile> = vec![
            Tile::new(Suit::Manzu, 1, 0),
            Tile::new(Suit::Manzu, 9, 1),
            Tile::new(Suit::Dragon, 1, 2),
            Tile::new(Suit::Dragon, 1, 3),
            Tile::new(Suit::Wind, 1, 4),
            Tile::new(Suit::Wind, 1, 5),
        ];
        for i in 0..6 {
            selected.push(Tile::new(Suit::Flower, (i % 4 + 1) as u8, 100 + i));
        }

        let mut run = test_run();
        run.available_yaku = vec![YakuKind::Yakuhai, YakuKind::Honroutou];
        run.hand = selected.clone();
        run.selected = vec![true; selected.len()];

        let mut first: Option<Vec<YakuKind>> = None;
        for _ in 0..32 {
            let (preview_sets, preview_effective, preview_original) =
                run.melds_for_yaku_preview(&selected);
            let yaku = yaku_after_pool_filter(
                &preview_effective,
                &preview_sets,
                Some(1),
                None,
                Some(preview_original.as_slice()),
                &run.available_yaku,
            );
            match &first {
                None => first = Some(yaku),
                Some(expected) => assert_eq!(yaku, *expected, "yaku preview must not flicker"),
            }
        }
    }

    mod yaku_preview_proptests {
        use proptest::prelude::*;

        use crate::core::hand::validate_selection;
        use crate::core::relic::{
            ScoreContext, ScoreEconomyBundle, ScorePatternBundle, ScoreRelicBundle,
            ScoreRoundBundle, ScoreTileBundle,
        };
        use crate::core::rules::ChamberKind;
        use crate::core::scoring::score_sets_with_original;
        use crate::core::structure::StructureTriggerMeta;
        use crate::core::tile::{Suit, Tile};
        use crate::core::yaku::{YakuKind, yaku_after_pool_filter, yaku_preview};

        use super::{DetectedMeld, RunState, test_run};

        const NUMBER_SUITS: [Suit; 3] = [Suit::Manzu, Suit::Souzu, Suit::Pinzu];
        const ALL_SUITS: [Suit; 5] = [
            Suit::Manzu,
            Suit::Souzu,
            Suit::Pinzu,
            Suit::Wind,
            Suit::Dragon,
        ];

        fn arb_meld(id_start: u32) -> BoxedStrategy<Vec<Tile>> {
            prop_oneof![
                (0..5usize, 1..=9u8).prop_map(move |(si, rank)| {
                    let suit = ALL_SUITS[si];
                    let rank = match suit {
                        Suit::Wind => (rank - 1) % 4 + 1,
                        Suit::Dragon => (rank - 1) % 3 + 1,
                        _ => rank,
                    };
                    vec![
                        Tile::new(suit, rank, id_start),
                        Tile::new(suit, rank, id_start + 1),
                    ]
                }),
                (0..5usize, 1..=9u8).prop_map(move |(si, rank)| {
                    let suit = ALL_SUITS[si];
                    let rank = match suit {
                        Suit::Wind => (rank - 1) % 4 + 1,
                        Suit::Dragon => (rank - 1) % 3 + 1,
                        _ => rank,
                    };
                    vec![
                        Tile::new(suit, rank, id_start),
                        Tile::new(suit, rank, id_start + 1),
                        Tile::new(suit, rank, id_start + 2),
                    ]
                }),
                (0..3usize, 1..=7u8).prop_map(move |(si, start)| {
                    let suit = NUMBER_SUITS[si];
                    vec![
                        Tile::new(suit, start, id_start),
                        Tile::new(suit, start + 1, id_start + 1),
                        Tile::new(suit, start + 2, id_start + 2),
                    ]
                }),
            ]
            .boxed()
        }

        fn assign_ids(melds: Vec<Vec<Tile>>, id_start: u32) -> Vec<Tile> {
            let mut out = Vec::new();
            let mut id = id_start;
            for meld in melds {
                for t in meld {
                    out.push(Tile::new(t.suit, t.rank, id));
                    id += 1;
                }
            }
            out
        }

        fn arb_meld_groups(min: usize, max: usize) -> BoxedStrategy<Vec<Vec<Tile>>> {
            (min..=max)
                .prop_flat_map(|count| {
                    let strategies: Vec<_> = (0..count).map(|i| arb_meld(i as u32 * 4)).collect();
                    strategies
                })
                .boxed()
        }

        fn arb_yaku_pool() -> BoxedStrategy<Vec<YakuKind>> {
            Just(YakuKind::all().to_vec())
                .prop_flat_map(|all| {
                    all.into_iter()
                        .map(|y| (Just(y), any::<bool>()))
                        .collect::<Vec<_>>()
                        .prop_map(|pairs| {
                            pairs
                                .into_iter()
                                .filter_map(|(y, keep)| keep.then_some(y))
                                .collect()
                        })
                })
                .boxed()
        }

        fn arb_scenario() -> BoxedStrategy<(Vec<Tile>, Vec<Tile>, u32, Vec<YakuKind>)> {
            (
                arb_meld_groups(0, 3),
                arb_meld_groups(0, 3),
                1..=4u32,
                arb_yaku_pool(),
            )
                .prop_filter(
                    "structure or selection must be non-empty",
                    |(s, sel, _, _)| !s.is_empty() || !sel.is_empty(),
                )
                .prop_map(|(structure_melds, selection_melds, wing, pool)| {
                    let structure_tiles = assign_ids(structure_melds, 0);
                    let selection_tiles = assign_ids(selection_melds, structure_tiles.len() as u32);
                    (structure_tiles, selection_tiles, wing, pool)
                })
                .boxed()
        }

        fn setup_run(
            structure_tiles: &[Tile],
            selection_tiles: &[Tile],
            wing: u32,
            pool: &[YakuKind],
        ) -> (RunState, Vec<Tile>) {
            let mut run = test_run();
            run.wing = wing;
            run.available_yaku = pool.to_vec();

            if !structure_tiles.is_empty() {
                run.structure_tiles = structure_tiles.to_vec();
                run.structure_sets = validate_selection(structure_tiles)
                    .expect("constructed structure melds must validate");
            }

            let selected = selection_tiles.to_vec();
            run.hand = selected.clone();
            run.selected = vec![selection_tiles.len() > 0; selection_tiles.len()];

            (run, selected)
        }

        fn detected_yaku_for_cash_in(
            run: &RunState,
            scoring_tiles: &[Tile],
            sets: &[DetectedMeld],
            original_tiles: &[Tile],
        ) -> Vec<YakuKind> {
            let rw = Some(ChamberKind::round_wind_for_wing(run.wing));
            let bonus_rw = run.bonus_round_wind_for_yaku();
            let scoring_tile_debuffs: Vec<crate::core::debuff::TileDebuff> = Vec::new();
            let meta = StructureTriggerMeta {
                meld_count: sets.len() as u32,
                inject_chicken_if_no_yaku: true,
            };
            let ctx = ScoreContext {
                relic: ScoreRelicBundle {
                    roster: &run.relics,
                    counters: run.relic_counters.clone(),
                },
                tiles: ScoreTileBundle {
                    debuffs: &scoring_tile_debuffs,
                    hand_for_ghost: &run.hand,
                },
                round: ScoreRoundBundle {
                    scored_last_turn: run.scored_last_turn,
                    plays_used: run.round_play_cap().saturating_sub(run.plays_remaining),
                    round_wind: rw,
                    bonus_round_wind: bonus_rw,
                    played_yaku_this_round: run.played_yaku_this_round.clone(),
                    is_final_play: run.plays_remaining == 0,
                },
                pattern: ScorePatternBundle {
                    dora_faces: run.wall.dora_faces(),
                    available_yaku: run.available_yaku.clone(),
                    yaku_levels: Some(run.yaku_levels.clone()),
                },
                economy: ScoreEconomyBundle {
                    yen: run.yen,
                    total_score: run.total_score_earned,
                },
                structure: Some(meta),
            };
            score_sets_with_original(scoring_tiles, sets, &ctx, &run.round_rules, original_tiles)
                .detected_yaku
        }

        fn preview_yaku_matching_score(
            effective: &[Tile],
            sets: &[DetectedMeld],
            original: Option<&[Tile]>,
            round_wind: Option<u8>,
            bonus_round_wind: Option<u8>,
            available: &[YakuKind],
        ) -> Vec<YakuKind> {
            let mut yaku = yaku_after_pool_filter(
                effective,
                sets,
                round_wind,
                bonus_round_wind,
                original,
                available,
            );
            // Mirror [`StructureTriggerMeta::inject_chicken_if_no_yaku`] in scoring.
            if yaku.is_empty() && !sets.is_empty() {
                yaku.push(YakuKind::ChickenHand);
            }
            yaku
        }

        proptest! {
            #![proptest_config(ProptestConfig { cases: 128, ..ProptestConfig::default() })]

            #[test]
            fn yaku_preview_matches_scoring_path(
                (structure_tiles, selection_tiles, wing, pool) in arb_scenario(),
            ) {
                let (run, selected) = setup_run(&structure_tiles, &selection_tiles, wing, &pool);
                let rw = Some(ChamberKind::round_wind_for_wing(run.wing));
                let bonus_rw = run.bonus_round_wind_for_yaku();

                let (preview_sets, preview_effective, preview_original) =
                    run.melds_for_yaku_preview(&selected);

                let preview_yaku = preview_yaku_matching_score(
                    &preview_effective,
                    &preview_sets,
                    Some(preview_original.as_slice()),
                    rw,
                    bonus_rw,
                    &run.available_yaku,
                );
                let score_yaku = detected_yaku_for_cash_in(
                    &run,
                    &preview_effective,
                    &preview_sets,
                    preview_original.as_slice(),
                );
                prop_assert_eq!(
                    preview_yaku.clone(),
                    score_yaku,
                    "yaku_after_pool_filter must match score_sets detected_yaku"
                );

                let ui_previews = yaku_preview(
                    &preview_original,
                    &run.available_yaku,
                    rw,
                    bonus_rw,
                    Some((preview_sets.as_slice(), preview_effective.as_slice())),
                );
                for preview in ui_previews
                    .iter()
                    .filter(|p| p.kind != YakuKind::ChickenHand)
                {
                    let active_in_filter = preview_yaku.iter().any(|y| *y == preview.kind);
                    prop_assert_eq!(
                        preview.active,
                        active_in_filter,
                        "{:?} active flag mismatch for pool {:?}",
                        preview.kind,
                        run.available_yaku,
                    );
                }

                if selected.is_empty() {
                    prop_assert_eq!(preview_sets, run.structure_sets);
                    prop_assert_eq!(preview_effective, run.structure_tiles);
                } else if let Some((validator_sets, scoring_tiles)) =
                    run.try_validate_with_wildcards(&selected)
                {
                    let best_sets =
                        run.pick_best_decomposition(validator_sets, &scoring_tiles, &selected);
                    let mut expected_sets = run.structure_sets.clone();
                    expected_sets.extend(best_sets);
                    prop_assert_eq!(
                        preview_sets, expected_sets,
                        "preview melds must match committed structure + pick_best(selection)"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod joker_tile_tests {
    use crate::core::deck::{Wall, build_joker_extras, build_wall, joker_extra_tile_id};
    use crate::core::relic::RelicId;
    use crate::core::rules::ChamberKind;
    use crate::core::tile::{Suit, Tile};
    use crate::game::game_mode::GameMode;
    use crate::game::run::RunState;

    #[test]
    fn joker_extras_inject_into_wall_build() {
        let faces = vec![(Suit::Manzu, 3), (Suit::Pinzu, 7)];
        let mut wall = Wall::from_unshuffled(build_wall());
        for tile in build_joker_extras(&faces) {
            wall.inject_into_remaining(tile);
        }
        let manzu_3 = wall
            .peek_next(wall.remaining())
            .iter()
            .filter(|t| t.suit == Suit::Manzu && t.rank == 3)
            .count();
        assert_eq!(manzu_3, 5, "standard 4 + 1 joker extra");
        assert_eq!(joker_extra_tile_id(1), build_joker_extras(&faces)[1].id);
    }

    #[test]
    fn joker_tile_adds_permanent_copy_from_starting_hand() {
        let mut run = RunState::new(GameMode::standard());
        run.grant_relic(RelicId::JokerTile);
        run.hand = vec![Tile::new(Suit::Manzu, 1, 0), Tile::new(Suit::Souzu, 5, 1)];
        run.selected = vec![false; run.hand.len()];
        run.wall = Wall::from_unshuffled(build_wall());
        let remaining_before = run.wall.remaining();

        run.joker_tile_add_starting_hand_copy();

        assert_eq!(run.joker_extra_faces.len(), 1);
        assert!(
            run.hand
                .iter()
                .any(|t| (t.suit, t.rank) == run.joker_extra_faces[0]),
            "copied face should come from starting hand"
        );
        assert_eq!(run.wall.remaining(), remaining_before + 1);
        assert!(run.relic_activations.contains(&RelicId::JokerTile));
    }

    #[test]
    fn apply_chamber_stacks_joker_copies_across_chambers() {
        let mut run = RunState::new(GameMode::standard());
        run.grant_relic(RelicId::JokerTile);
        run.apply_chamber(ChamberKind::Small, None);
        let after_first = run.joker_extra_faces.len();
        assert_eq!(after_first, 1);

        run.apply_chamber(ChamberKind::Big, None);
        assert_eq!(run.joker_extra_faces.len(), after_first + 1);
    }
}

#[cfg(test)]
mod wild_wind_tests {
    use crate::{
        core::{
            hand::MeldKind,
            tile::{Suit, Tile},
        },
        game::run::{try_wind_substitution, wind_candidate_faces},
    };

    fn tile(suit: Suit, rank: u8, id: u32) -> Tile {
        Tile::new(suit, rank, id)
    }

    #[test]
    fn two_winds_substitute_into_sequences() {
        // Hand: 2m W 4m | 7m 8m 9m | 4s 5s 6s | 7p 8p W
        // With Wild Winds, W->3m and W->9p (or 6p) should yield 4 sequences.
        let tiles = vec![
            tile(Suit::Manzu, 2, 1),
            tile(Suit::Wind, 3, 2), // West, should become 3m
            tile(Suit::Manzu, 4, 3),
            tile(Suit::Manzu, 7, 4),
            tile(Suit::Manzu, 8, 5),
            tile(Suit::Manzu, 9, 6),
            tile(Suit::Souzu, 4, 7),
            tile(Suit::Souzu, 5, 8),
            tile(Suit::Souzu, 6, 9),
            tile(Suit::Pinzu, 7, 10),
            tile(Suit::Pinzu, 8, 11),
            tile(Suit::Wind, 3, 12), // West, should become 9p (or 6p)
        ];
        let result = try_wind_substitution(&tiles, &[]);
        assert!(
            result.is_some(),
            "two-wind substitution should find a valid hand"
        );
        let (sets, _) = result.unwrap();
        assert_eq!(sets.len(), 4);
        assert!(sets.iter().all(|s| s.kind == MeldKind::Sequence));
    }

    #[test]
    fn single_wind_substitutes_into_sequence() {
        // 1m 2m W -> W becomes 3m
        let tiles = vec![
            tile(Suit::Manzu, 1, 1),
            tile(Suit::Manzu, 2, 2),
            tile(Suit::Wind, 1, 3), // East
        ];
        let result = try_wind_substitution(&tiles, &[]);
        assert!(result.is_some());
        let (sets, _) = result.unwrap();
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].kind, MeldKind::Sequence);
    }

    #[test]
    fn wind_substitutes_into_triplet() {
        // 5s 5s W -> W becomes 5s for a triplet
        let tiles = vec![
            tile(Suit::Souzu, 5, 1),
            tile(Suit::Souzu, 5, 2),
            tile(Suit::Wind, 2, 3),
        ];
        let result = try_wind_substitution(&tiles, &[]);
        assert!(result.is_some());
        let (sets, _) = result.unwrap();
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].kind, MeldKind::Triplet);
    }

    #[test]
    fn no_winds_returns_none() {
        let tiles = vec![
            tile(Suit::Manzu, 1, 1),
            tile(Suit::Manzu, 2, 2),
            tile(Suit::Manzu, 3, 3),
        ];
        assert!(try_wind_substitution(&tiles, &[]).is_none());
    }

    #[test]
    fn impossible_hand_returns_none() {
        // W alone can't form any meld
        let tiles = vec![tile(Suit::Wind, 1, 1)];
        assert!(try_wind_substitution(&tiles, &[]).is_none());
    }

    #[test]
    fn candidates_include_nearby_ranks() {
        let tiles = vec![tile(Suit::Manzu, 5, 1), tile(Suit::Wind, 3, 2)];
        let candidates = wind_candidate_faces(&tiles);
        // Should include 3m-7m (5 ± 2) and 5m itself
        for r in 3..=7 {
            assert!(
                candidates.contains(&(Suit::Manzu, r)),
                "candidates should include {}m",
                r
            );
        }
        // Should NOT include 1m (too far)
        assert!(!candidates.contains(&(Suit::Manzu, 1)));
    }

    mod proptests {
        use crate::{core::tile::Suit, game::run::try_wind_substitution};

        use super::*;
        use proptest::prelude::*;

        const NUMBER_SUITS: [Suit; 3] = [Suit::Manzu, Suit::Souzu, Suit::Pinzu];

        fn arb_number_tile(id: u32) -> BoxedStrategy<Tile> {
            (0..3usize, 1..=9u8)
                .prop_map(move |(si, rank)| Tile::new(NUMBER_SUITS[si], rank, id))
                .boxed()
        }

        fn arb_wind_tile(id: u32) -> BoxedStrategy<Tile> {
            (1..=4u8)
                .prop_map(move |rank| Tile::new(Suit::Wind, rank, id))
                .boxed()
        }

        fn arb_dragon_tile(id: u32) -> BoxedStrategy<Tile> {
            (1..=3u8)
                .prop_map(move |rank| Tile::new(Suit::Dragon, rank, id))
                .boxed()
        }

        /// Mixed hand with at least one wind tile, 3..=9 tiles total.
        ///
        /// The upper bound is deliberately below 14. `try_wind_substitution`
        /// is combinatorial in the number of winds × candidate faces, so the
        /// legacy 4-winds + 10-other worst case could take tens of seconds
        /// per proptest case. The shape we actually want to cover —
        /// substitution behaves consistently across small, medium, and larger
        /// wind-heavy hands — is still exercised here without making the
        /// test suite unusable.
        fn arb_wind_hand() -> BoxedStrategy<Vec<Tile>> {
            (1usize..=3, 2usize..=6)
                .prop_flat_map(|(n_winds, n_other)| {
                    let wind_strats: Vec<BoxedStrategy<Tile>> =
                        (0..n_winds).map(|i| arb_wind_tile(i as u32)).collect();
                    let other_strats: Vec<BoxedStrategy<Tile>> = (0..n_other)
                        .map(|i| {
                            let id = (n_winds + i) as u32;
                            prop_oneof![
                                arb_number_tile(id),
                                arb_wind_tile(id),
                                arb_dragon_tile(id),
                            ]
                            .boxed()
                        })
                        .collect();
                    (wind_strats, other_strats).prop_map(|(mut w, o)| {
                        w.extend(o);
                        w
                    })
                })
                .boxed()
        }

        /// Extract the multiset of (suit, rank) faces assigned to the wind tiles
        /// in `original` after substitution into `modified`.
        fn wind_face_multiset(original: &[Tile], modified: &[Tile]) -> Vec<(Suit, u8)> {
            let wind_ids: rustc_hash::FxHashSet<u32> = original
                .iter()
                .filter(|t| t.suit == Suit::Wind)
                .map(|t| t.id)
                .collect();
            let mut faces: Vec<(Suit, u8)> = modified
                .iter()
                .filter(|m| wind_ids.contains(&m.id))
                .map(|m| (m.suit, m.rank))
                .collect();
            faces.sort();
            faces
        }

        proptest! {
            #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

            // ── Property: permutation invariance (multiset) ───────────
            //
            // Reordering the input tiles must not change the *multiset* of faces
            // assigned to wind tiles. The per-id assignment can still vary when
            // multiple valid substitutions exist (e.g. three East winds with
            // 2m could become {1m,1m,2m,2m} with either wind taking which rank),
            // but the set of faces produced should be invariant. The old
            // HashSet-backed candidate list could pick structurally different
            // substitutions based on hash iteration order — this property would
            // catch that regression.
            #[test]
            fn permutation_invariance(
                tiles in arb_wind_hand(),
                perm_seed in any::<u64>(),
            ) {
                use rand::SeedableRng;
                use rand::seq::SliceRandom;

                let Some((_sets_a, modified_a)) = try_wind_substitution(&tiles, &[]) else {
                    return Ok(());
                };

                let mut shuffled = tiles.clone();
                let mut rng = rand::rngs::StdRng::seed_from_u64(perm_seed);
                shuffled.shuffle(&mut rng);

                let Some((_sets_b, modified_b)) = try_wind_substitution(&shuffled, &[]) else {
                    prop_assert!(
                        false,
                        "permuted hand rejected while original accepted: {:?} -> {:?}",
                        tiles,
                        shuffled
                    );
                    return Ok(());
                };

                let faces_a = wind_face_multiset(&tiles, &modified_a);
                let faces_b = wind_face_multiset(&shuffled, &modified_b);
                prop_assert_eq!(
                    faces_a,
                    faces_b,
                    "wind face multiset depends on input order (tiles={:?})",
                    tiles
                );
            }
        }

        proptest! {
            #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

            // ── Property: substitution output re-validates ────────────
            //
            // If substitution returns (sets, modified), the modified tile list
            // must itself validate without any further wildcard magic — otherwise
            // the scorer is handed melds that don't actually match the tiles.
            #[test]
            fn substitution_output_revalidates(tiles in arb_wind_hand()) {
                if let Some((sets, modified)) = try_wind_substitution(&tiles, &[]) {
                    let revalidated = crate::core::hand::validate_selection_with_rules(&modified, &[]);
                    prop_assert!(
                        revalidated.is_some(),
                        "substitution output failed to revalidate: modified={:?}",
                        modified
                    );
                    let revalidated = revalidated.unwrap();
                    prop_assert_eq!(sets.len(), revalidated.len(), "set count mismatch");
                }
            }
        }

        proptest! {
            #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

            // ── Property: tile IDs preserved exactly ──────────────────
            //
            // Substitution rewrites face (suit, rank) but must never drop,
            // duplicate, or invent tile IDs.
            #[test]
            fn ids_preserved(tiles in arb_wind_hand()) {
                if let Some((_sets, modified)) = try_wind_substitution(&tiles, &[]) {
                    let mut input_ids: Vec<u32> = tiles.iter().map(|t| t.id).collect();
                    let mut output_ids: Vec<u32> = modified.iter().map(|t| t.id).collect();
                    input_ids.sort();
                    output_ids.sort();
                    prop_assert_eq!(input_ids, output_ids);
                }
            }
        }

        proptest! {
            #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

            // ── Property: non-wind tiles unchanged ────────────────────
            //
            // Only wind tiles may be rewritten. A bug that substitutes the wrong
            // index would show up here.
            #[test]
            fn non_winds_unchanged(tiles in arb_wind_hand()) {
                if let Some((_sets, modified)) = try_wind_substitution(&tiles, &[]) {
                    for orig in &tiles {
                        if orig.suit != Suit::Wind {
                            let m = modified.iter().find(|m| m.id == orig.id).unwrap();
                            prop_assert_eq!(
                                (m.suit, m.rank),
                                (orig.suit, orig.rank),
                                "non-wind tile id={} was rewritten",
                                orig.id
                            );
                        }
                    }
                }
            }
        }

        proptest! {
            #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

            // ── Property: no panics on arbitrary input ────────────────
            #[test]
            fn no_panic_on_arbitrary_hand(tiles in arb_wind_hand()) {
                let _ = try_wind_substitution(&tiles, &[]);
            }
        }
    }
}

#[cfg(test)]
mod progression_snapshot_tests {
    use crate::core::progression::PlayerProgress;
    use crate::{core::relic::RelicId, game::run::RunState};

    #[test]
    fn run_relic_unlocks_only_change_when_a_new_run_applies_progression() {
        // `BigHands` first unlocks at level 7. We bump from L6 → L7 in one
        // step (`check_level_up` only re-emits the current level's relics)
        // and make sure `apply_progression` only refreshes the run's
        // available pool when a fresh `RunState` is created.
        let mut progress = PlayerProgress::new();
        progress.level_progress_points = PlayerProgress::min_points_for_level(6);
        progress.check_level_up();

        let mut current_run = RunState::new_demo();
        current_run.apply_progression(&progress);
        assert!(!current_run.available_relics.contains(&RelicId::BigHands));

        progress.level_progress_points = PlayerProgress::min_points_for_level(7);
        let result = progress
            .check_level_up()
            .expect("crossing into level 7 should unlock new relics");
        assert!(result.relics.contains(&RelicId::BigHands));
        assert!(!current_run.available_relics.contains(&RelicId::BigHands));

        let mut next_run = RunState::new_demo();
        next_run.apply_progression(&progress);
        assert!(next_run.available_relics.contains(&RelicId::BigHands));
    }
}

#[cfg(test)]
mod disgust_tests {
    use crate::{
        core::{
            hand::MeldKind,
            tile::{Suit, Tile},
        },
        game::run::try_disgust_substitution,
    };

    fn tile(suit: Suit, rank: u8, id: u32) -> Tile {
        Tile::new(suit, rank, id)
    }

    #[test]
    fn ew_validates_as_pair() {
        let tiles = vec![tile(Suit::Wind, 1, 0), tile(Suit::Wind, 3, 1)];
        let (sets, _) = try_disgust_substitution(&tiles, &[], false).expect("EW should be a pair");
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].kind, MeldKind::Pair);
    }

    #[test]
    fn eww_validates_as_triplet() {
        let tiles = vec![
            tile(Suit::Wind, 1, 0),
            tile(Suit::Wind, 3, 1),
            tile(Suit::Wind, 3, 2),
        ];
        let (sets, _) =
            try_disgust_substitution(&tiles, &[], false).expect("EWW should be a triplet");
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].kind, MeldKind::Triplet);
    }

    #[test]
    fn ewww_validates_as_kong() {
        let tiles = vec![
            tile(Suit::Wind, 1, 0),
            tile(Suit::Wind, 3, 1),
            tile(Suit::Wind, 3, 2),
            tile(Suit::Wind, 3, 3),
        ];
        let (sets, _) =
            try_disgust_substitution(&tiles, &[], false).expect("EWWW should be a kong");
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].kind, MeldKind::Kong);
    }

    #[test]
    fn requires_both_east_and_west() {
        // No East: should not fire.
        let tiles = vec![tile(Suit::Wind, 3, 0), tile(Suit::Wind, 3, 1)];
        assert!(try_disgust_substitution(&tiles, &[], false).is_none());
        // No West: should not fire.
        let tiles = vec![tile(Suit::Wind, 1, 0), tile(Suit::Wind, 1, 1)];
        assert!(try_disgust_substitution(&tiles, &[], false).is_none());
    }

    #[test]
    fn nonsense_selection_still_invalid() {
        // EW + a stray souzu cannot decompose even after relabel.
        let tiles = vec![
            tile(Suit::Wind, 1, 0),
            tile(Suit::Wind, 3, 1),
            tile(Suit::Souzu, 5, 2),
        ];
        assert!(try_disgust_substitution(&tiles, &[], false).is_none());
    }
}
