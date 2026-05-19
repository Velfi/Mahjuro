//! Settled gold coin piles and floating amount labels (shop + gameplay).

use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};

use crate::render::decal::{load_ui_font, measure_label_advances};
use crate::render::draw_cmd::{Object3d, Object3dKind, UiFrame};
use crate::render::primitive::{MaterialSpec, MeshId};
use crate::render::theme::{color, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use crate::ui::placement::Placement;

/// RNG seed for the shop coin pile layout (stable across frames).
pub const SHOP_GOLD_PILE_SEED: u64 = 0x5EED_E0D1_D151_0001;
/// RNG seed for gameplay — distinct from shop so layouts don't match exactly.
pub const GAMEPLAY_GOLD_PILE_SEED: u64 = 0xC01_C0FFEE;

/// Coin geometry shared by shop and gameplay piles.
pub fn gold_coin_dims(mm: impl Fn(f32) -> f32) -> (f32, f32, f32) {
    let coin_radius = mm(11.3);
    let coin_thickness = mm(3.5).max(2.0);
    let scatter_half = coin_radius * 3.0;
    (coin_radius, coin_thickness, scatter_half)
}

/// Screen-space anchor for the gameplay coin pile (`Object3d::pos` xy + dish floor z).
pub fn gameplay_gold_pile_anchor(
    layout: &crate::ui::layout::LayoutResult,
    placement: &Placement,
) -> [f32; 3] {
    let (_, _, scatter_half) = gold_coin_dims(|n| layout.mm(n));
    let coin_back_z_push = scatter_half * 0.5;
    let pile_cx = placement.nx * layout.window_w;
    let pile_cy = layout.score_panel.y + layout.score_panel.h * 0.5
        - coin_back_z_push
        - layout.window_h * placement.ny;
    let dish_floor_z = layout.mm(placement.lift_mm) + layout.mm(3.0);
    [pile_cx, pile_cy, dish_floor_z]
}

/// Settled metal coin cylinders on a dish floor — no procedural tray mesh (GLB or layout supplies the tray).
pub fn build_settled_gold_coin_pile(
    mm: impl Fn(f32) -> f32,
    gold: i32,
    anchor: [f32; 3],
    arrange_name: &'static str,
    rng_seed: u64,
) -> Vec<Object3d> {
    if gold <= 0 {
        return Vec::new();
    }
    let coin_count = (gold as usize).min(48);
    let (coin_radius, coin_thickness, scatter_half) = gold_coin_dims(&mm);
    let pile_cx = anchor[0];
    let pile_cy = anchor[1];
    let dish_floor_z = anchor[2];
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
        coins.push(Object3d {
            pos: [pile_cx + lx, pile_cy + lz, world_y],
            extents: [coin_radius * 2.0, coin_thickness, coin_radius * 2.0],
            rotation: [0.0, rot_y, 0.0],
            color: color::RELIC_GOLD,
            kind: Object3dKind::Primitive {
                shape: MeshId::Cylinder,
                material: MaterialSpec::metal(),
                pick_id: None,
                shadow_caster: true,
                silhouette: false,
            },
            hover_target: 0.0,
            anim_id: 0,
            arrange_name: Some(arrange_name),
        });
    }
    coins
}

/// Lacquered plaque + `Ng` label above the dish; returns the focus/hit rect.
pub fn push_gold_amount_label(
    frame: &mut UiFrame,
    window_w: f32,
    window_h: f32,
    gold: i32,
    label_center_px: (f32, f32),
) -> [f32; 4] {
    let gold_text = format!("{}g", gold.max(0));
    let credits_font_px = typography::size(typography::H20, window_h);
    let h_px = credits_font_px.max(1.0).round().max(1.0) as u32;
    let (credits_rw, credits_rh) = if let Some(ref font) = load_ui_font() {
        let (_, _, advances) =
            measure_label_advances(font, &gold_text, 8192, h_px, Some(credits_font_px));
        let text_w: f32 = advances.iter().sum();
        let rw = text_w.max(credits_font_px * 1.2).min(window_w * 0.92);
        let rh = credits_font_px * 1.38;
        (rw, rh)
    } else {
        let est_ch = gold_text.chars().count().max(1) as f32;
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
    let gold_label_rect: [f32; 4] = [bx - 4.0, by - 3.0, bw + 8.0, bh + 7.0];
    frame.quad(GpuInstance {
        rect: [bx - 4.0, by - 3.0, bw + 8.0, bh + 7.0],
        color: color::alpha(color::LACQUER, 0.48),
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
        text: gold_text,
        color: color::CHAMPAGNE,
        font_px: Some(credits_font_px),
        align: TextAlign::Center,
        no_glossary: false,
        scroll_offset: 0.0,
        flavor_spans: None,
        bold: false,
        italic: false,
        underline: false,
        text_effect: crate::render::text_effect::TextEffectId::Flat,
        rotation_quarters: 0,
        baseline_shift_px: 0.0,
        clip_rect: None,
    }]);
    gold_label_rect
}
