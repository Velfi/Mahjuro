//! Shop view (`THEME.md` storeroom): `Shop.glb` room, screen-space hit slots, and shelf props.
//! Stock and input dispatch live on [`super::ShopScene`].

use std::borrow::Cow;
use std::time::Instant;

use glam::Mat4;

use crate::core::consumable::Consumable;
use crate::core::relic::{
    Rarity, RelicId, all_relic_defs, relic_description_live, relic_sell_price_live,
};
use crate::core::tile_pack::PACK_ASPECT_W_OVER_H;
use crate::game::engine::{GameEngine, ShopReadModel, consumable_sell_price_for_mode};
use crate::game::game_mode::GameMode;
use crate::game::run::RunState;
use crate::render::draw_cmd::{
    camera_facing_euler_xyz_rad, CameraParams, Object3d, Object3dKind, UiFrame,
};
use crate::render::lamp_mesh::{shade_exclusion_radius, BULB_Z as LAMP_BULB_LOCAL_Z, SHADE_RIM_R};
use crate::render::table_transform::mat4_to_euler_xyz_rad;
use crate::render::shop_glb::{
    ShopEnvLightingTune, marker_translation, player_consumable_marker_name,
    player_relic_marker_name, shop_camera_from_glb_if_present, shop_env_world_scale, shop_glb_cpu,
    spawn_relic_marker_name,
};
use crate::render::table_transform::euler_xyz_rad_from_deg;
use crate::render::theme::{color, metrics, typography};
use crate::render::wgpu_renderer::GpuInstance;
use crate::render::wgpu_renderer::{
    PointLight, ShopHit, SpotLight, TextAlign, TextLabel, MAX_POINT_LIGHTS, MAX_SPOT_LIGHTS,
};
use crate::render::world_space::{
    object3d_pos_for_screen_at_world_z, surface_anchor_from_world_xyz,
};
use crate::scenes::journal_transition;
use crate::scenes::options::OptionsScene;
use crate::scenes::{
    BackgroundId, ButtonDef, DrawCtx, SceneBehavior, SceneTransition, UpdateCtx, YakuJournalScene,
};
use crate::ui::focus_nav::{clamp_rect_to_viewport, push_focus_ring};
use crate::ui::input::InputMode;
use crate::ui::inspect_plaque::push_focus_tooltip_panel_2d;

use super::layout::{
    ShopInventoryCounts, ShopLayout, consumable_color, is_tile_pack_pick, live_shop_hit,
    rarity_color, relic_half_extents, tile_pack_index_from_pick,
};
use super::{
    ConsumableShopItem, BUG_COUNT, BUG_PARAMS, SHOP_3D_HIT_ID, ShopFocus, ShopMode, ShopScene,
    push_free_badge,
};

/// Matches [`super::RELIC_GLOW_LIFETIME`] — keep glow envelope in sync.
const RELIC_GLOW_SECS: f32 = 0.9;

/// Screen-space hit ids (processed in `update` before the main shop pick pass).
const SHOP_SHELF_CLICK_BASE: u32 = 0xD000;
/// One UI/hit slot per `shop_spawn_relic_00` … `shop_spawn_relic_08` in Shop.glb.
const SHOP_SPAWN_SLOT_COUNT: usize = 9;
const SHOP_INV_CLICK_BASE: u32 = 0xD00A;
const SHOP_CLICK_JOURNAL: u32 = 0xD011;
const SHOP_CLICK_REROLL: u32 = 0xD012;
const SHOP_CLICK_LEAVE: u32 = 0xD013;
const SHOP_CLICK_SELL: u32 = 0xD014;

fn default_fill_point_lights(w: f32, h: f32) -> Vec<PointLight> {
    vec![
        PointLight {
            pos: [w * 0.52, h * 0.34, h * 0.26],
            radius: h * 2.5,
            color: [1.0, 0.93, 0.78],
            intensity: 3.6,
        },
        PointLight {
            pos: [w * 0.52, h * 0.46, h * 0.21],
            radius: h * 2.3,
            color: [0.98, 0.90, 0.74],
            intensity: 3.0,
        },
        PointLight {
            pos: [w * 0.52, h * 0.78, h * 0.13],
            radius: h * 2.0,
            color: [1.0, 0.92, 0.76],
            intensity: 2.7,
        },
    ]
}

/// True when `Shop.glb` carries `KHR_lights_punctual` lights — then we use **only** those (no lamp / default fills).
fn shop_glb_has_embedded_lights() -> bool {
    shop_glb_cpu().is_some_and(|cpu| {
        !cpu.embedded_point_lights.is_empty() || !cpu.embedded_spot_lights.is_empty()
    })
}

/// World-space value stored as `PointLight.radius` / spot radius (shared GPU buffer).
///
/// `shop_glb.wgsl` uses this as Khronos **range** (smooth cutoff); omitting glTF `range` could mean
/// infinite inverse-square (`0` there). **`lit_mesh` relic props use the same buffer** with a
/// smooth linear-distance falloff and `max(radius, 1)` — uploading `0` becomes **one world unit** at
/// shop scale, so candle pools vanish. When `range` is missing, use a wide cap so both paths behave.
#[inline]
fn glb_punctual_radius_upload_world(h: f32, _env_h: f32, scale: f32, range_doc: Option<f32>) -> f32 {
    match range_doc {
        Some(r) => (r * scale).max(1e-3),
        None => (h * 24.0).max(scale * 40.0),
    }
}

/// Point lights from glTF only (`KHR_lights_punctual`). Empty when the GLB has spots but no points.
#[inline]
fn gltf_punctual_linear_rgb(raw: [f32; 3], is_candle: bool, tune: &ShopEnvLightingTune) -> [f32; 3] {
    if is_candle {
        [
            (raw[0] * tune.candle_light_color_mul[0]).clamp(0.0, 1.0),
            (raw[1] * tune.candle_light_color_mul[1]).clamp(0.0, 1.0),
            (raw[2] * tune.candle_light_color_mul[2]).clamp(0.0, 1.0),
        ]
    } else {
        raw
    }
}

fn embedded_point_lights_runtime(
    w: f32,
    h: f32,
    env_h: f32,
    tune: &ShopEnvLightingTune,
) -> Vec<PointLight> {
    let Some(cpu) = shop_glb_cpu() else {
        return Vec::new();
    };
    if cpu.embedded_point_lights.is_empty() {
        return Vec::new();
    }
    let s = shop_env_world_scale(h, env_h);
    let budget = MAX_POINT_LIGHTS.saturating_sub(2);
    if cpu.embedded_point_lights.len() > budget {
        log::warn!(
            "Shop.glb: {} point lights exceed usable budget ({}) — truncating",
            cpu.embedded_point_lights.len(),
            budget
        );
    }
    cpu.embedded_point_lights
        .iter()
        .take(budget)
        .map(|l| {
            let world = l.pos_doc * s;
            let radius = glb_punctual_radius_upload_world(h, env_h, s, l.range_doc);
            PointLight {
                pos: surface_anchor_from_world_xyz(w, h, world),
                radius,
                color: gltf_punctual_linear_rgb(l.color_linear, l.is_candle, tune),
                intensity: (l.intensity * tune.gltf_light_intensity_scale).max(0.0),
            }
        })
        .collect()
}

fn spot_lights_from_glb(w: f32, h: f32, env_h: f32, tune: &ShopEnvLightingTune) -> Vec<SpotLight> {
    if !shop_glb_has_embedded_lights() {
        return Vec::new();
    }
    let Some(cpu) = shop_glb_cpu() else {
        return Vec::new();
    };
    if cpu.embedded_spot_lights.is_empty() {
        return Vec::new();
    }
    let s = shop_env_world_scale(h, env_h);
    if cpu.embedded_spot_lights.len() > MAX_SPOT_LIGHTS {
        log::warn!(
            "Shop.glb: {} spot lights exceed {} — truncating",
            cpu.embedded_spot_lights.len(),
            MAX_SPOT_LIGHTS
        );
    }
    cpu.embedded_spot_lights
        .iter()
        .take(MAX_SPOT_LIGHTS)
        .filter_map(|l| {
            let dir_w = l.dir_doc.normalize_or_zero();
            if dir_w.length_squared() < 1e-12 {
                return None;
            }
            let world = l.pos_doc * s;
            let radius = glb_punctual_radius_upload_world(h, env_h, s, l.range_doc);
            let cos_outer = l.outer_cone_rad.cos();
            let cos_inner = l.inner_cone_rad.cos().max(cos_outer);
            Some(SpotLight {
                pos: surface_anchor_from_world_xyz(w, h, world),
                dir: dir_w.to_array(),
                radius,
                cos_outer,
                cos_inner,
                color: gltf_punctual_linear_rgb(l.color_linear, l.is_candle, tune),
                intensity: (l.intensity * tune.gltf_light_intensity_scale).max(0.0),
            })
        })
        .collect()
}

fn shop_camera_params(_scene: &ShopScene, _w: f32, h: f32, env_h: f32) -> CameraParams {
    if let Some(cam) = shop_camera_from_glb_if_present(h, env_h) {
        return cam;
    }
    // ref_h: 1080 — fallback when Shop.glb has no usable perspective camera
    let cs = h / 1080_f32;
    CameraParams {
        eye: [0.0 * cs, -1517.6 * cs, 1557.2 * cs],
        target: [0.0 * cs, 1614.4 * cs, 548.0 * cs],
        up: [0.0, 0.0, 1.0],
        fovy_deg: 58.0,
    }
}

fn marker_screen_rect(
    win_w: f32,
    win_h: f32,
    cam: &CameraParams,
    node_name: &str,
    rw: f32,
    rh: f32,
    env_height_scale: f32,
) -> Option<[f32; 4]> {
    let cpu = shop_glb_cpu()?;
    let tw = marker_translation(cpu, node_name)? * shop_env_world_scale(win_h, env_height_scale);
    let (cx, cy) = cam.project_world_to_screen(win_w, win_h, tw);
    Some([cx - rw * 0.5, cy - rh * 0.5, rw, rh])
}

impl SceneBehavior for ShopScene {
    fn update(&mut self, mut ctx: UpdateCtx<'_>) -> SceneTransition {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let shop_rm = GameEngine::read_shop(ctx.run);
        self.stash_focus_rects(w, h, ctx.run);

        for &cid in ctx.button_clicks {
            if let Some(hit) = map_shop_ui_click_to_hit(cid, self, &shop_rm) {
                if let Some(next) = self.dispatch_shop_pick_from_hit(hit, &mut ctx) {
                    return Some(next);
                }
            }
        }

        self.update_impl(ctx)
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        self.draw_shop_frame(ctx)
    }

    fn has_blocking_overlay(&self) -> bool {
        self.has_blocking_overlay_impl()
    }

    fn pause_options_overlay(&self) -> Option<&OptionsScene> {
        self.pause_options_overlay_impl()
    }
}

impl ShopScene {
    pub(super) fn stash_focus_rects(&mut self, w: f32, h: f32, run: &RunState) {
        let shop = GameEngine::read_shop(run);
        let env_h = self.drawn_env_height_scale.get();
        let cam = shop_camera_params(self, w, h, env_h);
        *self.last_focus_rects.borrow_mut() = build_focus_rects(self, w, h, &cam, &shop);
    }

    pub(super) fn draw_shop_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let ui_scale = ctx.ui_scale;
        let env_h = ctx.shop_env_height_scale;
        self.drawn_env_height_scale.set(env_h);
        let shop_rm = GameEngine::read_shop(ctx.run);
        let scale = metrics::scene_scale(w, h, ui_scale);

        let mut frame = UiFrame::new();
        if shop_glb_cpu().is_some() {
            frame.quad(GpuInstance {
                rect: [0.0, 0.0, w, h],
                color: [0.04, 0.04, 0.055, 1.0],
            });
            frame.shop_environment();
        } else {
            frame.background(BackgroundId::ShopStoreroom);
        }
        frame.shop_env_gltf_punctual = shop_glb_has_embedded_lights();

        // Mesh anchors use ray/plane hits so props sit under vitrine pixels (plain
        // `pixel_to_world` drifts under perspective).
        let cam = shop_camera_params(self, w, h, env_h);
        frame.camera_override = Some(cam);

        let layout = ShopLayout::build(
            ctx.layout,
            &self.positions,
            ShopInventoryCounts {
                n_for_sale: self.items.len(),
                n_for_sale_zodiacs: self.zodiac_items.len(),
                n_for_sale_talismans: self.talisman_items.len(),
                n_owned_relics: shop_rm.owned_relics.len(),
                n_owned_zodiacs: shop_rm.owned_zodiacs.len(),
                n_owned_talismans: shop_rm.owned_talismans.len(),
            },
        );

        let lp = layout.lamp_center_px;
        let lamp_mesh_h = h * 0.30;
        let lamp_bulb_pos = [lp.0, lp.1, lp.2 + LAMP_BULB_LOCAL_Z * lamp_mesh_h];
        let tf = self.age_secs;
        let flick_fast = (tf * 37.3).sin() * 0.04 + (tf * 61.7).sin() * 0.025;
        let flick_slow = (tf * 4.1).sin() * 0.06;
        let brownout = {
            let d = (tf * 0.73).sin() * (tf * 1.19).sin();
            (d - 0.55).max(0.0) * 0.35
        };
        let lamp_flicker = (1.0 + flick_fast + flick_slow - brownout).clamp(0.55, 1.12);

        let use_glb_lights = shop_glb_has_embedded_lights();
        let shop_gltf_point_lights: Vec<PointLight> = if use_glb_lights {
            embedded_point_lights_runtime(w, h, env_h, &ctx.shop_env_lighting)
        } else {
            Vec::new()
        };
        let mut point_lights: Vec<PointLight> = if use_glb_lights {
            Vec::new()
        } else {
            let mut v = vec![
                PointLight {
                    pos: [lp.0, lp.1, lp.2],
                    radius: h * 1.15,
                    color: [0.86, 0.96, 0.98],
                    intensity: 2.15 * lamp_flicker,
                },
                PointLight {
                    pos: lamp_bulb_pos,
                    radius: h * 1.30,
                    color: [0.82, 0.94, 1.00],
                    intensity: 2.60 * lamp_flicker,
                },
                PointLight {
                    pos: [
                        lamp_bulb_pos[0],
                        lamp_bulb_pos[1],
                        lamp_bulb_pos[2] - h * 0.04,
                    ],
                    radius: h * 0.70,
                    color: [0.72, 0.38, 1.00],
                    intensity: 1.80 * lamp_flicker,
                },
            ];
            v.extend(default_fill_point_lights(w, h));
            v
        };

        let journal_cx = self.positions.book.nx * w;
        let journal_cy = self.positions.book.ny * h;
        let journal_cz = ctx.layout.mm(self.positions.book.lift_mm);

        let hover = self
            .focus
            .and_then(|f| f.to_hit())
            .or(ctx.picked_shop_object)
            .and_then(|hit| {
                live_shop_hit(
                    hit,
                    self,
                    &self.items,
                    &self.zodiac_items,
                    &self.talisman_items,
                    &self.pack_items,
                    &shop_rm,
                )
            });

        if let Some(hit) = hover {
            let n_for_sale_relics = self.items.len().min(layout.niche_count);
            let n_owned_relics = shop_rm.owned_relics.len();
            match hit {
                ShopHit::Relic(i) => {
                    let (px, py, wy) = if let Some((cx, cy)) =
                        screen_xy_for_hit(hit, self, &shop_rm, w, h, &cam)
                    {
                        (cx, cy, light_lift_at_screen_y(cy, h))
                    } else if i < n_for_sale_relics {
                        layout.niche_centers_px[i]
                    } else if let Some(si) = sale_slot_for_focus(self, ShopFocus::Relic(i)) {
                        let r = shop_shelf_slot_rect(w, h, &cam, si, env_h);
                        (
                            r[0] + r[2] * 0.5,
                            r[1] + r[3] * 0.5,
                            shop_shelf_slot_wz(h, si),
                        )
                    } else {
                        let oi = i - self.items.len();
                        if oi < n_owned_relics {
                            layout.owned_relic_pos(oi)
                        } else {
                            (w * 0.5, h * 0.5, h * 0.2)
                        }
                    };
                    point_lights.push(PointLight {
                        pos: [px, py - 30.0, wy + 60.0],
                        radius: h * 0.65,
                        color: [1.00, 0.92, 0.70],
                        intensity: 3.20,
                    });
                }
                ShopHit::Ribbon(_) | ShopHit::Talisman(_) => {
                    if let Some((cx, cy)) = screen_xy_for_hit(hit, self, &shop_rm, w, h, &cam)
                    {
                        let wy = light_lift_at_screen_y(cy, h);
                        point_lights.push(PointLight {
                            pos: [cx, cy + 35.0, wy + 50.0],
                            radius: h * 0.72,
                            color: [1.00, 0.92, 0.74],
                            intensity: 3.00,
                        });
                    }
                }
                ShopHit::Dish(id) => {
                    let center = if id == super::PICK_RELIC_DISH {
                        layout.relic_dish_center_px
                    } else if id == super::PICK_JOURNAL_BOOK {
                        (journal_cx, journal_cy, journal_cz)
                    } else {
                        layout.coin_dish_center_px
                    };
                    point_lights.push(PointLight {
                        pos: [center.0, center.1 - 20.0, center.2.max(80.0)],
                        radius: h * 0.55,
                        color: [1.00, 0.92, 0.70],
                        intensity: 2.50,
                    });
                }
                ShopHit::TilePack(id) => {
                    if let Some((cx, cy)) = screen_xy_for_hit(hit, self, &shop_rm, w, h, &cam)
                    {
                        let wy = light_lift_at_screen_y(cy, h);
                        point_lights.push(PointLight {
                            pos: [cx, cy - 28.0, wy + 55.0],
                            radius: h * 0.62,
                            color: [1.00, 0.92, 0.70],
                            intensity: 3.20,
                        });
                    } else if let Some(idx) = super::layout::tile_pack_index_from_pick(id) {
                        let center = layout
                            .pack_centers_px
                            .get(idx)
                            .copied()
                            .unwrap_or(layout.pack_centers_px[0]);
                        point_lights.push(PointLight {
                            pos: [center.0, center.1 - 30.0, center.2 + 60.0],
                            radius: h * 0.62,
                            color: [1.00, 0.92, 0.70],
                            intensity: 3.20,
                        });
                    }
                }
                ShopHit::EnvSpawnSlot(_)
                | ShopHit::EnvInvSlot(_)
                | ShopHit::EnvConsumableOrd(_) => {}
            }
        }

        frame.point_lights = point_lights;
        frame.shop_gltf_point_lights = shop_gltf_point_lights;
        frame.spot_lights = spot_lights_from_glb(w, h, env_h, &ctx.shop_env_lighting);

        let stock = push_stock_meshes(self, &shop_rm, w, h, &cam);
        if !stock.is_empty() {
            frame.object3d_batch(stock);
        }

        // Moths orbiting the pendant lamp.
        {
            let lamp_w = h * 0.22;
            let lamp_h = lamp_mesh_h;
            let lamp_hang_z = lp.2;
            let t_now = self.age_secs;
            let bulb_wx = lp.0 - w * 0.5;
            let bulb_wy = h * 0.5 - lp.1;
            let bulb_wz = lamp_hang_z + LAMP_BULB_LOCAL_Z * lamp_h;
            let bug_body_len = h * 0.022;
            let flap_hz: f32 = 25.0;
            let flap_amp: f32 = 1.1;

            let sample_bug = |scene: &ShopScene, i: usize, t_back: f32| -> ([f32; 3], [f32; 3], Mat4, f32) {
                let (r_frac, z_frac, speed, size_frac) = BUG_PARAMS[i];
                let fi = i as f32;
                let t = t_now - t_back;
                let phase = scene.bug_phases[i] - speed * t_back;

                let bob_freq = 2.3 + fi * 0.71;
                let drift_freq = 1.1 + fi * 0.43;
                let pitch_freq = 3.7 + fi * 0.57;

                let bob = (t * bob_freq + fi * 1.3).sin() * lamp_h * 0.15;
                let r_nom = lamp_w * r_frac;
                let r_drift = (t * drift_freq + fi * 2.1).sin() * r_nom * 0.20;
                let bug_wz = bulb_wz + lamp_h * z_frac + bob;

                let local_z = (bug_wz - lamp_hang_z) / lamp_h;
                let min_r_local = shade_exclusion_radius(local_z);
                let wing_half_span = 1.13 * size_frac * bug_body_len;
                let min_r_world =
                    min_r_local * (lamp_w / SHADE_RIM_R) + bug_body_len * 0.6 + wing_half_span;
                let orbit_r = (r_nom + r_drift).max(min_r_world);

                let bug_wx = bulb_wx + orbit_r * phase.cos();
                let bug_wy = bulb_wy + orbit_r * phase.sin();
                let bug_px = bug_wx + w * 0.5;
                let bug_py = h * 0.5 - bug_wy;
                let bug_sz = bug_body_len * size_frac;

                let tx = -phase.sin();
                let ty = phase.cos();
                let bank =
                    std::f32::consts::FRAC_PI_4 * 0.5 + (t * 1.9 + fi * 0.8).sin() * 0.30;
                let pitch = (t * pitch_freq + fi * 0.5).sin() * 0.25;
                let yaw = Mat4::from_cols(
                    glam::Vec4::new(tx, ty, 0.0, 0.0),
                    glam::Vec4::new(-ty, tx, 0.0, 0.0),
                    glam::Vec4::new(0.0, 0.0, 1.0, 0.0),
                    glam::Vec4::new(0.0, 0.0, 0.0, 1.0),
                );
                let rot = yaw * Mat4::from_rotation_x(bank) * Mat4::from_rotation_y(pitch);
                let flap = flap_amp * (t * flap_hz * std::f32::consts::TAU + fi * 1.3).sin();
                ([bug_px, bug_py, bug_wz], [bug_sz, bug_sz, bug_sz], rot, flap)
            };

            let mut bugs: Vec<Object3d> = Vec::with_capacity(BUG_COUNT);
            for i in 0..BUG_COUNT {
                let (pos, extents, rot, flap_rad) = sample_bug(self, i, 0.0);
                let fi = i as f32;
                let speed_factor = (t_now * flap_hz * std::f32::consts::TAU + fi * 1.3)
                    .cos()
                    .abs();
                let live_wing_alpha = 1.0 - 0.7 * speed_factor;
                let blur_alpha = 0.6 * speed_factor;
                bugs.push(Object3d {
                    pos,
                    extents,
                    rotation: mat4_to_euler_xyz_rad(rot),
                    color: [1.0, 1.0, 1.0, 1.0],
                    kind: Object3dKind::Bug {
                        slot: i,
                        flap_rad,
                        live_wing_alpha,
                        blur_alpha,
                    },
                    hover_target: 0.0,
                    anim_id: 0,
                    arrange_name: None,
                });
            }
            frame.object3d_batch(bugs);
        }

        let label_px = typography::size(typography::BODY, h, ui_scale).max(18.0);
        let credits_rw = w * 0.16;
        let credits_rh = h * 0.045;
        let mut credits_rect = marker_screen_rect(w, h, &cam, "Dish", credits_rw, credits_rh, env_h)
            .unwrap_or_else(|| rect_center_n(w, h, 0.88, 0.595, credits_rw, credits_rh));
        credits_rect[1] -= h * 0.028;
        frame.texts([TextLabel {
            rect: credits_rect,
            text: format!("Credits {}", shop_rm.display_gold),
            color: color::CHAMPAGNE,
            font_px: Some(label_px),
            align: TextAlign::Center,
            no_glossary: false,
            scroll_offset: 0.0,
        }]);

        if !self.pause_menu.paused && self.pack_celebration.is_none() {
            if let Some(ring_rect) = ring_target_rect(&ctx, self, &shop_rm, w, h, &cam)
                .and_then(|r| clamp_rect_to_viewport(r, w, h))
            {
                let mut quads = Vec::new();
                push_focus_ring(ring_rect, scale, w, h, &mut quads);
                frame.quads(quads);
            }

            let mut free_quads = Vec::new();
            let mut free_texts = Vec::new();
            for (slot_i, sf) in for_sale_slots(self).into_iter().enumerate() {
                if let Some(ShopFocus::Relic(idx)) = sf {
                    if let Some(item) = self.items.get(idx)
                        && !item.sold
                        && item.price == 0
                    {
                        let r = shop_shelf_slot_rect(w, h, &cam, slot_i, env_h);
                        push_free_badge(&mut free_quads, &mut free_texts, r, h, ui_scale);
                    }
                }
            }
            if !free_quads.is_empty() {
                frame.quads(free_quads);
                frame.texts(free_texts);
            }
        }

        if self.pack_celebration.is_none()
            && self.journal_transition.is_none()
            && let Some(hit) = hover
            && let Some((ref title, ref desc, ref cta, col)) =
                hover_tooltip_content(self, &shop_rm, &ctx.run.mode, hit)
            && !title.is_empty()
        {
            let f = ShopFocus::from_hit(hit);
            let tooltip_anchor = self
                .last_focus_rects
                .borrow()
                .iter()
                .find(|(g, _)| *g == f)
                .map(|(_, r)| *r);
            let hover_is_owned = matches!(hit, ShopHit::Relic(i) if i >= self.items.len())
                || matches!(hit, ShopHit::Ribbon(i) if i >= self.zodiac_items.len())
                || matches!(hit, ShopHit::Talisman(i) if i >= self.talisman_items.len());
            let skip_title = matches!(
                hit,
                ShopHit::Dish(id)
                    if id == super::PICK_JOURNAL_BOOK
                        || id == super::PICK_LEAVE_PROP
                        || id == super::PICK_REROLL_PROP
                        || id == super::PICK_SELL_TRAY
            );
            let mut tip_quads = Vec::new();
            let mut tip_texts = Vec::new();
            push_focus_tooltip_panel_2d(
                &mut tip_quads,
                &mut tip_texts,
                w,
                h,
                ui_scale,
                tooltip_anchor,
                title,
                desc,
                cta,
                col,
                hover_is_owned,
                skip_title,
            );
            frame.quads(tip_quads);
            frame.texts(tip_texts);
        }

        // Celebration / particles / floating labels after shelf geometry so they composite on top.
        self.push_tile_pack_celebration(&mut frame, &layout, w, h);

        // Journal book mesh + prepass: enables arrange-mode picking for `shop.props.journal`
        // and live page texture.
        let cam_euler = camera_facing_euler_xyz_rad(cam.eye, cam.target);
        if let Some(t) = self.journal_transition {
            let zp = t.zoom_progress();
            if zp > 0.001 {
                let smoothed = zp * zp * (3.0 - 2.0 * zp);
                let a = smoothed * 0.72;
                frame.quad(GpuInstance {
                    rect: [0.0, 0.0, w, h],
                    color: [0.03, 0.04, 0.06, a],
                });
            }
        }
        let (journal_zoom, journal_pos) = match self.journal_transition {
            Some(t) => {
                let z = t.zoom_progress();
                let smoothed = z * z * (3.0 - 2.0 * z);
                let zoom = 1.0 + smoothed * 7.0;
                let cx = w * 0.5;
                let cy = h * 0.5;
                let pos = [
                    journal_cx + (cx - journal_cx) * smoothed,
                    journal_cy + (cy - journal_cy) * smoothed,
                    journal_cz,
                ];
                (zoom, pos)
            }
            None => (1.0, [journal_cx, journal_cy, journal_cz]),
        };
        let (face_w, face_h) =
            journal_transition::book_cover_face_extents_xy(w, journal_zoom);
        frame.object3d(Object3d {
            pos: journal_pos,
            extents: [
                face_w,
                ctx.layout.mm(journal_transition::BOOK_SPINE_THICKNESS_MM) * journal_zoom,
                face_h,
            ],
            rotation: cam_euler,
            color: [1.0, 1.0, 1.0, 1.0],
            kind: Object3dKind::Book {
                spine_label: Cow::Borrowed("Journal"),
                pick_id: Some(super::PICK_JOURNAL_BOOK),
                open_amount: self.journal_open_amount,
            },
            hover_target: 0.0,
            anim_id: 0,
            arrange_name: Some("shop.props.journal"),
        });

        if self.journal_open_amount > 0.001 {
            let scratch = YakuJournalScene::new();
            let inner_ctx = DrawCtx {
                layout: ctx.layout,
                anim: ctx.anim,
                run: ctx.run,
                progress: ctx.progress,
                active_profile: ctx.active_profile,
                game_in_progress: ctx.game_in_progress,
                proj: ctx.proj,
                picked_gameplay_object: ctx.picked_gameplay_object,
                picked_shop_object: ctx.picked_shop_object,
                debug_visibility: ctx.debug_visibility,
                ui_scale: ctx.ui_scale,
                modal_active: ctx.modal_active,
                arrange_preview: ctx.arrange_preview.clone(),
                shop_env_height_scale: ctx.shop_env_height_scale,
                shop_env_lighting: ctx.shop_env_lighting,
                effect_layers: ctx.effect_layers,
                cursor_pos: ctx.cursor_pos,
                input_mode: ctx.input_mode,
            };
            let prepass = SceneBehavior::draw_frame(&scratch, inner_ctx);
            frame.journal_prepass_frame = Some(Box::new(prepass));
        }

        self.push_shop_particle_quads(&mut frame);
        self.push_shop_score_popup_labels(&mut frame, w, h);

        // Pointer targets in stack order: specific zones first (main loop uses first hit).
        // Pack celebration wins over pause hit targets; pause
        // overlay quads/texts are appended last so they composite above shelf geometry.
        if self.pack_celebration.is_some() {
            frame
                .buttons
                .push(ButtonDef::scene((0.0, 0.0, w, h), SHOP_3D_HIT_ID));
        } else if self.pause_menu.paused {
            let mut pause_quads: Vec<GpuInstance> = Vec::new();
            let mut pause_text: Vec<TextLabel> = Vec::new();
            let mut pause_buttons: Vec<ButtonDef> = Vec::new();
            self.pause_menu.draw(
                crate::ui::layout::ViewportCtx {
                    window_w: w,
                    window_h: h,
                    ui_scale,
                },
                scale,
                &mut pause_quads,
                &mut pause_text,
                &mut pause_buttons,
            );
            frame.quads(pause_quads);
            frame.texts(pause_text);
            pause_buttons.push(ButtonDef::scene((0.0, 0.0, w, h), u32::MAX));
            frame.buttons = pause_buttons;
        } else {
            for i in 0..SHOP_SPAWN_SLOT_COUNT {
                let r = shop_shelf_slot_rect(w, h, &cam, i, env_h);
                frame.buttons.push(ButtonDef::scene(
                    (r[0], r[1], r[2], r[3]),
                    SHOP_SHELF_CLICK_BASE + i as u32,
                ));
            }
            for i in 0..7 {
                let r = inv_slot_rect(w, h, &cam, self, &shop_rm, i, env_h);
                frame.buttons.push(ButtonDef::scene(
                    (r[0], r[1], r[2], r[3]),
                    SHOP_INV_CLICK_BASE + i as u32,
                ));
            }
            let jr = journal_btn_rect(w, h, &cam, env_h);
            // No `hover_label` here — [`push_focus_tooltip_panel_2d`] already shows name /
            // description / action for these hits; main/draw would duplicate it as a second
            // brass tooltip when the cursor hovers the same rect.
            frame.buttons.push(ButtonDef::scene(
                (jr[0], jr[1], jr[2], jr[3]),
                SHOP_CLICK_JOURNAL,
            ));
            let rr = reroll_btn_rect(w, h, &cam, env_h);
            frame
                .buttons
                .push(ButtonDef::scene((rr[0], rr[1], rr[2], rr[3]), SHOP_CLICK_REROLL));
            let lr = leave_btn_rect(w, h, &cam, env_h);
            frame
                .buttons
                .push(ButtonDef::scene((lr[0], lr[1], lr[2], lr[3]), SHOP_CLICK_LEAVE));
            let sr = sell_tray_rect(w, h);
            frame
                .buttons
                .push(ButtonDef::scene((sr[0], sr[1], sr[2], sr[3]), SHOP_CLICK_SELL));
        }

        frame
    }
}

/// Name, description, price line, and accent colour for hover tooltip.
fn hover_tooltip_content(
    scene: &ShopScene,
    shop: &ShopReadModel,
    mode: &GameMode,
    hit: ShopHit,
) -> Option<(String, String, String, [f32; 4])> {
    let n_for_sale_zodiacs = scene.zodiac_items.len();
    let n_for_sale_talismans = scene.talisman_items.len();
    let n_for_sale_relics = scene.items.len();
    let reroll_affordable =
        matches!(scene.mode, ShopMode::Standard) && shop.gold >= scene.reroll_cost as i32;

    let tuple_opt = match hit {
        ShopHit::Relic(i) if i < n_for_sale_relics => {
            let item = &scene.items[i];
            let can_afford = shop.gold >= item.price as i32 && !shop.relics_full && !item.sold;
            let cta = if item.sold {
                "SOLD".to_string()
            } else if !can_afford {
                if shop.relics_full {
                    "Relics full".to_string()
                } else {
                    format!("${} (have {}g)", item.price, shop.display_gold)
                }
            } else {
                item.buy_label()
            };
            let col = if item.sold {
                color::UMBER
            } else if can_afford {
                color::GOLD
            } else {
                color::RUBY
            };
            Some((
                item.name.to_string(),
                item.description.to_string(),
                cta,
                col,
            ))
        }
        ShopHit::Relic(i) => {
            let oi = i.checked_sub(n_for_sale_relics)?;
            let rid = *shop.owned_relics.get(oi)?;
            let defs = all_relic_defs();
            let def = defs.iter().find(|d| d.id == rid);
            let name = def
                .map(|d| d.name.to_string())
                .unwrap_or_else(|| "Relic".into());
            let desc = relic_description_live(rid, &shop.relic_counters, shop.total_score_earned);
            let sell = relic_sell_price_live(rid, &shop.relic_counters);
            Some((name, desc, format!("Sell {}g", sell), color::CHAMPAGNE))
        }
        ShopHit::Ribbon(i) if i < n_for_sale_zodiacs => {
            let item = &scene.zodiac_items[i];
            let price = item.price(mode);
            let can_afford = shop.gold >= price as i32 && !item.sold;
            let cta = if item.sold {
                "SOLD".to_string()
            } else if !can_afford {
                format!("${} (have {}g)", price, shop.display_gold)
            } else if price == 0 {
                "FREE".to_string()
            } else {
                format!("Buy {}g", price)
            };
            let col = if item.sold {
                color::UMBER
            } else if can_afford {
                color::GOLD
            } else {
                color::RUBY
            };
            Some((item.name(), item.description(), cta, col))
        }
        ShopHit::Ribbon(i) => {
            let oi = i.checked_sub(n_for_sale_zodiacs)?;
            let c = shop.owned_zodiacs.get(oi)?.consumable;
            let item = ConsumableShopItem {
                consumable: c,
                sold: false,
            };
            Some((
                item.name(),
                item.description(),
                "Use".to_string(),
                color::CHAMPAGNE,
            ))
        }
        ShopHit::Talisman(i) if i < n_for_sale_talismans => {
            let item = &scene.talisman_items[i];
            let price = item.price(mode);
            let can_afford = shop.gold >= price as i32 && !shop.consumables_full && !item.sold;
            let cta = if item.sold {
                "SOLD".to_string()
            } else if !can_afford {
                if shop.consumables_full {
                    "Inventory full".to_string()
                } else {
                    format!("${} (have {}g)", price, shop.display_gold)
                }
            } else if price == 0 {
                "FREE".to_string()
            } else {
                format!("Buy {}g", price)
            };
            let col = if item.sold {
                color::UMBER
            } else if can_afford {
                color::GOLD
            } else {
                color::RUBY
            };
            Some((item.name(), item.description(), cta, col))
        }
        ShopHit::Talisman(i) => {
            let oi = i.checked_sub(n_for_sale_talismans)?;
            let c = shop.owned_talismans.get(oi)?.consumable;
            let item = ConsumableShopItem {
                consumable: c,
                sold: false,
            };
            Some((
                item.name(),
                item.description(),
                format!("Sell {}g", consumable_sell_price_for_mode(c, mode)),
                color::CHAMPAGNE,
            ))
        }
        ShopHit::Dish(id) if id == super::PICK_COIN_DISH => Some((
            "Gold".to_string(),
            "Your current treasure".to_string(),
            format!("{}g", shop.gold),
            color::GOLD,
        )),
        ShopHit::Dish(id) if id == super::PICK_JOURNAL_BOOK => Some((
            "Yaku Journal".to_string(),
            "Levels, plays, and how to build every yaku".to_string(),
            "Open".to_string(),
            color::CHAMPAGNE,
        )),
        ShopHit::Dish(id) if id == super::PICK_LEAVE_PROP => Some((
            if matches!(scene.mode, ShopMode::Tutorial) {
                "Face Boss"
            } else {
                "Continue On"
            }
            .to_string(),
            "Continue to the next round".to_string(),
            String::new(),
            color::CHAMPAGNE,
        )),
        ShopHit::Dish(id) if id == super::PICK_REROLL_PROP => {
            Some(if matches!(scene.mode, ShopMode::Tutorial) {
                (
                    "Curated Stock".to_string(),
                    "Tutorial stock — restock is unavailable here.".to_string(),
                    String::new(),
                    color::CHAMPAGNE,
                )
            } else if scene.reroll_cost == 0 {
                (
                    "Restock".to_string(),
                    "Refresh the shop once at no gold cost.".to_string(),
                    "FREE".to_string(),
                    color::GOLD,
                )
            } else {
                (
                    "Restock".to_string(),
                    format!("Refresh shop for {}g", scene.reroll_cost),
                    if reroll_affordable {
                        format!("{}g", scene.reroll_cost)
                    } else {
                        format!("${} (have {}g)", scene.reroll_cost, shop.display_gold)
                    },
                    if reroll_affordable {
                        color::GOLD
                    } else {
                        color::RUBY
                    },
                )
            })
        }
        ShopHit::Dish(id) if id == super::PICK_SELL_TRAY => Some((
            "Sell".to_string(),
            "Focus an owned relic or consumable, then click here to sell it".to_string(),
            String::new(),
            color::CHAMPAGNE,
        )),
        ShopHit::Dish(id) if is_tile_pack_pick(id) => {
            let idx = tile_pack_index_from_pick(id).unwrap_or(0);
            scene.pack_items.get(idx).map(|pack| {
                let price = pack.kind.shop_price();
                let can_afford = shop.gold >= price as i32 && !pack.sold;
                let cta = if pack.sold {
                    "SOLD".to_string()
                } else if price == 0 {
                    "FREE".to_string()
                } else if can_afford {
                    format!("Buy {}g", price)
                } else {
                    format!("${} (have {}g)", price, shop.display_gold)
                };
                (
                    pack.kind.name().to_string(),
                    pack.kind.description().to_string(),
                    cta,
                    color::CHAMPAGNE,
                )
            })
        }
        ShopHit::Dish(_) => Some((
            "Relic dish".to_string(),
            "Hover an owned relic to sell it".to_string(),
            String::new(),
            color::UMBER,
        )),
        ShopHit::TilePack(id) => {
            let idx = tile_pack_index_from_pick(id).unwrap_or(0);
            scene.pack_items.get(idx).map(|pack| {
                let price = pack.kind.shop_price();
                let can_afford = shop.gold >= price as i32 && !pack.sold;
                let cta = if pack.sold {
                    "SOLD".to_string()
                } else if price == 0 {
                    "FREE".to_string()
                } else if can_afford {
                    format!("Buy {}g", price)
                } else {
                    format!("${} (have {}g)", price, shop.display_gold)
                };
                let col = if pack.sold {
                    color::UMBER
                } else if can_afford {
                    color::CHAMPAGNE
                } else {
                    color::UMBER
                };
                (
                    pack.kind.name().to_string(),
                    pack.kind.description().to_string(),
                    cta,
                    col,
                )
            })
        }
        ShopHit::EnvSpawnSlot(_) | ShopHit::EnvInvSlot(_) | ShopHit::EnvConsumableOrd(_) => None,
    };

    tuple_opt.filter(|t| !t.0.is_empty())
}

#[inline]
fn pt_in_rect(px: f32, py: f32, r: [f32; 4]) -> bool {
    px >= r[0] && px <= r[0] + r[2] && py >= r[1] && py <= r[1] + r[3]
}

fn cursor_hover_rect(
    cx: f32,
    cy: f32,
    w: f32,
    h: f32,
    cam: &CameraParams,
    scene: &ShopScene,
    shop: &ShopReadModel,
    env_h: f32,
) -> Option<[f32; 4]> {
    for i in 0..SHOP_SPAWN_SLOT_COUNT {
        let r = shop_shelf_slot_rect(w, h, cam, i, env_h);
        if pt_in_rect(cx, cy, r) {
            return Some(r);
        }
    }
    for i in 0..7 {
        let r = inv_slot_rect(w, h, cam, scene, shop, i, env_h);
        if pt_in_rect(cx, cy, r) {
            return Some(r);
        }
    }
    for r in [
        journal_btn_rect(w, h, cam, env_h),
        reroll_btn_rect(w, h, cam, env_h),
        leave_btn_rect(w, h, cam, env_h),
        sell_tray_rect(w, h),
    ] {
        if pt_in_rect(cx, cy, r) {
            return Some(r);
        }
    }
    None
}

fn ring_target_rect(
    ctx: &DrawCtx<'_>,
    scene: &ShopScene,
    shop: &ShopReadModel,
    w: f32,
    h: f32,
    cam: &CameraParams,
) -> Option<[f32; 4]> {
    if ctx.input_mode == InputMode::Cursor {
        let (cx, cy) = ctx.cursor_pos;
        return cursor_hover_rect(cx, cy, w, h, cam, scene, shop, ctx.shop_env_height_scale);
    }
    scene.focus.and_then(|f| {
        build_focus_rects(scene, w, h, cam, shop)
            .into_iter()
            .find(|(t, _)| *t == f)
            .map(|(_, r)| r)
    })
}

fn shop_shelf_slot_wz(h: f32, _index: usize) -> f32 {
    // Single shelf row (`shop_spawn_relic_00` … `_08`); markers define precise Z when GLB loads.
    h * 0.248
}

fn inv_slot_wz(h: f32) -> f32 {
    h * 0.105
}

fn light_lift_at_screen_y(cy: f32, h: f32) -> f32 {
    if cy > h * 0.72 {
        inv_slot_wz(h)
    } else {
        shop_shelf_slot_wz(h, 0)
    }
}

fn screen_xy_for_hit(
    hit: ShopHit,
    scene: &ShopScene,
    shop: &ShopReadModel,
    w: f32,
    h: f32,
    cam: &CameraParams,
) -> Option<(f32, f32)> {
    let focus = ShopFocus::from_hit(hit);
    build_focus_rects(scene, w, h, cam, shop)
        .into_iter()
        .find(|(f, _)| *f == focus)
        .map(|(_, r)| (r[0] + r[2] * 0.5, r[1] + r[3] * 0.5))
}

fn relic_glow(scene: &ShopScene, id: RelicId) -> f32 {
    let Some(start) = scene.relic_glow_starts.get(&id).copied() else {
        return 0.0;
    };
    let age = Instant::now()
        .saturating_duration_since(start)
        .as_secs_f32();
    if age >= RELIC_GLOW_SECS {
        return 0.0;
    }
    let t = (age / RELIC_GLOW_SECS).clamp(0.0, 1.0);
    let attack_end = 0.12_f32;
    if t < attack_end {
        (t / attack_end).clamp(0.0, 1.0)
    } else {
        let decay_t = (t - attack_end) / (1.0 - attack_end);
        (1.0 - decay_t).max(0.0).powi(2)
    }
}

#[inline]
fn lit_anchor(
    cam: &CameraParams,
    w: f32,
    h: f32,
    cx: f32,
    cy: f32,
    center_world_z: f32,
) -> [f32; 3] {
    object3d_pos_for_screen_at_world_z(w, h, cam, cx, cy, center_world_z)
}

/// GLB empty for this inventory bar cell, if one exists (`shop_player_relic_*` or `shop_player_consumable_*`).
fn inv_bar_index_for_consumable_ord(
    scene: &ShopScene,
    shop: &ShopReadModel,
    ord: usize,
) -> Option<usize> {
    let want = player_consumable_marker_name(ord);
    for idx in 0..7 {
        if inv_slot_glb_marker_name(scene, shop, idx).as_deref() == Some(want.as_str()) {
            return Some(idx);
        }
    }
    None
}

/// Map raw GLB collision [`ShopHit`] variants into flat shop hits using shelf/inventory slot tables.
pub(super) fn resolve_shop_glb_env_hit(
    scene: &ShopScene,
    shop: &ShopReadModel,
    hit: ShopHit,
) -> Option<ShopHit> {
    match hit {
        ShopHit::EnvSpawnSlot(slot) => for_sale_slots(scene)
            .get(slot)
            .copied()
            .flatten()?
            .to_hit(),
        ShopHit::EnvInvSlot(slot) => inventory_slots(scene, shop)
            .get(slot)
            .copied()
            .flatten()?
            .to_hit(),
        ShopHit::EnvConsumableOrd(ord) => {
            let idx = inv_bar_index_for_consumable_ord(scene, shop, ord)?;
            inventory_slots(scene, shop)
                .get(idx)
                .copied()
                .flatten()?
                .to_hit()
        }
        _ => unreachable!("resolve_shop_glb_env_hit only accepts Env* variants"),
    }
}

fn inv_slot_glb_marker_name(
    scene: &ShopScene,
    shop: &ShopReadModel,
    index: usize,
) -> Option<String> {
    let inv = inventory_slots(scene, shop);
    let foc = inv.get(index)?.as_ref()?;
    match foc {
        ShopFocus::Relic(_) => (index < 5).then(|| player_relic_marker_name(index)),
        ShopFocus::Ribbon(_) | ShopFocus::Talisman(_) => {
            let ord = inv[..index]
                .iter()
                .filter(|slot| {
                    matches!(
                        slot,
                        Some(ShopFocus::Ribbon(_)) | Some(ShopFocus::Talisman(_))
                    )
                })
                .count();
            (ord < 2).then(|| player_consumable_marker_name(ord))
        }
        _ => None,
    }
}

fn inv_marker_surface_anchor(
    w: f32,
    h: f32,
    cam: &CameraParams,
    scene: &ShopScene,
    shop: &ShopReadModel,
    slot_i: usize,
    env_h: f32,
    cx: f32,
    cy: f32,
    cz_fallback: f32,
) -> [f32; 3] {
    inv_slot_glb_marker_name(scene, shop, slot_i)
        .and_then(|name| shop_glb_cpu().and_then(|cpu| marker_translation(cpu, &name)))
        .map(|tw| surface_anchor_from_world_xyz(w, h, tw * shop_env_world_scale(h, env_h)))
        .unwrap_or_else(|| lit_anchor(cam, w, h, cx, cy, cz_fallback))
}

#[inline]
fn sale_anchor_at_slot(
    w: f32,
    h: f32,
    cam: &CameraParams,
    slot_i: usize,
    env_h: f32,
    cx: f32,
    cy: f32,
    cz_fallback: f32,
) -> [f32; 3] {
    let scale = shop_env_world_scale(h, env_h);
    if let Some(cpu) = shop_glb_cpu() {
        if let Some(tw) = marker_translation(cpu, &spawn_relic_marker_name(slot_i)) {
            let scaled = tw * scale;
            // Slot rect is centered on the niche; GLB empties can sit off-center — keep shelf Z, align XY to rect.
            return object3d_pos_for_screen_at_world_z(w, h, cam, cx, cy, scaled.z);
        }
    }
    lit_anchor(cam, w, h, cx, cy, cz_fallback)
}

fn sale_slot_for_focus(scene: &ShopScene, foc: ShopFocus) -> Option<usize> {
    for_sale_slots(scene)
        .iter()
        .enumerate()
        .find_map(|(idx, cell)| (*cell == Some(foc)).then_some(idx))
}

fn push_stock_meshes(
    scene: &ShopScene,
    shop: &ShopReadModel,
    w: f32,
    h: f32,
    cam: &CameraParams,
) -> Vec<Object3d> {
    let mut out = Vec::new();
    let niche_base = w * 0.048;
    let env_h = scene.drawn_env_height_scale.get();

    let sale = for_sale_slots(scene);
    for (slot_i, foc_opt) in sale.iter().enumerate() {
        let Some(foc) = foc_opt.as_ref() else {
            continue;
        };
        let r = shop_shelf_slot_rect(w, h, cam, slot_i, env_h);
        let cx = r[0] + r[2] * 0.5;
        let cy = r[1] + r[3] * 0.5;
        let wz = shop_shelf_slot_wz(h, slot_i);

        match *foc {
            ShopFocus::Relic(idx) => {
                if idx < scene.items.len() {
                    let item = &scene.items[idx];
                    let half = relic_half_extents(item.relic, niche_base);
                    let col = if item.sold {
                        color::alpha(rarity_color(item.rarity), 0.35)
                    } else {
                        rarity_color(item.rarity)
                    };
                    let cz = wz + half[2];
                    let relic_pos = sale_anchor_at_slot(w, h, cam, slot_i, env_h, cx, cy, cz);
                    out.push(Object3d {
                        pos: relic_pos,
                        extents: [half[0] * 2.0, half[1] * 2.0, half[2] * 2.0],
                        rotation: euler_xyz_rad_from_deg(super::SHOP_RELIC_LEAN_COUNTER, 0.0, 0.0),
                        color: col,
                        kind: Object3dKind::Relic {
                            relic_id: item.relic,
                            glow: relic_glow(scene, item.relic),
                            silhouette: false,
                            debuffed: false,
                            pick_id: None,
                        },
                        hover_target: 0.0,
                        anim_id: 0,
                        arrange_name: Some("shop.for_sale.relics"),
                    });
                }
            }
            ShopFocus::Pack(pid) => {
                if scene.pack_celebration.is_some() {
                    continue;
                }
                let k = (pid - super::PICK_TILE_PACK_BASE) as usize;
                let Some(pack) = scene.pack_items.get(k) else {
                    continue;
                };
                if pack.sold {
                    continue;
                }
                let pack_h = r[3] * 0.52;
                let pack_w = pack_h * PACK_ASPECT_W_OVER_H;
                let pack_t = pack_h * 0.11;
                let ext = [pack_w, pack_t, pack_h];
                let cz = wz + ext[2] * 0.5;
                let pack_pos = sale_anchor_at_slot(w, h, cam, slot_i, env_h, cx, cy, cz);
                out.push(Object3d {
                    pos: pack_pos,
                    extents: ext,
                    rotation: [0.0, 0.0, 0.0],
                    color: pack.kind.foil_tint(),
                    kind: Object3dKind::Pack {
                        kind: pack.kind,
                        pick_id: None,
                    },
                    hover_target: 0.0,
                    anim_id: 0,
                    arrange_name: Some("shop.for_sale.packs"),
                });
            }
            ShopFocus::Ribbon(i) => {
                let Some(item) = scene.zodiac_items.get(i) else {
                    continue;
                };
                let mut col = consumable_color(item.consumable);
                if item.sold {
                    col[3] = 0.30;
                }
                let ribbon_w = r[2] * 0.38;
                let ribbon_len = r[3] * 0.62;
                let z_k = if let Consumable::Zodiac(z) = item.consumable {
                    Some(z)
                } else {
                    None
                };
                let cz = wz + ribbon_len * 0.35;
                let ribbon_pos = sale_anchor_at_slot(w, h, cam, slot_i, env_h, cx, cy, cz);
                out.push(Object3d {
                    pos: ribbon_pos,
                    extents: [ribbon_w, ribbon_len, ribbon_w * 0.14],
                    rotation: euler_xyz_rad_from_deg(90.0, 0.0, 0.0),
                    color: [1.0, 1.0, 1.0, col[3]],
                    kind: Object3dKind::ZodiacRibbon { kind: z_k },
                    hover_target: 0.0,
                    anim_id: 0,
                    arrange_name: Some("shop.for_sale.ribbons"),
                });
            }
            ShopFocus::Talisman(i) => {
                let Some(item) = scene.talisman_items.get(i) else {
                    continue;
                };
                let mut col = consumable_color(item.consumable);
                if item.sold {
                    col[3] = 0.30;
                }
                if let Consumable::Talisman(tk) = item.consumable {
                    let tw = r[2] * 0.42;
                    let cz = wz + tw * 0.55;
                    let tal_pos = sale_anchor_at_slot(w, h, cam, slot_i, env_h, cx, cy, cz);
                    out.push(Object3d {
                        pos: tal_pos,
                        extents: [tw * 1.4, tw * 2.0, tw * 0.36],
                        rotation: euler_xyz_rad_from_deg(-90.0, 0.0, 0.0),
                        color: col,
                        kind: Object3dKind::Talisman { kind: tk },
                        hover_target: 0.0,
                        anim_id: 0,
                        arrange_name: Some("shop.for_sale.talismans"),
                    });
                }
            }
            _ => {}
        }
    }

    let inv = inventory_slots(scene, shop);
    let n_for_sale = scene.items.len();
    for (slot_i, foc_opt) in inv.iter().enumerate() {
        let Some(foc) = foc_opt.as_ref() else {
            continue;
        };
        let r = inv_slot_rect(w, h, cam, scene, shop, slot_i, env_h);
        let cx = r[0] + r[2] * 0.5;
        let cy = r[1] + r[3] * 0.5;
        let wz = inv_slot_wz(h);

        match *foc {
            ShopFocus::Relic(idx) => {
                if idx < n_for_sale {
                    continue;
                }
                let oi = idx - n_for_sale;
                let Some(&rid) = shop.owned_relics.get(oi) else {
                    continue;
                };
                let rarity = all_relic_defs()
                    .iter()
                    .find(|d| d.id == rid)
                    .map(|d| d.rarity)
                    .unwrap_or(Rarity::Common);
                let half = relic_half_extents(rid, niche_base * 0.92);
                let glow = relic_glow(scene, rid);
                let cz = wz + half[2];
                let relic_pos = inv_marker_surface_anchor(
                    w, h, cam, scene, shop, slot_i, env_h, cx, cy, cz,
                );
                out.push(Object3d {
                    pos: relic_pos,
                    extents: [half[0] * 2.0, half[1] * 2.0, half[2] * 2.0],
                    rotation: euler_xyz_rad_from_deg(super::SHOP_RELIC_LEAN_INVENTORY, 0.0, 0.0),
                    color: rarity_color(rarity),
                    kind: Object3dKind::Relic {
                        relic_id: rid,
                        glow,
                        silhouette: false,
                        debuffed: false,
                        pick_id: None,
                    },
                    hover_target: 0.0,
                    anim_id: 0,
                    arrange_name: Some("shop.shelf.relic_dish"),
                });
            }
            ShopFocus::Ribbon(i) => {
                let zodiac_for_sale = scene.zodiac_items.len();
                let oi = i.saturating_sub(zodiac_for_sale);
                let Some(owned) = shop.owned_zodiacs.get(oi) else {
                    continue;
                };
                if let Consumable::Zodiac(z) = owned.consumable {
                    let ribbon_w = r[2] * 0.36;
                    let ribbon_len = r[3] * 0.58;
                    let cz = wz + ribbon_len * 0.32;
                    let pos = inv_marker_surface_anchor(
                        w, h, cam, scene, shop, slot_i, env_h, cx, cy, cz,
                    );
                    out.push(Object3d {
                        pos,
                        extents: [ribbon_w, ribbon_len, ribbon_w * 0.13],
                        rotation: euler_xyz_rad_from_deg(-90.0, 0.0, 0.0),
                        color: [1.0, 1.0, 1.0, 1.0],
                        kind: Object3dKind::ZodiacRibbon { kind: Some(z) },
                        hover_target: 0.0,
                        anim_id: 0,
                        arrange_name: Some("shop.shelf.ribbon_tray"),
                    });
                }
            }
            ShopFocus::Talisman(i) => {
                let talisman_for_sale = scene.talisman_items.len();
                let oi = i.saturating_sub(talisman_for_sale);
                let Some(owned) = shop.owned_talismans.get(oi) else {
                    continue;
                };
                if let Consumable::Talisman(tk) = owned.consumable {
                    let tw = r[2] * 0.38;
                    let cz = wz + tw * 0.45;
                    let pos = inv_marker_surface_anchor(
                        w, h, cam, scene, shop, slot_i, env_h, cx, cy, cz,
                    );
                    out.push(Object3d {
                        pos,
                        extents: [tw * 1.4, tw * 2.0, tw * 0.34],
                        rotation: euler_xyz_rad_from_deg(-90.0, 0.0, 0.0),
                        color: consumable_color(owned.consumable),
                        kind: Object3dKind::Talisman { kind: tk },
                        hover_target: 0.0,
                        anim_id: 0,
                        arrange_name: Some("shop.shelf.owned_talismans"),
                    });
                }
            }
            _ => {}
        }
    }

    out
}

fn rect_center_n(window_w: f32, window_h: f32, nx: f32, ny: f32, rw: f32, rh: f32) -> [f32; 4] {
    let cx = nx * window_w;
    let cy = ny * window_h;
    [cx - rw * 0.5, cy - rh * 0.5, rw, rh]
}

/// Fallback rects when Shop.glb markers are missing — one horizontal row of nine spawn empties.
fn shop_shelf_slot_rect(w: f32, h: f32, cam: &CameraParams, index: usize, env_h: f32) -> [f32; 4] {
    let rw = w * 0.065;
    let rh = h * 0.125;
    if let Some(r) =
        marker_screen_rect(w, h, cam, &spawn_relic_marker_name(index), rw, rh, env_h)
    {
        return r;
    }
    let nx_span_start = 0.14_f32;
    let nx_span_end = 0.86_f32;
    let ny = 0.34_f32;
    let nx = if SHOP_SPAWN_SLOT_COUNT <= 1 {
        0.5
    } else {
        nx_span_start
            + (nx_span_end - nx_span_start) * (index as f32) / ((SHOP_SPAWN_SLOT_COUNT - 1) as f32)
    };
    rect_center_n(w, h, nx, ny, rw, rh)
}

/// Slanted console row — seven inventory frames.
fn inv_slot_rect(
    w: f32,
    h: f32,
    cam: &CameraParams,
    scene: &ShopScene,
    shop: &ShopReadModel,
    index: usize,
    env_h: f32,
) -> [f32; 4] {
    let rw = w * 0.058;
    let rh = h * 0.108;
    if let Some(name) = inv_slot_glb_marker_name(scene, shop, index) {
        if let Some(r) = marker_screen_rect(w, h, cam, &name, rw, rh, env_h) {
            return r;
        }
    }
    let nx = 0.268 + index as f32 * 0.072;
    // Slightly below prior ny — matches painted inventory recess on storeroom art + hover ring.
    let ny = 0.824;
    rect_center_n(w, h, nx, ny, rw, rh)
}

fn journal_btn_rect(w: f32, h: f32, cam: &CameraParams, env_h: f32) -> [f32; 4] {
    let rw = w * 0.095;
    let rh = h * 0.17;
    if let Some(r) = marker_screen_rect(w, h, cam, "journal_btn", rw, rh, env_h) {
        return r;
    }
    rect_center_n(w, h, 0.905, 0.14, rw, rh)
}

fn reroll_btn_rect(w: f32, h: f32, cam: &CameraParams, env_h: f32) -> [f32; 4] {
    let rw = w * 0.11;
    let rh = h * 0.22;
    if let Some(r) = marker_screen_rect(w, h, cam, "restock_btn", rw, rh, env_h) {
        return r;
    }
    rect_center_n(w, h, 0.89, 0.455, rw, rh)
}

fn leave_btn_rect(w: f32, h: f32, cam: &CameraParams, env_h: f32) -> [f32; 4] {
    let rw = w * 0.068;
    let rh = h * 0.095;
    if let Some(r) = marker_screen_rect(w, h, cam, "exit_btn", rw, rh, env_h) {
        return r;
    }
    rect_center_n(w, h, 0.925, 0.865, rw, rh)
}

/// Sell / returns — bottom-left counter near lantern in the concept.
fn sell_tray_rect(w: f32, h: f32) -> [f32; 4] {
    rect_center_n(w, h, 0.11, 0.88, w * 0.1, h * 0.085)
}

/// Maps compact index `k` (0..`n`) into a centered slot along the 1×9 spawn row (`n` items total).
fn sale_index_to_centered_slot(k: usize, n: usize) -> usize {
    debug_assert!(n > 0 && k < n && n <= SHOP_SPAWN_SLOT_COUNT);
    let start = (SHOP_SPAWN_SLOT_COUNT - n) / 2;
    start + k
}

/// Maps stock into [`SHOP_SPAWN_SLOT_COUNT`] cells (`shop_spawn_relic_00` … `_08`).
/// Order: relics, then packs, then talismans, then ribbons; the group is centered in the row
/// when fewer than nine items.
fn for_sale_slots(scene: &ShopScene) -> [Option<ShopFocus>; SHOP_SPAWN_SLOT_COUNT] {
    let mut out = [None; SHOP_SPAWN_SLOT_COUNT];

    let mut ordered: Vec<ShopFocus> = Vec::new();
    ordered.extend((0..scene.items.len()).map(ShopFocus::Relic));
    ordered.extend(
        (0..scene.pack_items.len())
            .filter(|&k| !scene.pack_items[k].sold)
            .map(|k| ShopFocus::Pack(super::PICK_TILE_PACK_BASE + k as u32)),
    );
    ordered.extend(
        (0..scene.talisman_items.len())
            .filter(|&i| !scene.talisman_items[i].sold)
            .map(ShopFocus::Talisman),
    );
    ordered.extend(
        (0..scene.zodiac_items.len())
            .filter(|&i| !scene.zodiac_items[i].sold)
            .map(ShopFocus::Ribbon),
    );

    let n = ordered.len().min(SHOP_SPAWN_SLOT_COUNT);
    for k in 0..n {
        let slot = sale_index_to_centered_slot(k, n);
        out[slot] = Some(ordered[k]);
    }

    out
}

fn inventory_slots(scene: &ShopScene, shop: &ShopReadModel) -> [Option<ShopFocus>; 7] {
    let mut out = [None; 7];
    let mut slot = 0usize;
    let n_for_sale = scene.items.len();
    for i in 0..shop.owned_relics.len() {
        if slot >= 7 {
            break;
        }
        out[slot] = Some(ShopFocus::Relic(n_for_sale + i));
        slot += 1;
    }
    let talisman_for_sale = scene.talisman_items.len();
    for j in 0..shop.owned_talismans.len() {
        if slot >= 7 {
            break;
        }
        out[slot] = Some(ShopFocus::Talisman(talisman_for_sale + j));
        slot += 1;
    }
    let zodiac_for_sale = scene.zodiac_items.len();
    for j in 0..shop.owned_zodiacs.len() {
        if slot >= 7 {
            break;
        }
        out[slot] = Some(ShopFocus::Ribbon(zodiac_for_sale + j));
        slot += 1;
    }
    out
}

fn build_focus_rects(
    scene: &ShopScene,
    w: f32,
    h: f32,
    cam: &CameraParams,
    shop: &ShopReadModel,
) -> Vec<(ShopFocus, [f32; 4])> {
    let env_h = scene.drawn_env_height_scale.get();
    let mut v = Vec::new();
    let sale_slots = for_sale_slots(scene);
    for (i, sf) in sale_slots.into_iter().enumerate() {
        if let Some(foc) = sf {
            v.push((foc, shop_shelf_slot_rect(w, h, cam, i, env_h)));
        }
    }
    let inv = inventory_slots(scene, shop);
    for (i, sf) in inv.into_iter().enumerate() {
        if let Some(foc) = sf {
            v.push((foc, inv_slot_rect(w, h, cam, scene, shop, i, env_h)));
        }
    }
    v.push((
        ShopFocus::Dish(super::PICK_JOURNAL_BOOK),
        journal_btn_rect(w, h, cam, env_h),
    ));
    v.push((ShopFocus::Reroll, reroll_btn_rect(w, h, cam, env_h)));
    v.push((ShopFocus::NextRound, leave_btn_rect(w, h, cam, env_h)));
    v.push((ShopFocus::SellTray, sell_tray_rect(w, h)));
    v
}

fn map_shop_ui_click_to_hit(cid: u32, scene: &ShopScene, shop: &ShopReadModel) -> Option<ShopHit> {
    match cid {
        SHOP_CLICK_JOURNAL => Some(ShopHit::Dish(super::PICK_JOURNAL_BOOK)),
        SHOP_CLICK_REROLL => Some(ShopHit::Dish(super::PICK_REROLL_PROP)),
        SHOP_CLICK_LEAVE => Some(ShopHit::Dish(super::PICK_LEAVE_PROP)),
        SHOP_CLICK_SELL => Some(ShopHit::Dish(super::PICK_SELL_TRAY)),
        _ => {
            if (SHOP_SHELF_CLICK_BASE..SHOP_SHELF_CLICK_BASE + SHOP_SPAWN_SLOT_COUNT as u32).contains(&cid) {
                let idx = (cid - SHOP_SHELF_CLICK_BASE) as usize;
                return for_sale_slots(scene)[idx].and_then(|f| f.to_hit());
            }
            if (SHOP_INV_CLICK_BASE..SHOP_INV_CLICK_BASE + 7).contains(&cid) {
                let idx = (cid - SHOP_INV_CLICK_BASE) as usize;
                return inventory_slots(scene, shop)[idx].and_then(|f| f.to_hit());
            }
            None
        }
    }
}
