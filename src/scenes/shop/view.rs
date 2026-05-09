//! Shop view (`THEME.md` storeroom): `Shop.glb` room, screen-space hit slots, and shelf props.
//! Stock and input dispatch live on [`super::ShopScene`].

use std::borrow::Cow;
use std::time::Instant;

use glam::{Mat4, Vec3};

use crate::core::consumable::Consumable;
use crate::core::relic::{
    Rarity, RelicId, all_relic_defs, relic_description_live, relic_sell_price_live,
};
use crate::core::tile_pack::PACK_ASPECT_W_OVER_H;
use crate::game::engine::{GameEngine, ShopReadModel, consumable_sell_price_for_mode};
use crate::game::game_mode::GameMode;
use crate::game::run::RunState;
use crate::render::decal::{load_ui_font, measure_label_advances};
use crate::render::draw_cmd::{
    CameraParams, Object3d, Object3dKind, PromptIconQuad, ScenePunctualLight, UiFrame,
    camera_facing_euler_xyz_rad,
};
use crate::render::shop_glb::{
    ShopEnvLightingTune, marker_translation, player_consumable_marker_name,
    player_gold_dish_marker_translation, player_relic_marker_name,
    screen_rect_for_marker_mesh_bounds, shop_camera_fit_fovy_for_corners,
    shop_camera_from_glb_if_present, shop_env_world_scale, shop_world_bounds_corners_centered,
    spawn_relic_marker_name, with_shop_glb_cpu,
};
use crate::render::table_transform::euler_xyz_rad_from_deg;
use crate::render::table_transform::mat4_to_euler_xyz_rad;
use crate::render::theme::{color, metrics, typography};
use crate::render::wgpu_renderer::GpuInstance;
use crate::render::wgpu_renderer::{
    MAX_POINT_LIGHTS, MAX_SPOT_LIGHTS, PointLight, ShopHit, SpotLight, TextAlign, TextLabel,
};
use crate::render::world_space::{
    object3d_pos_for_screen_at_world_z, object3d_pos_triple_for_world_center,
    surface_anchor_from_world_xyz,
};
use crate::scenes::journal_transition;
use crate::scenes::object3d_inspect::{
    InspectFrameEnv, InspectRig, ItemInspectOrbitState, apply_inspect_view_to_frame,
    inspect_orbit_camera,
};
use crate::scenes::options::OptionsScene;
use crate::scenes::{ButtonDef, DrawCtx, SceneBehavior, SceneTransition, UpdateCtx, YakuJournalScene};
use crate::ui::button_prompts::{ButtonPrompt, PromptInputSurface, SHOP_LEGEND_VERB_LABELS};
use crate::ui::focus_nav::{clamp_rect_to_viewport, push_focus_ring, rect_center};
use crate::ui::input::InputMode;
use crate::ui::inspect_plaque::push_focus_tooltip_panel_2d;
use crate::ui::kenney_prompt_paths::shop_prompt_icon_paths;

use super::layout::{
    ShopInventoryCounts, ShopLayout, consumable_color, is_tile_pack_pick, live_shop_hit,
    rarity_color, relic_half_extents, tile_pack_index_from_pick,
};
use super::shared::shop_focus_inspectable;
use super::{
    BUG_COUNT, BUG_PARAMS, ConsumableShopItem, ShopFocus, ShopMode, ShopScene, push_free_badge,
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
    with_shop_glb_cpu(|opt| {
        opt.is_some_and(|cpu| {
            !cpu.embedded_point_lights.is_empty() || !cpu.embedded_spot_lights.is_empty()
        })
    })
}

/// Point lights from glTF only (`KHR_lights_punctual`). Empty when the GLB has spots but no points.
#[inline]
fn gltf_punctual_linear_rgb(
    raw: [f32; 3],
    is_candle: bool,
    tune: &ShopEnvLightingTune,
) -> [f32; 3] {
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
    with_shop_glb_cpu(|opt| {
        let Some(cpu) = opt else {
            return Vec::new();
        };
        if cpu.embedded_point_lights.is_empty() {
            return Vec::new();
        }
        let s = shop_env_world_scale(h, env_h);
        let center_doc = cpu
            .environment_bounds_doc
            .map(|b| b.center())
            .unwrap_or(glam::Vec3::ZERO);
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
                let world = (l.pos_doc - center_doc) * s;
                let radius = crate::render::shop_glb::glb_punctual_range_world_upload(h, s, l.range_doc);
                PointLight {
                    pos: surface_anchor_from_world_xyz(w, h, world),
                    radius,
                    color: gltf_punctual_linear_rgb(l.color_linear, l.is_candle, tune),
                    intensity: (l.intensity * tune.gltf_light_intensity_scale).max(0.0),
                }
            })
            .collect()
    })
}

fn spot_lights_from_glb(w: f32, h: f32, env_h: f32, tune: &ShopEnvLightingTune) -> Vec<SpotLight> {
    if !shop_glb_has_embedded_lights() {
        return Vec::new();
    }
    with_shop_glb_cpu(|opt| {
        let Some(cpu) = opt else {
            return Vec::new();
        };
        if cpu.embedded_spot_lights.is_empty() {
            return Vec::new();
        }
        let s = shop_env_world_scale(h, env_h);
        let center_doc = cpu
            .environment_bounds_doc
            .map(|b| b.center())
            .unwrap_or(glam::Vec3::ZERO);
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
                let world = (l.pos_doc - center_doc) * s;
                let radius = crate::render::shop_glb::glb_punctual_range_world_upload(h, s, l.range_doc);
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
    })
}

pub(super) fn shop_camera_base(w: f32, h: f32, env_h: f32) -> CameraParams {
    let from_glb = shop_camera_from_glb_if_present(h, env_h);
    let cam = from_glb.unwrap_or_else(|| {
        // ref_h: 1080 — fallback when Shop.glb has no usable perspective camera (room centered at origin).
        let cs = h / 1080_f32;
        CameraParams {
            eye: [0.0 * cs, -1517.6 * cs, 1557.2 * cs],
            target: [0.0, 0.0, 0.0],
            up: [0.0, 0.0, 1.0],
            fovy_deg: 58.0,
        }
    });
    // Auto-fit widens vertical FOV past the authored value; keep GLB eye / target / up / fovy intact.
    if from_glb.is_none() {
        return with_shop_glb_cpu(|opt| {
            if let Some(cpu) = opt {
                let corners = shop_world_bounds_corners_centered(h, env_h, cpu);
                shop_camera_fit_fovy_for_corners(w, h, cam, &corners, 0.94)
            } else {
                cam
            }
        });
    }
    cam
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
                sale_anchor_at_slot(w, h, cam, slot_i, env_h, cx, cy, cz)
            }
            ShopFocus::Pack(pid) => {
                let k = (pid - super::PICK_TILE_PACK_BASE) as usize;
                let pack = scene.pack_items.get(k)?;
                if pack.sold {
                    return None;
                }
                let pack_h = r[3] * 0.52;
                let pack_w = pack_h * PACK_ASPECT_W_OVER_H;
                let pack_t = pack_h * 0.11;
                let ext = [pack_w, pack_t, pack_h];
                let cz = wz + ext[2] * 0.5;
                sale_anchor_at_slot(w, h, cam, slot_i, env_h, cx, cy, cz)
            }
            ShopFocus::Ribbon(i) => {
                scene.zodiac_items.get(i)?;
                let ribbon_len = r[3] * 0.62;
                let cz = wz + ribbon_len * 0.35;
                sale_anchor_at_slot(w, h, cam, slot_i, env_h, cx, cy, cz)
            }
            ShopFocus::Talisman(i) => {
                let item = scene.talisman_items.get(i)?;
                let Consumable::Talisman(_) = item.consumable else {
                    return None;
                };
                let tw = r[2] * 0.42;
                let cz = wz + tw * 0.55;
                sale_anchor_at_slot(w, h, cam, slot_i, env_h, cx, cy, cz)
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
                inv_marker_surface_anchor(w, h, cam, scene, shop, slot_i, env_h, cx, cy, cz)
            }
            ShopFocus::Ribbon(i) => {
                let zodiac_for_sale = scene.zodiac_items.len();
                let oi = i.saturating_sub(zodiac_for_sale);
                shop.owned_zodiacs.get(oi)?;
                let ribbon_len = r[3] * 0.58;
                let cz = wz + ribbon_len * 0.32;
                inv_marker_surface_anchor(w, h, cam, scene, shop, slot_i, env_h, cx, cy, cz)
            }
            ShopFocus::Talisman(i) => {
                let talisman_for_sale = scene.talisman_items.len();
                let oi = i.saturating_sub(talisman_for_sale);
                let owned = shop.owned_talismans.get(oi)?;
                let Consumable::Talisman(_) = owned.consumable else {
                    return None;
                };
                let tw = r[2] * 0.38;
                let cz = wz + tw * 0.45;
                inv_marker_surface_anchor(w, h, cam, scene, shop, slot_i, env_h, cx, cy, cz)
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

/// World-space inspect target for orbit camera (matches shelf mesh anchor under `base` cam).
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

/// Keep orbit pivot aligned with camera-dependent shelf anchors (see [`shop_inspect_pivot_world`]).
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
    let env_h = scene.drawn_env_height_scale.get();
    let rig = InspectRig::shop(h, env_h);
    for _ in 0..4 {
        let cam_try = inspect_orbit_camera(orbit, &rig);
        let Some(pivot) = shop_inspect_pivot_world(scene, &shop_rm, w, h, &cam_try, env_h, foc)
        else {
            break;
        };
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
        if let Some(r) = screen_rect_for_marker_mesh_bounds(
            win_w,
            win_h,
            cam,
            env_height_scale,
            cpu,
            node_name,
            rw,
            rh,
        ) {
            return Some(r);
        }
        let tw =
            marker_translation(cpu, node_name)? * shop_env_world_scale(win_h, env_height_scale);
        let (cx, cy) = cam.project_world_to_screen(win_w, win_h, tw);
        Some([cx - rw * 0.5, cy - rh * 0.5, rw, rh])
    })
}

/// [`Object3d::pos`] for the gameplay-style coin pile: [`crate::render::shop_glb::PLAYER_GOLD_DISH_MARKER`]
/// (or legacy `PlayerGoldDish`) when the room loads, otherwise [`ShopLayout::coin_dish_center_px`]
/// with a perspective-correct lift.
fn player_gold_dish_object3d_anchor(
    w: f32,
    h: f32,
    cam: &CameraParams,
    env_h: f32,
    layout: &ShopLayout,
) -> [f32; 3] {
    let scale = shop_env_world_scale(h, env_h);
    if let Some(tw) =
        with_shop_glb_cpu(|opt| opt.and_then(|cpu| player_gold_dish_marker_translation(cpu)))
    {
        let world = tw * scale;
        return surface_anchor_from_world_xyz(w, h, world);
    }
    let cx = layout.coin_dish_center_px.0;
    let cy = layout.coin_dish_center_px.1;
    let lift = layout.mm(10.0);
    object3d_pos_for_screen_at_world_z(w, h, cam, cx, cy, lift)
}

/// Settled metal coin cylinders matching gameplay (`animation_state` pile), without a procedural
/// dish mesh — the GLB supplies the tray.
fn shop_gameplay_style_gold_pile(
    layout: &ShopLayout,
    gold: u32,
    anchor: [f32; 3],
) -> Vec<Object3d> {
    if gold == 0 {
        return Vec::new();
    }
    let coin_count = (gold as usize).min(48);
    let coin_radius = layout.mm(11.3);
    let coin_thickness = layout.mm(3.5).max(2.0);
    let scatter_half = coin_radius * 3.0;
    let pile_cx = anchor[0];
    let pile_cy = anchor[1];
    // Nudge above the marker / dish interior so coins don’t sink into or z-fight the GLB mesh.
    let dish_floor_z = anchor[2] + layout.mm(3.0);
    let overlap_r = coin_radius * 2.0;
    let overlap_r2 = overlap_r * overlap_r;
    const CANDIDATES_PER_COIN: u32 = 12;

    use rand::rngs::StdRng;
    use rand::{RngExt, SeedableRng};

    let mut rng = StdRng::seed_from_u64(0x5EED_E0D1_D151_0001);
    let mut coins: Vec<Object3d> = Vec::with_capacity(coin_count);
    let mut placed: Vec<(f32, f32, f32)> = Vec::with_capacity(coin_count);
    for _ in 0..coin_count {
        let mut best: Option<(f32, f32, f32, f32)> = None;
        for _ in 0..CANDIDATES_PER_COIN {
            let lx = rng.random_range(-scatter_half..scatter_half);
            let lz = rng.random_range(-scatter_half..scatter_half);
            let rot_y = rng.random_range(-std::f32::consts::PI..std::f32::consts::PI);
            let mut support_y = dish_floor_z;
            for (ox, oz, top_y) in &placed {
                let ddx = lx - ox;
                let ddz = lz - oz;
                if ddx * ddx + ddz * ddz < overlap_r2 && *top_y > support_y {
                    support_y = *top_y;
                }
            }
            match best {
                None => best = Some((lx, lz, support_y, rot_y)),
                Some((_, _, by, _)) if support_y < by => {
                    best = Some((lx, lz, support_y, rot_y));
                }
                _ => {}
            }
        }
        let (lx, lz, support_y, rot_y) = best.unwrap();
        let world_y = support_y + coin_thickness * 0.5;
        placed.push((lx, lz, world_y + coin_thickness * 0.5));
        coins.push(Object3d {
            pos: [pile_cx + lx, pile_cy + lz, world_y],
            extents: [coin_radius * 2.0, coin_thickness, coin_radius * 2.0],
            rotation: [0.0, rot_y, 0.0],
            color: [1.00, 0.78, 0.30, 1.0],
            kind: Object3dKind::Primitive {
                shape: crate::render::primitive::MeshId::Cylinder,
                material: crate::render::primitive::MaterialSpec::metal(),
                pick_id: None,
                shadow_caster: true,
                silhouette: false,
            },
            hover_target: 0.0,
            anim_id: 0,
            arrange_name: Some("shop.shelf.coin_dish"),
        });
    }
    coins
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
        render_shop_frame(self, ctx)
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
        let cam = shop_camera_params(w, h, env_h);
        *self.last_focus_rects.borrow_mut() = build_focus_rects(self, w, h, &cam, &shop);
    }
}

/// Pivot sync for shop item orbit (storeroom face + isolated showcase inspect).
fn resolve_shop_orbit_target_for_draw(
    shop: &ShopScene,
    ins: &ItemInspectOrbitState,
    shop_rm: &ShopReadModel,
    w: f32,
    h: f32,
    env_h: f32,
) -> ItemInspectOrbitState {
    let inspect_rig = InspectRig::shop(h, env_h);
    if let Some(foc) = shop.focus {
        let mut o = *ins;
        // One frame may draw before inspect `update` runs (push ordering).
        for _ in 0..2 {
            let cam_try = inspect_orbit_camera(&o, &inspect_rig);
            if let Some(pivot) = shop_inspect_pivot_world(shop, shop_rm, w, h, &cam_try, env_h, foc)
            {
                o.target_world = pivot.to_array();
            } else {
                break;
            }
        }
        o
    } else {
        *ins
    }
}

fn push_shop_inspect_overlay_chrome(frame: &mut UiFrame, ctx: &DrawCtx<'_>, w: f32, h: f32) {
    let hint_font = typography::size(typography::CAPTION, h, ctx.ui_scale);
    let hint_h = (hint_font / 0.55).ceil();
    let hint_y = h - hint_h - h * 0.02;
    let surface = match ctx.input_mode {
        InputMode::Controller => PromptInputSurface::Controller,
        InputMode::Keyboard | InputMode::Cursor => PromptInputSurface::MouseOrKeyboard,
    };
    let orbit = ButtonPrompt::shop_inspect_mode_hint(surface, ctx.gamepad_style);
    let line = match surface {
        PromptInputSurface::Controller => format!("B · Esc: close   ·   {orbit}"),
        PromptInputSurface::MouseOrKeyboard => {
            format!("Esc · Backspace · E: close   ·   {orbit}")
        }
    };
    frame.texts(vec![TextLabel {
        rect: [w * 0.05, hint_y, w * 0.9, hint_h.max(20.0)],
        text: line,
        color: [0.70, 0.72, 0.82, 0.92],
        font_px: Some((hint_font * 0.95).max(12.0)),
        align: TextAlign::Center,
        no_glossary: false,
        scroll_offset: 0.0,
    }]);
}

/// Showcase shop inspect: dark backdrop + orbit camera + focused stock mesh only (no storeroom GLB or shop HUD).
pub(crate) fn render_shop_inspect_isolated_frame(
    shop: &ShopScene,
    ctx: DrawCtx<'_>,
    orbit: &ItemInspectOrbitState,
) -> UiFrame {
    let w = ctx.layout.window_w;
    let h = ctx.layout.window_h;
    let env_h = ctx.shop_env_height_scale;
    shop.drawn_env_height_scale.set(env_h);
    let shop_rm = GameEngine::read_shop(ctx.run);

    let mut frame = UiFrame::new();
    frame.shop_inspect_lit_mesh_hdr = true;

    frame.quad(GpuInstance {
        rect: [0.0, 0.0, w, h],
        color: [0.0, 0.0, 0.0, 1.0],
    });

    let inspect_rig = InspectRig::shop(h, env_h);
    let o = resolve_shop_orbit_target_for_draw(shop, orbit, &shop_rm, w, h, env_h);
    let cam = inspect_orbit_camera(&o, &inspect_rig);
    apply_inspect_view_to_frame(
        &mut frame,
        w,
        h,
        &o,
        &inspect_rig,
        o.target_world,
        InspectFrameEnv::Neutral,
    );

    let inspect_anchor = match shop.focus {
        Some(f) if shop_focus_inspectable(f) => Some((f, o.target_world)),
        _ => None,
    };
    let (_stock_dim, stock_subj) = push_stock_meshes(shop, &shop_rm, w, h, &cam, inspect_anchor);

    if let Some(subj) = stock_subj {
        frame.clear_scene_depth();
        frame.shop_inspect_lit_mesh_subject_hdr();
        frame.object3d_batch(vec![subj]);
    }

    push_shop_inspect_overlay_chrome(&mut frame, &ctx, w, h);
    frame
}

/// Storeroom draw for the normal shop face. Item inspect uses [`render_shop_inspect_isolated_frame`] on the showcase overlay.
pub(crate) fn render_shop_frame(shop: &ShopScene, ctx: DrawCtx<'_>) -> UiFrame {
    let w = ctx.layout.window_w;
    let h = ctx.layout.window_h;
    let ui_scale = ctx.ui_scale;
    let env_h = ctx.shop_env_height_scale;
    shop.drawn_env_height_scale.set(env_h);
    let shop_rm = GameEngine::read_shop(ctx.run);

    let scale = metrics::scene_scale(w, h, ui_scale);

    let mut frame = UiFrame::new();
    frame.quad(GpuInstance {
        rect: [0.0, 0.0, w, h],
        color: [0.04, 0.04, 0.055, 1.0],
    });
    if with_shop_glb_cpu(|opt| opt.is_some()) {
        frame.shop_environment();
    }

    // Mesh anchors use ray/plane hits so props sit under vitrine pixels (plain
    // `pixel_to_world` drifts under perspective).
    let base = shop_camera_base(w, h, env_h);
    frame.camera_override = Some(base);
    let cam = base;

    let layout = ShopLayout::build(
        ctx.layout,
        &shop.positions,
        ShopInventoryCounts {
            n_for_sale: shop.items.len(),
            n_for_sale_zodiacs: shop.zodiac_items.len(),
            n_for_sale_talismans: shop.talisman_items.len(),
            n_owned_relics: shop_rm.owned_relics.len(),
        },
    );
    let gold_dish_anchor = player_gold_dish_object3d_anchor(w, h, &cam, env_h, &layout);

    let lp = layout.lamp_center_px;
    let lamp_mesh_h = h * 0.30;
    let tf = shop.age_secs;
    let flick_fast = (tf * 37.3).sin() * 0.04 + (tf * 61.7).sin() * 0.025;
    let flick_slow = (tf * 4.1).sin() * 0.06;
    let brownout = {
        let d = (tf * 0.73).sin() * (tf * 1.19).sin();
        (d - 0.55).max(0.0) * 0.35
    };
    let lamp_flicker = (1.0 + flick_fast + flick_slow - brownout).clamp(0.55, 1.12);

    let journal_cx = shop.positions.book.nx * w;
    let journal_cy = shop.positions.book.ny * h;
    let journal_cz = ctx.layout.mm(shop.positions.book.lift_mm);

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
        frame.scene_lighting.embedded_gltf_punctual = room_glb_lights;
        frame.scene_lighting.room_shop_glb_brdf = room_glb_lights;
        let use_glb_lights = room_glb_lights;
        let mut merged_punctual: Vec<ScenePunctualLight> = if use_glb_lights {
            embedded_point_lights_runtime(w, h, env_h, &ctx.shop_env_lighting)
                .into_iter()
                .map(ScenePunctualLight::InverseSquare)
                .collect()
        } else {
            Vec::new()
        };
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
            let n_for_sale_relics = shop.items.len().min(layout.niche_count);
            let n_owned_relics = shop_rm.owned_relics.len();
            match hit {
                ShopHit::Relic(i) => {
                    let (px, py, wy) = if let Some((cx, cy)) =
                        screen_xy_for_hit(hit, shop, &shop_rm, w, h, &cam)
                    {
                        (cx, cy, light_lift_at_screen_y(cy, h))
                    } else if i < n_for_sale_relics {
                        layout.niche_centers_px[i]
                    } else if let Some(si) = sale_slot_for_focus(shop, ShopFocus::Relic(i)) {
                        let r = shop_shelf_slot_rect(w, h, &cam, si, env_h);
                        (
                            r[0] + r[2] * 0.5,
                            r[1] + r[3] * 0.5,
                            shop_shelf_slot_wz(h, si),
                        )
                    } else {
                        let oi = i - shop.items.len();
                        if oi < n_owned_relics {
                            layout.owned_relic_pos(oi)
                        } else {
                            (w * 0.5, h * 0.5, h * 0.2)
                        }
                    };
                    point_lights.push(PointLight {
                        pos: [px, py - 30.0, wy + 60.0],
                        radius: h * 0.65 * hover_r_mul,
                        color: [1.00, 0.92, 0.70],
                        intensity: 3.20 * hover_i_mul,
                    });
                }
                ShopHit::Ribbon(_) | ShopHit::Talisman(_) => {
                    if let Some((cx, cy)) = screen_xy_for_hit(hit, shop, &shop_rm, w, h, &cam) {
                        let wy = light_lift_at_screen_y(cy, h);
                        point_lights.push(PointLight {
                            pos: [cx, cy + 35.0, wy + 50.0],
                            radius: h * 0.72 * hover_r_mul,
                            color: [1.00, 0.92, 0.74],
                            intensity: 3.00 * hover_i_mul,
                        });
                    }
                }
                ShopHit::Dish(id) => {
                    let center = if id == super::PICK_RELIC_DISH {
                        layout.relic_dish_center_px
                    } else if id == super::PICK_JOURNAL_BOOK {
                        (journal_cx, journal_cy, journal_cz)
                    } else if id == super::PICK_COIN_DISH {
                        (
                            gold_dish_anchor[0],
                            gold_dish_anchor[1],
                            gold_dish_anchor[2],
                        )
                    } else {
                        layout.coin_dish_center_px
                    };
                    point_lights.push(PointLight {
                        pos: [center.0, center.1 - 20.0, center.2.max(80.0)],
                        radius: h * 0.55 * hover_r_mul,
                        color: [1.00, 0.92, 0.70],
                        intensity: 2.50 * hover_i_mul,
                    });
                }
                ShopHit::TilePack(id) => {
                    if let Some((cx, cy)) = screen_xy_for_hit(hit, shop, &shop_rm, w, h, &cam) {
                        let wy = light_lift_at_screen_y(cy, h);
                        point_lights.push(PointLight {
                            pos: [cx, cy - 28.0, wy + 55.0],
                            radius: h * 0.62 * hover_r_mul,
                            color: [1.00, 0.92, 0.70],
                            intensity: 3.20 * hover_i_mul,
                        });
                    } else if let Some(idx) = super::layout::tile_pack_index_from_pick(id) {
                        let center = layout
                            .pack_centers_px
                            .get(idx)
                            .copied()
                            .unwrap_or(layout.pack_centers_px[0]);
                        point_lights.push(PointLight {
                            pos: [center.0, center.1 - 30.0, center.2 + 60.0],
                            radius: h * 0.62 * hover_r_mul,
                            color: [1.00, 0.92, 0.70],
                            intensity: 3.20 * hover_i_mul,
                        });
                    }
                }
                ShopHit::EnvSpawnSlot(_)
                | ShopHit::EnvInvSlot(_)
                | ShopHit::EnvConsumableOrd(_) => {}
            }
        }

        merged_punctual.extend(
            point_lights
                .into_iter()
                .map(ScenePunctualLight::Smooth),
        );
        frame.scene_lighting.punctual = merged_punctual;
        frame.scene_lighting.spot_lights =
            spot_lights_from_glb(w, h, env_h, &ctx.shop_env_lighting);
    }

    let inspect_anchor = None;
    let (stock_dim, stock_subj) = push_stock_meshes(shop, &shop_rm, w, h, &cam, inspect_anchor);
    let gold_pile = shop_gameplay_style_gold_pile(&layout, shop_rm.display_gold, gold_dish_anchor);

    let mut stock_all = stock_dim;
    if let Some(s) = stock_subj {
        stock_all.push(s);
    }
    stock_all.extend(gold_pile);
    if !stock_all.is_empty() {
        frame.object3d_batch(stock_all);
    }

    // Moths orbiting the pendant lamp.
    {
        let lamp_w = h * 0.22;
        let lamp_h = lamp_mesh_h;
        let lamp_hang_z = lp.2;
        let bulb_wz = lamp_hang_z * lamp_h;
        let t_now = shop.age_secs;
        let bulb_wx = lp.0 - w * 0.5;
        let bulb_wy = h * 0.5 - lp.1;
        let bug_body_len = h * 0.022;
        let flap_hz: f32 = 25.0;
        let flap_amp: f32 = 1.1;

        let sample_bug =
            |scene: &ShopScene, i: usize, t_back: f32| -> ([f32; 3], [f32; 3], Mat4, f32) {
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

                let wing_half_span = 1.13 * size_frac * bug_body_len;
                let orbit_r =
                    (r_nom + r_drift).max(lamp_w * 0.72 + bug_body_len * 0.6 + wing_half_span);

                let bug_wx = bulb_wx + orbit_r * phase.cos();
                let bug_wy = bulb_wy + orbit_r * phase.sin();
                let bug_px = bug_wx + w * 0.5;
                let bug_py = h * 0.5 - bug_wy;
                let bug_sz = bug_body_len * size_frac;

                let tx = -phase.sin();
                let ty = phase.cos();
                let bank = std::f32::consts::FRAC_PI_4 * 0.5 + (t * 1.9 + fi * 0.8).sin() * 0.30;
                let pitch = (t * pitch_freq + fi * 0.5).sin() * 0.25;
                let yaw = Mat4::from_cols(
                    glam::Vec4::new(tx, ty, 0.0, 0.0),
                    glam::Vec4::new(-ty, tx, 0.0, 0.0),
                    glam::Vec4::new(0.0, 0.0, 1.0, 0.0),
                    glam::Vec4::new(0.0, 0.0, 0.0, 1.0),
                );
                let rot = yaw * Mat4::from_rotation_x(bank) * Mat4::from_rotation_y(pitch);
                let flap = flap_amp * (t * flap_hz * std::f32::consts::TAU + fi * 1.3).sin();
                (
                    [bug_px, bug_py, bug_wz],
                    [bug_sz, bug_sz, bug_sz],
                    rot,
                    flap,
                )
            };

        let mut bugs: Vec<Object3d> = Vec::with_capacity(BUG_COUNT);
        for i in 0..BUG_COUNT {
            let (pos, extents, rot, flap_rad) = sample_bug(shop, i, 0.0);
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

    let credits_font_px = typography::size(typography::BODY, h, ui_scale).max(18.0) * 2.0;
    let gold_text = format!("{}g", shop_rm.display_gold);
    let h_px = credits_font_px.max(1.0).round().max(1.0) as u32;
    let (credits_rw, credits_rh) = if let Some(ref font) = load_ui_font() {
        let (_, _, advances) =
            measure_label_advances(font, &gold_text, 8192, h_px, Some(credits_font_px));
        let text_w: f32 = advances.iter().sum();
        let rw = text_w.max(credits_font_px * 1.2).min(w * 0.92);
        let rh = credits_font_px * 1.38;
        (rw, rh)
    } else {
        let est_ch = gold_text.chars().count().max(1) as f32;
        let rw = (credits_font_px * 0.62 * est_ch).min(w * 0.92);
        let rh = credits_font_px * 1.38;
        (rw, rh)
    };
    let mut credits_rect = with_shop_glb_cpu(|opt| {
        let cpu = opt?;
        let tw = player_gold_dish_marker_translation(cpu)? * shop_env_world_scale(h, env_h);
        let (cx, cy) = cam.project_world_to_screen(w, h, tw);
        Some([
            cx - credits_rw * 0.5,
            cy - credits_rh * 0.5,
            credits_rw,
            credits_rh,
        ])
    })
    .unwrap_or_else(|| rect_center_n(w, h, 0.88, 0.595, credits_rw, credits_rh));
    // Marker sits in the dish; float the label above it.
    credits_rect[1] -= credits_rh * 0.52 + h * 0.014;
    let pad = credits_font_px * 0.24;
    let bx = credits_rect[0] - pad;
    let by = credits_rect[1] - pad * 0.4;
    let bw = credits_rect[2] + pad * 2.0;
    let bh = credits_rect[3] + pad * 1.05;
    frame.quad(GpuInstance {
        rect: [bx - 4.0, by - 3.0, bw + 8.0, bh + 7.0],
        color: [0.02, 0.02, 0.04, 0.48],
    });
    frame.quad(GpuInstance {
        rect: [bx, by, bw, bh],
        color: [
            color::WALNUT_DEEP[0],
            color::WALNUT_DEEP[1],
            color::WALNUT_DEEP[2],
            0.88,
        ],
    });
    frame.texts([TextLabel {
        rect: credits_rect,
        text: gold_text,
        color: color::CHAMPAGNE,
        font_px: Some(credits_font_px),
        align: TextAlign::Center,
        no_glossary: false,
        scroll_offset: 0.0,
    }]);

    // Shelf focus ring uses shelf-slot screen rects.
    if !shop.pause_menu.paused {
        if let Some(ring_rect) = ring_target_rect(&ctx, shop, &shop_rm, w, h, &cam)
            .and_then(|r| clamp_rect_to_viewport(r, w, h))
        {
            let mut quads = Vec::new();
            let ring_scale = if room_glb_lights {
                scale * 1.24
            } else {
                scale
            };
            push_focus_ring(ring_rect, ring_scale, w, h, &mut quads);
            frame.quads(quads);
        }

        let mut free_quads = Vec::new();
        let mut free_texts = Vec::new();
        for (slot_i, sf) in for_sale_slots(shop).into_iter().enumerate() {
            if let Some(ShopFocus::Relic(idx)) = sf {
                if let Some(item) = shop.items.get(idx)
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

    if shop.journal_transition.is_none()
        && let Some(hit) = hover
        && let Some((ref title, ref desc, ref cta, col)) =
            hover_tooltip_content(shop, &shop_rm, &ctx.run.mode, hit)
        && !title.is_empty()
    {
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
                    || id == super::PICK_REROLL_PROP
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

    // Journal book mesh + prepass: enables arrange-mode picking for `shop.props.journal`
    // and live page texture.
    let cam_euler = camera_facing_euler_xyz_rad(cam.eye, cam.target);
    if let Some(t) = shop.journal_transition {
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
    let (journal_zoom, journal_pos) = match shop.journal_transition {
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
    let (face_w, face_h) = journal_transition::book_cover_face_extents_xy(w, journal_zoom);
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
            open_amount: shop.journal_open_amount,
        },
        hover_target: 0.0,
        anim_id: 0,
        arrange_name: Some("shop.props.journal"),
    });

    if shop.journal_open_amount > 0.001 {
        let scratch = YakuJournalScene::new();
        let inner_ctx = DrawCtx::new(
            ctx.layout,
            ctx.anim,
            ctx.run,
            ctx.progress,
            ctx.active_profile,
            ctx.game_in_progress,
            ctx.proj,
            ctx.picked_gameplay_object,
            ctx.picked_shop_object,
            ctx.debug_visibility,
            ctx.ui_scale,
            ctx.modal_active,
            ctx.arrange_preview.clone(),
            ctx.shop_env_height_scale,
            ctx.shop_env_lighting,
            ctx.effect_layers,
            ctx.cursor_pos,
            ctx.input_mode,
            ctx.gamepad_swap_ab,
            ctx.gamepad_swap_xy,
            ctx.gamepad_style,
            None,
            None,
            ctx.tile_preset,
        );
        let prepass = SceneBehavior::draw_frame(&scratch, inner_ctx);
        frame.journal_prepass_frame = Some(Box::new(prepass));
    }

    shop.push_shop_particle_quads(&mut frame);
    shop.push_shop_score_popup_labels(&mut frame, w, h);

    // Pointer targets in stack order: specific zones first (main loop uses first hit).
    // Pause overlay quads/texts are appended last so they composite above shelf geometry.
    if shop.pause_menu.paused {
        let mut pause_quads: Vec<GpuInstance> = Vec::new();
        let mut pause_text: Vec<TextLabel> = Vec::new();
        let mut pause_buttons: Vec<ButtonDef> = Vec::new();
        shop.pause_menu.draw(
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
            let r = inv_slot_rect(w, h, &cam, shop, &shop_rm, i, env_h);
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
        frame.buttons.push(ButtonDef::scene(
            (rr[0], rr[1], rr[2], rr[3]),
            SHOP_CLICK_REROLL,
        ));
        let lr = leave_btn_rect(w, h, &cam, env_h);
        frame.buttons.push(ButtonDef::scene(
            (lr[0], lr[1], lr[2], lr[3]),
            SHOP_CLICK_LEAVE,
        ));
    }

    // Floating control hints — copy reflects [`DrawCtx::input_mode`] + swap toggles.
    if !shop.pause_menu.paused {
        let surface = match ctx.input_mode {
            InputMode::Controller => PromptInputSurface::Controller,
            InputMode::Keyboard | InputMode::Cursor => PromptInputSurface::MouseOrKeyboard,
        };
        let inspect_active = false;
        let pad_bottom = h * 0.014;
        let x = w * 0.05;
        let bw = w * 0.90;
        let inner_left = x + bw * 0.02;
        let inner_right = x + bw * 0.98;
        let inner_w = (inner_right - inner_left).max(8.0);

        let font_px = typography::size(typography::CAPTION, h, ui_scale).max(14.0);
        let legend_font_px = font_px * 2.0;

        // Primary row: four equal columns, each `[icon][label]` for Exit → Select → Sell → Inspect.
        // Icons are ~3× the old bar-relative size; each prompt gets its own backer (no full-width bar).
        let bar_h_ref = h * 0.056;
        let icon_cap_3x = if inspect_active {
            bar_h_ref * 0.38 * 3.0
        } else {
            bar_h_ref * 0.72 * 3.0
        };

        let col_w = inner_w / 4.0;
        let col_pad = (col_w * 0.045).min(8.0_f32).max(2.0);
        let mut icon_px = icon_cap_3x.clamp(48.0, 132.0);
        let mut gap_after_icon = icon_px * 0.18;
        loop {
            let slot = icon_px + gap_after_icon;
            if slot <= col_w - col_pad * 2.0 || icon_px <= 18.0 {
                break;
            }
            icon_px -= 1.0;
            gap_after_icon = icon_px * 0.18;
        }

        let ui_font = load_ui_font();
        let legend_line_h = ui_font
            .as_ref()
            .and_then(|f| f.horizontal_line_metrics(legend_font_px))
            .map(|lm| lm.new_line_size)
            .unwrap_or(legend_font_px * 1.2)
            .max(legend_font_px * 0.85);

        let primary_h = (icon_px * 1.06)
            .max(legend_line_h)
            .max(font_px * 1.35);
        let inspect_line_h = if inspect_active {
            (font_px * 0.92).max(12.0) * 1.4 + h * 0.008
        } else {
            0.0
        };
        let gap_before_inspect_line = if inspect_active { h * 0.01 } else { 0.0 };
        let block_h = primary_h + gap_before_inspect_line + inspect_line_h;
        let primary_y0 = h - pad_bottom - block_h;
        let iy = primary_y0 + (primary_h - icon_px) * 0.5;

        let paths = shop_prompt_icon_paths(
            surface,
            ctx.gamepad_style,
            ctx.gamepad_swap_ab,
            ctx.gamepad_swap_xy,
        );

        let pill_bg = [0.06_f32, 0.055, 0.07, 0.82];
        let pill_pad_x =
            (icon_px * 0.10).clamp(6.0, 16.0) + (h * 0.003).clamp(4.0, 8.0);

        let legend_text_h_px = legend_line_h.max(8.0).round().max(1.0) as u32;
        let pill_pad_y = (legend_line_h * 0.14).clamp(3.0, 9.0);
        let legend_text_y = primary_y0 + (primary_h - legend_line_h) * 0.5;

        let mut pill_quads: Vec<GpuInstance> = Vec::with_capacity(4);
        let mut icon_cmds: Vec<PromptIconQuad> = Vec::with_capacity(4);
        let mut legend_texts: Vec<TextLabel> =
            Vec::with_capacity(if inspect_active { 5 } else { 4 });

        for i in 0..4 {
            let col_x = inner_left + i as f32 * col_w;
            let ix = col_x + col_pad;
            let text_x = ix + icon_px + gap_after_icon;
            let max_text_w = (col_x + col_w - col_pad - text_x).max(10.0);
            let measured_w: f32 = if let Some(ref font) = ui_font {
                let (_, _, advances) = measure_label_advances(
                    font,
                    SHOP_LEGEND_VERB_LABELS[i],
                    8192,
                    legend_text_h_px,
                    Some(legend_font_px),
                );
                advances.iter().copied().sum()
            } else {
                let est_ch = SHOP_LEGEND_VERB_LABELS[i].chars().count().max(1) as f32;
                (legend_font_px * 0.52 * est_ch).max(8.0)
            };
            let text_w = measured_w.min(max_text_w).max(1.0);
            let pill_left = text_x - pill_pad_x;
            let pill_w = (text_w + pill_pad_x * 2.0).max(1.0);
            pill_quads.push(GpuInstance {
                rect: [
                    pill_left,
                    legend_text_y - pill_pad_y,
                    pill_w,
                    legend_line_h + pill_pad_y * 2.0,
                ],
                color: pill_bg,
            });
            icon_cmds.push(PromptIconQuad {
                inst: GpuInstance {
                    rect: [ix, iy, icon_px, icon_px],
                    color: [0.92, 0.88, 0.82, 0.96],
                },
                asset_rel_path: paths[i],
            });
            legend_texts.push(TextLabel {
                rect: [
                    text_x,
                    legend_text_y,
                    text_w,
                    legend_line_h,
                ],
                text: SHOP_LEGEND_VERB_LABELS[i].to_string(),
                color: [0.88, 0.84, 0.78, 0.96],
                font_px: Some(legend_font_px),
                align: TextAlign::Left,
                no_glossary: false,
                scroll_offset: 0.0,
            });
        }

        frame.squircle_quads(pill_quads);

        const HOLD_SELL_LEGEND_COL: usize = 2;
        if let Some(started) = shop.west_sell_hold_started {
            let elapsed = Instant::now()
                .saturating_duration_since(started)
                .as_secs_f32();
            let progress = (elapsed / super::SHOP_SELL_HOLD_SECONDS).clamp(0.0, 1.0);
            let col_x = inner_left + HOLD_SELL_LEGEND_COL as f32 * col_w;
            let ix = col_x + col_pad;
            let cx = ix + icon_px * 0.5;
            let cy = iy + icon_px * 0.5;
            let r = icon_px * 0.58;
            let thickness = (icon_px * 0.12).max(3.5);
            let mut ring_quads: Vec<GpuInstance> = Vec::with_capacity(72);
            crate::ui::prompt_hold_ring::push_hold_prompt_ring(
                &mut ring_quads,
                cx,
                cy,
                r,
                thickness,
                progress,
            );
            frame.quads(ring_quads);
        }

        frame.prompt_icon_quads(icon_cmds);

        if inspect_active {
            legend_texts.push(TextLabel {
                rect: [
                    inner_left,
                    primary_y0 + primary_h + gap_before_inspect_line,
                    inner_w,
                    inspect_line_h,
                ],
                text: ButtonPrompt::shop_inspect_mode_hint(surface, ctx.gamepad_style),
                color: [0.82, 0.78, 0.72, 0.94],
                font_px: Some((font_px * 0.92).max(12.0)),
                align: TextAlign::Center,
                no_glossary: false,
                scroll_offset: 0.0,
            });
        }

        frame.texts(legend_texts);
    }

    frame
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
    let i_got_a_guy_charges = if shop.relic_state.has(RelicId::IGotAGuy) {
        shop.relic_counters
            .get(&RelicId::IGotAGuy)
            .copied()
            .unwrap_or(0)
    } else {
        0
    };
    let reroll_affordable = matches!(scene.mode, ShopMode::Standard)
        && (scene.reroll_cost == 0
            || shop.gold >= scene.reroll_cost as i32
            || i_got_a_guy_charges > 0);

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
            let desc = relic_description_live(
                rid,
                &shop.relic_counters,
                shop.total_score_earned,
                Some((&shop.relic_state, oi)),
                None,
            );
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
                let cta = if shop.gold >= scene.reroll_cost as i32 {
                    format!("{}g", scene.reroll_cost)
                } else if i_got_a_guy_charges > 0 {
                    format!("FREE ({} left)", i_got_a_guy_charges)
                } else {
                    format!("${} (have {}g)", scene.reroll_cost, shop.display_gold)
                };
                (
                    "Restock".to_string(),
                    format!("Refresh shop for {}g", scene.reroll_cost),
                    cta,
                    if reroll_affordable {
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
        .and_then(|name| {
            with_shop_glb_cpu(|opt| opt.and_then(|cpu| marker_translation(cpu, &name)))
        })
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
fn euler_rad_add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
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
        / super::SHOP_SELL_HOLD_SECONDS)
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
    if let Some((ifoc, tw)) = inspect_anchor {
        if ifoc == foc {
            return object3d_pos_triple_for_world_center(w, h, Vec3::from_array(tw));
        }
    }
    fallback
}

#[inline]
fn partition_shop_inspect_stock_mesh(
    inspect_anchor: Option<(ShopFocus, [f32; 3])>,
    foc: ShopFocus,
    mesh: Object3d,
    dim: &mut Vec<Object3d>,
    subject: &mut Option<Object3d>,
) {
    if let Some((ifoc, _)) = inspect_anchor {
        if ifoc == foc {
            *subject = Some(mesh);
            return;
        }
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
    let mut dim = Vec::new();
    let mut subject = None;
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
                    let relic_pos = object3d_pos_for_shop_inspect_focus(
                        inspect_anchor,
                        *foc,
                        w,
                        h,
                        sale_anchor_at_slot(w, h, cam, slot_i, env_h, cx, cy, cz),
                    );
                    partition_shop_inspect_stock_mesh(
                        inspect_anchor,
                        *foc,
                        Object3d {
                            pos: relic_pos,
                            extents: [half[0] * 2.0, half[1] * 2.0, half[2] * 2.0],
                            rotation: euler_xyz_rad_from_deg(
                                super::SHOP_RELIC_LEAN_COUNTER,
                                0.0,
                                0.0,
                            ),
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
                        },
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
                let pack_h = r[3] * 0.52;
                let pack_w = pack_h * PACK_ASPECT_W_OVER_H;
                let pack_t = pack_h * 0.11;
                let ext = [pack_w, pack_t, pack_h];
                let cz = wz + ext[2] * 0.5;
                let pack_pos = object3d_pos_for_shop_inspect_focus(
                    inspect_anchor,
                    *foc,
                    w,
                    h,
                    sale_anchor_at_slot(w, h, cam, slot_i, env_h, cx, cy, cz),
                );
                partition_shop_inspect_stock_mesh(
                    inspect_anchor,
                    *foc,
                    Object3d {
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
                    },
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
                let ribbon_w = r[2] * 0.38;
                let ribbon_len = r[3] * 0.62;
                let z_k = if let Consumable::Zodiac(z) = item.consumable {
                    Some(z)
                } else {
                    None
                };
                let cz = wz + ribbon_len * 0.35;
                let ribbon_pos = object3d_pos_for_shop_inspect_focus(
                    inspect_anchor,
                    *foc,
                    w,
                    h,
                    sale_anchor_at_slot(w, h, cam, slot_i, env_h, cx, cy, cz),
                );
                partition_shop_inspect_stock_mesh(
                    inspect_anchor,
                    *foc,
                    Object3d {
                        pos: ribbon_pos,
                        extents: [ribbon_w, ribbon_len, ribbon_w * 0.14],
                        rotation: euler_xyz_rad_from_deg(90.0, 0.0, 0.0),
                        color: [1.0, 1.0, 1.0, col[3]],
                        kind: Object3dKind::ZodiacRibbon { kind: z_k },
                        hover_target: 0.0,
                        anim_id: 0,
                        arrange_name: Some("shop.for_sale.ribbons"),
                    },
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
                    let tw = r[2] * 0.40;
                    let cz = wz + tw * 0.55;
                    let tal_pos = object3d_pos_for_shop_inspect_focus(
                        inspect_anchor,
                        *foc,
                        w,
                        h,
                        sale_anchor_at_slot(w, h, cam, slot_i, env_h, cx, cy, cz),
                    );
                    partition_shop_inspect_stock_mesh(
                        inspect_anchor,
                        *foc,
                        Object3d {
                            pos: tal_pos,
                            extents: [tw * 1.15, tw * 1.72, tw * 0.32],
                            rotation: euler_xyz_rad_from_deg(-90.0, 0.0, 0.0),
                            color: col,
                            kind: Object3dKind::Talisman { kind: tk },
                            hover_target: 0.0,
                            anim_id: 0,
                            arrange_name: Some("shop.for_sale.talismans"),
                        },
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
                let relic_pos = object3d_pos_for_shop_inspect_focus(
                    inspect_anchor,
                    *foc,
                    w,
                    h,
                    inv_marker_surface_anchor(w, h, cam, scene, shop, slot_i, env_h, cx, cy, cz),
                );
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
                            pick_id: None,
                        },
                        hover_target: 0.0,
                        anim_id: 0,
                        arrange_name: Some("shop.shelf.relic_dish"),
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
                    let ribbon_w = r[2] * 0.36;
                    let ribbon_len = r[3] * 0.58;
                    let cz = wz + ribbon_len * 0.32;
                    let pos = object3d_pos_for_shop_inspect_focus(
                        inspect_anchor,
                        *foc,
                        w,
                        h,
                        inv_marker_surface_anchor(
                            w, h, cam, scene, shop, slot_i, env_h, cx, cy, cz,
                        ),
                    );
                    let base_rot = euler_xyz_rad_from_deg(-90.0, 0.0, 0.0);
                    partition_shop_inspect_stock_mesh(
                        inspect_anchor,
                        *foc,
                        Object3d {
                            pos,
                            extents: [ribbon_w, ribbon_len, ribbon_w * 0.13],
                            rotation: euler_rad_add(
                                base_rot,
                                sell_hold_wobble_euler_rad(scene, *foc),
                            ),
                            color: [1.0, 1.0, 1.0, 1.0],
                            kind: Object3dKind::ZodiacRibbon { kind: Some(z) },
                            hover_target: 0.0,
                            anim_id: 0,
                            arrange_name: Some("shop.shelf.ribbon_tray"),
                        },
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
                if let Consumable::Talisman(tk) = owned.consumable {
                    let tw = r[2] * 0.36;
                    let cz = wz + tw * 0.45;
                    let pos = object3d_pos_for_shop_inspect_focus(
                        inspect_anchor,
                        *foc,
                        w,
                        h,
                        inv_marker_surface_anchor(
                            w, h, cam, scene, shop, slot_i, env_h, cx, cy, cz,
                        ),
                    );
                    let base_rot = euler_xyz_rad_from_deg(-90.0, 0.0, 0.0);
                    partition_shop_inspect_stock_mesh(
                        inspect_anchor,
                        *foc,
                        Object3d {
                            pos,
                            extents: [tw * 1.15, tw * 1.72, tw * 0.30],
                            rotation: euler_rad_add(
                                base_rot,
                                sell_hold_wobble_euler_rad(scene, *foc),
                            ),
                            color: consumable_color(owned.consumable),
                            kind: Object3dKind::Talisman { kind: tk },
                            hover_target: 0.0,
                            anim_id: 0,
                            arrange_name: Some("shop.shelf.owned_talismans"),
                        },
                        &mut dim,
                        &mut subject,
                    );
                }
            }
            _ => {}
        }
    }

    (dim, subject)
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
    if let Some(r) = marker_screen_rect(w, h, cam, &spawn_relic_marker_name(index), rw, rh, env_h) {
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
    let env_h = scene.drawn_env_height_scale.get();
    let cam = shop_camera_params(w, h, env_h);
    let rects = build_focus_rects(scene, w, h, &cam, &shop);

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
    v
}

fn map_shop_ui_click_to_hit(cid: u32, scene: &ShopScene, shop: &ShopReadModel) -> Option<ShopHit> {
    match cid {
        SHOP_CLICK_JOURNAL => Some(ShopHit::Dish(super::PICK_JOURNAL_BOOK)),
        SHOP_CLICK_REROLL => Some(ShopHit::Dish(super::PICK_REROLL_PROP)),
        SHOP_CLICK_LEAVE => Some(ShopHit::Dish(super::PICK_LEAVE_PROP)),
        _ => {
            if (SHOP_SHELF_CLICK_BASE..SHOP_SHELF_CLICK_BASE + SHOP_SPAWN_SLOT_COUNT as u32)
                .contains(&cid)
            {
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
