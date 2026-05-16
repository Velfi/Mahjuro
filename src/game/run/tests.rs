use super::*;
#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::deck::build_wall;
    use crate::core::relic::{
        RelicId, ScoreContext, ScoreEconomyBundle, ScorePatternBundle, ScoreRelicBundle,
        ScoreRoundBundle, ScoreTileBundle,
    };

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
            starting_gold: 0,
            starting_rules: vec![],
            starting_yaku: vec![],
            ..GameMode::standard()
        };
        RunState {
            ante: 1,
            available_rules: vec![],
            available_yaku: vec![],
            available_relics: default_available_relics(),
            base_target: mode.base_target,
            blind: BlindKind::Small,
            boss: BossState::default(),
            consumables: crate::core::consumable::ConsumableInventory::default(),
            discards_remaining: mode.starting_discards,
            discards_max: mode.starting_discards,
            full_hand_played_this_round: false,
            gold: mode.starting_gold as i32,
            hand,
            structure_sets: vec![],
            structure_tiles: vec![],
            joker_used: false,
            last_breakdown: None,
            mode: mode.clone(),
            auto_cash_in_on_full_structure: true,
            hints_enabled: false,
            played_yaku_this_round: vec![],
            tile_debuffs: vec![],
            honors_scored_this_round: false,
            yaku_times_played: rustc_hash::FxHashMap::default(),
            tiles_played: 0,
            tiles_discarded: 0,
            times_restocked: 0,
            best_structure_score: 0,
            best_structure_name: String::new(),
            plays_remaining: mode.starting_plays,
            plays_max: mode.starting_plays,
            quickdraw_uses_remaining: 0,
            relics: RelicState::default(),
            round_rules: vec![],
            round_score: 0,
            run_number: 1,
            scored_last_turn: false,
            selected,
            target_score: mode.base_target,
            tile_enhancements: BTreeMap::new(),
            global_buff_enhancement: None,
            removed_tile_ids: rustc_hash::FxHashSet::default(),
            upcoming_blind: BlindKind::Small,
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
            small_blind_tag: None,
            big_blind_tag: None,
            tag_free_reroll: false,
            tag_patron_gift: false,
            tag_rich_stock: false,
            tag_bonus_plays: 0,
            tag_bonus_discards: 0,
            tag_bonus_hand_size: 0,
            pending_zodiac_celebration: None,
            finished_zodiac_celebration: None,
            pending_shop_focus_snap_after_pack_celebration: false,
            relic_counters: BTreeMap::new(),
            tutorial: None,
            onboarding: None,
            relic_activations: Vec::new(),
        }
    }

    fn bus() -> EventBus {
        EventBus::default()
    }

    fn winning_structure() -> (Vec<Tile>, Vec<DetectedMeld>) {
        let tiles = vec![
            Tile::new(Suit::Characters, 1, 1),
            Tile::new(Suit::Characters, 1, 2),
            Tile::new(Suit::Characters, 2, 3),
            Tile::new(Suit::Characters, 3, 4),
            Tile::new(Suit::Characters, 4, 5),
            Tile::new(Suit::Dots, 2, 6),
            Tile::new(Suit::Dots, 3, 7),
            Tile::new(Suit::Dots, 4, 8),
            Tile::new(Suit::Bamboos, 5, 9),
            Tile::new(Suit::Bamboos, 6, 10),
            Tile::new(Suit::Bamboos, 7, 11),
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
    fn apply_blind_rebuilds_round_resources_from_current_bonuses() {
        let mut run = test_run();
        run.plays_remaining = 1;
        run.discards_remaining = 0;
        run.tag_bonus_plays = 1;
        run.tag_bonus_discards = 1;

        run.apply_blind(BlindKind::Small, None);

        assert_eq!(run.plays_remaining, STARTING_PLAYS + 1);
        assert_eq!(run.plays_max, STARTING_PLAYS + 1);
        assert_eq!(run.discards_remaining, STARTING_DISCARDS + 1);
        assert_eq!(run.discards_max, STARTING_DISCARDS + 1);
        assert_eq!(run.tag_bonus_plays, 0);
        assert_eq!(run.tag_bonus_discards, 0);
    }

    #[test]
    fn second_wind_salvages_round_instead_of_game_over() {
        let mut run = test_run();
        let mut bus = bus();
        run.relics.active.push(RelicId::SecondWind);
        run.hand = vec![
            Tile::new(Suit::Characters, 1, 1),
            Tile::new(Suit::Characters, 3, 2),
            Tile::new(Suit::Characters, 5, 3),
            Tile::new(Suit::Characters, 7, 4),
            Tile::new(Suit::Characters, 9, 5),
            Tile::new(Suit::Bamboos, 2, 6),
            Tile::new(Suit::Bamboos, 4, 7),
            Tile::new(Suit::Bamboos, 6, 8),
            Tile::new(Suit::Bamboos, 8, 9),
            Tile::new(Suit::Dots, 1, 10),
            Tile::new(Suit::Dots, 3, 11),
            Tile::new(Suit::Dots, 5, 12),
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
        run.forfeit_current_blind_second_wind(&mut bus);
        assert_eq!(run.upcoming_blind, BlindKind::Big);
        assert_eq!(run.run_number, 2);
    }

    #[test]
    fn second_wind_plays_used_uses_effective_round_cap() {
        let mut run = test_run();
        run.apply_blind(BlindKind::Small, None);
        run.plays_remaining -= 2;

        let rw = Some(BlindKind::round_wind_for_ante(run.ante));
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
                played_yaku_this_round: run.played_yaku_this_round.clone(),
                is_final_play: run.plays_remaining == 0,
            },
            pattern: ScorePatternBundle {
                dora_faces: run.wall.dora_faces(),
                available_yaku: run.available_yaku.clone(),
                yaku_levels: Some(run.yaku_levels.clone()),
            },
            economy: ScoreEconomyBundle {
                gold: run.gold,
                total_score: run.total_score_earned,
            },
            structure: None,
        };

        assert_eq!(ctx.round.plays_used, 2);
    }

    #[test]
    fn apply_blind_uses_material_starting_discards_before_skip_bonus() {
        let mut run = RunState::new(GameMode::with_material(
            crate::persistence::TileMaterial::Plastic,
        ));
        run.discards_remaining = 0;
        run.tag_bonus_discards = 1;

        run.apply_blind(BlindKind::Small, None);

        assert_eq!(run.discards_remaining, 6);
        assert_eq!(run.discards_max, 6);
        assert_eq!(run.tag_bonus_discards, 0);
    }

    #[test]
    fn apply_blind_tracks_reduced_round_caps_for_boss_taxes() {
        let mut run = test_run();
        run.boss.effect = Some(crate::core::boss::ResolvedBossEffect::from_static(
            &crate::core::boss::BossKind::Drought.def().effect,
        ));

        run.apply_blind(BlindKind::Boss, None);

        assert_eq!(run.discards_remaining, STARTING_DISCARDS / 2);
        assert_eq!(run.discards_max, STARTING_DISCARDS / 2);
    }

    #[test]
    fn big_hands_increases_effective_hand_and_reduces_discard_cap() {
        let mut run = test_run();
        run.relics.active.push(RelicId::BigHands);
        assert_eq!(boss::effective_hand_size(&run), HAND_SIZE + 2);
        run.reset_round_resources();
        assert_eq!(run.discards_remaining, STARTING_DISCARDS - 1);
        assert_eq!(run.discards_max, STARTING_DISCARDS - 1);
    }

    #[test]
    fn tiny_hands_decreases_effective_hand_and_adds_discard_cap() {
        let mut run = test_run();
        run.relics.active.push(RelicId::TinyHands);
        assert_eq!(boss::effective_hand_size(&run), HAND_SIZE - 2);
        run.reset_round_resources();
        assert_eq!(run.discards_remaining, STARTING_DISCARDS + 2);
        assert_eq!(run.discards_max, STARTING_DISCARDS + 2);
    }

    #[test]
    fn big_hands_and_tiny_hands_cancel_hand_delta() {
        let mut run = test_run();
        run.relics.active.push(RelicId::BigHands);
        run.relics.active.push(RelicId::TinyHands);
        assert_eq!(boss::effective_hand_size(&run), HAND_SIZE);
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
    fn apply_blind_promotes_wide_hand_bonus_to_round_hand_size() {
        let mut run = test_run();
        run.apply_tag(crate::core::tag::TagKind::WideHand, None);

        run.apply_blind(BlindKind::Small, None);

        assert_eq!(run.hand.len(), HAND_SIZE + 2);
        assert_eq!(boss::effective_hand_size(&run), HAND_SIZE + 2);
        assert_eq!(run.tag_bonus_hand_size, 0);
    }

    #[test]
    fn skipping_with_wide_hand_carries_bonus_into_next_blind() {
        let mut run = test_run();

        run.apply_tag(crate::core::tag::TagKind::WideHand, None);
        run.skip_to_next_blind();

        assert_eq!(run.tag_bonus_hand_size, 2);

        run.apply_blind(BlindKind::Big, None);

        assert_eq!(run.hand.len(), HAND_SIZE + 2);
        assert_eq!(boss::effective_hand_size(&run), HAND_SIZE + 2);
        assert_eq!(run.tag_bonus_hand_size, 0);
    }

    #[test]
    fn advance_round_after_boss_preserves_pending_shop_skip_rewards() {
        let mut run = test_run();
        let mut bus = bus();
        run.blind = BlindKind::Boss;
        run.upcoming_blind = BlindKind::Boss;
        run.tag_free_reroll = true;
        run.tag_patron_gift = true;
        run.tag_rich_stock = true;

        run.advance_round(&mut bus);

        assert!(run.tag_free_reroll);
        assert!(run.tag_patron_gift);
        assert!(run.tag_rich_stock);
    }

    #[test]
    fn advance_round_after_boss_clears_unconsumed_next_blind_skip_bonuses() {
        let mut run = test_run();
        let mut bus = bus();
        run.blind = BlindKind::Boss;
        run.upcoming_blind = BlindKind::Boss;
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
    fn dragon_allows_non_honor_structure_but_debuffs_its_score() {
        let tiles = vec![
            Tile::new(Suit::Characters, 1, 1),
            Tile::new(Suit::Characters, 1, 2),
            Tile::new(Suit::Characters, 1, 3),
        ];
        let sets = vec![DetectedMeld {
            kind: MeldKind::Triplet,
            tile_ids: vec![1, 2, 3],
        }];

        let mut baseline = test_run();
        baseline.blind = BlindKind::Small;
        baseline.structure_tiles = tiles.clone();
        baseline.structure_sets = sets.clone();
        let mut baseline_bus = bus();
        let baseline_earned = baseline.trigger_structure_manual(&mut baseline_bus);
        assert!(baseline_earned > 0);
        let baseline_score = baseline.round_score;

        let mut dragon = test_run();
        dragon.blind = BlindKind::Boss;
        dragon.upcoming_blind = BlindKind::Boss;
        dragon.boss.upcoming = Some(BossKind::Dragon);
        dragon.structure_tiles = tiles;
        dragon.structure_sets = sets;
        let mut dragon_bus = bus();
        let dragon_earned = dragon.trigger_structure_manual(&mut dragon_bus);
        assert!(
            dragon_earned > 0,
            "Dragon should still allow structure cash-in"
        );
        assert!(
            dragon.round_score > 0,
            "debuffed Dragon cash-in should still score something"
        );
        assert!(
            dragon.round_score < baseline_score,
            "Dragon should weaken non-honor cash-ins vs a non-Dragon blind"
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
        let mut run = test_run();
        let mut bus = bus();
        run.hand = vec![
            Tile::new(Suit::Characters, 1, 1),
            Tile::new(Suit::Characters, 3, 2),
            Tile::new(Suit::Characters, 5, 3),
            Tile::new(Suit::Characters, 7, 4),
            Tile::new(Suit::Characters, 9, 5),
            Tile::new(Suit::Bamboos, 2, 6),
            Tile::new(Suit::Bamboos, 4, 7),
            Tile::new(Suit::Bamboos, 6, 8),
            Tile::new(Suit::Bamboos, 8, 9),
            Tile::new(Suit::Dots, 1, 10),
            Tile::new(Suit::Dots, 3, 11),
            Tile::new(Suit::Dots, 5, 12),
            Tile::new(Suit::Wind, 1, 13),
            Tile::new(Suit::Dragon, 1, 14),
        ];
        run.selected = vec![false; run.hand.len()];
        run.discards_remaining = 0;
        run.plays_remaining = 3;
        run.structure_sets.clear();
        run.structure_tiles.clear();

        run.refill_hand(&mut bus);

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
            Tile::new(Suit::Characters, 1, 1),
            Tile::new(Suit::Characters, 3, 2),
            Tile::new(Suit::Characters, 5, 3),
            Tile::new(Suit::Characters, 7, 4),
            Tile::new(Suit::Characters, 9, 5),
            Tile::new(Suit::Bamboos, 2, 6),
            Tile::new(Suit::Bamboos, 4, 7),
            Tile::new(Suit::Bamboos, 6, 8),
            Tile::new(Suit::Bamboos, 8, 9),
            Tile::new(Suit::Dots, 1, 10),
            Tile::new(Suit::Dots, 3, 11),
            Tile::new(Suit::Dots, 5, 12),
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
}

#[cfg(test)]
mod joker_tile_tests {
    use super::*;

    fn tile(suit: Suit, rank: u8, id: u32) -> Tile {
        Tile::new(suit, rank, id)
    }

    #[test]
    fn joker_completes_sequence() {
        // 1m 2m 5s — joker should turn 5s into 3m
        let tiles = vec![
            tile(Suit::Characters, 1, 0),
            tile(Suit::Characters, 2, 1),
            tile(Suit::Bamboos, 5, 2),
        ];
        let result = try_joker_substitution(&tiles, &[]);
        assert!(result.is_some(), "joker should complete the sequence");
        let (sets, modified) = result.unwrap();
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].kind, MeldKind::Sequence);
        // The modified tile should now be 3m
        assert_eq!(modified[2].suit, Suit::Characters);
        assert_eq!(modified[2].rank, 3);
    }

    #[test]
    fn joker_completes_triplet() {
        // 7p 7p 1s — joker should turn 1s into 7p
        let tiles = vec![
            tile(Suit::Dots, 7, 0),
            tile(Suit::Dots, 7, 1),
            tile(Suit::Bamboos, 1, 2),
        ];
        let result = try_joker_substitution(&tiles, &[]);
        assert!(result.is_some());
        let (sets, _) = result.unwrap();
        assert_eq!(sets[0].kind, MeldKind::Triplet);
    }

    #[test]
    fn joker_makes_pair_from_two_tiles() {
        // 1m 5s — joker turns 5s into 1m for a pair
        let tiles = vec![tile(Suit::Characters, 1, 0), tile(Suit::Bamboos, 5, 1)];
        let result = try_joker_substitution(&tiles, &[]);
        assert!(result.is_some());
        let (sets, _) = result.unwrap();
        assert_eq!(sets[0].kind, MeldKind::Pair);
    }

    #[test]
    fn joker_only_substitutes_one_tile() {
        // 1m 5s 9p — all different, need 2 subs to make a meld, joker can only do 1
        let tiles = vec![
            tile(Suit::Characters, 1, 0),
            tile(Suit::Bamboos, 5, 1),
            tile(Suit::Dots, 9, 2),
        ];
        assert!(try_joker_substitution(&tiles, &[]).is_none());
    }

    #[test]
    fn joker_respects_no_sequences_rule() {
        // 1m 2m 5s — would be a sequence with joker, but NoSequences blocks it
        let tiles = vec![
            tile(Suit::Characters, 1, 0),
            tile(Suit::Characters, 2, 1),
            tile(Suit::Bamboos, 5, 2),
        ];
        let result = try_joker_substitution(&tiles, &[RuleModifier::NoSequences]);
        // Could still work if joker turns 5s into 1m or 2m for a triplet — but
        // we only have 2 of those, so a triplet needs the joker tile to match one.
        // 1m 2m 1m → not a valid decomposition (pair 1m + leftover 2m).
        // 1m 2m 2m → pair 2m + leftover 1m. Also invalid.
        // No triplet possible, so should be None.
        assert!(result.is_none());
    }
}

#[cfg(test)]
mod wild_wind_tests {
    use super::*;

    fn tile(suit: Suit, rank: u8, id: u32) -> Tile {
        Tile::new(suit, rank, id)
    }

    #[test]
    fn two_winds_substitute_into_sequences() {
        // Hand: 2m W 4m | 7m 8m 9m | 4s 5s 6s | 7p 8p W
        // With Wild Winds, W->3m and W->9p (or 6p) should yield 4 sequences.
        let tiles = vec![
            tile(Suit::Characters, 2, 1),
            tile(Suit::Wind, 3, 2), // West, should become 3m
            tile(Suit::Characters, 4, 3),
            tile(Suit::Characters, 7, 4),
            tile(Suit::Characters, 8, 5),
            tile(Suit::Characters, 9, 6),
            tile(Suit::Bamboos, 4, 7),
            tile(Suit::Bamboos, 5, 8),
            tile(Suit::Bamboos, 6, 9),
            tile(Suit::Dots, 7, 10),
            tile(Suit::Dots, 8, 11),
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
            tile(Suit::Characters, 1, 1),
            tile(Suit::Characters, 2, 2),
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
            tile(Suit::Bamboos, 5, 1),
            tile(Suit::Bamboos, 5, 2),
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
            tile(Suit::Characters, 1, 1),
            tile(Suit::Characters, 2, 2),
            tile(Suit::Characters, 3, 3),
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
        let tiles = vec![tile(Suit::Characters, 5, 1), tile(Suit::Wind, 3, 2)];
        let candidates = wind_candidate_faces(&tiles);
        // Should include 3m-7m (5 ± 2) and 5m itself
        for r in 3..=7 {
            assert!(
                candidates.contains(&(Suit::Characters, r)),
                "candidates should include {}m",
                r
            );
        }
        // Should NOT include 1m (too far)
        assert!(!candidates.contains(&(Suit::Characters, 1)));
    }

    mod proptests {
        use super::*;
        use proptest::prelude::*;

        const NUMBER_SUITS: [Suit; 3] = [Suit::Characters, Suit::Bamboos, Suit::Dots];

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
    use super::*;

    #[test]
    fn run_relic_unlocks_only_change_when_a_new_run_applies_progression() {
        // `BigHands` first unlocks at level 7. We bump from L6 → L7 in one
        // step (`check_level_up` only re-emits the current level's relics)
        // and make sure `apply_progression` only refreshes the run's
        // available pool when a fresh `RunState` is created.
        let mut progress = crate::core::progression::PlayerProgress::new();
        progress.runs_completed = 6;
        progress.check_level_up();

        let mut current_run = RunState::new_demo();
        current_run.apply_progression(&progress);
        assert!(!current_run.available_relics.contains(&RelicId::BigHands));

        progress.runs_completed = 7;
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
    use super::*;

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
        // EW + a stray bamboo cannot decompose even after relabel.
        let tiles = vec![
            tile(Suit::Wind, 1, 0),
            tile(Suit::Wind, 3, 1),
            tile(Suit::Bamboos, 5, 2),
        ];
        assert!(try_disgust_substitution(&tiles, &[], false).is_none());
    }
}
