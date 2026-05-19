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
use crate::render::candle_mesh::WICK_TIP_Y;
use crate::render::draw_cmd::{Object3d, Object3dKind};
use crate::render::flame_volume::{FlameEmitter, flame_emitter_scale, flame_flicker_multiplier};
use crate::render::wgpu_renderer::{PointLight, SpotLight};
use crate::render::world_space::pixel_to_world;
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
        scene.candle_wind_dim = 1.0;
    }
    scene.particles.update(dt);
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
                scene.hand_slide_y[i] = 0.0; // trigger pop-in
            }
            // Animate slide_y toward 1.0 (settled position).
            let speed = 6.0_f32; // slots per second
            scene.hand_slide_y[i] = (scene.hand_slide_y[i] + dt * speed).min(1.0);
            // Decay slide_x toward 0 (sort shuffle settles).
            scene.hand_slide_x[i] *= (1.0_f32 - dt * 12.0).max(0.0);
        }
    }
}

/// Cache wind delay/duration from the cascade tuning, then detect deal
/// events and stamp `last_deal_at` / `light_ramp_anchor`.
pub(super) fn tick_wind_and_deal_detection(
    scene: &mut GameplayScene,
    ctx: &mut UpdateCtx<'_>,
    now: Instant,
) {
    // Cache the latest wind timing from the cascade tuning so live
    // tweaks in the debug overlay take effect on the next frame and so
    // `draw()` (no `cascade_tuning` access) can read these.
    scene.wind_delay_secs = ctx.cascade_tuning.wind_delay_ms as f32 / 1000.0;
    scene.wind_duration_secs = ctx.cascade_tuning.wind_duration_ms as f32 / 1000.0;
    // Detect deal events: any time the hand grows (initial round deal,
    // post-discard refill) we stamp `last_deal_at` so the post-deal wind
    // gust can fire `wind_delay_secs` later. While `pending_blind` is
    // set, we intentionally ignore the stale previous-round hand — seed
    // `prev_hand_len` to it so the detector fires only once apply_blind
    // clears and redeals (marking the true round-start deal).
    let interaction = GameEngine::read_interaction(ctx.run);
    let cur_hand_len = interaction.hand_len;
    if scene.pending_blind.is_some() {
        scene.prev_hand_len = cur_hand_len;
    } else {
        if cur_hand_len > scene.prev_hand_len {
            scene.last_deal_at = Some(now);
            if scene.light_ramp < 1.0 && scene.light_ramp_anchor.is_none() {
                scene.light_ramp_anchor = Some(now);
            }
        }
        scene.prev_hand_len = cur_hand_len;
    }
}

/// Detect gold changes and spawn flying coin animations.
pub(super) fn tick_gold_change_coins(scene: &mut GameplayScene, ctx: &mut UpdateCtx<'_>) {
    let gameplay = GameEngine::read(ctx.run);
    let cur_gold = gameplay.gold;
    let delta = cur_gold - scene.prev_gold;
    if delta != 0 && scene.prev_gold != 0 {
        // Recompute the dish center from layout (mirrors draw_frame).
        let layout = ctx.layout;
        let anchor = crate::render::gold_display::gameplay_gold_pile_anchor(
            layout,
            &scene.positions.coin_pile,
        );
        let (coin_radius, coin_thickness, _) =
            crate::render::gold_display::gold_coin_dims(|n| layout.mm(n));
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
    // Post-deal wind snuffs flames/lights via brightness (shape stays upright indoors).
    scene.candle_wind_dim = scene
        .last_deal_at
        .map(|d| {
            let elapsed = now.saturating_duration_since(d).as_secs_f32();
            let delay = scene.wind_delay_secs;
            let dur = scene.wind_duration_secs.max(0.001);
            if elapsed < delay || elapsed >= delay + dur {
                1.0
            } else {
                let t = (elapsed - delay) / dur;
                let env = (4.0 * t * (1.0 - t)).clamp(0.0, 1.0);
                1.0 - 0.88 * env
            }
        })
        .unwrap_or(1.0);
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
    // Uses `light_ramp_anchor`, not `last_deal_at`, so the opening-smoke
    // path can clear the latter without freezing brightness mid-ramp.
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

/// Buffers populated by [`build_candles_and_spotlights`].
pub(super) struct CandleAndLightBuffers {
    pub(super) flame_emitters: Vec<FlameEmitter>,
    pub(super) point_lights: Vec<PointLight>,
    pub(super) spot_lights: Vec<SpotLight>,
    pub(super) candle_placements: Vec<Object3d>,
}

/// Build candle 3D placements + flame additive quads + point lights, plus
/// hint / plinth spotlight & point-light highlights. Identical to
/// the inline draw_frame chunks; relocated for organisation.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_candles_and_spotlights(
    scene: &GameplayScene,
    layout: &crate::ui::layout::LayoutResult,
    _run: &crate::game::run::RunState,
    gameplay: &crate::game::engine::GameplayReadModel,
    hand_slots: &[(f32, f32, f32, f32)],
    hint_indices: &[usize],
    _discard_bowl_placement: Option<&Object3d>,
    _bronze_mirror_placement: Option<&Object3d>,
    debug_visibility_hide_candles: bool,
    progress_dora_enabled: bool,
) -> CandleAndLightBuffers {
    // ── Candles ─────────────────────────────────────────────────────
    // Score L/R, hand strip L/R (upper row), hand strip L/R (lower row),
    // footlight. Each: 3D wax + wick, additive `Flame` quad, `PointLight`.
    let mut flame_emitters: Vec<FlameEmitter> = Vec::new();
    let mut point_lights: Vec<PointLight> = Vec::new();
    let mut spot_lights: Vec<SpotLight> = Vec::new();
    let mut candle_placements: Vec<Object3d> = Vec::new();
    // `scale_c` is still used by per-candle jitter offsets below; the
    // jitters are positional fudges, not physical mesh sizes, so they
    // stay in pixel space.
    let scale_c = (layout.window_w / 600.0).max(0.5);
    // The wax mesh is now ~0.555 tall and ~0.36 wide in local units
    // (votive proportions), so `candle_scale` (= the uniform mesh
    // scale) is the height of the votive in world units. Sized in
    // real-world millimeters via `layout.mm()` so the candle stays
    // at true votive proportions next to the mahjong tiles.
    let candle_h = layout.mm(crate::render::flame_volume::CANDLE_HEIGHT_MM);
    let candle_w = candle_h * 0.72; // votive footprint, used for layout padding
    let radius_px = layout.mm(244.0); // candlelight pool radius
    let win_w = layout.window_w;
    let win_h = layout.window_h;
    let edge_pad = layout.mm(12.0);
    // Position centres for each candle: (cx, cy_base) where cy_base is
    // the bottom of the candle body. Order matches `self.candles[]`.
    let sp = layout.score_panel;
    let strip_y = if let Some(first) = layout.hand_slots.first() {
        first.y + first.h * 0.5
    } else {
        layout.window_h - 100.0 * scale_c
    };
    let strip_left = layout.hand_slots.first().map(|r| r.x).unwrap_or(0.0);
    let strip_right = layout
        .hand_slots
        .last()
        .map(|r| r.x + r.w)
        .unwrap_or(layout.window_w);
    let rack_bottom = layout
        .hand_slots
        .first()
        .map(|s| s.y + s.h)
        .unwrap_or_else(|| layout.hand_strip.y + layout.hand_strip.h);
    // Bottom candles are pushed well outboard from the hand strip
    // *and* shifted backward in Z (smaller pixel-y → farther from
    // the camera), so they sit behind the tile strip in depth as
    // well as beside it horizontally. Without the Z shift, even an
    // outboard candle's tall silhouette can creep into the tile
    // sightline because the camera is in front of and slightly
    // above the table — putting them at strip_y meant the candle's
    // wick projected into the same screen band as the tiles.
    let bottom_pad = edge_pad + candle_w * 1.6;
    let bottom_z_back = candle_h * scene.positions.candle_bottom_z_back_candle_h_frac;
    let back_z_push = candle_w * scene.positions.candle_back_z_push_candle_w_frac;
    let cy_hand_upper = strip_y - bottom_z_back;
    let cy_hand_lower = rack_bottom + candle_w * 0.5 + edge_pad * 0.5;
    let candle_centers: [(f32, f32); 7] = [
        (
            (sp.x - candle_w - edge_pad).max(candle_w * 0.5 + 4.0),
            sp.y + sp.h * 0.5 - back_z_push,
        ),
        (
            (sp.x + sp.w + candle_w + edge_pad).min(layout.window_w - candle_w * 0.5 - 4.0),
            sp.y + sp.h * 0.5 - back_z_push,
        ),
        (
            (strip_left - candle_w * 0.5 - bottom_pad).max(candle_w * 0.5 + 4.0),
            cy_hand_upper,
        ),
        (
            (strip_right + candle_w * 0.5 + bottom_pad).min(layout.window_w - candle_w * 0.5 - 4.0),
            cy_hand_upper,
        ),
        (
            (strip_left - candle_w * 0.5 - bottom_pad).max(candle_w * 0.5 + 4.0),
            cy_hand_lower,
        ),
        (
            (strip_right + candle_w * 0.5 + bottom_pad).min(layout.window_w - candle_w * 0.5 - 4.0),
            cy_hand_lower,
        ),
        (layout.window_w * 0.5, layout.window_h * 1.55),
    ];

    // The candles stand vertically on the (now horizontal) wood table.
    // We pass them to the renderer in pixel-layout coordinates: the
    // first two components are the layout x/y of the candle's base on
    // the table (renderer maps pixel y → table z = front/back), and
    // the third component is the height above the wood (always 0 so
    // the wax base sits flush on the table).
    //
    // The candle mesh's local Y axis is "up", so once the renderer
    // translates to (table_x, 0, table_z), scaling by `candle_scale`
    // gives a candle of approximately `candle_scale` pixels in
    // visible height. The wick tip lives at WICK_TIP_Y * scale above
    // the base in true world units.
    // Cheap deterministic hash → 4 pseudorandom values in [-1, 1].
    // Used to jitter candle position, scale, and height per index so
    // votives don't snap to a perfectly symmetric grid.
    fn candle_jitter(seed: u32) -> (f32, f32, f32, f32) {
        let s = seed.wrapping_add(1);
        let h = |k: u32| -> f32 {
            let mut x = s.wrapping_mul(k) ^ 0x9E3779B9;
            x ^= x >> 16;
            x = x.wrapping_mul(0x7feb352d);
            x ^= x >> 15;
            x = x.wrapping_mul(0x846ca68b);
            x ^= x >> 16;
            ((x as f32) / (u32::MAX as f32)) * 2.0 - 1.0
        };
        (h(0x68E31DA4), h(0xB5297A4D), h(0x1B56C4E9), h(0xA3D70F8C))
    }

    let candle_scale_base = candle_h;
    // Debug visibility: when the candle row is hidden, skip the entire
    // push loop. We can't filter `CandleBatch` / `Flame` post-hoc because
    // each candle also pushes a `PointLight` and the table shader would
    // keep getting lit by invisible flames.
    let hide_candles = debug_visibility_hide_candles;
    if !hide_candles {
        for (i, &(cx, cy_anchor)) in candle_centers.iter().enumerate() {
            let candle = scene.candles[i];

            // Per-candle jitter. Bottom (front) candles get directionally
            // constrained offsets so they never drift forward into the
            // tile sightline; top candles can wander freely.
            let (jx, jy, js, jh) = candle_jitter(i as u32);
            let (jitter_x_pix, jitter_y_pix) = match i {
                0 | 1 => (jx * 22.0 * scale_c, jy * 16.0 * scale_c),
                2 | 4 => (-jx.abs() * 26.0 * scale_c, -jy.abs() * 22.0 * scale_c),
                3 | 5 => (jx.abs() * 26.0 * scale_c, -jy.abs() * 22.0 * scale_c),
                _ => (jx * 14.0 * scale_c, jy.abs() * 8.0 * scale_c),
            };
            let cx_j = cx + jitter_x_pix;
            let cy_j = cy_anchor + jitter_y_pix;
            // ±12% scale variation so the table votives read as a real
            // set rather than identical instances.
            let candle_scale = candle_scale_base * (1.0 + js * 0.12);
            // ±15% height variation so they aren't all the same tallness.
            let height_scale = 1.0 + jh * 0.15;

            candle_placements.push(Object3d {
                pos: [cx_j, cy_j, 0.0],
                extents: [1.0, 1.0, 1.0],
                rotation: [0.0, 0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
                kind: Object3dKind::Candle {
                    scale: candle_scale,
                    height_scale,
                },
                hover_target: 0.0,
                anim_id: 0,
                arrange_name: None,
            });
            let phase01 = (candle.phase / std::f32::consts::TAU).fract().abs();
            let flare_mul = 1.0 + scene.candle_flare;
            let base_brightness = scene.light_ramp * flare_mul * scene.candle_wind_dim;
            let wick_lift = WICK_TIP_Y * candle_scale * height_scale;
            let wick_world = pixel_to_world(win_w, win_h, cx_j, cy_j, wick_lift);
            flame_emitters.push(FlameEmitter {
                wick_world,
                scale: flame_emitter_scale(candle_scale, height_scale),
                wind: glam::Vec2::ZERO,
                brightness: base_brightness,
                phase: phase01,
                flicker_amp: crate::render::flame_volume::FLAME_FLICKER_AMP,
            });

            // Point light at the wick tip — sits at world_y =
            // WICK_TIP_Y * candle_scale above the table, at the candle's
            // jittered table-plane (cx_j, cy_j) anchor. The renderer
            // maps the pixel-layout x/y onto the table.
            let wick_world_y = wick_lift;
            // The footlight (index 6) sits well behind the camera, so its
            // wick is much farther from the action row than any of the
            // table-edge candles — bump its radius and intensity to
            // compensate, otherwise the front row stays in shadow.
            let (light_radius_mul, light_intensity) = if i == 6 { (2.2, 1.0) } else { (1.0, 2.05) };
            // Flare multiplier: boosts intensity and radius when a
            // monster hand clears the whole blind in one shot.
            let flick = flame_flicker_multiplier(phase01, scene.candle_time);
            point_lights.push(PointLight {
                pos: [cx_j, cy_j, wick_world_y],
                radius: radius_px * light_radius_mul * 1.15 * flare_mul,
                // Slightly desaturated vs pure amber so candle key doesn’t dye every albedo neon-warm.
                color: [1.0, 0.62, 0.34],
                intensity: light_intensity * base_brightness * flick,
            });
            let _ = candle_w;
        }
    }

    // ── Hint spotlights ──────────────────────────────────────────────
    // Each hinted tile gets a directional green SpotLight positioned
    // above and slightly forward of the tile, aimed at the tile face.
    // Because the spotlight shader masks contribution outside the cone
    // (smoothstep between cos_inner and cos_outer), the green pool is
    // sharp and round on the face instead of spilling across the hand.
    if !hint_indices.is_empty() {
        let pulse = 0.75 + 0.25 * (scene.candle_time * 4.0).sin();
        let hint_budget =
            crate::render::wgpu_renderer::MAX_SPOT_LIGHTS.saturating_sub(spot_lights.len());
        let hint_color = [0.30, 1.00, 0.45];
        // 24° inner / 36° outer — a tight pool with a soft edge.
        let cos_inner = (24.0_f32).to_radians().cos();
        let cos_outer = (36.0_f32).to_radians().cos();
        // Hand tiles are rendered at hand_slots[i] + the hand_strip
        // offset (see the showcase placement at the bottom of
        // draw_frame). The spotlight has to match, or it lands on
        // bare felt hundreds of pixels off.
        let strip_dx = scene.positions.hand_strip.nx * layout.window_w;
        let strip_dy = scene.positions.hand_strip.ny * layout.window_h;
        for &idx in hint_indices.iter().take(hint_budget) {
            let Some(&(sx, sy, sw, sh)) = hand_slots.get(idx) else {
                continue;
            };
            // Tile mesh center in pixel space — same formula the
            // showcase placement uses (see HAND_TILE_MESH_Y_FRAC).
            let cx = sx + sw * 0.5 + strip_dx;
            let cy = sy + sh * crate::ui::layout::HAND_TILE_MESH_Y_FRAC + strip_dy;
            // Place the spotlight above and slightly behind the tile
            // in world-Y (smaller pixel-y = +world-Y), aimed down and
            // forward so the beam hits the tile's -Y-facing front.
            // In world-space (Z-up, Y-forward): direction = (0, +Y fwd
            // → but light travels toward player = -world-Y, and down
            // = -Z). Since pos_pixel_y maps with y flip, pushing
            // pixel-y downward from cy means the light's world-Y is
            // larger (farther from player) so the direction toward
            // the tile front is -world-Y plus -Z downward.
            let lift = layout.mm(90.0);
            let behind_px = sh * 0.15;
            spot_lights.push(SpotLight {
                pos: [cx, cy - behind_px, lift],
                // Aim at a point just in front of the tile face at
                // table height: forward = -world-Y, down = -world-Z.
                // Normalised in the GPU upload.
                dir: [0.0, -0.55, -1.0],
                radius: layout.mm(260.0),
                cos_outer,
                cos_inner,
                color: hint_color,
                intensity: 9.0 * pulse,
            });
        }
    }

    // ── Plinth spotlights (dora + round wind) ─────────────────────────
    // Tight 24°/36° cones aimed down and forward
    const PLINTH_SPOT_INTENSITY: f32 = 2.0;
    let cos_inner = (24.0_f32).to_radians().cos();
    let cos_outer = (36.0_f32).to_radians().cos();
    let plinth_spot = |spot_lights: &mut Vec<SpotLight>,
                       cx: f32,
                       cy: f32,
                       tile_lift: f32,
                       tile_size_px: f32,
                       color: [f32; 3],
                       pulse: f32| {
        let budget =
            crate::render::wgpu_renderer::MAX_SPOT_LIGHTS.saturating_sub(spot_lights.len());
        if budget == 0 {
            return;
        }
        spot_lights.push(SpotLight {
            pos: [cx, cy - tile_size_px * 0.15, tile_lift + layout.mm(90.0)],
            dir: [0.0, -0.55, -1.0],
            radius: layout.mm(260.0),
            cos_outer,
            cos_inner,
            color,
            intensity: PLINTH_SPOT_INTENSITY * pulse,
        });
    };

    if progress_dora_enabled && !gameplay.dora_indicator_tiles.is_empty() {
        let pulse = 0.80 + 0.20 * (scene.candle_time * 2.5).sin();
        let dora_color = [1.00, 0.22, 0.18];
        let dora_p = &scene.positions.dora;
        let plinth_h = layout.mm(20.0);
        let plinth_cx = dora_p.nx * layout.window_w;
        let plinth_cy = dora_p.ny * layout.window_h;
        let plinth_lift = layout.mm(dora_p.lift_mm);
        let tile_lift = plinth_lift + plinth_h * (0.5 + 0.36) + layout.mm(15.0);
        let tile_size_px = layout.mm(22.0);
        let spacing = layout.mm(24.0);
        let count = gameplay.dora_indicator_tiles.len().min(2);
        for i in 0..count {
            let offset = if count == 1 {
                0.0
            } else {
                (i as f32 - 0.5) * spacing
            };
            plinth_spot(
                &mut spot_lights,
                plinth_cx + offset,
                plinth_cy,
                tile_lift,
                tile_size_px,
                dora_color,
                pulse,
            );
        }
    }

    {
        let pulse = 0.80 + 0.20 * (scene.candle_time * 2.5).sin();
        let wind_color = [0.28, 0.52, 1.00];
        let rw_p = &scene.positions.round_wind;
        let plinth_h = layout.mm(20.0);
        let plinth_cx = rw_p.nx * layout.window_w;
        let plinth_cy = rw_p.ny * layout.window_h;
        let plinth_lift = layout.mm(rw_p.lift_mm);
        let tile_lift = plinth_lift + plinth_h * (0.5 + 0.36) + layout.mm(15.0);
        let tile_size_px = layout.mm(22.0);
        let spacing = layout.mm(24.0);
        let wind_count = 1 + usize::from(gameplay.bonus_round_wind_rank.is_some());
        for i in 0..wind_count.min(2) {
            let offset = if wind_count == 1 {
                0.0
            } else {
                (i as f32 - 0.5) * spacing
            };
            plinth_spot(
                &mut spot_lights,
                plinth_cx + offset,
                plinth_cy,
                tile_lift,
                tile_size_px,
                wind_color,
                pulse,
            );
        }
    }

    CandleAndLightBuffers {
        flame_emitters,
        point_lights,
        spot_lights,
        candle_placements,
    }
}

/// Build the ambient table objects: dora indicator plinth + indicator
/// tile faces, and the coin pile dish + scattered coins. Behaviour is a
/// verbatim lift of the inline `draw_frame` chunk; relocated for
/// organisation.
pub(super) fn build_ambient_table_objects(
    scene: &GameplayScene,
    layout: &crate::ui::layout::LayoutResult,
    gameplay: &crate::game::engine::GameplayReadModel,
    progress_dora_enabled: bool,
    frame: &mut crate::render::draw_cmd::UiFrame,
) {
    use crate::render::draw_cmd::{Object3d, Object3dKind};
    // Phase 7: ambient table objects — physical coin pile (gold) and the
    // dora indicator stand. None of these are clickable; they're pure
    // atmosphere. Wall tiles remaining are shown in the lower-right HUD.
    //
    // Dora indicator plinth — ornate brass pedestal at center-stage
    // (back of the table by default; user-arrangeable). Holds 1–2
    // dora indicator tile faces. Only drawn once dora is unlocked
    // (level 4+); the focusable rect powers the focus ring + tooltip
    // below.
    if progress_dora_enabled {
        let dora_p = &scene.positions.dora;
        let plinth_w = layout.mm(48.0);
        let plinth_h = layout.mm(20.0);
        let plinth_d = layout.mm(34.0);
        let plinth_cx = dora_p.nx * layout.window_w;
        let plinth_cy = dora_p.ny * layout.window_h;
        let plinth_lift = layout.mm(dora_p.lift_mm);
        frame.object3d(Object3d {
            pos: [plinth_cx, plinth_cy, plinth_lift],
            extents: [plinth_w, plinth_h, plinth_d],
            rotation: crate::render::table_transform::mat4_to_euler_xyz_rad(
                glam::Mat4::from_rotation_y(dora_p.ry_deg.to_radians())
                    * glam::Mat4::from_rotation_x(dora_p.rx_deg.to_radians())
                    * glam::Mat4::from_rotation_z(dora_p.rz_deg.to_radians()),
            ),
            color: [1.0, 1.0, 1.0, 1.0],
            kind: crate::render::draw_cmd::Object3dKind::DoraPlinth { glow: 0.0 },
            hover_target: 0.0,
            anim_id: 0,
            // Must match `GAMEPLAY_HIERARCHY` / `gameplay.json` key `gameplay.dora`
            // so arrange pick, committed rotations, and Tab navigation agree.
            arrange_name: Some("gameplay.dora"),
        });

        // Indicator tile face(s) sitting on the platform. The mesh
        // platform's top is at local-y = +0.36 (lip rim), so place
        // tiles slightly above that in world units. With Dora Crown,
        // a second tile sits beside the first; otherwise just one.
        //
        // While `pending_blind` is set, the round hasn't actually
        // started yet (smoke curtain is still clearing and
        // `apply_blind` hasn't run). Keep the plinth bare in that
        // window so the indicator appears to arrive with the deal.
        let indicators: &[crate::core::tile::Tile] = if scene.pending_blind.is_some() {
            &[]
        } else {
            gameplay.dora_indicator_tiles.as_slice()
        };
        if !indicators.is_empty() {
            use crate::render::draw_cmd::ShowcaseTilePlacement;
            let tile_size_px = layout.mm(22.0);
            // Platform-top lift in world units. Mesh local-y goes up
            // to +0.36 (lip top); the plinth's full Y extent is
            // `plinth_h`, so the platform sits at +0.36 / 0.5 *
            // (plinth_h * 0.5) above the plinth center. Plinth
            // center is at lift + plinth_h*0.5 (because mesh is
            // centered and we lifted by half-height in the renderer).
            let platform_top = plinth_lift + plinth_h * (0.5 + 0.36);
            // Stand the tile half its long dimension above the
            // platform so it appears upright resting in the lip.
            let tile_lift = platform_top + layout.mm(15.0);
            let count = indicators.len().min(2);
            let spacing = layout.mm(24.0);
            let mut tile_placements: Vec<ShowcaseTilePlacement> = Vec::with_capacity(count);
            for (i, &t) in indicators.iter().take(count).enumerate() {
                let offset = if count == 1 {
                    0.0
                } else {
                    (i as f32 - 0.5) * spacing
                };
                tile_placements.push(ShowcaseTilePlacement {
                    tile: t,
                    center_pos: [plinth_cx + offset, plinth_cy, tile_lift],
                    // Stand upright with face toward the camera: Rx(+π/2)
                    // sends the +Z face normal to -Y, and Rz(π) keeps the
                    // tile's top edge up. The small negative lean tilts
                    // the face slightly forward for the high camera.
                    rotation: [
                        std::f32::consts::FRAC_PI_2 - 15.0_f32.to_radians(),
                        0.0,
                        std::f32::consts::PI,
                    ],
                    scale: 1.0,
                    size_px: tile_size_px,
                    brightness: 1.0,
                    selected: false,
                    hovered: false,
                    outline: false,
                    glow: false,
                    glow_color: None,
                    pick_id: None,
                });
            }
            frame.showcase_tile_batch(tile_placements);
        }
    }

    // Round wind plinth — same brass pedestal mesh as dora, parked beside
    // it by default. Shows the ante's round wind (and a second face when
    // Windreader is active).
    {
        use crate::core::tile::{Suit, Tile};
        use crate::render::draw_cmd::ShowcaseTilePlacement;

        let rw_p = &scene.positions.round_wind;
        let plinth_w = layout.mm(48.0);
        let plinth_h = layout.mm(20.0);
        let plinth_d = layout.mm(34.0);
        let plinth_cx = rw_p.nx * layout.window_w;
        let plinth_cy = rw_p.ny * layout.window_h;
        let plinth_lift = layout.mm(rw_p.lift_mm);
        frame.object3d(Object3d {
            pos: [plinth_cx, plinth_cy, plinth_lift],
            extents: [plinth_w, plinth_h, plinth_d],
            rotation: crate::render::table_transform::mat4_to_euler_xyz_rad(
                glam::Mat4::from_rotation_y(rw_p.ry_deg.to_radians())
                    * glam::Mat4::from_rotation_x(rw_p.rx_deg.to_radians())
                    * glam::Mat4::from_rotation_z(rw_p.rz_deg.to_radians()),
            ),
            color: [1.0, 1.0, 1.0, 1.0],
            kind: Object3dKind::DoraPlinth { glow: 0.0 },
            hover_target: 0.0,
            anim_id: 0,
            arrange_name: Some("gameplay.round_wind"),
        });

        let mut winds = vec![Tile::new(Suit::Wind, gameplay.round_wind_rank, 0)];
        if let Some(bonus) = gameplay.bonus_round_wind_rank {
            winds.push(Tile::new(Suit::Wind, bonus, 1));
        }
        let tile_size_px = layout.mm(22.0);
        let platform_top = plinth_lift + plinth_h * (0.5 + 0.36);
        let tile_lift = platform_top + layout.mm(15.0);
        let count = winds.len().min(2);
        let spacing = layout.mm(24.0);
        let mut tile_placements: Vec<ShowcaseTilePlacement> = Vec::with_capacity(count);
        for (i, t) in winds.into_iter().take(count).enumerate() {
            let offset = if count == 1 {
                0.0
            } else {
                (i as f32 - 0.5) * spacing
            };
            tile_placements.push(ShowcaseTilePlacement {
                tile: t,
                center_pos: [plinth_cx + offset, plinth_cy, tile_lift],
                rotation: [
                    std::f32::consts::FRAC_PI_2 - 15.0_f32.to_radians(),
                    0.0,
                    std::f32::consts::PI,
                ],
                scale: 1.0,
                size_px: tile_size_px,
                brightness: 1.0,
                selected: false,
                hovered: false,
                outline: false,
                glow: false,
                glow_color: None,
                pick_id: None,
            });
        }
        frame.showcase_tile_batch(tile_placements);
    }

    // Gold coin pile — same settled cylinders as the shop (no procedural dish).
    let gold_anchor =
        crate::render::gold_display::gameplay_gold_pile_anchor(layout, &scene.positions.coin_pile);
    let coins = crate::render::gold_display::build_settled_gold_coin_pile(
        |n| layout.mm(n),
        gameplay.gold,
        gold_anchor,
        "gameplay.score_panel.coin_pile",
        crate::render::gold_display::GAMEPLAY_GOLD_PILE_SEED,
    );
    if !coins.is_empty() {
        frame.object3d_batch(coins);
    }
}
