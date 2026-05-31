//! Per-frame animation state ticking for the gameplay scene.
//!
//! Holds the small chunks of [`super::GameplayScene::update`] that advance
//! pure presentation state — particle systems, flying coins, score popups,
//! candle flicker, light-ramp, candle flare decay, hand-tile slide tweens.
//! Behaviour is identical to the inline code; this is purely organisational.

use std::time::Instant;

use super::GameplayScene;
use super::{CANDLE_FLARE_DECAY, LIGHT_RAMP_DELAY_SECS, LIGHT_RAMP_DURATION_SECS};
use crate::game::engine::GameEngine;
use crate::scenes::UpdateCtx;

/// Tick particles, flying coins, score popups, score reel target, and
/// per-slot hand tile tweens. Returns the freshly read interaction snapshot
/// (`hand_len`) so the caller can reuse it without re-reading.
pub(super) fn tick_basic_animations(
    scene: &mut GameplayScene,
    ctx: &mut UpdateCtx<'_>,
    now: Instant,
    dt: f32,
) {
    if ctx.headless {
        scene.light_ramp = 1.0;
        scene.light_ramp_anchor = None;
    }
    scene.particles.update(dt, None);
    scene.flying_coins.update(dt);
    scene.score_popups.update(now);
    let gameplay = GameEngine::read(ctx.run);
    let interaction = GameEngine::read_interaction(ctx.run);
    // When the target changes (scene init or a new round), shrink the
    // reel back down to exactly `digits(target)` zero columns. Columns
    // that grew mid-round because the player overshot the target are
    // dropped here.
    let cur_target = gameplay.target_score as u64;
    if scene.score_reel_target != cur_target {
        scene.score_reel.reset_for_target(cur_target);
        scene.score_reel_target = cur_target;
    }
    // When idle (no cascade), snap the reel to the real score so scene
    // init and round transitions display the correct value immediately.
    if scene.cascade_queue.is_empty() {
        scene.score_reel.snap(gameplay.round_score);
    }

    // Hand tile animation state: resize vecs to current hand length,
    // detect newly dealt tiles (uid mismatch), and advance slide_y tween.
    {
        let hand_len = interaction.hand_len;
        scene.hand_tile_uids.resize(hand_len, u32::MAX);
        scene.hand_slide_y.resize(hand_len, 1.0);
        scene.hand_slide_x.resize(hand_len, 0.0);
        for i in 0..hand_len {
            let uid = interaction.hand_ids[i];
            if scene.hand_tile_uids[i] != uid {
                scene.hand_tile_uids[i] = uid;
                if !ctx.headless {
                    scene.hand_slide_y[i] = 0.0; // trigger pop-in
                }
            }
            if ctx.headless {
                // Headless ticks run back-to-back with ~0 dt; skip deal pop-in so
                // screenshot captures show a full rack instead of 5% scale tiles.
                scene.hand_slide_y[i] = 1.0;
                scene.hand_slide_x[i] = 0.0;
            } else {
                // Animate slide_y toward 1.0 (settled position).
                let speed = 6.0_f32; // slots per second
                scene.hand_slide_y[i] = (scene.hand_slide_y[i] + dt * speed).min(1.0);
                // Decay slide_x toward 0 (sort shuffle settles).
                scene.hand_slide_x[i] *= (1.0_f32 - dt * 12.0).max(0.0);
            }
        }
    }
}

/// Detect deal events and stamp `light_ramp_anchor`.
pub(super) fn tick_deal_detection(
    scene: &mut GameplayScene,
    ctx: &mut UpdateCtx<'_>,
    now: Instant,
) {
    // Detect deal events: any time the hand grows (initial round deal,
    // post-discard refill) we may stamp `light_ramp_anchor`. While
    // `pending_chamber` is set, we intentionally ignore the stale
    // previous-round hand — seed `prev_hand_len` to it so the detector
    // fires only once apply_chamber clears and redeals (marking the true
    // round-start deal).
    let interaction = GameEngine::read_interaction(ctx.run);
    let cur_hand_len = interaction.hand_len;
    if scene.pending_chamber.is_some() {
        scene.prev_hand_len = cur_hand_len;
    } else {
        if cur_hand_len > scene.prev_hand_len
            && scene.light_ramp < 1.0 && scene.light_ramp_anchor.is_none() {
                scene.light_ramp_anchor = Some(now);
            }
        scene.prev_hand_len = cur_hand_len;
    }
}

/// Detect gold changes and spawn flying coin animations.
pub(super) fn tick_yen_change_coins(scene: &mut GameplayScene, ctx: &mut UpdateCtx<'_>) {
    let gameplay = GameEngine::read(ctx.run);
    let cur_gold = gameplay.yen;
    let delta = cur_gold - scene.prev_gold;
    if delta != 0 && scene.prev_gold != 0 {
        // Recompute the dish center from layout (mirrors draw_frame).
        let layout = ctx.layout;
        let anchor = match super::glb_anchors::resolve_player_gold_anchor(
            layout.window_w,
            layout.window_h,
            ctx.room_gltf_height_scale,
        ) {
            Ok(a) => a,
            Err(_) => return,
        };
        let (coin_radius, coin_thickness, _) =
            crate::render::yen_display::yen_coin_dims(|n| layout.mm(n));
        let pile_cx = anchor[0];
        let pile_cy = anchor[1];
        let dish_floor_z = anchor[2];
        // Scale coin count with the magnitude of the change so bigger
        // payouts produce a more dramatic shower.
        let count = (delta.unsigned_abs() as usize).clamp(1, 12);
        if delta > 0 {
            scene.flying_coins.gain(
                pile_cx,
                pile_cy,
                dish_floor_z,
                coin_radius,
                coin_thickness,
                count,
            );
        } else {
            scene.flying_coins.lose(
                pile_cx,
                pile_cy,
                dish_floor_z,
                coin_radius,
                coin_thickness,
                count,
            );
        }
    }
    scene.prev_gold = cur_gold;
}

/// Advance candle flicker, decay candle flare, and tick the light ramp.
pub(super) fn tick_candle_and_light_ramp(scene: &mut GameplayScene, now: Instant, dt: f32) {
    scene.candle_time += dt;
    let _ = dt;

    // Decay the candle flare (exponential fall-off so it fades fast
    // at first then lingers subtly). Kill it once negligible.
    if scene.candle_flare > 0.0 {
        scene.candle_flare *= (-dt * CANDLE_FLARE_DECAY).exp();
        if scene.candle_flare < 0.01 {
            scene.candle_flare = 0.0;
        }
    }

    // Light ramp: candles start dark and spark on after the opening deal.
    // Uses `light_ramp_anchor` so the opening-smoke path can clear deal
    // timestamps without freezing brightness mid-ramp.
    if scene.light_ramp < 1.0
        && let Some(t0) = scene.light_ramp_anchor
    {
        let elapsed = now.saturating_duration_since(t0).as_secs_f32();
        if elapsed > LIGHT_RAMP_DELAY_SECS {
            let t = ((elapsed - LIGHT_RAMP_DELAY_SECS) / LIGHT_RAMP_DURATION_SECS).clamp(0.0, 1.0);
            // Ease-in curve so the candles spark slowly at first
            // then bloom to full brightness.
            scene.light_ramp = t * t;
        }
    }
    if scene.light_ramp >= 1.0 {
        scene.light_ramp_anchor = None;
    }
}

/// Dora / round-wind indicator tiles on `tile_plinth` empties and settled gold at `player_gold`.
/// Plinth meshes live in `gameplay.glb`; only dynamic tile faces and coins are spawned here.
pub(super) fn build_ambient_table_objects(
    scene: &GameplayScene,
    layout: &crate::ui::layout::LayoutResult,
    gameplay: &crate::game::engine::GameplayReadModel,
    progress_dora_enabled: bool,
    frame: &mut crate::render::draw_cmd::UiFrame,
    tile_plinth_poses: &[crate::render::gameplay_glb::GameplayMarkerPose; 3],
    gold_pile_pose: crate::render::gameplay_glb::GameplayMarkerPose,
    cam: &crate::render::draw_cmd::CameraParams,
    window_w: f32,
    window_h: f32,
    env_height_scale: f32,
    vis: crate::scenes::DebugVisibility,
) -> anyhow::Result<()> {
    use crate::core::tile::{Suit, Tile};
    use crate::render::draw_cmd::ShowcaseTilePlacement;

    let hand_slot_w = layout
        .hand_slots
        .first()
        .map(|r| r.w)
        .unwrap_or(layout.window_w * crate::ui::layout::HAND_SLOT_W_RATIO);
    let tile_size_px = hand_slot_w * (22.0 / crate::ui::layout::TILE_WIDTH_MM);
    let tile_spacing = hand_slot_w * (24.0 / crate::ui::layout::TILE_WIDTH_MM);
    let tile_on_plinth_lift = |z: f32| z;

    if !vis.hide_plinth_tiles && progress_dora_enabled {
        let plinth = &tile_plinth_poses[0];
        let [plinth_cx, plinth_cy, plinth_lift] = plinth.anchor;
        let tile_lift = tile_on_plinth_lift(plinth_lift);
        let tile_size_px = tile_size_px * plinth.uniform_author_scale(window_h, env_height_scale);
        let indicators: &[Tile] = if scene.pending_chamber.is_some() {
            &[]
        } else {
            gameplay.dora_indicator_tiles.as_slice()
        };
        if !indicators.is_empty() {
            let count = indicators.len().min(2);
            let mut tile_placements: Vec<ShowcaseTilePlacement> = Vec::with_capacity(count);
            for (i, &t) in indicators.iter().take(count).enumerate() {
                let offset = if count == 1 {
                    0.0
                } else {
                    (i as f32 - 0.5) * tile_spacing
                };
                let center_pos = crate::render::gameplay_glb::showcase_anchor_spread_px(
                    window_w,
                    window_h,
                    cam,
                    [plinth_cx, plinth_cy, tile_lift],
                    plinth.rotation_rad,
                    offset,
                );
                tile_placements.push(ShowcaseTilePlacement {
                    tile: t,
                    center_pos,
                    rotation: plinth.rotation_rad,
                    scale: 1.0,
                    size_px: tile_size_px,
                    brightness: 1.0,
                    selected: false,
                    hovered: false,
                    outline: false,
                    glow: false,
                    glow_color: None,
                    pick_id: None,
                    overlay_rect_group: Some(
                        crate::render::draw_cmd::TileOverlayRectGroup::DoraTiles,
                    ),
                });
            }
            frame.showcase_tile_batch(tile_placements);
        }
    }

    if !vis.hide_plinth_tiles {
        let plinth = &tile_plinth_poses[1];
        let [plinth_cx, plinth_cy, plinth_lift] = plinth.anchor;
        let tile_lift = tile_on_plinth_lift(plinth_lift);
        let tile_size_px = tile_size_px * plinth.uniform_author_scale(window_h, env_height_scale);
        let mut winds = vec![Tile::new(Suit::Wind, gameplay.round_wind_rank, 0)];
        if let Some(bonus) = gameplay.bonus_round_wind_rank {
            winds.push(Tile::new(Suit::Wind, bonus, 1));
        }
        let count = winds.len().min(2);
        let mut tile_placements: Vec<ShowcaseTilePlacement> = Vec::with_capacity(count);
        for (i, t) in winds.into_iter().take(count).enumerate() {
            let offset = if count == 1 {
                0.0
            } else {
                (i as f32 - 0.5) * tile_spacing
            };
            let center_pos = crate::render::gameplay_glb::showcase_anchor_spread_px(
                window_w,
                window_h,
                cam,
                [plinth_cx, plinth_cy, tile_lift],
                plinth.rotation_rad,
                offset,
            );
            tile_placements.push(ShowcaseTilePlacement {
                tile: t,
                center_pos,
                rotation: plinth.rotation_rad,
                scale: 1.0,
                size_px: tile_size_px,
                brightness: 1.0,
                selected: false,
                hovered: false,
                outline: false,
                glow: false,
                glow_color: None,
                pick_id: None,
                overlay_rect_group: Some(
                    crate::render::draw_cmd::TileOverlayRectGroup::RoundWindTiles,
                ),
            });
        }
        frame.showcase_tile_batch(tile_placements);
    }

    if !vis.hide_yen_pile {
        let coins = crate::render::yen_display::build_settled_yen_coin_pile(
            |n| layout.mm(n),
            gameplay.yen,
            gold_pile_pose.anchor,
            crate::render::yen_display::GAMEPLAY_GOLD_PILE_SEED,
            Some((layout.window_w, layout.window_h)),
            gold_pile_pose.uniform_author_scale(window_h, env_height_scale),
        );
        if !coins.is_empty() {
            frame.object3d_batch(coins);
        }
    }
    Ok(())
}
