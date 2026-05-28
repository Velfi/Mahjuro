//! Outgoing shelf animation when the player restocks the shop.

use std::time::Instant;

use crate::core::consumable::Consumable;
use crate::core::relic::{Rarity, RelicId};
use crate::core::talisman::TalismanKind;
use crate::core::tile_pack::TilePackKind;
use crate::core::zodiac::ZodiacKind;

use super::{ConsumableShopItem, ShopFocus, ShopItem, ShopScene, TilePackShopItem};

/// Delay between adjacent spawn slots (left → right).
pub(super) const RESTOCK_EXIT_STAGGER: f32 = 0.042;
/// Screen-down travel (anchor py) before an entry is culled.
pub(super) const RESTOCK_EXIT_OFFSCREEN_PY_FRAC: f32 = 0.58;

pub(super) struct PendingShopStock {
    pub items: Vec<ShopItem>,
    pub zodiac_items: Vec<ConsumableShopItem>,
    pub talisman_items: Vec<ConsumableShopItem>,
    pub pack_items: Vec<TilePackShopItem>,
    pub focus: Option<ShopFocus>,
}

/// One restock’s worth of shelf items still falling after new stock is live.
pub(super) struct ShopDepartingBatch {
    pub started_at: Instant,
    pub entries: Vec<DepartingShelfEntry>,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum DepartingShelfEntry {
    Relic {
        slot_i: usize,
        relic: RelicId,
        rarity: Rarity,
    },
    Pack {
        slot_i: usize,
        kind: TilePackKind,
    },
    Ribbon {
        slot_i: usize,
        zodiac: ZodiacKind,
    },
    Talisman {
        slot_i: usize,
        kind: TalismanKind,
    },
}

impl DepartingShelfEntry {
    #[inline]
    pub(super) fn slot_i(self) -> usize {
        match self {
            Self::Relic { slot_i, .. }
            | Self::Pack { slot_i, .. }
            | Self::Ribbon { slot_i, .. }
            | Self::Talisman { slot_i, .. } => slot_i,
        }
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct RestockExitMeshDelta {
    pub pos_offset: [f32; 3],
    pub rot_add: [f32; 3],
    pub alpha_mul: f32,
}

impl ShopScene {
    #[inline]
    pub(super) fn departing_stock_active(&self) -> bool {
        !self.departing_stock.is_empty()
    }

    /// Back-compat name used across shop input/layout guards (reroll only).
    #[inline]
    pub(super) fn restock_exit_active(&self) -> bool {
        self.departing_stock_active()
    }

    pub(super) fn tick_departing_stock(&mut self, now: Instant, window_h: f32) {
        if self.departing_stock.is_empty() {
            return;
        }
        let h = window_h.max(1.0);
        self.departing_stock.retain_mut(|batch| {
            batch.entries.retain(|entry| {
                !restock_exit_offscreen_for_slot(entry.slot_i(), h, batch.started_at, now)
            });
            !batch.entries.is_empty()
        });
    }

    pub(super) fn apply_pending_shop_stock(&mut self, pending: PendingShopStock) {
        self.items = pending.items;
        self.zodiac_items = pending.zodiac_items;
        self.talisman_items = pending.talisman_items;
        self.pack_items = pending.pack_items;
        self.focus = pending.focus;
    }

    pub(super) fn begin_restock_exit(
        &mut self,
        pending: PendingShopStock,
        now: Instant,
        animate: bool,
    ) {
        let entries = if animate {
            snapshot_departing_for_sale(self)
        } else {
            Vec::new()
        };
        self.apply_pending_shop_stock(pending);
        if !entries.is_empty() {
            self.departing_stock.push(ShopDepartingBatch {
                started_at: now,
                entries,
            });
        }
    }
}

pub(super) fn snapshot_departing_for_sale(scene: &ShopScene) -> Vec<DepartingShelfEntry> {
    let sale = super::view::for_sale_slots(scene);
    let mut out = Vec::new();
    for (slot_i, foc_opt) in sale.iter().enumerate() {
        let Some(foc) = foc_opt else {
            continue;
        };
        match foc {
            ShopFocus::Relic(i) if *i < scene.items.len() => {
                let item = &scene.items[*i];
                if !item.sold {
                    out.push(DepartingShelfEntry::Relic {
                        slot_i,
                        relic: item.relic,
                        rarity: item.rarity,
                    });
                }
            }
            ShopFocus::Pack(pid) => {
                let k = (*pid - super::PICK_TILE_PACK_BASE) as usize;
                if let Some(pack) = scene.pack_items.get(k)
                    && !pack.sold
                {
                    out.push(DepartingShelfEntry::Pack {
                        slot_i,
                        kind: pack.kind,
                    });
                }
            }
            ShopFocus::Ribbon(i) if *i < scene.zodiac_items.len() => {
                let item = &scene.zodiac_items[*i];
                if !item.sold
                    && let Consumable::Zodiac(z) = item.consumable
                {
                    out.push(DepartingShelfEntry::Ribbon { slot_i, zodiac: z });
                }
            }
            ShopFocus::Talisman(i) if *i < scene.talisman_items.len() => {
                let item = &scene.talisman_items[*i];
                if !item.sold
                    && let Consumable::Talisman(tk) = item.consumable
                {
                    out.push(DepartingShelfEntry::Talisman { slot_i, kind: tk });
                }
            }
            _ => {}
        }
    }
    out
}

#[inline]
pub(super) fn restock_exit_offscreen_for_slot(
    slot_i: usize,
    h: f32,
    started_at: Instant,
    now: Instant,
) -> bool {
    let delta = restock_exit_mesh_delta(slot_i, h, started_at, now);
    delta.pos_offset[1] >= h * RESTOCK_EXIT_OFFSCREEN_PY_FRAC || delta.alpha_mul <= 0.01
}

/// Gravity-style ease for tip/roll only (0..1).
#[inline]
fn fall_ease(t: f32) -> f32 {
    (t.clamp(0.0, 1.0)).powi(3)
}

/// Unbounded screen-down drop — items keep falling until culled off-screen.
pub(super) fn restock_exit_mesh_delta(
    slot_i: usize,
    h: f32,
    started_at: Instant,
    now: Instant,
) -> RestockExitMeshDelta {
    let elapsed = now.saturating_duration_since(started_at).as_secs_f32();
    let slot_delay = slot_i as f32 * RESTOCK_EXIT_STAGGER;
    let t = (elapsed - slot_delay).max(0.0);
    if t <= 0.0 {
        return RestockExitMeshDelta {
            alpha_mul: 1.0,
            ..Default::default()
        };
    }
    let scale = h / 1080.0;
    // py anchor units — quadratic gravity, no end cap.
    let py = 0.5 * 520.0 * scale * t * t;
    let tip_phase = fall_ease((t / 0.35).min(1.0));
    let tip = tip_phase * 0.18;
    let roll = (elapsed * 9.5 + slot_i as f32 * 0.7).sin() * 0.09 * (0.35 + tip_phase * 0.65);
    let lateral = (slot_i as f32 - 4.0) * h * 0.005 * (t / 0.5).min(1.2);
    let alpha_mul = if py < h * 0.36 {
        1.0
    } else {
        ((h * RESTOCK_EXIT_OFFSCREEN_PY_FRAC - py) / (h * 0.14)).clamp(0.0, 1.0)
    };
    RestockExitMeshDelta {
        pos_offset: [lateral, py, -h * 0.05 * (t / 0.6).min(1.5)],
        rot_add: [tip, 0.0, roll],
        alpha_mul,
    }
}

#[inline]
pub(super) fn apply_restock_exit_to_mesh(
    mesh: &mut crate::render::draw_cmd::Object3d,
    delta: RestockExitMeshDelta,
) {
    mesh.pos[0] += delta.pos_offset[0];
    mesh.pos[1] += delta.pos_offset[1];
    mesh.pos[2] += delta.pos_offset[2];
    mesh.rotation = super::view::euler_rad_add(mesh.rotation, delta.rot_add);
    mesh.color[3] *= delta.alpha_mul;
}
