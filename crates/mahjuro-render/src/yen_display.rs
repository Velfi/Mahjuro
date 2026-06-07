//! Settled yen coin piles and floating amount labels (shop + gameplay).

use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};

use glam::Vec3;

use crate::decal::{load_ui_font, measure_label_advances};
use crate::draw_cmd::{Object3d, Object3dKind, UiFrame};
use crate::primitive::{MaterialSpec, MeshId};
use crate::theme::{color, typography};
use crate::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use crate::world_space::{object3d_pos_triple_for_world_center, pixel_to_world};
/// RNG seed for the shop coin pile layout (stable across frames).
pub const SHOP_GOLD_PILE_SEED: u64 = 0x5EED_E0D1_D151_0001;
/// RNG seed for gameplay — distinct from shop so layouts don't match exactly.
pub const GAMEPLAY_GOLD_PILE_SEED: u64 = 0xC01_C0FFEE;

/// Coin geometry shared by shop and gameplay piles.
pub fn yen_coin_dims(mm: impl Fn(f32) -> f32) -> (f32, f32, f32) {
    let coin_radius = mm(11.3);
    let coin_thickness = mm(3.5).max(2.0);
    let scatter_half = coin_radius * 3.0;
    (coin_radius, coin_thickness, scatter_half)
}

/// Settled GLB coins on a dish floor — no procedural tray mesh (GLB or layout supplies the tray).
///
/// When `window` is `Some((w, h))`, the anchor is a [`surface_anchor`](crate::world_space::surface_anchor_from_world_xyz)
/// and scatter happens in world XY (for gameplay GLB `player_yen`). Otherwise the anchor is already
/// in `Object3d::pos` form and scatter offsets are added directly (legacy gameplay layout / shop).
pub fn build_settled_yen_coin_pile(
    mm: impl Fn(f32) -> f32,
    yen: i32,
    anchor: [f32; 3],
    rng_seed: u64,
    window: Option<(f32, f32)>,
    scale_mul: f32,
) -> Vec<Object3d> {
    if yen <= 0 {
        return Vec::new();
    }
    let coin_count = (yen as usize).min(48);
    let (coin_radius, coin_thickness, scatter_half) = yen_coin_dims(&mm);
    let coin_radius = coin_radius * scale_mul;
    let coin_thickness = coin_thickness * scale_mul;
    let scatter_half = scatter_half * scale_mul;
    let (pile_cx, pile_cy, dish_floor_z, world_scatter) = if let Some((w, h)) = window {
        let floor = pixel_to_world(w, h, anchor[0], anchor[1], anchor[2]);
        (floor.x, floor.y, floor.z, true)
    } else {
        (anchor[0], anchor[1], anchor[2], false)
    };
    let overlap_r = coin_radius * 2.0;
    let overlap_r2 = overlap_r * overlap_r;
    const CANDIDATES_PER_COIN: u32 = 12;

    let mut rng = StdRng::seed_from_u64(rng_seed);
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
        let center = Vec3::new(pile_cx + lx, pile_cy + lz, world_y);
        let pos = if world_scatter {
            let (w, h) = window.expect("world scatter requires window size");
            object3d_pos_triple_for_world_center(w, h, center)
        } else {
            [center.x, center.y, center.z]
        };
        coins.push(Object3d {
            pos,
            extents: [coin_radius * 2.0, coin_thickness, coin_radius * 2.0],
            rotation: [0.0, rot_y, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
            kind: Object3dKind::Primitive {
                shape: MeshId::Coin,
                material: MaterialSpec::coin_glb(),
                pick_id: None,
                silhouette: false,
            },
            hover_target: 0.0,
            anim_id: 0,
        });
    }
    coins
}

/// Lacquered plaque + `Ng` label above the dish; returns the focus/hit rect.
pub fn push_yen_amount_label(
    frame: &mut UiFrame,
    window_w: f32,
    window_h: f32,
    yen: i32,
    label_center_px: (f32, f32),
) -> [f32; 4] {
    let yen_text = format!("¥{}", yen.max(0));
    let credits_font_px = typography::size(typography::H20, window_h);
    let h_px = credits_font_px.max(1.0).round().max(1.0) as u32;
    let (credits_rw, credits_rh) = if let Some(font) = load_ui_font() {
        let (_, _, advances) =
            measure_label_advances(font, &yen_text, 8192, h_px, Some(credits_font_px));
        let text_w: f32 = advances.iter().sum();
        let rw = text_w.max(credits_font_px * 1.2).min(window_w * 0.92);
        let rh = credits_font_px * 1.38;
        (rw, rh)
    } else {
        let est_ch = yen_text.chars().count().max(1) as f32;
        let rw = (credits_font_px * 0.62 * est_ch).min(window_w * 0.92);
        let rh = credits_font_px * 1.38;
        (rw, rh)
    };
    let mut credits_rect = [
        label_center_px.0 - credits_rw * 0.5,
        label_center_px.1 - credits_rh * 0.5,
        credits_rw,
        credits_rh,
    ];
    credits_rect[1] -= credits_rh * 0.52 + window_h * 0.014;
    let pad = credits_font_px * 0.24;
    let bx = credits_rect[0] - pad;
    let by = credits_rect[1] - pad * 0.4;
    let bw = credits_rect[2] + pad * 2.0;
    let bh = credits_rect[3] + pad * 1.05;
    let yen_label_rect: [f32; 4] = [bx - 4.0, by - 3.0, bw + 8.0, bh + 7.0];
    frame.quad(GpuInstance {
        rect: [bx - 4.0, by - 3.0, bw + 8.0, bh + 7.0],
        color: color::alpha(color::WALNUT_INK, 0.48),
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: [bx, by, bw, bh],
        color: [
            color::WALNUT_DEEP[0],
            color::WALNUT_DEEP[1],
            color::WALNUT_DEEP[2],
            0.88,
        ],
        user: 0,
    });
    frame.texts([TextLabel {
        rect: credits_rect,
        text: yen_text,
        color: color::CHAMPAGNE,
        font_px: Some(credits_font_px),
        align: TextAlign::Center,
        block_vertical_align: Default::default(),
        scroll_offset: 0.0,
        flavor_spans: None,
        bold: false,
        italic: false,
        underline: false,
        text_effect: crate::text_effect::TextEffectId::Flat,
        rotation_quarters: 0,
        baseline_shift_px: 0.0,
        clip_rect: None,
        mono: false,
    }]);
    yen_label_rect
}
