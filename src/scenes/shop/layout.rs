use super::*;

pub(super) fn live_shop_hit(
    hit: ShopHit,
    scene: &ShopScene,
    items: &[ShopItem],
    zodiac_items: &[ConsumableShopItem],
    talisman_items: &[ConsumableShopItem],
    pack_items: &[TilePackShopItem],
    shop: &ShopReadModel,
) -> Option<ShopHit> {
    let hit = match hit {
        ShopHit::EnvSpawnSlot(_) | ShopHit::EnvInvSlot(_) | ShopHit::EnvConsumableOrd(_) => {
            super::view::resolve_shop_glb_env_hit(scene, shop, hit)?
        }
        h => h,
    };

    let valid = match hit {
        ShopHit::Relic(i) => i < items.len() + shop.owned_relics.len(),
        ShopHit::Ribbon(i) => i < zodiac_items.len() + shop.owned_zodiacs.len(),
        ShopHit::Talisman(i) => i < talisman_items.len() + shop.owned_talismans.len(),
        ShopHit::Dish(id) => {
            if matches!(
                id,
                PICK_COIN_DISH | PICK_JOURNAL_BOOK | PICK_LEAVE_PROP | PICK_REROLL_PROP
            ) {
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
        ShopHit::EnvSpawnSlot(_) | ShopHit::EnvInvSlot(_) | ShopHit::EnvConsumableOrd(_) => false,
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
pub(crate) struct ShopLayout {
    pub niche_centers_px: [(f32, f32, f32); KIOSK_RELIC_SLOTS],
    pub niche_count: usize,
    pub pack_centers_px: [(f32, f32, f32); N_TILE_PACKS],
    pub relic_dish_center_px: (f32, f32, f32),
    pub relic_dish_extents: [f32; 3],
    pub coin_dish_center_px: (f32, f32, f32),
    pub owned_relic_count: usize,
    pub lamp_center_px: (f32, f32, f32),
    pub ppmm: f32,
}

impl ShopLayout {
    pub(crate) fn mm(&self, n: f32) -> f32 {
        self.ppmm * n
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ShopInventoryCounts {
    pub n_for_sale: usize,
    pub n_for_sale_talismans: usize,
    pub n_owned_relics: usize,
}

impl ShopLayout {
    pub(crate) fn build(
        layout: &crate::ui::layout::LayoutResult,
        positions: &crate::ui::scene_layout::ShopPositions,
        counts: ShopInventoryCounts,
    ) -> Self {
        let ShopInventoryCounts {
            n_for_sale,
            n_for_sale_talismans,
            n_owned_relics,
            ..
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
            clip_near: None,
            clip_far: None,
        };

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
        let coin_dish_center_px = (remap_nx(positions.coin_dish.nx), shelf_pixel_y, 0.0);

        let lamp_center_px = (
            positions.lamp.nx * w,
            positions.lamp.ny * h,
            layout.mm(positions.lamp.lift_mm),
        );

        Self {
            niche_centers_px,
            niche_count: n_niches,
            pack_centers_px,
            relic_dish_center_px,
            relic_dish_extents,
            coin_dish_center_px,
            owned_relic_count: n_owned_relics,
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
}

pub(super) fn rarity_color(rarity: Rarity) -> [f32; 4] {
    color::rarity(rarity.tier())
}

pub(super) fn relic_half_extents(id: RelicId, base: f32) -> [f32; 3] {
    let seed = (id as u32).wrapping_mul(2654435761) ^ 0x9E3779B9;
    let r0 = ((seed >> 8) & 0xFF) as f32 / 255.0;
    let r2 = ((seed >> 24) & 0xFF) as f32 / 255.0;
    let face = base * (0.65 + r0 * 0.45);
    [face, base * (0.04 + r2 * 0.02), face]
}

pub(super) fn consumable_color(c: Consumable) -> [f32; 4] {
    use crate::render::theme::color;
    match c {
        Consumable::Zodiac(z) => {
            let palette = [
                [0.96, 0.62, 0.42, 1.0],
                color::RELIC_GOLD,
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
