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
