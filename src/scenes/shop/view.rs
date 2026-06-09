//! Shop view (`THEME.md` storeroom): `shop.glb` room, screen-space hit slots, and shelf props.
//! Stock and input dispatch live on [`super::ShopScene`].

use std::time::Instant;

use glam::{Vec2, Vec3};

use super::layout::{
    consumable_color, is_tile_pack_pick, live_shop_hit, rarity_color, relic_half_extents,
    tile_pack_index_from_pick,
};
use super::shared::{focused_sell_action, shop_focus_inspectable};
use super::{
    ConsumableShopItem, ShopFocus, ShopItem, ShopMode, ShopScene, TilePackShopItem, push_free_badge,
};
use crate::core::consumable::Consumable;
use crate::core::relic::{
    Rarity, RelicId, all_relic_defs, relic_description_live, relic_sell_price_live,
};
use crate::core::tile_pack::PACK_ASPECT_W_OVER_H;
use crate::game::engine::{GameEngine, ShopReadModel, consumable_sell_price_for_mode};
use crate::game::game_mode::GameMode;
use crate::game::run::RunState;
use crate::render::consumable_prop_scale::{
    FOR_SALE_TALISMAN_W_FRAC, OWNED_TALISMAN_W_FRAC, for_sale_ribbon_length,
    for_sale_talisman_tablet_extent, owned_ribbon_length, owned_talisman_tablet_extent,
};
use crate::render::draw_cmd::{CameraParams, Object3d, Object3dKind, ScenePunctualLight, UiFrame};
use crate::render::flame_volume::FlameEmitter;
use crate::render::ribbon_mesh::{ZodiacRibbonSpec, ribbon_display_length, zodiac_ribbon_object3d};
use crate::render::room_glb::{
    MarkerScreenRectParams, marker_translation, player_consumable_marker_name,
    player_gold_dish_marker_translation, player_relic_marker_name,
    room_camera_fit_fovy_for_corners, room_camera_with_room_clip_planes, room_env_world_scale,
    room_world_bounds_corners_centered, screen_rect_for_marker_mesh_bounds,
    shop_camera_from_glb_if_present, shop_embedded_point_lights_runtime_tagged,
    shop_embedded_spot_lights_runtime, shop_glb_has_embedded_lights, spawn_relic_marker_name,
    with_shop_glb_cpu,
};
use crate::render::table_transform::euler_xyz_rad_from_deg;
use crate::render::theme::{color, metrics};
use crate::render::wgpu_renderer::GpuInstance;
use crate::render::wgpu_renderer::{PointLight, ShopHit, TextLabel};
use crate::render::world_space::{
    object3d_pos_for_screen_at_world_z, object3d_pos_triple_for_world_center,
    surface_anchor_from_world_xyz,
};
use crate::scenes::object3d_inspect::{
    InspectRig, ItemInspectOrbitState, inspect_orbit_camera, lerp_camera,
    prepend_inspect_orbit_subject_rotation, tick_inspect_dolly,
};
use crate::scenes::options::OptionsScene;
use crate::scenes::{ButtonDef, DrawCtx, SceneBehavior, SceneTransition, UpdateCtx};
use crate::scenes::{OverlayRequest, Scene, WallLedgerScene};
use crate::ui::controller_hints::{
    HintStyle, InlineHintIconSlot, inspect_camera_hint_row, is_confirm_hint_key,
    is_hold_sell_hint_key, is_shop_buy_hint_key, push_screen_footer_hint, screen_footer_top,
    shop_storeroom_footer_row,
};
use crate::ui::focus_nav::{clamp_rect_to_viewport, push_focus_ring, rect_center};
use crate::ui::input::InputMode;
use crate::ui::inspect_plaque::{
    FocusTooltipPanelParams, push_floating_relic_flavor_labels, push_focus_tooltip_panel_2d,
};

/// Matches [`super::RELIC_GLOW_LIFETIME`] — keep glow envelope in sync.
const RELIC_GLOW_SECS: f32 = 0.9;

/// Vertical bob for for-sale / inventory stock (mm at 1080p ref height).
const SHOP_STOCK_BOB_AMP_MM: f32 = 3.8;
/// ~0.95 Hz — slow enough to read as floating, not jittery.
const SHOP_STOCK_BOB_HZ: f32 = 0.95;

#[inline]
fn shop_stock_bob_lift_z(h: f32, bob_seed: u32, age_secs: f32) -> f32 {
    let phase = (bob_seed as f32 * 2.399_963_2).fract() * std::f32::consts::TAU;
    let t = age_secs * SHOP_STOCK_BOB_HZ * std::f32::consts::TAU;
    let amp = h * (SHOP_STOCK_BOB_AMP_MM / 1080.0);
    (t + phase).sin() * amp
}

#[inline]
fn apply_shop_stock_bob(pos: &mut [f32; 3], h: f32, bob_seed: u32, age_secs: f32) {
    pos[2] += shop_stock_bob_lift_z(h, bob_seed, age_secs);
}

/// Screen-space hit ids (processed in `update` before the main shop pick pass).
const SHOP_SHELF_CLICK_BASE: u32 = 0xD000;
/// One UI/hit slot per `shop_spawn_relic_00` … `shop_spawn_relic_08` in shop.glb.
pub(in crate::scenes::shop) const SHOP_SPAWN_SLOT_COUNT: usize = 9;
const SHOP_CLICK_JOURNAL: u32 = 0xD011;
const SHOP_CLICK_RESTOCK: u32 = 0xD012;
const SHOP_CLICK_LEAVE: u32 = 0xD013;
const SHOP_CLICK_WALL: u32 = 0xD014;

fn default_fill_point_lights(w: f32, h: f32) -> Vec<PointLight> {
    vec![
        PointLight {
            pos: [w * 0.52, h * 0.34, h * 0.26],
            radius: h * 2.5,
            color: color::rgb(color::PARCHMENT),
            intensity: 3.6,
        },
        PointLight {
            pos: [w * 0.52, h * 0.46, h * 0.21],
            radius: h * 2.3,
            color: color::rgb(color::PARCHMENT),
            intensity: 3.0,
        },
        PointLight {
            pos: [w * 0.52, h * 0.78, h * 0.13],
            radius: h * 2.0,
            color: color::rgb(color::PARCHMENT),
            intensity: 2.7,
        },
    ]
}

/// Procedural flame particles at each `light_candle*` punctual light in `shop.glb`.
fn shop_gltf_candle_flame_emitters(
    _w: f32,
    h: f32,
    env_h: f32,
    _age_secs: f32,
    lamp_flicker: f32,
    _layout: &crate::ui::layout::LayoutResult,
    tuning: &crate::render::flame_tuning::FlameTuning,
) -> Vec<FlameEmitter> {
    with_shop_glb_cpu(|opt| {
        let Some(cpu) = opt else {
            return Vec::new();
        };
        let s = room_env_world_scale(h, env_h);
        let center_doc = cpu
            .environment_bounds_doc
            .map(|b| b.center())
            .unwrap_or(Vec3::ZERO);
        let flame_scale = tuning.emitter_scale(
            crate::render::flame_volume::SHOP_GLTF_CANDLE_HEIGHT_DOC_M * s.max(1e-6),
        );
        cpu.embedded_point_lights
            .iter()
            .filter(|l| l.is_candle)
            .enumerate()
            .map(|(i, l)| {
                let light_world = (l.pos_doc - center_doc) * s;
                let seed = (i as u32)
                    .wrapping_mul(0x9E37_79B9)
                    .wrapping_add(0xA5A5_A5A5);
                let scale_jitter = 0.88 + (seed & 0x3f) as f32 / 63.0 * 0.24;
                let emitter_scale = flame_scale * scale_jitter;
                let world = tuning.wick_from_light(light_world, emitter_scale);
                let phase = (seed as f32 * 2.328_306e-10).fract();
                let brightness = lamp_flicker;
                FlameEmitter {
                    wick_world: world,
                    scale: emitter_scale,
                    wind: Vec2::ZERO,
                    brightness,
                    phase,
                    flicker_amp: tuning.candle_flicker_amp,
                }
            })
            .collect()
    })
}

pub(super) fn shop_camera_base(w: f32, h: f32, env_h: f32) -> CameraParams {
    let from_glb = shop_camera_from_glb_if_present(h, env_h);
    with_shop_glb_cpu(|opt| {
        let mut cam = from_glb.unwrap_or_else(|| {
            // ref_h: 1080 — fallback when shop.glb has no usable perspective camera (room centered at origin).
            let cs = h / 1080_f32;
            CameraParams {
                eye: [0.0 * cs, -1517.6 * cs, 1557.2 * cs],
                target: [0.0, 0.0, 0.0],
                up: [0.0, 0.0, 1.0],
                fovy_deg: 58.0,
                clip_near: None,
                clip_far: None,
            }
        });
        // Auto-fit widens vertical FOV past the authored value; keep GLB eye / target / up / fovy intact.
        if from_glb.is_none()
            && let Some(cpu) = opt
        {
            let corners = room_world_bounds_corners_centered(h, env_h, cpu);
            cam = room_camera_fit_fovy_for_corners(w, h, cam, &corners, 0.94);
        }
        if let Some(cpu) = opt {
            cam = room_camera_with_room_clip_planes(cam, h, env_h, cpu);
        }
        cam
    })
}

fn shop_camera_params(w: f32, h: f32, env_h: f32) -> CameraParams {
    shop_camera_base(w, h, env_h)
}

/// World-space pivot matching the focused stock mesh anchor (inverse of
/// [`crate::render::world_space::object3d_pos_triple_for_world_center`]), for the given camera.
fn shop_inspect_pivot_world(
    scene: &ShopScene,
    shop: &ShopReadModel,
    w: f32,
    h: f32,
    cam: &CameraParams,
    env_h: f32,
    focus: ShopFocus,
) -> Option<glam::Vec3> {
    if !shop_focus_inspectable(focus) {
        return None;
    }
    let niche_base = w * 0.048;

    let packed: [f32; 3] = if let Some(slot_i) = sale_slot_for_focus(scene, focus) {
        let r = shop_shelf_slot_rect(w, h, cam, slot_i, env_h);
        let cx = r[0] + r[2] * 0.5;
        let cy = r[1] + r[3] * 0.5;
        let wz = shop_shelf_slot_wz(h, slot_i);
        match focus {
            ShopFocus::Relic(idx) => {
                let item = scene.items.get(idx)?;
                let half = relic_half_extents(item.relic, niche_base);
                let cz = wz + half[2];
                sale_anchor_at_slot(SaleAnchorAtSlot {
                    screen: ShopScreenAnchor {
                        w,
                        h,
                        cam,
                        env_h,
                        cx,
                        cy,
                        cz_fallback: cz,
                    },
                    slot_i,
                })
            }
            ShopFocus::Pack(pid) => {
                let k = (pid - super::PICK_TILE_PACK_BASE) as usize;
                let pack = scene.pack_items.get(k)?;
                if pack.sold {
                    return None;
                }
                let pack_h = r[3] * 1.04;
                let pack_w = pack_h * PACK_ASPECT_W_OVER_H;
                let pack_t = pack_h * 0.11;
                let ext = [pack_w, pack_t, pack_h];
                let cz = wz + ext[2] * 0.5;
                sale_anchor_at_slot(SaleAnchorAtSlot {
                    screen: ShopScreenAnchor {
                        w,
                        h,
                        cam,
                        env_h,
                        cx,
                        cy,
                        cz_fallback: cz,
                    },
                    slot_i,
                })
            }
            ShopFocus::Ribbon(i) => {
                scene.zodiac_items.get(i)?;
                let ribbon_len = for_sale_ribbon_length(r[2], r[3]);
                let ribbon_world = ribbon_display_length(ribbon_len);
                let cz = wz + ribbon_world * 0.35 - ribbon_world * 0.5;
                sale_anchor_at_slot(SaleAnchorAtSlot {
                    screen: ShopScreenAnchor {
                        w,
                        h,
                        cam,
                        env_h,
                        cx,
                        cy,
                        cz_fallback: cz,
                    },
                    slot_i,
                })
            }
            ShopFocus::Talisman(i) => {
                let item = scene.talisman_items.get(i)?;
                let Consumable::Talisman(_) = item.consumable else {
                    return None;
                };
                let tw = r[2] * 0.84;
                let cz = wz + tw * 0.55;
                sale_anchor_at_slot(SaleAnchorAtSlot {
                    screen: ShopScreenAnchor {
                        w,
                        h,
                        cam,
                        env_h,
                        cx,
                        cy,
                        cz_fallback: cz,
                    },
                    slot_i,
                })
            }
            _ => return None,
        }
    } else {
        let inv = inventory_slots(scene, shop);
        let slot_i = inv.iter().position(|cell| *cell == Some(focus))?;
        let r = inv_slot_rect(w, h, cam, scene, shop, slot_i, env_h);
        let cx = r[0] + r[2] * 0.5;
        let cy = r[1] + r[3] * 0.5;
        let wz = inv_slot_wz(h);
        match focus {
            ShopFocus::Relic(idx) => {
                let n_for_sale = scene.items.len();
                if idx < n_for_sale {
                    return None;
                }
                let oi = idx - n_for_sale;
                let rid = *shop.owned_relics.get(oi)?;
                let half = relic_half_extents(rid, niche_base * 0.92);
                let cz = wz + half[2];
                inv_marker_surface_anchor(InvMarkerSurfaceAnchor {
                    screen: ShopScreenAnchor {
                        w,
                        h,
                        cam,
                        env_h,
                        cx,
                        cy,
                        cz_fallback: cz,
                    },
                    scene,
                    shop,
                    slot_i,
                })
            }
            ShopFocus::Ribbon(i) => {
                let zodiac_for_sale = scene.zodiac_items.len();
                let oi = i.saturating_sub(zodiac_for_sale);
                shop.owned_zodiacs.get(oi)?;
                let ribbon_len = owned_ribbon_length(r[2], r[3]);
                let ribbon_world = ribbon_display_length(ribbon_len);
                let cz = wz + ribbon_world * 0.32 - ribbon_world * 0.5;
                inv_marker_surface_anchor(InvMarkerSurfaceAnchor {
                    screen: ShopScreenAnchor {
                        w,
                        h,
                        cam,
                        env_h,
                        cx,
                        cy,
                        cz_fallback: cz,
                    },
                    scene,
                    shop,
                    slot_i,
                })
            }
            ShopFocus::Talisman(i) => {
                let talisman_for_sale = scene.talisman_items.len();
                let oi = i.saturating_sub(talisman_for_sale);
                shop.owned_talismans.get(oi)?;
                let tw = r[2] * 0.76;
                let cz = wz + tw * 0.45;
                inv_marker_surface_anchor(InvMarkerSurfaceAnchor {
                    screen: ShopScreenAnchor {
                        w,
                        h,
                        cam,
                        env_h,
                        cx,
                        cy,
                        cz_fallback: cz,
                    },
                    scene,
                    shop,
                    slot_i,
                })
            }
            _ => return None,
        }
    };

    Some(glam::Vec3::new(
        packed[0] - w * 0.5,
        h * 0.5 - packed[1],
        packed[2],
    ))
}

/// World-space inspect target for fixed inspect camera (matches shelf mesh anchor under `base` cam).
pub(super) fn shop_inspect_target_world(
    scene: &ShopScene,
    w: f32,
    h: f32,
    env_h: f32,
    shop: &ShopReadModel,
    focus: ShopFocus,
) -> Option<[f32; 3]> {
    let base = shop_camera_base(w, h, env_h);
    shop_inspect_pivot_world(scene, shop, w, h, &base, env_h, focus).map(|v| v.to_array())
}

/// Initial orbit state for a pushdown [`crate::scenes::ItemInspectScene`] from shop focus.
pub(super) fn shop_item_inspect_orbit_for_focus(
    scene: &ShopScene,
    w: f32,
    h: f32,
    env_h: f32,
    shop: &ShopReadModel,
    focus: ShopFocus,
) -> Option<ItemInspectOrbitState> {
    let tw = shop_inspect_target_world(scene, w, h, env_h, shop, focus)?;
    Some(ItemInspectOrbitState {
        target_world: tw,
        yaw: 0.0,
        pitch: 0.0,
        zoom: 1.0,
    })
}

/// Keep orbit pivot on the focused stock mesh while the window or storeroom layout changes.
///
/// Uses the resting storeroom camera only. Projecting shelf anchors under the inspect camera
/// while the look target tracks that projection makes the pivot chase a moving ray hit (tremble).
pub(super) fn shop_sync_item_inspect_orbit_target(
    scene: &ShopScene,
    run: &RunState,
    w: f32,
    h: f32,
    orbit: &mut ItemInspectOrbitState,
) {
    let Some(foc) = scene.focus else {
        return;
    };
    if !shop_focus_inspectable(foc) {
        return;
    }
    let shop_rm = GameEngine::read_shop(run);
    let env_h = scene.drawn_room_gltf_height_scale.get();
    let base = shop_camera_base(w, h, env_h);
    if let Some(pivot) = shop_inspect_pivot_world(scene, &shop_rm, w, h, &base, env_h, foc) {
        orbit.target_world = pivot.to_array();
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
    with_shop_glb_cpu(|opt| {
        let cpu = opt?;
        if let Some(r) = screen_rect_for_marker_mesh_bounds(&MarkerScreenRectParams {
            win_w,
            win_h,
            cam,
            env_height_scale,
            cpu,
            node_name,
            min_rw: rw,
            min_rh: rh,
        }) {
            return Some(r);
        }
        let tw =
            marker_translation(cpu, node_name)? * room_env_world_scale(win_h, env_height_scale);
        let (cx, cy) = cam.project_world_to_screen(win_w, win_h, tw);
        Some([cx - rw * 0.5, cy - rh * 0.5, rw, rh])
    })
}

/// [`Object3d::pos`] for the gameplay-style coin pile: [`crate::render::room_glb::PLAYER_GOLD_DISH_MARKER`]
/// (or legacy `PlayerGoldDish`) when the room loads, otherwise a fixed screen fallback.
fn player_gold_dish_object3d_anchor(
    w: f32,
    h: f32,
    cam: &CameraParams,
    env_h: f32,
    ppmm: f32,
) -> [f32; 3] {
    let scale = room_env_world_scale(h, env_h);
    if let Some(tw) = with_shop_glb_cpu(|opt| opt.and_then(player_gold_dish_marker_translation)) {
        let world = tw * scale;
        return surface_anchor_from_world_xyz(w, h, world);
    }
    let cx = w * 0.742_847_26;
    let cy = h * 0.84;
    object3d_pos_for_screen_at_world_z(w, h, cam, cx, cy, ppmm * 10.0)
}

fn owned_relic_hover_center(
    shop: &ShopScene,
    shop_rm: &ShopReadModel,
    oi: usize,
    w: f32,
    h: f32,
    cam: &CameraParams,
    env_h: f32,
) -> (f32, f32, f32) {
    let n_for_sale = shop.items.len();
    let inv = inventory_slots(shop, shop_rm);
    for (slot_i, foc) in inv.iter().enumerate() {
        if matches!(foc, Some(ShopFocus::Relic(idx)) if *idx == n_for_sale + oi) {
            let r = inv_slot_rect(w, h, cam, shop, shop_rm, slot_i, env_h);
            return (r[0] + r[2] * 0.5, r[1] + r[3] * 0.5, inv_slot_wz(h));
        }
    }
    (w * 0.5, h * 0.5, h * 0.2)
}

impl SceneBehavior for ShopScene {
    fn face_button_bindings(
        &self,
        _ctx: crate::ui::input::FaceBindingCtx,
    ) -> crate::ui::input::FaceButtonBindings {
        crate::ui::input::FaceButtonBindings {
            west_press: Some(crate::ui::input::UiAction::WestFacePress),
            north_press: Some(crate::ui::input::UiAction::NorthFacePress),
            west_release: Some(crate::ui::input::UiAction::WestFaceRelease),
            suppress_trigger_structure: true,
        }
    }

    fn update(&mut self, mut ctx: UpdateCtx<'_>) -> SceneTransition {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let shop_rm = GameEngine::read_shop(ctx.run);
        self.stash_focus_rects(w, h, ctx.run);

        for &cid in ctx.button_clicks {
            if cid == SHOP_CLICK_WALL {
                *ctx.overlay_request = Some(OverlayRequest::Push(Box::new(Scene::WallLedger(
                    WallLedgerScene::shop_preview(),
                ))));
                return None;
            }
            if let Some(hit) = map_shop_ui_click_to_hit(cid, self, &shop_rm)
                && let Some(next) = self.dispatch_shop_pick_from_hit(hit, &mut ctx)
            {
                return Some(next);
            }
        }

        self.update_impl(ctx)
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        render_shop_frame(self, ctx, None)
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
        let _g = crate::render::cpu_profiler::scope("update.shop_stash_focus_rects");
        let shop = GameEngine::read_shop(run);
        let env_h = self.drawn_room_gltf_height_scale.get();
        let cam = shop_camera_params(w, h, env_h);
        *self.last_focus_rects.borrow_mut() = build_focus_rects(self, w, h, &cam, &shop, run);
    }
}

/// Storeroom draw for the normal shop face. Pass `inspect: Some` from [`crate::scenes::ShopInspectPresenter`] to ease the camera into orbit while keeping the room.
pub(crate) fn render_shop_frame(
    shop: &ShopScene,
    mut ctx: DrawCtx<'_>,
    inspect: Option<&ItemInspectOrbitState>,
) -> UiFrame {
    let w = ctx.layout.window_w;
    let h = ctx.layout.window_h;
    let env_h = ctx.room_env_for("shop").1;
    shop.drawn_room_gltf_height_scale.set(env_h);
    let shop_rm = GameEngine::read_shop(ctx.run);

    let scale = metrics::scene_scale(w, h);

    let mut frame = UiFrame::new();
    frame.shop_gltf_anim_samples = shop.gltf_anim_samples();
    frame.quad(GpuInstance {
        rect: [0.0, 0.0, w, h],
        color: color::WALNUT_INK,
        user: 0,
    });
    if with_shop_glb_cpu(|opt| opt.is_some()) {
        frame.shop_environment();
    }

    // Mesh anchors use ray/plane hits so props sit under display-case pixels (plain
    // `pixel_to_world` drifts under perspective).
    let base = {
        let base = shop_camera_base(w, h, env_h);
        if inspect.is_none()
            && (shop.storeroom_orbit_yaw.abs() > 1e-6 || shop.storeroom_orbit_pitch.abs() > 1e-6)
        {
            crate::scenes::object3d_inspect::orbit_camera_around_pivot(
                &base,
                base.target,
                shop.storeroom_orbit_yaw,
                shop.storeroom_orbit_pitch,
            )
        } else {
            base
        }
    };
    let inspect_rig = InspectRig::shop(h, env_h);
    let (inspect_anchor, _inspect_cam_now, cam) = {
        let _g = crate::render::cpu_profiler::scope("draw_frame.shop_camera_inspect");
        let (inspect_anchor, inspect_cam_now) = if let Some(ins) = inspect {
            let ic = inspect_orbit_camera(ins, &inspect_rig);
            let anchor = shop.focus.and_then(|f| {
                if !shop_focus_inspectable(f) {
                    return None;
                }
                let tw = shop_inspect_target_world(shop, w, h, env_h, &shop_rm, f)
                    .unwrap_or(ins.target_world);
                Some((f, tw))
            });
            (anchor, Some(ic))
        } else {
            (None, None)
        };
        if let Some(ic) = inspect_cam_now {
            shop.last_inspect_cam.set(Some(ic));
        }
        let target_phase = if inspect.is_some() { 1.0 } else { 0.0 };
        let eased = tick_inspect_dolly(&shop.inspect_dolly, target_phase);
        let inspect_cam_for_lerp = inspect_cam_now.or_else(|| shop.last_inspect_cam.get());
        let final_cam = match (inspect_cam_for_lerp, eased > 1e-4) {
            (Some(ic), true) => lerp_camera(&base, &ic, eased, h),
            _ => base,
        };
        frame.camera_override = Some(final_cam);
        (inspect_anchor, inspect_cam_now, final_cam)
    };

    let ppmm = ctx.layout.mm(1.0);
    let gold_dish_anchor = player_gold_dish_object3d_anchor(w, h, &cam, env_h, ppmm);

    // Legacy synthesized lamp when `shop.glb` has no embedded punctual lights.
    let lp = (w * 0.5, h * 0.28, ppmm * 180.575_9);
    let tf = shop.age_secs;
    let flick_fast = (tf * 37.3).sin() * 0.022 + (tf * 61.7).sin() * 0.014;
    let flick_slow = (tf * 4.1).sin() * 0.034;
    let brownout = {
        let d = (tf * 0.73).sin() * (tf * 1.19).sin();
        (d - 0.55).max(0.0) * 0.22
    };
    let lamp_flicker = (1.0 + flick_fast + flick_slow - brownout).clamp(0.68, 1.08);

    let hover = shop
        .focus
        .and_then(|f| f.to_hit())
        .or(ctx.picked_shop_object)
        .and_then(|hit| {
            live_shop_hit(
                hit,
                shop,
                &shop.items,
                &shop.zodiac_items,
                &shop.talisman_items,
                &shop.pack_items,
                &shop_rm,
            )
        });

    let room_glb_lights = shop_glb_has_embedded_lights();
    {
        let _g = crate::render::cpu_profiler::scope("draw_frame.shop_scene_lighting");
        frame.scene_lighting.embedded_gltf_punctual = room_glb_lights;
        frame.scene_lighting.room_glb_brdf = room_glb_lights;
        let use_glb_lights = room_glb_lights;
        let embedded_tagged = if use_glb_lights {
            shop_embedded_point_lights_runtime_tagged(
                w,
                h,
                env_h,
                &ctx.room_env_for("shop").0,
                shop.age_secs,
                lamp_flicker,
                ctx.flame_tuning.candle_flicker_amp,
            )
        } else {
            Vec::new()
        };
        let mut merged_punctual: Vec<ScenePunctualLight> = embedded_tagged
            .iter()
            .map(|t| ScenePunctualLight::InverseSquare(t.light))
            .collect();
        let mut punctual_gltf_nodes: Vec<Option<String>> = embedded_tagged
            .into_iter()
            .map(|t| Some(t.gltf_node_name))
            .collect();
        let mut point_lights: Vec<PointLight> = if use_glb_lights {
            Vec::new()
        } else {
            let mut v = vec![PointLight {
                pos: [lp.0, lp.1, lp.2],
                radius: h * 1.15,
                color: [0.86, 0.96, 0.98],
                intensity: 2.15 * lamp_flicker,
            }];
            v.extend(default_fill_point_lights(w, h));
            v
        };

        // Hover fill: full-strength when we synthesize shop lighting; with embedded
        // `KHR_lights_punctual`, use a ~10% pool so props stay readable (see focus ring below).
        let (hover_i_mul, hover_r_mul) = if use_glb_lights {
            (0.10_f32, 1.08_f32)
        } else {
            (1.0_f32, 1.0_f32)
        };
        if let Some(hit) = hover {
            match hit {
                ShopHit::Relic(i) => {
                    let (px, py, wy) = if let Some((cx, cy)) =
                        screen_xy_for_hit(hit, shop, &shop_rm, ctx.run, w, h, &cam)
                    {
                        (cx, cy, light_lift_at_screen_y(cy, h))
                    } else if let Some(si) = sale_slot_for_focus(shop, ShopFocus::Relic(i)) {
                        let r = shop_shelf_slot_rect(w, h, &cam, si, env_h);
                        (
                            r[0] + r[2] * 0.5,
                            r[1] + r[3] * 0.5,
                            shop_shelf_slot_wz(h, si),
                        )
                    } else if i >= shop.items.len() {
                        owned_relic_hover_center(
                            shop,
                            &shop_rm,
                            i - shop.items.len(),
                            w,
                            h,
                            &cam,
                            env_h,
                        )
                    } else {
                        (w * 0.5, h * 0.5, h * 0.2)
                    };
                    point_lights.push(PointLight {
                        pos: [px, py - 30.0, wy + 60.0],
                        radius: h * 0.65 * hover_r_mul,
                        color: color::rgb(color::PARCHMENT),
                        intensity: 3.20 * hover_i_mul,
                    });
                }
                ShopHit::Ribbon(_) | ShopHit::Talisman(_) => {
                    if let Some((cx, cy)) =
                        screen_xy_for_hit(hit, shop, &shop_rm, ctx.run, w, h, &cam)
                    {
                        let wy = light_lift_at_screen_y(cy, h);
                        point_lights.push(PointLight {
                            pos: [cx, cy + 35.0, wy + 50.0],
                            radius: h * 0.72 * hover_r_mul,
                            color: color::rgb(color::PARCHMENT),
                            intensity: 3.00 * hover_i_mul,
                        });
                    }
                }
                ShopHit::Dish(_) => {
                    let center = (
                        gold_dish_anchor[0],
                        gold_dish_anchor[1],
                        gold_dish_anchor[2],
                    );
                    point_lights.push(PointLight {
                        pos: [center.0, center.1 - 20.0, center.2.max(80.0)],
                        radius: h * 0.55 * hover_r_mul,
                        color: color::rgb(color::PARCHMENT),
                        intensity: 2.50 * hover_i_mul,
                    });
                }
                ShopHit::TilePack(id) => {
                    if let Some((cx, cy)) =
                        screen_xy_for_hit(hit, shop, &shop_rm, ctx.run, w, h, &cam)
                    {
                        let wy = light_lift_at_screen_y(cy, h);
                        point_lights.push(PointLight {
                            pos: [cx, cy - 28.0, wy + 55.0],
                            radius: h * 0.62 * hover_r_mul,
                            color: color::rgb(color::PARCHMENT),
                            intensity: 3.20 * hover_i_mul,
                        });
                    } else if let Some(idx) = super::layout::tile_pack_index_from_pick(id) {
                        let pid = super::PICK_TILE_PACK_BASE + idx as u32;
                        if let Some(si) = sale_slot_for_focus(shop, ShopFocus::Pack(pid)) {
                            let r = shop_shelf_slot_rect(w, h, &cam, si, env_h);
                            let center = (
                                r[0] + r[2] * 0.5,
                                r[1] + r[3] * 0.5,
                                shop_shelf_slot_wz(h, si),
                            );
                            point_lights.push(PointLight {
                                pos: [center.0, center.1 - 30.0, center.2 + 60.0],
                                radius: h * 0.62 * hover_r_mul,
                                color: color::rgb(color::PARCHMENT),
                                intensity: 3.20 * hover_i_mul,
                            });
                        }
                    }
                }
                ShopHit::EnvSpawnSlot(_)
                | ShopHit::EnvInvSlot(_)
                | ShopHit::EnvConsumableOrd(_) => {}
            }
        }

        let proc_count = point_lights.len();
        merged_punctual.extend(point_lights.into_iter().map(ScenePunctualLight::Smooth));
        punctual_gltf_nodes.extend(std::iter::repeat_n(None, proc_count));
        frame.scene_lighting.punctual = merged_punctual;
        frame.scene_lighting.punctual_gltf_nodes = punctual_gltf_nodes;
        frame
            .scene_lighting
            .set_gltf_embedded_spot_lights(shop_embedded_spot_lights_runtime(
                w,
                h,
                env_h,
                &ctx.room_env_for("shop").0,
            ));
        if use_glb_lights {
            let candle_flames = shop_gltf_candle_flame_emitters(
                w,
                h,
                env_h,
                shop.age_secs,
                lamp_flicker,
                ctx.layout,
                &ctx.flame_tuning,
            );
            frame.candle_light_count = candle_flames.len() as u32;
            frame.flame_height_world = ctx.flame_tuning.flame_height_world(
                room_env_world_scale(h, env_h),
                crate::render::flame_volume::SHOP_GLTF_CANDLE_HEIGHT_DOC_M,
            );
            frame.procedural_flame_emitters = candle_flames;
        }
    }

    let gold_pile_anchor = [
        gold_dish_anchor[0],
        gold_dish_anchor[1],
        gold_dish_anchor[2] + ppmm * 3.0,
    ];
    let gold_pile;
    let (stock_dim, stock_subj);
    {
        let _g = crate::render::cpu_profiler::scope("draw_frame.shop_stock_meshes");
        gold_pile = crate::render::yen_display::build_settled_yen_coin_pile(
            |n| ppmm * n,
            shop_rm.display_yen as i32,
            gold_pile_anchor,
            crate::render::yen_display::SHOP_GOLD_PILE_SEED,
            None,
            1.0,
        );
        (stock_dim, stock_subj) = push_stock_meshes(shop, &shop_rm, w, h, &cam, inspect_anchor);
    }

    let mut stock_all = stock_dim;
    if let Some(mut s) = stock_subj {
        if let Some(ins) = inspect {
            s = prepend_inspect_orbit_subject_rotation(s, ins, &inspect_rig);
        }
        stock_all.push(s);
    }
    stock_all.extend(gold_pile);
    if !stock_all.is_empty() {
        frame.object3d_batch(stock_all);
    }

    // Volumetric candle flames: after shelf/env meshes (depth buffer) but before
    // any 2D HUD (tooltips, focus rings, pause). `frame.flames` at end of draw
    // stamped additive fire over tooltip panels.
    if !frame.procedural_flame_emitters.is_empty() {
        frame.flame_batch();
    }

    // Gold plaque + label sit on the post-tonemap overlay layer (text label
    // routes through `frame.texts`), so they'd punch through the pause /
    // options dim — skip them while the pause menu is up.
    let gold_label_rect = if shop.pause_menu.paused {
        [0.0, 0.0, 0.0, 0.0]
    } else {
        let _g = crate::render::cpu_profiler::scope("draw_frame.shop_gold_label");
        let gold_label_center = with_shop_glb_cpu(|opt| {
            let cpu = opt?;
            let tw = player_gold_dish_marker_translation(cpu)? * room_env_world_scale(h, env_h);
            let (cx, cy) = cam.project_world_to_screen(w, h, tw);
            Some((cx, cy))
        })
        .unwrap_or((gold_dish_anchor[0], gold_dish_anchor[1]));
        crate::render::yen_display::push_yen_amount_label(
            &mut frame,
            w,
            h,
            shop_rm.display_yen as i32,
            gold_label_center,
        )
    };

    // Shelf focus ring uses shelf-slot screen rects.
    if !shop.pause_menu.paused && inspect.is_none() {
        let _g = crate::render::cpu_profiler::scope("draw_frame.shop_focus_ring_and_badges");
        if let Some(ring_rect) = ring_target_rect(&ctx, shop, &shop_rm, w, h, &cam)
            .and_then(|r| clamp_rect_to_viewport(r, w, h))
        {
            let mut quads = Vec::new();
            let ring_scale = if room_glb_lights { scale * 1.24 } else { scale };
            push_focus_ring(ring_rect, ring_scale, w, h, &mut quads);
            frame.quads(quads);
        }

        let mut free_quads = Vec::new();
        let mut free_texts = Vec::new();
        for (slot_i, sf) in for_sale_slots(shop).into_iter().enumerate() {
            if let Some(ShopFocus::Relic(idx)) = sf
                && let Some(item) = shop.items.get(idx)
                && !item.sold
                && item.price == 0
            {
                let r = shop_shelf_slot_rect(w, h, &cam, slot_i, env_h);
                push_free_badge(&mut free_quads, &mut free_texts, r, h);
            }
        }
        if !free_quads.is_empty() {
            frame.quads(free_quads);
            frame.texts(free_texts);
        }
    }

    if !shop.pause_menu.paused
        && inspect.is_none()
        && let Some(hit) = hover
        && let Some((ref title, ref desc, ref cta, col)) =
            hover_tooltip_content(shop, &shop_rm, &ctx.run.mode, hit)
        && !title.is_empty()
    {
        let _g = crate::render::cpu_profiler::scope("draw_frame.shop_hover_tooltip");
        let f = ShopFocus::from_hit(hit);
        let tooltip_anchor = shop
            .last_focus_rects
            .borrow()
            .iter()
            .find(|(g, _)| *g == f)
            .map(|(_, r)| *r);
        let hover_is_owned = matches!(hit, ShopHit::Relic(i) if i >= shop.items.len())
            || matches!(hit, ShopHit::Ribbon(i) if i >= shop.zodiac_items.len())
            || matches!(hit, ShopHit::Talisman(i) if i >= shop.talisman_items.len());
        let skip_title = matches!(
            hit,
            ShopHit::Dish(id)
                if id == super::PICK_JOURNAL_BOOK
                    || id == super::PICK_LEAVE_PROP
                    || id == super::PICK_RESTOCK_PROP
        );
        let mut tip_quads = Vec::new();
        let mut tip_texts = Vec::new();
        push_focus_tooltip_panel_2d(
            &mut tip_quads,
            &mut tip_texts,
            FocusTooltipPanelParams {
                window_w: w,
                window_h: h,
                anchor_rect: tooltip_anchor,
                title,
                desc,
                cta,
                accent_color: col,
                hover_is_owned,
                skip_title_block: skip_title,
                avoid_rect: Some(gold_label_rect),
            },
        );
        // Relic: hover = name + mechanical; E inspect = name + flavor (no mechanical).
        frame.overlay_quads(tip_quads);
        frame.texts(tip_texts);
    }

    {
        let _g = crate::render::cpu_profiler::scope("draw_frame.shop_particles_popups");
        shop.push_shop_particle_quads(&mut frame);
        shop.push_shop_score_popup_labels(&mut frame, w, h);
    }

    // Pointer targets in stack order: specific zones first (main loop uses first hit).
    // Pause overlay quads/texts are appended last so they composite above shelf geometry.
    if shop.pause_menu.paused {
        let _g = crate::render::cpu_profiler::scope("draw_frame.shop_pause_menu");
        let mut pause_quads: Vec<GpuInstance> = Vec::new();
        let mut pause_text: Vec<TextLabel> = Vec::new();
        let mut pause_buttons: Vec<ButtonDef> = Vec::new();
        shop.pause_menu.draw(
            crate::ui::layout::ViewportCtx {
                window_w: w,
                window_h: h,
            },
            scale,
            crate::scenes::options::options_scroll_fade_backdrop(true),
            &mut pause_quads,
            &mut pause_text,
            &mut pause_buttons,
        );
        frame.quads(pause_quads);
        frame.texts(pause_text);
        pause_buttons.push(ButtonDef::scene((0.0, 0.0, w, h), u32::MAX));
        frame.buttons = pause_buttons;
    } else if inspect.is_none() {
        let _g = crate::render::cpu_profiler::scope("draw_frame.shop_hit_buttons");
        for (i, sf) in for_sale_slots(shop).into_iter().enumerate() {
            if sf.is_none() {
                continue;
            }
            let r = shop_shelf_slot_rect(w, h, &cam, i, env_h);
            frame.buttons.push(ButtonDef::scene(
                (r[0], r[1], r[2], r[3]),
                SHOP_SHELF_CLICK_BASE + i as u32,
            ));
        }
        let jr = journal_btn_rect(w, h, &cam, env_h);
        frame.buttons.push(ButtonDef::scene(
            (jr[0], jr[1], jr[2], jr[3]),
            SHOP_CLICK_JOURNAL,
        ));
        let rr = restock_btn_rect(w, h, &cam, env_h);
        frame.buttons.push(ButtonDef::scene(
            (rr[0], rr[1], rr[2], rr[3]),
            SHOP_CLICK_RESTOCK,
        ));
        let lr = leave_btn_rect(w, h, &cam, env_h);
        frame.buttons.push(ButtonDef::scene(
            (lr[0], lr[1], lr[2], lr[3]),
            SHOP_CLICK_LEAVE,
        ));
        let wall_count = crate::game::wall_ledger::shop_wall_hud_count(ctx.run);
        let wall = crate::render::wall_display::wall_hud_layout(w, h, wall_count);
        let wr = wall.block_rect;
        frame.buttons.push(ButtonDef::scene(
            (wr[0], wr[1], wr[2], wr[3]),
            SHOP_CLICK_WALL,
        ));
        // Catch-all for Object3D / GLB collision picks (inventory + props). Pushed
        // last so shelf + HUD rects win when they overlap the cursor.
        if ctx.picked_shop_object.is_some() {
            frame
                .buttons
                .push(ButtonDef::scene((0.0, 0.0, w, h), super::SHOP_3D_HIT_ID));
        }
    }

    // Floating control hints — copy reflects [`DrawCtx::input_mode`] + swap toggles.
    if !shop.pause_menu.paused {
        let _g = crate::render::cpu_profiler::scope("draw_frame.shop_floating_hints");
        let inspect_active = inspect.is_some();

        let sell_hint_hit = ctx
            .picked_shop_object
            .and_then(|hit| {
                live_shop_hit(
                    hit,
                    shop,
                    &shop.items,
                    &shop.zodiac_items,
                    &shop.talisman_items,
                    &shop.pack_items,
                    &shop_rm,
                )
            })
            .or_else(|| {
                shop.focus.and_then(|f| f.to_hit()).and_then(|hit| {
                    live_shop_hit(
                        hit,
                        shop,
                        &shop.items,
                        &shop.zodiac_items,
                        &shop.talisman_items,
                        &shop.pack_items,
                        &shop_rm,
                    )
                })
            });
        let hover_sellable = sell_hint_hit.is_some_and(|hit| {
            focused_sell_action(
                Some(ShopFocus::from_hit(hit)),
                shop.items.len(),
                &shop.zodiac_items,
                &shop.talisman_items,
                &shop_rm,
            )
            .is_some()
        });
        let hover_buyable = sell_hint_hit.is_some_and(|hit| {
            super::shared::shop_action_for_hit(
                hit,
                &shop.items,
                &shop.zodiac_items,
                &shop.talisman_items,
                &shop_rm,
            )
            .is_some()
        });
        let show_hold_sell_hint =
            inspect.is_none() && (shop.west_sell_hold_started.is_some() || hover_sellable);
        let show_buy_hint = inspect.is_none() && hover_buyable;
        let show_inspect_hint =
            inspect.is_none() && shop.focus.is_some_and(shop_focus_inspectable);

        let hint_style = HintStyle::standard(w, h);
        let show_inspect_preview =
            inspect_active && shop.shop_inspect_preview_available(ctx.run);
        let hint_row = if inspect_active {
            inspect_camera_hint_row(ctx.input_mode, show_inspect_preview)
        } else {
            shop_storeroom_footer_row(
                ctx.input_mode,
                show_buy_hint,
                show_hold_sell_hint,
                show_inspect_hint,
            )
        };
        let icon_slots = push_screen_footer_hint(&mut frame, &ctx, hint_row, hint_style);
        if inspect_active
            && show_inspect_preview
            && let Some(InlineHintIconSlot { icon_rect, .. }) =
                icon_slots.iter().find(|s| is_confirm_hint_key(s.key))
        {
            frame.buttons.push(ButtonDef::scene(
                (icon_rect[0], icon_rect[1], icon_rect[2], icon_rect[3]),
                super::SHOP_INSPECT_PREVIEW_ID,
            ));
        }
        let hint_band_top = screen_footer_top(h, hint_style);

        if inspect.is_none() {
            let wall_count = crate::game::wall_ledger::shop_wall_hud_count(ctx.run);
            crate::render::wall_display::push_wall_remaining_hud(&mut frame, w, h, wall_count);
        }

        let push_hold_ring = |frame: &mut crate::render::draw_cmd::UiFrame,
                              icon_rect: &[f32; 4],
                              progress: f32,
                              invalid: bool| {
            let [ix, iy, icon_px, _] = *icon_rect;
            let cx = ix + icon_px * 0.5;
            let cy = iy + icon_px * 0.5;
            let r = icon_px * 0.58;
            let thickness = (icon_px * 0.12).max(3.5);
            frame.arc_ring_quads([crate::ui::prompt_hold_ring::hold_prompt_ring(
                cx, cy, r, thickness, progress, invalid,
            )]);
        };

        let now = Instant::now();
        if !inspect_active
            && (shop.west_sell_hold_started.is_some() || hover_sellable)
            && let Some(InlineHintIconSlot { icon_rect, .. }) =
                icon_slots.iter().find(|s| is_hold_sell_hint_key(s.key))
        {
            let sell_invalid =
                shop.west_sell_hold_started.is_some() && !shop.sell_hold_valid_for(&shop_rm);
            let progress = shop
                .sell_hold_progress(now, &shop_rm)
                .unwrap_or(0.0);
            push_hold_ring(&mut frame, icon_rect, progress, sell_invalid);
        }

        let show_buy_hold_ring = ctx.input_mode != InputMode::Cursor
            && (shop.confirm_buy_hold_started.is_some() || hover_buyable);
        if !inspect_active
            && show_buy_hold_ring
            && let Some(InlineHintIconSlot { icon_rect, .. }) =
                icon_slots.iter().find(|s| is_shop_buy_hint_key(s.key))
        {
            let buy_invalid = shop.confirm_buy_hold_started.is_some()
                && !shop.buy_hold_valid_for(ctx.run, &shop_rm);
            let progress = shop
                .buy_hold_progress(now, ctx.run, &shop_rm)
                .unwrap_or(0.0);
            push_hold_ring(&mut frame, icon_rect, progress, buy_invalid);
        }

        if inspect_active && let Some(ShopFocus::Relic(i)) = shop.focus {
            let n_sale = shop.items.len();
            let def_opt = if i < n_sale {
                shop.items
                    .get(i)
                    .and_then(|it| all_relic_defs().iter().find(|d| d.id == it.relic))
            } else {
                shop_rm
                    .owned_relics
                    .get(i.saturating_sub(n_sale))
                    .and_then(|rid| all_relic_defs().iter().find(|d| d.id == *rid))
            };
            if let Some(d) = def_opt
                && !d.flavor.is_empty()
            {
                let mut flavor_gradients = Vec::new();
                let mut flavor_texts = Vec::new();
                push_floating_relic_flavor_labels(
                    &mut flavor_gradients,
                    &mut flavor_texts,
                    w,
                    h,
                    d.flavor,
                    h - hint_band_top,
                );
                frame.gradient_quads(flavor_gradients);
                frame.texts(flavor_texts);
            }
        }
    }

    if shop.pause_menu.paused {
        shop.pause_menu.stash_focus_nav_debug(&mut ctx, w, h);
    } else {
        let focus_rects = shop.last_focus_rects.borrow().clone();
        ctx.stash_focus_nav_graph(
            &focus_rects,
            &[],
            shop.focus,
            shop.focus_nav.memory(),
            |f| format!("{f:?}"),
        );
    }

    frame
}

/// Price line for hover tooltip on a standard purchasable slot.
#[inline]
fn purchasable_tooltip_cta(
    price: u32,
    sold: bool,
    can_afford: bool,
    display_gold: u32,
    unaffordable_label: Option<&str>,
) -> String {
    if sold {
        "SOLD".to_string()
    } else if !can_afford {
        unaffordable_label
            .map(str::to_string)
            .unwrap_or_else(|| format!("${} (have ¥{})", price, display_gold))
    } else if price == 0 {
        "FREE".to_string()
    } else {
        format!("Buy ¥{}", price)
    }
}

fn hover_tooltip_content(
    scene: &ShopScene,
    shop: &ShopReadModel,
    mode: &GameMode,
    hit: ShopHit,
) -> Option<(String, String, String, [f32; 4])> {
    let n_for_sale_zodiacs = scene.zodiac_items.len();
    let n_for_sale_talismans = scene.talisman_items.len();
    let n_for_sale_relics = scene.items.len();
    let i_got_a_guy_charges = if shop.relic_state.has(RelicId::IGotAGuy) {
        shop.relic_counters
            .get(&RelicId::IGotAGuy)
            .copied()
            .unwrap_or(0)
    } else {
        0
    };
    let restock_cost = scene.restock_cost(mode.season);
    let restock_affordable = matches!(scene.mode, ShopMode::Standard)
        && (restock_cost == 0
            || shop.yen >= restock_cost as i32
            || i_got_a_guy_charges > 0);

    let tuple_opt = match hit {
        ShopHit::Relic(i) if i < n_for_sale_relics => {
            let item = &scene.items[i];
            let can_afford = shop.yen >= item.price as i32 && !shop.relics_full && !item.sold;
            let unaffordable = shop.relics_full.then_some("Relics full");
            let cta = purchasable_tooltip_cta(
                item.price,
                item.sold,
                can_afford,
                shop.display_yen,
                unaffordable,
            );
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
            let def = all_relic_defs().iter().find(|d| d.id == rid);
            let name = def
                .map(|d| d.name.to_string())
                .unwrap_or_else(|| "Relic".into());
            let desc = relic_description_live(
                rid,
                &shop.relic_counters,
                shop.yen,
                Some((&shop.relic_state, oi)),
                None,
                Some(shop.wing),
            );
            let sell = relic_sell_price_live(rid, &shop.relic_counters);
            Some((name, desc, format!("Sell ¥{}", sell), color::CHAMPAGNE))
        }
        ShopHit::Ribbon(i) if i < n_for_sale_zodiacs => {
            let item = &scene.zodiac_items[i];
            let price = item.price(mode, &shop.relic_state);
            let can_afford = shop.yen >= price as i32 && !item.sold;
            let cta = purchasable_tooltip_cta(price, item.sold, can_afford, shop.display_yen, None);
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
            let price = item.price(mode, &shop.relic_state);
            let can_afford = shop.yen >= price as i32 && !shop.consumables_full && !item.sold;
            let unaffordable = shop.consumables_full.then_some("Inventory full");
            let cta = purchasable_tooltip_cta(
                price,
                item.sold,
                can_afford,
                shop.display_yen,
                unaffordable,
            );
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
                format!(
                    "Sell ¥{}",
                    consumable_sell_price_for_mode(c, mode, &shop.relic_state)
                ),
                color::CHAMPAGNE,
            ))
        }
        ShopHit::Dish(id) if id == super::PICK_COIN_DISH => Some((
            "Yen".to_string(),
            "Your wealth in yen".to_string(),
            format!("¥{}", shop.yen),
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
        ShopHit::Dish(id) if id == super::PICK_RESTOCK_PROP => {
            Some(if matches!(scene.mode, ShopMode::Tutorial) {
                (
                    "Curated Stock".to_string(),
                    "Tutorial stock — restock is unavailable here.".to_string(),
                    String::new(),
                    color::CHAMPAGNE,
                )
            } else if restock_cost == 0 {
                (
                    "Restock".to_string(),
                    "Refresh the shop once at no gold cost.".to_string(),
                    "FREE".to_string(),
                    color::GOLD,
                )
            } else if i_got_a_guy_charges > 0 {
                (
                    "Restock".to_string(),
                    "Refresh the shop at no gold cost.".to_string(),
                    format!("FREE ({} left)", i_got_a_guy_charges),
                    color::GOLD,
                )
            } else {
                let cta = if shop.yen >= restock_cost as i32 {
                    format!("¥{}", restock_cost)
                } else {
                    format!("${} (have ¥{})", restock_cost, shop.display_yen)
                };
                (
                    "Restock".to_string(),
                    format!("Refresh shop for ¥{}", restock_cost),
                    cta,
                    if restock_affordable {
                        color::GOLD
                    } else {
                        color::RUBY
                    },
                )
            })
        }
        ShopHit::Dish(id) if is_tile_pack_pick(id) => {
            let idx = tile_pack_index_from_pick(id).unwrap_or(0);
            scene.pack_items.get(idx).map(|pack| {
                let price =
                    mode.scale_shop_price(crate::core::relic::apply_merchants_eye_discount(
                        pack.kind.shop_price(),
                        &shop.relic_state,
                    ));
                let can_afford = shop.yen >= price as i32 && !pack.sold;
                let cta =
                    purchasable_tooltip_cta(price, pack.sold, can_afford, shop.display_yen, None);
                let col = if pack.sold {
                    color::UMBER
                } else if can_afford {
                    color::GOLD
                } else {
                    color::RUBY
                };
                (
                    pack.kind.name().to_string(),
                    pack.kind.description().to_string(),
                    cta,
                    col,
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
                let price =
                    mode.scale_shop_price(crate::core::relic::apply_merchants_eye_discount(
                        pack.kind.shop_price(),
                        &shop.relic_state,
                    ));
                let can_afford = shop.yen >= price as i32 && !pack.sold;
                let cta =
                    purchasable_tooltip_cta(price, pack.sold, can_afford, shop.display_yen, None);
                let col = if pack.sold {
                    color::UMBER
                } else if can_afford {
                    color::GOLD
                } else {
                    color::RUBY
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

struct CursorHoverCtx<'a> {
    cx: f32,
    cy: f32,
    w: f32,
    h: f32,
    scene: &'a ShopScene,
    cam: &'a CameraParams,
    env_h: f32,
}

fn cursor_hover_rect(ctx: CursorHoverCtx<'_>) -> Option<[f32; 4]> {
    let CursorHoverCtx {
        cx,
        cy,
        w,
        h,
        scene,
        cam,
        env_h,
    } = ctx;
    let sale_slots = for_sale_slots(scene);
    for (i, slot) in sale_slots.iter().enumerate().take(SHOP_SPAWN_SLOT_COUNT) {
        if slot.is_none() {
            continue;
        }
        let r = shop_shelf_slot_rect(w, h, cam, i, env_h);
        if pt_in_rect(cx, cy, r) {
            return Some(r);
        }
    }
    [
        journal_btn_rect(w, h, cam, env_h),
        restock_btn_rect(w, h, cam, env_h),
        leave_btn_rect(w, h, cam, env_h),
    ]
    .into_iter()
    .find(|&r| pt_in_rect(cx, cy, r))
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
        let env_h = ctx.room_gltf_height_scale;
        return cursor_hover_rect(CursorHoverCtx {
            cx,
            cy,
            w,
            h,
            scene,
            cam,
            env_h,
        })
        .or_else(|| {
            ctx.picked_shop_object.and_then(|hit| {
                let resolved = live_shop_hit(
                    hit,
                    scene,
                    &scene.items,
                    &scene.zodiac_items,
                    &scene.talisman_items,
                    &scene.pack_items,
                    shop,
                )?;
                let foc = ShopFocus::from_hit(resolved);
                build_focus_rects(scene, w, h, cam, shop, ctx.run)
                    .into_iter()
                    .find(|(t, _)| *t == foc)
                    .map(|(_, r)| r)
            })
        });
    }
    scene.focus.and_then(|f| {
        build_focus_rects(scene, w, h, cam, shop, ctx.run)
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
    run: &RunState,
    w: f32,
    h: f32,
    cam: &CameraParams,
) -> Option<(f32, f32)> {
    let focus = ShopFocus::from_hit(hit);
    build_focus_rects(scene, w, h, cam, shop, run)
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
    (0..7).find(|&idx| inv_slot_glb_marker_name(scene, shop, idx).as_deref() == Some(want.as_str()))
}

/// Map raw GLB collision [`ShopHit`] variants into flat shop hits using shelf/inventory slot tables.
pub(super) fn resolve_shop_glb_env_hit(
    scene: &ShopScene,
    shop: &ShopReadModel,
    hit: ShopHit,
) -> Option<ShopHit> {
    match hit {
        ShopHit::EnvSpawnSlot(slot) => for_sale_slots(scene).get(slot).copied().flatten()?.to_hit(),
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

struct ShopScreenAnchor<'a> {
    w: f32,
    h: f32,
    cam: &'a CameraParams,
    env_h: f32,
    cx: f32,
    cy: f32,
    cz_fallback: f32,
}

struct InvMarkerSurfaceAnchor<'a> {
    screen: ShopScreenAnchor<'a>,
    scene: &'a ShopScene,
    shop: &'a ShopReadModel,
    slot_i: usize,
}

fn inv_marker_surface_anchor(p: InvMarkerSurfaceAnchor<'_>) -> [f32; 3] {
    let InvMarkerSurfaceAnchor {
        screen,
        scene,
        shop,
        slot_i,
    } = p;
    let ShopScreenAnchor {
        w,
        h,
        cam,
        env_h,
        cx,
        cy,
        cz_fallback,
    } = screen;
    inv_slot_glb_marker_name(scene, shop, slot_i)
        .and_then(|name| {
            with_shop_glb_cpu(|opt| opt.and_then(|cpu| marker_translation(cpu, &name)))
        })
        .map(|tw| surface_anchor_from_world_xyz(w, h, tw * room_env_world_scale(h, env_h)))
        .unwrap_or_else(|| lit_anchor(cam, w, h, cx, cy, cz_fallback))
}

struct SaleAnchorAtSlot<'a> {
    screen: ShopScreenAnchor<'a>,
    slot_i: usize,
}

#[inline]
fn sale_anchor_at_slot(p: SaleAnchorAtSlot<'_>) -> [f32; 3] {
    let SaleAnchorAtSlot { screen, slot_i } = p;
    let ShopScreenAnchor {
        w,
        h,
        cam,
        env_h,
        cx,
        cy,
        cz_fallback,
    } = screen;
    let scale = room_env_world_scale(h, env_h);
    if let Some(tw) = with_shop_glb_cpu(|opt| {
        opt.and_then(|cpu| marker_translation(cpu, &spawn_relic_marker_name(slot_i)))
    }) {
        let scaled = tw * scale;
        // Slot rect is centered on the niche; GLB empties can sit off-center — keep shelf Z, align XY to rect.
        return object3d_pos_for_screen_at_world_z(w, h, cam, cx, cy, scaled.z);
    }
    lit_anchor(cam, w, h, cx, cy, cz_fallback)
}

fn sale_slot_for_focus(scene: &ShopScene, foc: ShopFocus) -> Option<usize> {
    for_sale_slots(scene)
        .iter()
        .enumerate()
        .find_map(|(idx, cell)| (*cell == Some(foc)).then_some(idx))
}

#[inline]
pub(in crate::scenes::shop) fn euler_rad_add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// Extra Euler XYZ (radians) for the owned-slot mesh while hold-to-sell is charging.
fn sell_hold_wobble_euler_rad(scene: &ShopScene, slot_focus: ShopFocus) -> [f32; 3] {
    if scene.focus != Some(slot_focus) {
        return [0.0; 3];
    }
    let Some(started) = scene.west_sell_hold_started else {
        return [0.0; 3];
    };
    let progress = (Instant::now()
        .saturating_duration_since(started)
        .as_secs_f32()
        / crate::ui::prompt_hold_ring::hold_act_seconds())
        .clamp(0.0, 1.0);
    if progress <= 0.0 {
        return [0.0; 3];
    }
    let amp = progress * (8.0_f32.to_radians());
    let t = scene.age_secs;
    let pitch = (t * 19.0).sin() * amp;
    let yaw = (t * 23.0).sin() * amp * 0.55;
    let roll = (t * 15.0).cos() * amp * 0.4;
    [pitch, yaw, roll]
}

#[inline]
fn object3d_pos_for_shop_inspect_focus(
    inspect_anchor: Option<(ShopFocus, [f32; 3])>,
    foc: ShopFocus,
    w: f32,
    h: f32,
    fallback: [f32; 3],
) -> [f32; 3] {
    if let Some((ifoc, tw)) = inspect_anchor
        && ifoc == foc
    {
        return object3d_pos_triple_for_world_center(w, h, Vec3::from_array(tw));
    }
    fallback
}

#[inline]
fn partition_shop_inspect_stock_mesh(
    inspect_anchor: Option<(ShopFocus, [f32; 3])>,
    foc: ShopFocus,
    mut mesh: Object3d,
    dim: &mut Vec<Object3d>,
    subject: &mut Option<Object3d>,
) {
    if let Some((ifoc, _)) = inspect_anchor
        && ifoc == foc
    {
        mesh.anim_id = super::shared::SHOP_INSPECT_SUBJECT_ANIM_ID;
        *subject = Some(mesh);
        return;
    }
    dim.push(mesh);
}

fn push_stock_meshes(
    scene: &ShopScene,
    shop: &ShopReadModel,
    w: f32,
    h: f32,
    cam: &CameraParams,
    inspect_anchor: Option<(ShopFocus, [f32; 3])>,
) -> (Vec<Object3d>, Option<Object3d>) {
    let _g = crate::render::cpu_profiler::scope("draw_frame.shop_push_stock_meshes");
    let mut dim = Vec::new();
    let mut subject = None;
    let niche_base = w * 0.048;
    let env_h = scene.drawn_room_gltf_height_scale.get();
    let stock_bobs = |foc: ShopFocus| !inspect_anchor.map(|(ifoc, _)| ifoc == foc).unwrap_or(false);
    let enter_at = scene.restock_enter_at;
    let enter_now = std::time::Instant::now();
    let enter_scale = |slot_i: usize| {
        enter_at.map(|at| super::restock_exit::restock_enter_scale(slot_i, at, enter_now))
    };
    let apply_enter_pop = |slot_i: usize, mesh: &mut Object3d| {
        if let Some(s) = enter_scale(slot_i) {
            super::restock_exit::apply_restock_enter_scale(mesh, s);
        }
    };

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
                    let mut relic_pos = object3d_pos_for_shop_inspect_focus(
                        inspect_anchor,
                        *foc,
                        w,
                        h,
                        sale_anchor_at_slot(SaleAnchorAtSlot {
                            screen: ShopScreenAnchor {
                                w,
                                h,
                                cam,
                                env_h,
                                cx,
                                cy,
                                cz_fallback: cz,
                            },
                            slot_i,
                        }),
                    );
                    if stock_bobs(*foc) {
                        apply_shop_stock_bob(&mut relic_pos, h, slot_i as u32, scene.age_secs);
                    }
                    let mut mesh = Object3d {
                        pos: relic_pos,
                        extents: [half[0] * 2.0, half[1] * 2.0, half[2] * 2.0],
                        rotation: euler_xyz_rad_from_deg(super::SHOP_RELIC_LEAN_COUNTER, 0.0, 0.0),
                        color: col,
                        kind: Object3dKind::Relic {
                            relic_id: item.relic,
                            glow: relic_glow(scene, item.relic),
                            silhouette: false,
                            debuffed: false,
                        },
                        hover_target: 0.0,
                        anim_id: 0,
                    };
                    apply_enter_pop(slot_i, &mut mesh);
                    partition_shop_inspect_stock_mesh(
                        inspect_anchor,
                        *foc,
                        mesh,
                        &mut dim,
                        &mut subject,
                    );
                }
            }
            ShopFocus::Pack(pid) => {
                let k = (pid - super::PICK_TILE_PACK_BASE) as usize;
                let Some(pack) = scene.pack_items.get(k) else {
                    continue;
                };
                if pack.sold {
                    continue;
                }
                let pack_h = r[3] * 1.04;
                let pack_w = pack_h * PACK_ASPECT_W_OVER_H;
                let pack_t = pack_h * 0.11;
                let ext = [pack_w, pack_t, pack_h];
                let cz = wz + ext[2] * 0.5;
                let mut pack_pos = object3d_pos_for_shop_inspect_focus(
                    inspect_anchor,
                    *foc,
                    w,
                    h,
                    sale_anchor_at_slot(SaleAnchorAtSlot {
                        screen: ShopScreenAnchor {
                            w,
                            h,
                            cam,
                            env_h,
                            cx,
                            cy,
                            cz_fallback: cz,
                        },
                        slot_i,
                    }),
                );
                if stock_bobs(*foc) {
                    apply_shop_stock_bob(&mut pack_pos, h, slot_i as u32 + 16, scene.age_secs);
                }
                let mut mesh = Object3d {
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
                };
                apply_enter_pop(slot_i, &mut mesh);
                partition_shop_inspect_stock_mesh(
                    inspect_anchor,
                    *foc,
                    mesh,
                    &mut dim,
                    &mut subject,
                );
            }
            ShopFocus::Ribbon(i) => {
                let Some(item) = scene.zodiac_items.get(i) else {
                    continue;
                };
                let mut col = consumable_color(item.consumable);
                if item.sold {
                    col[3] = 0.30;
                }
                // Largest 3:1 ribbon that fits the inspect slot's
                // (0.38 × 0.62) envelope — width-bound when the slot is
                // very tall, length-bound otherwise.
                let ribbon_len =
                    for_sale_ribbon_length(r[2], r[3]) * enter_scale(slot_i).unwrap_or(1.0);
                let ribbon_world = ribbon_display_length(ribbon_len);
                let z_k = if let Consumable::Zodiac(z) = item.consumable {
                    Some(z)
                } else {
                    None
                };
                let cz = wz + ribbon_world * 0.35 - ribbon_world * 0.5;
                let mut ribbon_pos = object3d_pos_for_shop_inspect_focus(
                    inspect_anchor,
                    *foc,
                    w,
                    h,
                    sale_anchor_at_slot(SaleAnchorAtSlot {
                        screen: ShopScreenAnchor {
                            w,
                            h,
                            cam,
                            env_h,
                            cx,
                            cy,
                            cz_fallback: cz,
                        },
                        slot_i,
                    }),
                );
                if stock_bobs(*foc) {
                    apply_shop_stock_bob(&mut ribbon_pos, h, slot_i as u32 + 32, scene.age_secs);
                }
                partition_shop_inspect_stock_mesh(
                    inspect_anchor,
                    *foc,
                    zodiac_ribbon_object3d(ZodiacRibbonSpec {
                        pos: ribbon_pos,
                        length: ribbon_len,
                        rotation: euler_xyz_rad_from_deg(90.0, 0.0, 0.0),
                        color: [1.0, 1.0, 1.0, col[3]],
                        kind: z_k,
                        hover_target: 0.0,
                        anim_id: 0,
                        placement_rot_deg: [0.0, 0.0, 0.0],
                    }),
                    &mut dim,
                    &mut subject,
                );
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
                    // Slightly smaller than ribbons/packs in the same row so
                    // adjacent buff tablets (e.g. Pearl + Gilded) don't read as
                    // one mass under the shared for-sale talisman tilt.
                    let tw = r[2] * FOR_SALE_TALISMAN_W_FRAC;
                    let cz = wz + tw * 0.55;
                    let mut tal_pos = object3d_pos_for_shop_inspect_focus(
                        inspect_anchor,
                        *foc,
                        w,
                        h,
                        sale_anchor_at_slot(SaleAnchorAtSlot {
                            screen: ShopScreenAnchor {
                                w,
                                h,
                                cam,
                                env_h,
                                cx,
                                cy,
                                cz_fallback: cz,
                            },
                            slot_i,
                        }),
                    );
                    if stock_bobs(*foc) {
                        apply_shop_stock_bob(&mut tal_pos, h, slot_i as u32 + 48, scene.age_secs);
                    }
                    let mut mesh = Object3d {
                        pos: tal_pos,
                        extents: crate::render::talisman_mesh::talisman_object_extents(
                            for_sale_talisman_tablet_extent(r[2]),
                        ),
                        rotation: crate::render::talisman_mesh::talisman_face_camera_rotation(0.0),
                        color: col,
                        kind: Object3dKind::Talisman { kind: tk },
                        hover_target: 0.0,
                        anim_id: 0,
                    };
                    apply_enter_pop(slot_i, &mut mesh);
                    partition_shop_inspect_stock_mesh(
                        inspect_anchor,
                        *foc,
                        mesh,
                        &mut dim,
                        &mut subject,
                    );
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
                let mut relic_pos = object3d_pos_for_shop_inspect_focus(
                    inspect_anchor,
                    *foc,
                    w,
                    h,
                    inv_marker_surface_anchor(InvMarkerSurfaceAnchor {
                        screen: ShopScreenAnchor {
                            w,
                            h,
                            cam,
                            env_h,
                            cx,
                            cy,
                            cz_fallback: cz,
                        },
                        scene,
                        shop,
                        slot_i,
                    }),
                );
                if stock_bobs(*foc) {
                    apply_shop_stock_bob(&mut relic_pos, h, slot_i as u32 + 64, scene.age_secs);
                }
                let base_rot = euler_xyz_rad_from_deg(super::SHOP_RELIC_LEAN_INVENTORY, 0.0, 0.0);
                partition_shop_inspect_stock_mesh(
                    inspect_anchor,
                    *foc,
                    Object3d {
                        pos: relic_pos,
                        extents: [half[0] * 2.0, half[1] * 2.0, half[2] * 2.0],
                        rotation: euler_rad_add(base_rot, sell_hold_wobble_euler_rad(scene, *foc)),
                        color: rarity_color(rarity),
                        kind: Object3dKind::Relic {
                            relic_id: rid,
                            glow,
                            silhouette: false,
                            debuffed: false,
                        },
                        hover_target: 0.0,
                        anim_id: 0,
                    },
                    &mut dim,
                    &mut subject,
                );
            }
            ShopFocus::Ribbon(i) => {
                let zodiac_for_sale = scene.zodiac_items.len();
                let oi = i.saturating_sub(zodiac_for_sale);
                let Some(owned) = shop.owned_zodiacs.get(oi) else {
                    continue;
                };
                if let Consumable::Zodiac(z) = owned.consumable {
                    // Largest 3:1 ribbon that fits the inventory slot's
                    // (0.36 × 0.58) envelope.
                    let ribbon_len = owned_ribbon_length(r[2], r[3]);
                    let ribbon_world = ribbon_display_length(ribbon_len);
                    let cz = wz + ribbon_world * 0.32 - ribbon_world * 0.5;
                    let mut pos = object3d_pos_for_shop_inspect_focus(
                        inspect_anchor,
                        *foc,
                        w,
                        h,
                        inv_marker_surface_anchor(InvMarkerSurfaceAnchor {
                            screen: ShopScreenAnchor {
                                w,
                                h,
                                cam,
                                env_h,
                                cx,
                                cy,
                                cz_fallback: cz,
                            },
                            scene,
                            shop,
                            slot_i,
                        }),
                    );
                    if stock_bobs(*foc) {
                        apply_shop_stock_bob(&mut pos, h, slot_i as u32 + 80, scene.age_secs);
                    }
                    let base_rot = euler_xyz_rad_from_deg(90.0, 0.0, 0.0);
                    partition_shop_inspect_stock_mesh(
                        inspect_anchor,
                        *foc,
                        zodiac_ribbon_object3d(ZodiacRibbonSpec {
                            pos,
                            length: ribbon_len,
                            rotation: euler_rad_add(
                                base_rot,
                                sell_hold_wobble_euler_rad(scene, *foc),
                            ),
                            color: [1.0, 1.0, 1.0, 1.0],
                            kind: Some(z),
                            hover_target: 0.0,
                            anim_id: 0,
                            placement_rot_deg: [0.0, 0.0, 0.0],
                        }),
                        &mut dim,
                        &mut subject,
                    );
                }
            }
            ShopFocus::Talisman(i) => {
                let talisman_for_sale = scene.talisman_items.len();
                let oi = i.saturating_sub(talisman_for_sale);
                let Some(owned) = shop.owned_talismans.get(oi) else {
                    continue;
                };
                let tw = r[2] * OWNED_TALISMAN_W_FRAC;
                let cz = wz + tw * 0.45;
                let mut pos = object3d_pos_for_shop_inspect_focus(
                    inspect_anchor,
                    *foc,
                    w,
                    h,
                    inv_marker_surface_anchor(InvMarkerSurfaceAnchor {
                        screen: ShopScreenAnchor {
                            w,
                            h,
                            cam,
                            env_h,
                            cx,
                            cy,
                            cz_fallback: cz,
                        },
                        scene,
                        shop,
                        slot_i,
                    }),
                );
                if stock_bobs(*foc) {
                    apply_shop_stock_bob(&mut pos, h, slot_i as u32 + 96, scene.age_secs);
                }
                let base_rot = euler_xyz_rad_from_deg(90.0, 0.0, 0.0);
                let (kind, color) = match owned.consumable {
                    Consumable::Talisman(tk) => (
                        Object3dKind::Talisman { kind: tk },
                        consumable_color(owned.consumable),
                    ),
                    Consumable::Memorial(mk) => (
                        Object3dKind::MemorialTalisman { kind: mk },
                        consumable_color(owned.consumable),
                    ),
                    _ => continue,
                };
                partition_shop_inspect_stock_mesh(
                    inspect_anchor,
                    *foc,
                    Object3d {
                        pos,
                        extents: crate::render::talisman_mesh::talisman_object_extents(
                            owned_talisman_tablet_extent(r[2]),
                        ),
                        rotation: euler_rad_add(base_rot, sell_hold_wobble_euler_rad(scene, *foc)),
                        color,
                        kind,
                        hover_target: 0.0,
                        anim_id: 0,
                    },
                    &mut dim,
                    &mut subject,
                );
            }
            _ => {}
        }
    }

    push_departing_stock_meshes(scene, w, h, cam, env_h, niche_base, &mut dim);

    (dim, subject)
}

fn push_departing_stock_meshes(
    scene: &ShopScene,
    w: f32,
    h: f32,
    cam: &CameraParams,
    env_h: f32,
    niche_base: f32,
    dim: &mut Vec<Object3d>,
) {
    if scene.departing_stock.is_empty() {
        return;
    }
    let now = std::time::Instant::now();
    for batch in &scene.departing_stock {
        for entry in &batch.entries {
            let slot_i = entry.slot_i();
            if super::restock_exit::restock_exit_offscreen_for_slot(
                slot_i,
                h,
                batch.started_at,
                now,
            ) {
                continue;
            }
            let r = shop_shelf_slot_rect(w, h, cam, slot_i, env_h);
            let cx = r[0] + r[2] * 0.5;
            let cy = r[1] + r[3] * 0.5;
            let wz = shop_shelf_slot_wz(h, slot_i);
            let anchor = |cz: f32| {
                sale_anchor_at_slot(SaleAnchorAtSlot {
                    screen: ShopScreenAnchor {
                        w,
                        h,
                        cam,
                        env_h,
                        cx,
                        cy,
                        cz_fallback: cz,
                    },
                    slot_i,
                })
            };
            let mut mesh = match entry {
                super::restock_exit::DepartingShelfEntry::Relic { relic, rarity, .. } => {
                    let half = relic_half_extents(*relic, niche_base);
                    let cz = wz + half[2];
                    Object3d {
                        pos: anchor(cz),
                        extents: [half[0] * 2.0, half[1] * 2.0, half[2] * 2.0],
                        rotation: euler_xyz_rad_from_deg(super::SHOP_RELIC_LEAN_COUNTER, 0.0, 0.0),
                        color: rarity_color(*rarity),
                        kind: Object3dKind::Relic {
                            relic_id: *relic,
                            glow: 0.0,
                            silhouette: false,
                            debuffed: false,
                        },
                        hover_target: 0.0,
                        anim_id: 0,
                    }
                }
                super::restock_exit::DepartingShelfEntry::Pack { kind, .. } => {
                    let pack_h = r[3] * 1.04;
                    let pack_w = pack_h * PACK_ASPECT_W_OVER_H;
                    let pack_t = pack_h * 0.11;
                    let ext = [pack_w, pack_t, pack_h];
                    let cz = wz + ext[2] * 0.5;
                    Object3d {
                        pos: anchor(cz),
                        extents: ext,
                        rotation: [0.0, 0.0, 0.0],
                        color: kind.foil_tint(),
                        kind: Object3dKind::Pack {
                            kind: *kind,
                            pick_id: None,
                        },
                        hover_target: 0.0,
                        anim_id: 0,
                    }
                }
                super::restock_exit::DepartingShelfEntry::Ribbon { zodiac, .. } => {
                    let ribbon_len = for_sale_ribbon_length(r[2], r[3]);
                    let ribbon_world = ribbon_display_length(ribbon_len);
                    let cz = wz + ribbon_world * 0.35 - ribbon_world * 0.5;
                    let mut col = consumable_color(Consumable::Zodiac(*zodiac));
                    col[3] = 1.0;
                    zodiac_ribbon_object3d(ZodiacRibbonSpec {
                        pos: anchor(cz),
                        length: ribbon_len,
                        rotation: euler_xyz_rad_from_deg(90.0, 0.0, 0.0),
                        color: [1.0, 1.0, 1.0, col[3]],
                        kind: Some(*zodiac),
                        hover_target: 0.0,
                        anim_id: 0,
                        placement_rot_deg: [0.0, 0.0, 0.0],
                    })
                }
                super::restock_exit::DepartingShelfEntry::Talisman { kind, .. } => {
                    let tw = r[2] * FOR_SALE_TALISMAN_W_FRAC;
                    let cz = wz + tw * 0.55;
                    Object3d {
                        pos: anchor(cz),
                        extents: crate::render::talisman_mesh::talisman_object_extents(
                            for_sale_talisman_tablet_extent(r[2]),
                        ),
                        rotation: crate::render::talisman_mesh::talisman_face_camera_rotation(0.0),
                        color: consumable_color(Consumable::Talisman(*kind)),
                        kind: Object3dKind::Talisman { kind: *kind },
                        hover_target: 0.0,
                        anim_id: 0,
                    }
                }
            };
            let delta =
                super::restock_exit::restock_exit_mesh_delta(slot_i, h, batch.started_at, now);
            super::restock_exit::apply_restock_exit_to_mesh(&mut mesh, delta);
            dim.push(mesh);
        }
    }
}

fn rect_center_n(window_w: f32, window_h: f32, nx: f32, ny: f32, rw: f32, rh: f32) -> [f32; 4] {
    let cx = nx * window_w;
    let cy = ny * window_h;
    [cx - rw * 0.5, cy - rh * 0.5, rw, rh]
}

/// Fallback rects when shop.glb markers are missing — one horizontal row of nine spawn empties.
fn shop_shelf_slot_rect(w: f32, h: f32, cam: &CameraParams, index: usize, env_h: f32) -> [f32; 4] {
    let rw = w * 0.065;
    let rh = h * 0.125;
    // For-sale focus/click rects should hug stock (tile/relic/ribbon/talisman),
    // not the pedestal mesh under it. Use marker translation + fixed extents
    // here instead of marker mesh bounds, which can include the full plinth.
    if let Some((cx, cy)) = with_shop_glb_cpu(|opt| {
        let cpu = opt?;
        let tw = marker_translation(cpu, &spawn_relic_marker_name(index))?
            * room_env_world_scale(h, env_h);
        Some(cam.project_world_to_screen(w, h, tw))
    }) {
        return [cx - rw * 0.5, cy - rh * 0.5, rw, rh];
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
    if let Some(name) = inv_slot_glb_marker_name(scene, shop, index)
        && let Some(r) = marker_screen_rect(w, h, cam, &name, rw, rh, env_h)
    {
        return r;
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

fn restock_btn_rect(w: f32, h: f32, cam: &CameraParams, env_h: f32) -> [f32; 4] {
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

/// Maps compact index `k` (0..`n`) into a centered slot along the 1×9 spawn row (`n` items total).
fn sale_index_to_centered_slot(k: usize, n: usize) -> usize {
    debug_assert!(n > 0 && k < n && n <= SHOP_SPAWN_SLOT_COUNT);
    let start = (SHOP_SPAWN_SLOT_COUNT - n) / 2;
    start + k
}

/// Maps stock into [`SHOP_SPAWN_SLOT_COUNT`] cells (`shop_spawn_relic_00` … `_08`).
/// Order: relics, then packs, then talismans, then ribbons; the group is centered in the row
/// when fewer than nine items.
pub(in crate::scenes::shop) fn for_sale_slots(
    scene: &ShopScene,
) -> [Option<ShopFocus>; SHOP_SPAWN_SLOT_COUNT] {
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
    for (k, &focus) in ordered.iter().enumerate().take(n) {
        let slot = sale_index_to_centered_slot(k, n);
        out[slot] = Some(focus);
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

#[inline]
pub(in crate::scenes::shop) fn shop_has_purchasable_stock(scene: &ShopScene) -> bool {
    !scene.items.is_empty()
        || !scene.zodiac_items.is_empty()
        || !scene.talisman_items.is_empty()
        || scene.pack_items.iter().any(|p| !p.sold)
}

#[inline]
pub(in crate::scenes::shop) fn is_purchasable_shop_focus(f: ShopFocus, scene: &ShopScene) -> bool {
    match f {
        ShopFocus::Relic(i) => i < scene.items.len(),
        ShopFocus::Ribbon(i) => i < scene.zodiac_items.len(),
        ShopFocus::Talisman(i) => i < scene.talisman_items.len(),
        ShopFocus::Pack(id) => tile_pack_index_from_pick(id)
            .is_some_and(|idx| scene.pack_items.get(idx).is_some_and(|p| !p.sold)),
        _ => false,
    }
}

/// Default focus for a fresh shelf: first for-sale relic, else first unsold pack,
/// talisman, then ribbon (same order as [`for_sale_slots`]). When nothing is for sale,
/// the leave bell so the player can still advance immediately.
pub(in crate::scenes::shop) fn default_shop_focus_for_stock(
    items: &[ShopItem],
    zodiac_items: &[ConsumableShopItem],
    talisman_items: &[ConsumableShopItem],
    pack_items: &[TilePackShopItem],
) -> ShopFocus {
    if !items.is_empty() {
        return ShopFocus::Relic(0);
    }
    for (k, p) in pack_items.iter().enumerate() {
        if !p.sold {
            return ShopFocus::Pack(super::PICK_TILE_PACK_BASE + k as u32);
        }
    }
    for (i, t) in talisman_items.iter().enumerate() {
        if !t.sold {
            return ShopFocus::Talisman(i);
        }
    }
    for (i, z) in zodiac_items.iter().enumerate() {
        if !z.sold {
            return ShopFocus::Ribbon(i);
        }
    }
    ShopFocus::NextRound
}

fn closest_purchasable_focus(
    from: (f32, f32),
    rects: &[(ShopFocus, [f32; 4])],
    scene: &ShopScene,
) -> Option<ShopFocus> {
    let mut best: Option<(ShopFocus, f32)> = None;
    for &(f, r) in rects {
        if !is_purchasable_shop_focus(f, scene) {
            continue;
        }
        let (cx, cy) = rect_center(r);
        let d = (cx - from.0).hypot(cy - from.1);
        if best.as_ref().map(|(_, bd)| d < *bd).unwrap_or(true) {
            best = Some((f, d));
        }
    }
    best.map(|(f, _)| f)
}

fn first_purchasable_focus(
    rects: &[(ShopFocus, [f32; 4])],
    scene: &ShopScene,
) -> Option<ShopFocus> {
    rects
        .iter()
        .find_map(|(f, _)| is_purchasable_shop_focus(*f, scene).then_some(*f))
}

/// After a purchase or sell changes shop/run inventory, move focus to the
/// nearest remaining purchasable shelf item (screen-space). Falls back to
/// [`ShopFocus::NextRound`] only when nothing is left to buy.
pub(in crate::scenes::shop) fn snap_focus_after_shop_purchase(
    scene: &mut ShopScene,
    prev_focus: Option<ShopFocus>,
    w: f32,
    h: f32,
    run: &RunState,
) {
    if !shop_has_purchasable_stock(scene) {
        scene.focus = Some(ShopFocus::NextRound);
        scene.stash_focus_rects(w, h, run);
        return;
    }

    let shop = GameEngine::read_shop(run);
    let env_h = scene.drawn_room_gltf_height_scale.get();
    let cam = shop_camera_params(w, h, env_h);
    let rects = build_focus_rects(scene, w, h, &cam, &shop, run);

    let from_center = prev_focus.and_then(|pf| {
        scene
            .last_focus_rects
            .borrow()
            .iter()
            .find_map(|(t, r)| (*t == pf).then_some(rect_center(*r)))
    });

    scene.focus = from_center
        .and_then(|from| closest_purchasable_focus(from, &rects, scene))
        .or_else(|| first_purchasable_focus(&rects, scene));

    if scene.focus.is_none() {
        scene.focus = Some(ShopFocus::NextRound);
    }
    scene.stash_focus_rects(w, h, run);
}

#[cfg_attr(not(test), allow(dead_code))]
pub(in crate::scenes::shop) fn projected_shop_focus_rects(
    scene: &ShopScene,
    w: f32,
    h: f32,
    run: &RunState,
) -> Vec<(ShopFocus, [f32; 4])> {
    let shop = GameEngine::read_shop(run);
    let env_h = scene.drawn_room_gltf_height_scale.get();
    let cam = shop_camera_params(w, h, env_h);
    build_focus_rects(scene, w, h, &cam, &shop, run)
}

fn build_focus_rects(
    scene: &ShopScene,
    w: f32,
    h: f32,
    cam: &CameraParams,
    shop: &ShopReadModel,
    run: &RunState,
) -> Vec<(ShopFocus, [f32; 4])> {
    let env_h = scene.drawn_room_gltf_height_scale.get();
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
    v.push((ShopFocus::Restock, restock_btn_rect(w, h, cam, env_h)));
    v.push((ShopFocus::NextRound, leave_btn_rect(w, h, cam, env_h)));
    let wall_count = crate::game::wall_ledger::shop_wall_hud_count(run);
    let wall = crate::render::wall_display::wall_hud_layout(w, h, wall_count);
    v.push((ShopFocus::WallHud, wall.block_rect));
    v
}

fn map_shop_ui_click_to_hit(cid: u32, scene: &ShopScene, _shop: &ShopReadModel) -> Option<ShopHit> {
    match cid {
        SHOP_CLICK_JOURNAL => Some(ShopHit::Dish(super::PICK_JOURNAL_BOOK)),
        SHOP_CLICK_RESTOCK => Some(ShopHit::Dish(super::PICK_RESTOCK_PROP)),
        SHOP_CLICK_LEAVE => Some(ShopHit::Dish(super::PICK_LEAVE_PROP)),
        _ => {
            if (SHOP_SHELF_CLICK_BASE..SHOP_SHELF_CLICK_BASE + SHOP_SPAWN_SLOT_COUNT as u32)
                .contains(&cid)
            {
                let idx = (cid - SHOP_SHELF_CLICK_BASE) as usize;
                return for_sale_slots(scene)[idx].and_then(|f| f.to_hit());
            }
            None
        }
    }
}
