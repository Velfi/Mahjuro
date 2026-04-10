//! Headless bot runner used for tuning balance.
//!
//! The bot picks the highest-scoring valid play available in its current hand each turn.
//! Between turns it strategically discards isolated tiles via 1-step rollout when the
//! best play falls below the pace needed to clear. Between blinds it picks the relic
//! that most increases its current best-play value, visits the shop and buys the
//! affordable relic with the largest marginal value, and skips Small/Big blinds when
//! its expected score comfortably exceeds the target.
//!
//! Run with: `cargo run --release -- --bot 200`

use rand::RngExt;
use rand::seq::SliceRandom;

use crate::core::deck::Wall;
use crate::core::hand::{detect_all_sets, validate_selection_with_rules};
use crate::core::relic::{RelicId, ScoreContext, all_relic_defs, relic_buy_price};
use crate::core::rules::{BlindKind, RuleModifier};
use crate::core::scoring::score_sets;
use crate::core::tile::Tile;
use crate::game::event_bus::{EventBus, GameEvent};
use crate::game::game_mode::GameMode;
use crate::game::run::RunState;

/// Per-run telemetry collected during a single bot playthrough.
#[derive(Debug, Clone)]
pub struct RunStats {
    /// How many blinds the bot successfully cleared in this run.
    pub blinds_cleared: u32,
    /// How many antes the bot fully cleared (each ante = Small+Big+Boss).
    pub antes_cleared: u32,
    /// Whether the bot survived all `FINAL_ANTE` antes.
    pub victory: bool,
    /// The ante the bot died on (or `FINAL_ANTE+1` on victory).
    pub died_on_ante: u32,
    /// The blind the bot died on (only meaningful if `!victory`).
    pub died_on_blind: BlindKind,
    /// Cumulative score across every blind in the run.
    pub total_score: u64,
    /// Plays consumed across the run.
    pub plays_used: u32,
    /// Discards consumed across the run.
    pub discards_used: u32,
    /// Subset of `discards_used` that were strategic (vs. random fallback).
    pub strategic_discards: u32,
    /// Final gold balance when the run ended.
    pub final_gold: i32,
    /// How many Small/Big blinds the bot chose to skip (banking gold).
    pub blinds_skipped: u32,
    /// How many relics the bot purchased from the shop.
    pub relics_bought: u32,
    /// Total gold spent in shops.
    pub gold_spent: u32,
}

impl Default for RunStats {
    fn default() -> Self {
        Self {
            blinds_cleared: 0,
            antes_cleared: 0,
            victory: false,
            died_on_ante: 1,
            died_on_blind: BlindKind::Small,
            total_score: 0,
            plays_used: 0,
            discards_used: 0,
            strategic_discards: 0,
            final_gold: 0,
            blinds_skipped: 0,
            relics_bought: 0,
            gold_spent: 0,
        }
    }
}

/// Aggregate statistics across many bot runs.
#[derive(Debug, Default, Clone)]
pub struct AggregateStats {
    pub runs: u32,
    pub blinds_cleared_total: u64,
    pub antes_cleared_total: u64,
    pub victories: u32,
    pub max_ante_reached: u32,
    pub total_score: u64,
    pub total_plays: u64,
    pub total_discards: u64,
    pub total_strategic_discards: u64,
    pub total_blinds_skipped: u64,
    pub total_relics_bought: u64,
    pub total_gold_spent: u64,
    /// Histogram of (ante -> count of runs that died on that ante).
    pub deaths_by_ante: std::collections::BTreeMap<u32, u32>,
    /// Histogram of (blind -> count of runs that died on that blind).
    pub deaths_by_blind: std::collections::BTreeMap<&'static str, u32>,
}

impl AggregateStats {
    fn record(&mut self, s: &RunStats) {
        self.runs += 1;
        self.blinds_cleared_total += s.blinds_cleared as u64;
        self.antes_cleared_total += s.antes_cleared as u64;
        if s.victory {
            self.victories += 1;
        }
        self.max_ante_reached = self.max_ante_reached.max(s.died_on_ante);
        self.total_score += s.total_score;
        self.total_plays += s.plays_used as u64;
        self.total_discards += s.discards_used as u64;
        self.total_strategic_discards += s.strategic_discards as u64;
        self.total_blinds_skipped += s.blinds_skipped as u64;
        self.total_relics_bought += s.relics_bought as u64;
        self.total_gold_spent += s.gold_spent as u64;
        *self.deaths_by_ante.entry(s.died_on_ante).or_insert(0) += 1;
        if !s.victory {
            *self
                .deaths_by_blind
                .entry(s.died_on_blind.name())
                .or_insert(0) += 1;
        }
    }

    pub fn print_summary(&self) {
        println!("\n=== Bot Stats ({} runs) ===", self.runs);
        if self.runs == 0 {
            return;
        }
        let avg_blinds = self.blinds_cleared_total as f64 / self.runs as f64;
        let avg_antes = self.antes_cleared_total as f64 / self.runs as f64;
        let avg_score = self.total_score as f64 / self.runs as f64;
        let avg_plays = self.total_plays as f64 / self.runs as f64;
        let avg_discards = self.total_discards as f64 / self.runs as f64;
        let avg_strategic = self.total_strategic_discards as f64 / self.runs as f64;
        let win_rate = self.victories as f64 * 100.0 / self.runs as f64;
        println!(
            "victories:           {} / {} ({:.1}%)",
            self.victories, self.runs, win_rate
        );
        println!("avg blinds cleared:  {:.2}", avg_blinds);
        println!("avg antes cleared:   {:.2}", avg_antes);
        println!("max ante reached:    {}", self.max_ante_reached);
        println!("avg total score:     {:.0}", avg_score);
        println!("avg plays used:      {:.2}", avg_plays);
        println!(
            "avg discards used:   {:.2} ({:.2} strategic, {:.2} random)",
            avg_discards,
            avg_strategic,
            avg_discards - avg_strategic
        );
        println!(
            "avg blinds skipped:  {:.2}",
            self.total_blinds_skipped as f64 / self.runs as f64
        );
        println!(
            "avg relics bought:   {:.2} (avg gold spent: {:.1})",
            self.total_relics_bought as f64 / self.runs as f64,
            self.total_gold_spent as f64 / self.runs as f64
        );
        println!("\ndeaths by ante:");
        for (ante, count) in &self.deaths_by_ante {
            let pct = *count as f64 * 100.0 / self.runs as f64;
            let bar = "#".repeat(((pct / 2.0).round() as usize).min(50));
            println!("  ante {:>2}: {:>4} ({:>5.1}%) {}", ante, count, pct, bar);
        }
        println!("\ndeaths by blind:");
        for (blind, count) in &self.deaths_by_blind {
            let pct = *count as f64 * 100.0 / self.runs as f64;
            println!("  {:<12} {:>4} ({:>5.1}%)", blind, count, pct);
        }
    }
}

/// Build a `ScoreContext` borrowing from the run.
fn ctx_for(run: &RunState) -> ScoreContext<'_> {
    ScoreContext {
        relics: &run.relics,
        scored_last_turn: run.scored_last_turn,
        dora_faces: run.wall.dora_faces(),
        available_yaku: run.available_yaku.clone(),
        round_wind: Some(crate::core::rules::BlindKind::round_wind_for_ante(run.ante)),
        first_full_hand_of_round: !run.full_hand_played_this_round,
        plays_used: run.mode.starting_plays.saturating_sub(run.plays_remaining),
        riichi_active: false,
        yaku_levels: Some(run.yaku_levels.clone()),
        yaku_loadout: run.yaku_loadout.clone(),
        played_yaku_this_round: run.played_yaku_this_round.clone(),
        gold: run.gold,
        total_score: run.total_score_earned,
        is_final_play: run.plays_remaining == 1,
        tile_polisher_bonus: run.tile_polisher_bonus,
        relic_counters: run.relic_counters.clone(),
        unscored_hand_tiles: 0,
        river_runner_bonus: run.river_runner_bonus,
    }
}

/// Find the best (score, indices) playable selection from `hand`.
/// Pure function — used both for the live hand and rollout hands.
fn best_play_in_hand(
    hand: &[Tile],
    rules: &[RuleModifier],
    ctx: &ScoreContext,
) -> Option<(i32, Vec<usize>)> {
    let n = hand.len();
    if n < 2 || n > 20 {
        return None;
    }
    let mut best: Option<(i32, Vec<usize>)> = None;
    let limit: u32 = 1u32 << n;
    for mask in 1u32..limit {
        let count = mask.count_ones() as usize;
        if matches!(count, 0 | 1 | 4 | 7 | 10 | 13) {
            continue;
        }
        let mut tiles: Vec<Tile> = Vec::with_capacity(count);
        for i in 0..n {
            if mask & (1 << i) != 0 {
                tiles.push(hand[i]);
            }
        }
        let Some(sets) = validate_selection_with_rules(&tiles, rules) else {
            continue;
        };
        let breakdown = score_sets(&tiles, &sets, ctx, rules);
        if breakdown.total <= 0 {
            continue;
        }
        if best
            .as_ref()
            .map(|(s, _)| breakdown.total > *s)
            .unwrap_or(true)
        {
            let indices: Vec<usize> = (0..n).filter(|i| mask & (1 << i) != 0).collect();
            best = Some((breakdown.total, indices));
        }
    }
    best
}

/// Search for the highest-scoring playable selection in the current hand.
/// Returns `(score, indices)`, or `None` if no positive-scoring play exists.
pub fn pick_best_play(run: &RunState) -> Option<(i32, Vec<usize>)> {
    let ctx = ctx_for(run);
    best_play_in_hand(&run.hand, &run.round_rules, &ctx)
}

/// Rate each tile by how many *potential* melds in the current hand it participates in.
/// A tile that appears in zero detected sets is "isolated" and a prime discard target.
/// Returns a vector parallel to `hand` containing usage counts.
fn tile_meld_participation(hand: &[Tile]) -> Vec<u32> {
    let sets = detect_all_sets(hand);
    let mut counts = vec![0u32; hand.len()];
    for s in &sets {
        for id in &s.tile_ids {
            if let Some(idx) = hand.iter().position(|t| t.id == *id) {
                counts[idx] += 1;
            }
        }
    }
    counts
}

/// Generate up to `max_k` discard candidates ordered by tile participation: candidate K
/// drops the K lowest-participation tiles. Tiles already pulling weight in detected
/// melds are dumped last so we never voluntarily throw away a built partial.
fn discard_candidates(hand: &[Tile], max_k: usize) -> Vec<Vec<usize>> {
    if hand.len() < 3 {
        return Vec::new();
    }
    let counts = tile_meld_participation(hand);
    let mut indexed: Vec<(usize, u32)> = counts.into_iter().enumerate().collect();
    indexed.sort_by_key(|(_, c)| *c);
    let order: Vec<usize> = indexed.into_iter().map(|(i, _)| i).collect();
    let cap = max_k.min(hand.len() - 2);
    (1..=cap)
        .map(|k| order.iter().take(k).copied().collect())
        .collect()
}

/// Simulate discarding `discard_indices` from the hand and drawing replacements off the
/// top of the wall (peeked, not consumed). Returns the best playable score that the
/// resulting hand could produce. Uses 1-step lookahead with the actual upcoming tiles —
/// "perfect-information" oracle, which gives us a tuning ceiling rather than a
/// realistic player bot.
fn rollout_post_discard_score(run: &RunState, discard_indices: &[usize]) -> i32 {
    use std::collections::HashSet;
    let drop_set: HashSet<usize> = discard_indices.iter().copied().collect();
    let k = discard_indices.len();
    let peeked = run.wall.peek_next(k);
    let mut new_hand: Vec<Tile> = run
        .hand
        .iter()
        .enumerate()
        .filter(|(i, _)| !drop_set.contains(i))
        .map(|(_, t)| *t)
        .collect();
    new_hand.extend_from_slice(peeked);
    new_hand.sort();
    let ctx = ctx_for(run);
    best_play_in_hand(&new_hand, &run.round_rules, &ctx)
        .map(|(s, _)| s)
        .unwrap_or(0)
}

/// Play the current blind to completion. Returns `true` if the bot reached the target.
fn play_blind(run: &mut RunState, stats: &mut RunStats) -> bool {
    let mut bus = EventBus::default();
    let mut rng = rand::rng();

    loop {
        if run.round_score >= run.target_score {
            return true;
        }
        if run.plays_remaining == 0 {
            return false;
        }

        let best = pick_best_play(run);
        let best_score = best.as_ref().map(|(s, _)| *s).unwrap_or(0);

        // Strategic discard via 1-step rollout: try several candidate discard subsets,
        // peek the actual upcoming wall tiles, evaluate the best play in each
        // hypothetical hand, and take the discard whose post-rollout best play beats
        // the current best by a meaningful margin. The margin requirement prevents the
        // bot from burning a discard for a marginal +1 swing.
        let can_discard = run.discards_remaining > 0 && run.plays_remaining > 1;
        let mut did_discard = false;
        if can_discard {
            let candidates = discard_candidates(&run.hand, 5);
            // Margin scales with how far we are from target — late in the round we
            // need bigger swings to be worth losing a play.
            let need = run.target_score.saturating_sub(run.round_score) as i32;
            let margin = (need / (run.plays_remaining as i32 + 1)).max(5);
            let mut best_after: Option<(i32, Vec<usize>)> = None;
            for cand in candidates {
                let hyp = rollout_post_discard_score(run, &cand);
                if best_after.as_ref().map(|(s, _)| hyp > *s).unwrap_or(true) {
                    best_after = Some((hyp, cand));
                }
            }
            if let Some((after_score, indices)) = best_after {
                if after_score >= best_score + margin {
                    run.clear_selection();
                    for i in &indices {
                        run.toggle_select(*i);
                    }
                    run.discard_selected(&mut bus);
                    stats.discards_used += 1;
                    stats.strategic_discards += 1;
                    for _ in bus.drain() {}
                    did_discard = true;
                }
            }
        }
        if did_discard {
            continue;
        }

        if let Some((_, indices)) = best {
            run.clear_selection();
            for i in indices {
                run.toggle_select(i);
            }
            run.score_selected_tiles(&mut bus);
            stats.plays_used += 1;
            for ev in bus.drain() {
                if let GameEvent::RoundComplete { payout, .. } = ev {
                    run.gold = run.gold.saturating_add(payout.total as i32);
                }
            }
            continue;
        }

        // No positive-scoring play and no strategic discard helped — random discard
        // as a last-resort shake-up before busting.
        if run.discards_remaining == 0 {
            return false;
        }
        run.clear_selection();
        let hand_n = run.hand.len();
        if hand_n == 0 {
            return false;
        }
        let drop_n = rng.random_range(1..=hand_n.min(5));
        let mut indices: Vec<usize> = (0..hand_n).collect();
        indices.shuffle(&mut rng);
        for i in indices.into_iter().take(drop_n) {
            run.toggle_select(i);
        }
        run.discard_selected(&mut bus);
        stats.discards_used += 1;
        for _ in bus.drain() {}
    }
}

/// Number of synthetic random hands sampled when evaluating a relic's value.
/// Higher = more accurate signal but slower (each sample runs `best_play_in_hand`,
/// which is the bot's hot loop).
const RELIC_EVAL_SAMPLES: usize = 4;

/// Draw a random 14-tile hand from a fresh shuffled wall. Used for relic value
/// sampling — gives a "typical hand" the relic would face in future plays, not
/// just the bot's specific current hand.
fn sample_random_hand(size: usize) -> Vec<Tile> {
    let mut wall = Wall::from_standard_shuffled();
    let mut hand = Vec::with_capacity(size);
    for _ in 0..size {
        if let Some(t) = wall.draw() {
            hand.push(t);
        }
    }
    hand.sort();
    hand
}

/// Estimate the value of owning `candidate` by averaging the best-play score
/// improvement across the current hand *and* several synthetic random hands.
///
/// We need the random sampling because most relics' effects are
/// hand-conditional — `TripletBoost` is worthless on a hand with no triplets,
/// `BambooCharm` does nothing without bamboo tiles. Evaluating only the current
/// hand systematically under-values relics whose payoff is "applies whenever you
/// happen to draw the right tiles." A handful of synthetic hands surface that
/// expected value.
///
/// Wall-mutating relics (`Overflow`, `SetMagnet`, `QuickDraw`, `WildWinds`,
/// `JokerTile`) are still under-valued because we don't simulate draws between
/// plays. Rarity tie-break compensates.
fn relic_marginal_value(run: &RunState, candidate: RelicId) -> i32 {
    if run.relics.has(candidate) {
        return -1;
    }
    if run.relics.is_full() {
        return 0;
    }

    let mut hypothetical = run.relics.clone();
    hypothetical.active.push(candidate);

    let round_wind = Some(crate::core::rules::BlindKind::round_wind_for_ante(run.ante));
    let first_full = !run.full_hand_played_this_round;
    let plays_used = run.mode.starting_plays.saturating_sub(run.plays_remaining);
    let yaku_levels = Some(run.yaku_levels.clone());
    let yaku_loadout = run.yaku_loadout.clone();
    let played_yaku = run.played_yaku_this_round.clone();
    let baseline_ctx = ScoreContext {
        relics: &run.relics,
        scored_last_turn: run.scored_last_turn,
        dora_faces: run.wall.dora_faces(),
        available_yaku: run.available_yaku.clone(),
        round_wind,
        first_full_hand_of_round: first_full,
        plays_used,
        riichi_active: false,
        yaku_levels: yaku_levels.clone(),
        yaku_loadout: yaku_loadout.clone(),
        played_yaku_this_round: played_yaku.clone(),
        gold: run.gold,
        total_score: run.total_score_earned,
        is_final_play: run.plays_remaining == 1,
        tile_polisher_bonus: run.tile_polisher_bonus,
        relic_counters: run.relic_counters.clone(),
        unscored_hand_tiles: 0,
        river_runner_bonus: run.river_runner_bonus,
    };
    let hypo_ctx = ScoreContext {
        relics: &hypothetical,
        scored_last_turn: run.scored_last_turn,
        dora_faces: run.wall.dora_faces(),
        available_yaku: run.available_yaku.clone(),
        round_wind,
        first_full_hand_of_round: first_full,
        plays_used,
        riichi_active: false,
        yaku_levels,
        yaku_loadout,
        played_yaku_this_round: played_yaku,
        gold: run.gold,
        total_score: run.total_score_earned,
        is_final_play: run.plays_remaining == 1,
        tile_polisher_bonus: run.tile_polisher_bonus,
        relic_counters: run.relic_counters.clone(),
        unscored_hand_tiles: 0,
        river_runner_bonus: run.river_runner_bonus,
    };

    let score = |hand: &[Tile], ctx: &ScoreContext| -> i32 {
        best_play_in_hand(hand, &run.round_rules, ctx)
            .map(|(s, _)| s)
            .unwrap_or(0)
    };

    // Sample 1: the bot's actual current hand (weighted heavily).
    let mut delta_sum: i32 = score(&run.hand, &hypo_ctx) - score(&run.hand, &baseline_ctx);
    let mut sample_count: i32 = 1;

    // Samples 2..N: synthetic random hands from fresh walls.
    for _ in 0..RELIC_EVAL_SAMPLES {
        let hand = sample_random_hand(run.mode.hand_size);
        delta_sum += score(&hand, &hypo_ctx) - score(&hand, &baseline_ctx);
        sample_count += 1;
    }

    delta_sum / sample_count
}

/// Headless analogue of `ShopScene::new` + buy loop. Rolls 3 random non-owned relics
/// (matching `ShopScene::new`'s pool generation) and buys the one with the largest
/// positive marginal value the bot can afford. Repeats while gold and relic slots
/// allow another worthwhile purchase.
fn visit_shop(run: &mut RunState, stats: &mut RunStats) {
    // Consume tag-granted shop modifiers (headless analogue of ShopScene::new).
    let extra_relics: usize = if run.tag_rich_stock { 2 } else { 0 };
    let patron_gift = run.tag_patron_gift;
    // Free reroll is a no-op for the bot (it doesn't reroll).
    run.tag_free_reroll = false;
    run.tag_patron_gift = false;
    run.tag_rich_stock = false;

    let defs = all_relic_defs();
    let shop_excluded = [RelicId::IronLantern, RelicId::PhantomRelic];
    let mut pool: Vec<RelicId> = defs
        .iter()
        .filter(|d| !run.relics.has(d.id) && !shop_excluded.contains(&d.id))
        .map(|d| d.id)
        .collect();
    pool.shuffle(&mut rand::rng());
    let mut shop: Vec<RelicId> = pool.into_iter().take(3 + extra_relics).collect();

    let mut free_relic = patron_gift;
    loop {
        if run.relics.is_full() || shop.is_empty() {
            break;
        }
        // Find the best affordable item with positive marginal value.
        let mut best: Option<(usize, i32)> = None;
        for (i, &id) in shop.iter().enumerate() {
            let price = if free_relic { 0 } else { relic_buy_price(id) };
            if price as i32 > run.gold {
                continue;
            }
            let mv = relic_marginal_value(run, id);
            // Only buy if it actually helps; otherwise let the bot bank gold.
            if mv <= 0 {
                continue;
            }
            if best.as_ref().map(|(_, b)| mv > *b).unwrap_or(true) {
                best = Some((i, mv));
            }
        }
        let Some((idx, _)) = best else { break };
        let id = shop.remove(idx);
        let price = if free_relic { 0 } else { relic_buy_price(id) };
        free_relic = false;
        run.gold -= price as i32;
        run.relics.active.push(id);
        // Initialize counters for stateful relics.
        match id {
            RelicId::MeltingIce => {
                run.relic_counters.insert(RelicId::MeltingIce, 80);
            }
            RelicId::SilkThread => {
                run.relic_counters.insert(RelicId::SilkThread, 40);
            }
            RelicId::TeaCeremony => {
                run.relic_counters.insert(RelicId::TeaCeremony, 3);
            }
            _ => {}
        }
        run.recompute_capacities();
        stats.relics_bought += 1;
        stats.gold_spent += price;
    }
}

/// Decide whether to skip the upcoming non-Boss blind. We skip when the bot can
/// reasonably expect to clear the blind anyway *and* it can't, so the gold reward
/// from skipping is more valuable than the gold reward from clearing. Specifically:
/// the bot's projected total score (best play × plays_remaining) must comfortably
/// exceed the blind's target so we'd be wasting plays clearing a trivially-easy
/// blind. Boss blinds can never be skipped.
fn should_skip_blind(run: &RunState, blind: BlindKind) -> bool {
    if matches!(blind, BlindKind::Boss) {
        return false;
    }
    let target = (run.base_target as f32 * blind.target_multiplier()) as u32;
    let best = pick_best_play(run).map(|(s, _)| s as u32).unwrap_or(0);
    if best == 0 {
        return false;
    }
    // Optimistic projection: assume the bot can repeat its current best play for
    // every remaining play (ignores discard refresh).
    let projected = best.saturating_mul(run.plays_remaining);
    // Only skip if we'd over-shoot by a wide margin (≥ 2× target). The threshold
    // prevents skipping borderline blinds where the gold-from-clearing reward is
    // comparable to the skip reward.
    projected >= target.saturating_mul(2)
}

/// Tuning overrides applied on top of `GameMode::standard()` for headless runs.
/// Any field left at its default uses the standard mode value.
#[derive(Clone, Debug, Default)]
pub struct BotConfig {
    pub base_target: Option<u32>,
    pub target_scaling: Option<f32>,
    pub starting_plays: Option<u32>,
    pub starting_discards: Option<u32>,
    pub starting_gold: Option<u32>,
}

impl BotConfig {
    fn into_mode(self) -> GameMode {
        let mut mode = GameMode::standard();
        if let Some(v) = self.base_target {
            mode.base_target = v;
        }
        if let Some(v) = self.target_scaling {
            mode.target_scaling = v;
        }
        if let Some(v) = self.starting_plays {
            mode.starting_plays = v;
        }
        if let Some(v) = self.starting_discards {
            mode.starting_discards = v;
        }
        if let Some(v) = self.starting_gold {
            mode.starting_gold = v;
        }
        mode
    }
}

/// Play one full bot run from a fresh `RunState` until the bot busts or wins.
/// Mirrors the actual game flow: 8 antes × (Small → Big → Boss), with `advance_round`
/// (which scales `base_target` by `target_scaling` and rolls to the next blind) called
/// after every cleared blind. Run ends on bust or when ante > FINAL_ANTE.
pub fn play_run_with(config: BotConfig) -> RunStats {
    let mode = config.into_mode();
    let mut run = RunState::new(mode);
    let mut stats = RunStats::default();
    let mut bus = EventBus::default();

    loop {
        if run.is_run_complete() {
            stats.victory = true;
            stats.died_on_ante = run.ante;
            break;
        }
        let blind = run.upcoming_blind;

        // Skip strategy: bank gold on Small/Big when projected score comfortably
        // overshoots the target. Tag rewards replace flat gold — apply them
        // the same way the pick-blind scene does.
        if should_skip_blind(&run, blind) {
            if let Some(tag) = run.tag_for_blind(blind) {
                run.apply_tag(tag);
            }
            run.skip_to_next_blind();
            stats.blinds_skipped += 1;
            continue;
        }

        run.apply_blind(blind);
        let cleared = play_blind(&mut run, &mut stats);
        stats.total_score += run.round_score as u64;
        stats.died_on_ante = run.ante;
        stats.died_on_blind = blind;
        if !cleared {
            stats.final_gold = run.gold;
            break;
        }
        stats.blinds_cleared += 1;
        if matches!(blind, BlindKind::Boss) {
            stats.antes_cleared += 1;
        }

        run.advance_round(&mut bus);

        // Shop visit happens after advance_round (matching Shop → PickBlind scene
        // flow), so we evaluate purchases against the freshly-drawn next hand.
        visit_shop(&mut run, &mut stats);
    }

    stats.final_gold = run.gold;
    stats
}

/// Run the bot `n` times with the given tuning config and return aggregate stats.
pub fn run_with(n: u32, config: BotConfig) -> AggregateStats {
    let mut agg = AggregateStats::default();
    for _ in 0..n {
        let s = play_run_with(config.clone());
        agg.record(&s);
    }
    agg
}

/// Run the bot `n` times and print aggregate stats. Entry point from `main.rs`.
pub fn run_headless(n: u32, config: BotConfig) {
    let mode = config.clone().into_mode();
    println!(
        "Running bot for {} runs (base_target={}, target_scaling={}, plays={}, discards={}, gold={})...",
        n,
        mode.base_target,
        mode.target_scaling,
        mode.starting_plays,
        mode.starting_discards,
        mode.starting_gold,
    );
    let mut agg = AggregateStats::default();
    for i in 0..n {
        let s = play_run_with(config.clone());
        agg.record(&s);
        if (i + 1) % 25 == 0 || i + 1 == n {
            let outcome = if s.victory {
                format!("VICTORY (ante {})", s.died_on_ante)
            } else {
                format!("died ante {} on {}", s.died_on_ante, s.died_on_blind.name())
            };
            println!(
                "  [{:>4}/{}] last: {} (cleared {} blinds, score {})",
                i + 1,
                n,
                outcome,
                s.blinds_cleared,
                s.total_score,
            );
        }
    }
    agg.print_summary();
}

/// Sweep `target_scaling` × `base_target` and print a compact win-rate matrix.
/// Useful for finding tuning sweet spots quickly. Each cell runs `runs_per_cell`
/// full bot games and reports `(antes_cleared_avg, win_rate_pct)`.
pub fn run_sweep(runs_per_cell: u32, base_targets: &[u32], scalings: &[f32], plays_values: &[u32]) {
    println!(
        "Sweep: {} bases × {} scalings × {} plays-values × {} runs/cell = {} runs total",
        base_targets.len(),
        scalings.len(),
        plays_values.len(),
        runs_per_cell,
        base_targets.len() * scalings.len() * plays_values.len() * runs_per_cell as usize,
    );
    println!();
    println!(
        "Each cell shows: antes_cleared_avg / win_rate_pct (avg blinds_cleared, avg total_score)"
    );

    for &plays in plays_values {
        println!("\n── starting_plays = {} ──", plays);
        // Header row: target scalings.
        print!("{:>10} |", "base \\ sc");
        for s in scalings {
            print!(" {:^22} |", format!("{:.2}", s));
        }
        println!();
        print!("{:->10}-+", "");
        for _ in scalings {
            print!("{:->24}+", "");
        }
        println!();

        for &base in base_targets {
            print!("{:>10} |", base);
            for &scaling in scalings {
                let cfg = BotConfig {
                    base_target: Some(base),
                    target_scaling: Some(scaling),
                    starting_plays: Some(plays),
                    ..Default::default()
                };
                let agg = run_with(runs_per_cell, cfg);
                let avg_antes = agg.antes_cleared_total as f64 / agg.runs as f64;
                let win_pct = agg.victories as f64 * 100.0 / agg.runs as f64;
                let avg_blinds = agg.blinds_cleared_total as f64 / agg.runs as f64;
                let avg_score = agg.total_score as f64 / agg.runs as f64;
                print!(
                    " {:>4.1}/{:>4.1}% ({:>3.1}b {:>5.0}) |",
                    avg_antes, win_pct, avg_blinds, avg_score
                );
            }
            println!();
        }
    }
    println!();
}
