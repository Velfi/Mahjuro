use super::*;

pub(super) fn live_shop_hit(
    hit: ShopHit,
    items: &[ShopItem],
    zodiac_items: &[ConsumableShopItem],
    talisman_items: &[ConsumableShopItem],
    pack_items: &[TilePackShopItem],
    shop: &ShopReadModel,
) -> Option<ShopHit> {
    let valid = match hit {
        ShopHit::Relic(i) => i < items.len() + shop.owned_relics.len(),
        ShopHit::Ribbon(i) => i < zodiac_items.len() + shop.owned_zodiacs.len(),
        ShopHit::Talisman(i) => i < talisman_items.len() + shop.owned_talismans.len(),
        ShopHit::Dish(id) => {
            if matches!(id, PICK_COIN_DISH | PICK_JOURNAL_BOOK) {
                true
            } else if let Some(idx) = tile_pack_index_from_pick(id) {
                pack_items.get(idx).is_some_and(|p| !p.sold)
            } else {
                false
            }
        }
        ShopHit::TilePack(id) => tile_pack_index_from_pick(id)
            .and_then(|idx| pack_items.get(idx))
            .is_some_and(|p| !p.sold),
    };
    valid.then_some(hit)
}

pub(super) fn owned_ribbon_inventory_index(
    ribbon_idx: usize,
    zodiac_items: &[ConsumableShopItem],
    shop: &ShopReadModel,
) -> Option<usize> {
    if ribbon_idx < zodiac_items.len() {
        return None;
    }
    let oi = ribbon_idx - zodiac_items.len();
    shop.owned_zodiacs.get(oi).map(|item| item.inventory_index)
}

pub(super) fn owned_talisman_inventory_index(
    talisman_idx: usize,
    talisman_items: &[ConsumableShopItem],
    shop: &ShopReadModel,
) -> Option<usize> {
    if talisman_idx < talisman_items.len() {
        return None;
    }
    let oi = talisman_idx - talisman_items.len();
    shop.owned_talismans
        .get(oi)
        .map(|item| item.inventory_index)
}

pub(super) fn is_tile_pack_pick(id: u32) -> bool {
    id >= PICK_TILE_PACK_BASE && id < PICK_TILE_PACK_BASE + N_TILE_PACKS as u32
}

pub(super) fn tile_pack_index_from_pick(id: u32) -> Option<usize> {
    if is_tile_pack_pick(id) {
        Some((id - PICK_TILE_PACK_BASE) as usize)
    } else {
        None
    }
}

#[derive(Clone, Copy)]
pub(super) struct ShopLayout {
    pub camera: CameraParams,
    pub counter_pixel_x: f32,
    pub counter_world_y: f32,
    pub counter_extents: [f32; 3],
    pub niche_centers_px: [(f32, f32, f32); KIOSK_RELIC_SLOTS],
    pub niche_count: usize,
    pub ribbon_anchors_px: [(f32, f32, f32); 8],
    pub ribbon_count: usize,
    pub ribbon_length: f32,
    pub ribbon_width: f32,
    pub talisman_anchors_px: [(f32, f32, f32); 4],
    pub talisman_anchor_count: usize,
    pub talisman_wall_width: f32,
    pub pack_centers_px: [(f32, f32, f32); N_TILE_PACKS],
    pub pack_extents: [f32; 3],
    pub relic_dish_center_px: (f32, f32, f32),
    pub relic_dish_extents: [f32; 3],
    pub ribbon_tray_center_px: (f32, f32, f32),
    pub ribbon_tray_extents: [f32; 3],
    pub talisman_tray_center_px: (f32, f32, f32),
    pub talisman_tray_extents: [f32; 3],
    pub owned_talisman_offset: (f32, f32, f32),
    pub coin_dish_center_px: (f32, f32, f32),
    pub coin_dish_extents: [f32; 3],
    pub owned_relic_count: usize,
    pub ribbon_owned_count: usize,
    pub talisman_owned_count: usize,
    pub consumable_length: f32,
    pub consumable_width: f32,
    pub lamp_center_px: (f32, f32, f32),
    pub ppmm: f32,
}

impl ShopLayout {
    pub(super) fn mm(&self, n: f32) -> f32 {
        self.ppmm * n
    }
}

pub(super) struct ShopInventoryCounts {
    pub n_for_sale: usize,
    pub n_for_sale_zodiacs: usize,
    pub n_for_sale_talismans: usize,
    pub n_owned_relics: usize,
    pub n_owned_zodiacs: usize,
    pub n_owned_talismans: usize,
}

impl ShopLayout {
    pub(super) fn build(
        layout: &crate::ui::layout::LayoutResult,
        positions: &crate::ui::scene_layout::ShopPositions,
        counts: ShopInventoryCounts,
    ) -> Self {
        let ShopInventoryCounts {
            n_for_sale,
            n_for_sale_zodiacs,
            n_for_sale_talismans,
            n_owned_relics,
            n_owned_zodiacs,
            n_owned_talismans,
        } = counts;
        let w = layout.window_w;
        let h = layout.window_h;

        let camera = CameraParams {
            eye: [
                0.0,
                -h * positions.camera_eye_y_frac,
                h * positions.camera_eye_z_frac,
            ],
            target: [
                0.0,
                h * positions.camera_target_y_frac,
                h * positions.camera_target_z_frac,
            ],
            up: [0.0, 0.0, 1.0],
            fovy_deg: 58.0,
        };

        let counter_extents = [w * 0.80, layout.mm(30.0), h * 0.17];
        let counter_pixel_x = positions.counter.nx * w;
        let counter_pixel_y = positions.counter.ny * h;
        let counter_world_y = counter_pixel_y - h * 0.5;

        let col_px_x: [f32; 4] = [
            positions.relics.nx * w,
            positions.packs.nx * w,
            positions.talismans.nx * w,
            positions.ribbons.nx * w,
        ];

        let relic_pixel_y = positions.relics.ny * h;
        let relic_wz = layout.mm(positions.relics.lift_mm);
        let pack_pixel_y = positions.packs.ny * h;
        let pack_wz = layout.mm(positions.packs.lift_mm);
        let talisman_pixel_y = positions.talismans.ny * h;
        let talisman_wz = layout.mm(positions.talismans.lift_mm);
        let ribbon_pixel_y = positions.ribbons.ny * h;
        let ribbon_wz = layout.mm(positions.ribbons.lift_mm);

        let mut niche_centers_px = [(0.0, 0.0, 0.0); KIOSK_RELIC_SLOTS];
        let n_niches = n_for_sale.min(KIOSK_RELIC_SLOTS);
        let relic_spread = positions.relic_spread_nx * w;
        for (i, slot) in niche_centers_px.iter_mut().enumerate().take(n_niches) {
            let off = if n_niches <= 1 {
                0.0
            } else {
                (i as f32 - (n_niches as f32 - 1.0) * 0.5) * relic_spread
            };
            *slot = (col_px_x[0] + off, relic_pixel_y, relic_wz);
        }

        let ribbon_width = h * 0.055;
        let ribbon_length = ribbon_width * 2.0;
        let ribbon_pin_wz = ribbon_wz + ribbon_length;
        let mut ribbon_anchors_px = [(0.0, 0.0, 0.0); 8];
        let n_ribbons = n_for_sale_zodiacs.min(8);
        for (i, slot) in ribbon_anchors_px.iter_mut().enumerate().take(n_ribbons) {
            let off = (i as f32 - (n_ribbons as f32 - 1.0) * 0.5) * positions.ribbon_spread_nx * w;
            *slot = (col_px_x[3] + off, ribbon_pixel_y, ribbon_pin_wz);
        }

        let talisman_wall_width = h * 0.072;
        let mut talisman_anchors_px = [(0.0, 0.0, 0.0); 4];
        let n_talisman_anchors = n_for_sale_talismans.min(4);
        for (i, slot) in talisman_anchors_px
            .iter_mut()
            .enumerate()
            .take(n_talisman_anchors)
        {
            let off = (i as f32 - (n_talisman_anchors as f32 - 1.0) * 0.5)
                * positions.talisman_spread_nx
                * w;
            *slot = (
                col_px_x[2] + off,
                talisman_pixel_y,
                talisman_wz + talisman_wall_width,
            );
        }

        let pack_height = h * 0.090;
        let pack_width = pack_height * crate::core::tile_pack::PACK_ASPECT_W_OVER_H;
        let pack_thickness = pack_height * 0.10;
        let pack_extents = [pack_width, pack_thickness, pack_height];
        let pack_gap = pack_width * 0.35;
        let pack_spacing = pack_width + pack_gap;
        let pack_z = pack_wz + layout.mm(10.0);
        let mut pack_centers_px = [(0.0, 0.0, 0.0); N_TILE_PACKS];
        for (i, slot) in pack_centers_px.iter_mut().enumerate() {
            let off = (i as f32 - (N_TILE_PACKS as f32 - 1.0) * 0.5) * pack_spacing;
            *slot = (col_px_x[1] + off, pack_pixel_y, pack_z);
        }

        let shelf_pixel_y = positions.relic_dish.ny * h;
        let dish_rim = layout.mm(8.0);
        let tray_depth = h * 0.024;
        let shelf_world_y = h * 0.5 - shelf_pixel_y;
        let (frust_world_x_min, frust_world_x_max) =
            camera.frustum_x_range_at(w, h, shelf_world_y, 0.0);
        let frust_px_min = (frust_world_x_min + w * 0.5).max(0.0);
        let frust_px_max = (frust_world_x_max + w * 0.5).min(w);
        let margin = w * 0.03;
        let vis_px_min = frust_px_min + margin;
        let vis_px_max = frust_px_max - margin;
        let vis_w = (vis_px_max - vis_px_min).max(1.0);
        let remap_nx = |nx: f32| vis_px_min + nx * vis_w;

        let relic_dish_center_px = (remap_nx(positions.relic_dish.nx), shelf_pixel_y, 0.0);
        let relic_dish_extents = [vis_w * 0.14, dish_rim, tray_depth];
        let talisman_tray_center_px = (remap_nx(positions.talisman_tray.nx), shelf_pixel_y, 0.0);
        let talisman_tray_extents = [vis_w * 0.14, dish_rim, tray_depth];
        let ribbon_tray_center_px = (remap_nx(positions.ribbon_tray.nx), shelf_pixel_y, 0.0);
        let ribbon_tray_extents = [vis_w * 0.14, dish_rim, tray_depth];
        let coin_dish_center_px = (remap_nx(positions.coin_dish.nx), shelf_pixel_y, 0.0);
        let coin_dish_extents = [vis_w * 0.13, dish_rim, tray_depth];

        let consumable_width = layout.mm(9.0);
        let consumable_length = consumable_width * 1.5;
        let ot = &positions.owned_talismans;
        let owned_talisman_offset = (ot.nx * w, ot.ny * h, layout.mm(ot.lift_mm));
        let lamp_center_px = (
            positions.lamp.nx * w,
            positions.lamp.ny * h,
            layout.mm(positions.lamp.lift_mm),
        );

        Self {
            camera,
            counter_pixel_x,
            counter_world_y,
            counter_extents,
            niche_centers_px,
            niche_count: n_niches,
            ribbon_anchors_px,
            ribbon_count: n_ribbons,
            ribbon_length,
            ribbon_width,
            talisman_anchors_px,
            talisman_anchor_count: n_talisman_anchors,
            talisman_wall_width,
            pack_centers_px,
            pack_extents,
            relic_dish_center_px,
            relic_dish_extents,
            ribbon_tray_center_px,
            ribbon_tray_extents,
            talisman_tray_center_px,
            talisman_tray_extents,
            owned_talisman_offset,
            coin_dish_center_px,
            coin_dish_extents,
            owned_relic_count: n_owned_relics,
            ribbon_owned_count: n_owned_zodiacs,
            talisman_owned_count: n_owned_talismans,
            consumable_length,
            consumable_width,
            lamp_center_px,
            ppmm: layout.mm(1.0),
        }
    }

    pub(super) fn owned_relic_pos(&self, idx: usize) -> (f32, f32, f32) {
        let n = self.owned_relic_count.max(1) as f32;
        let dish_w = self.relic_dish_extents[0] * 0.85;
        let start_x = self.relic_dish_center_px.0 - dish_w * 0.5 + (dish_w / n) * 0.5;
        let px = start_x + (dish_w / n) * idx as f32;
        let py = self.relic_dish_center_px.1;
        let lift = self.relic_dish_extents[1] + 4.0;
        (px, py, lift)
    }

    pub(super) fn owned_ribbon_pos(&self, idx: usize) -> (f32, f32, f32) {
        let n = self.ribbon_owned_count.max(1) as f32;
        let row_w = self.ribbon_tray_extents[0] * 0.85;
        let start_x = self.ribbon_tray_center_px.0 - row_w * 0.5 + (row_w / n) * 0.5;
        let px = start_x + (row_w / n) * idx as f32;
        let py = self.ribbon_tray_center_px.1;
        let lift = self.ribbon_tray_extents[1] + self.consumable_length * 0.5 + 6.0;
        (px, py, lift)
    }

    pub(super) fn owned_talisman_pos(&self, idx: usize) -> (f32, f32, f32) {
        let n = self.talisman_owned_count.max(1) as f32;
        let row_w = self.talisman_tray_extents[0] * 0.85;
        let start_x = self.talisman_tray_center_px.0 - row_w * 0.5 + (row_w / n) * 0.5;
        let px = start_x + (row_w / n) * idx as f32;
        let py = self.talisman_tray_center_px.1;
        let lift = self.talisman_tray_extents[1] + self.consumable_width * 0.5 + 6.0;
        let (ox, oy, olift) = self.owned_talisman_offset;
        (px + ox, py + oy, lift + olift)
    }
}

pub(super) fn rarity_color(rarity: Rarity) -> [f32; 4] {
    let tier = match rarity {
        Rarity::Common => 0,
        Rarity::Uncommon => 1,
        Rarity::Rare => 2,
        Rarity::Legendary => 3,
    };
    color::rarity(tier)
}

pub(super) fn relic_half_extents(id: RelicId, base: f32) -> [f32; 3] {
    let seed = (id as u32).wrapping_mul(2654435761) ^ 0x9E3779B9;
    let r0 = ((seed >> 8) & 0xFF) as f32 / 255.0;
    let r2 = ((seed >> 24) & 0xFF) as f32 / 255.0;
    let face = base * (0.65 + r0 * 0.45);
    [face, base * (0.08 + r2 * 0.04), face]
}

pub(super) fn consumable_color(c: Consumable) -> [f32; 4] {
    match c {
        Consumable::Zodiac(z) => {
            let palette = [
                [0.96, 0.62, 0.42, 1.0],
                [0.95, 0.78, 0.32, 1.0],
                [0.78, 0.42, 0.34, 1.0],
                [0.50, 0.78, 0.55, 1.0],
                [0.55, 0.62, 0.92, 1.0],
                [0.85, 0.55, 0.85, 1.0],
                [0.92, 0.46, 0.62, 1.0],
                [0.88, 0.86, 0.55, 1.0],
                [0.45, 0.72, 0.78, 1.0],
                [0.95, 0.50, 0.30, 1.0],
                [0.62, 0.85, 0.42, 1.0],
                [0.78, 0.66, 0.92, 1.0],
            ];
            palette[(z as usize) % palette.len()]
        }
        Consumable::Talisman(t) => t.accent_color(),
    }
}

pub(super) fn coin_display_layout(
    gold: u32,
    dish_center_px: (f32, f32, f32),
    dish_extents: [f32; 3],
    time: f32,
) -> (Vec<Object3d>, Vec<Object3d>) {
    if gold == 0 {
        return (Vec::new(), Vec::new());
    }

    let coin_radius = 9.0_f32;
    let coin_thickness = 3.5_f32;
    let coins_per_string: u32 = 10;
    let dish_top_y = dish_center_px.2 + dish_extents[1] + 2.0;
    let gold_color: [f32; 4] = [1.00, 0.78, 0.30, 1.0];
    let bar_color: [f32; 4] = [0.92, 0.72, 0.22, 1.0];

    let big_bars = gold / 100;
    let after_big = gold % 100;
    let mini_bars = after_big / 25;
    let coin_gold = after_big % 25;

    let total_bars = (big_bars + mini_bars) as usize;
    let n_coin_strings = if coin_gold > 0 {
        ((coin_gold - 1) / coins_per_string + 1) as usize
    } else {
        0
    };
    const MAX_SHOP_COINS: usize = 64;
    let total_coins = (coin_gold as usize).min(MAX_SHOP_COINS);
    let footprint_x = dish_extents[0] * 0.90;

    let mut bars = Vec::with_capacity(total_bars);
    if total_bars > 0 {
        let big_he: [f32; 3] = [7.0, 4.0, 5.0];
        let mini_he: [f32; 3] = [5.0, 3.0, 3.5];
        let max_per_row = ((footprint_x * 2.0) / (big_he[0] * 2.5)).floor().max(1.0) as usize;
        let bar_specs: Vec<(usize, [f32; 3])> = std::iter::repeat_n((0, big_he), big_bars as usize)
            .chain(std::iter::repeat_n((1, mini_he), mini_bars as usize))
            .collect();
        for (spec_i, (_kind, he)) in bar_specs.iter().enumerate() {
            let row = spec_i / max_per_row;
            let col = spec_i % max_per_row;
            let cols_this_row = max_per_row.min(total_bars - row * max_per_row);
            let row_width = cols_this_row as f32 * he[0] * 2.5;
            let x_off = -row_width * 0.5 + he[0] * 1.25 + col as f32 * he[0] * 2.5;
            let world_y = dish_top_y + he[1] + row as f32 * (big_he[1] * 2.0 + 1.0);
            let z_off = -dish_extents[2] * 0.25;
            let rot = 0.02 * (time * 0.5 + spec_i as f32 * 2.3).sin();
            bars.push(Object3d {
                pos: [dish_center_px.0 + x_off, dish_center_px.1 + z_off, world_y],
                extents: [he[0] * 2.0, he[1] * 2.0, he[2] * 2.0],
                rotation: rot_z_rad(rot),
                color: bar_color,
                kind: Object3dKind::Primitive {
                    shape: crate::render::primitive::MeshId::Cube,
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
    }

    let mut coins = Vec::with_capacity(total_coins);
    if total_coins > 0 {
        let z_off = if total_bars > 0 {
            dish_extents[2] * 0.25
        } else {
            0.0
        };
        let string_spacing = (coin_radius * 2.2).min(if n_coin_strings > 1 {
            (footprint_x * 2.0) / (n_coin_strings as f32)
        } else {
            coin_radius * 2.2
        });
        let row_width = n_coin_strings as f32 * string_spacing;
        let mut placed = 0u32;
        for s in 0..n_coin_strings {
            let x_off = -row_width * 0.5 + string_spacing * 0.5 + s as f32 * string_spacing;
            let coins_in_this_string = coins_per_string.min(coin_gold - placed);
            for c in 0..coins_in_this_string {
                let si = s as f32;
                let ci = c as f32;
                let sway = 0.04 * (time * 1.2 + si * 1.8).sin();
                let bob = 0.3 * (time * 0.9 + si * 2.1 + ci * 0.4).sin();
                let base_rot = si * 0.15;
                let world_y = dish_top_y + ci * coin_thickness + bob;
                coins.push(Object3d {
                    pos: [dish_center_px.0 + x_off, dish_center_px.1 + z_off, world_y],
                    extents: [coin_radius * 2.0, coin_thickness, coin_radius * 2.0],
                    rotation: rot_z_rad(base_rot + sway),
                    color: gold_color,
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
                placed += 1;
            }
        }
    }

    (bars, coins)
}

pub(super) fn shop_plaque_lines(scene: &ShopScene, shop: &ShopReadModel) -> (String, String) {
    let top = format!(
        "Shop  ·  Round {}  ·  Gold {}g",
        scene.came_from_round, shop.display_gold
    );
    (top, String::new())
}
