//! Screen-space scale for consumable props (shop inventory + gameplay dish).
//!
//! Keep factors in sync with [`crate::scenes::shop::view`] shelf/inventory draws.

use crate::ribbon_mesh::ribbon_length_fitting_rect;

/// Applied after the slot-width factor for talisman tablets (`tw * 1.15`).
pub const TALISMAN_EXTENT_MUL: f32 = 1.15;

/// Owned inventory talisman draw (`tw = slot_w * 0.72`).
pub const OWNED_TALISMAN_W_FRAC: f32 = 0.72;

/// Owned inventory ribbon envelope inside the slot rect.
pub const OWNED_RIBBON_W_FRAC: f32 = 0.36;
pub const OWNED_RIBBON_H_FRAC: f32 = 0.58;

/// For-sale shelf talisman (`tw = slot_w * 0.80`).
pub const FOR_SALE_TALISMAN_W_FRAC: f32 = 0.80;

/// For-sale shelf ribbon envelope.
pub const FOR_SALE_RIBBON_W_FRAC: f32 = 0.38;
pub const FOR_SALE_RIBBON_H_FRAC: f32 = 0.62;

#[inline]
pub fn talisman_tablet_extent(slot_w: f32, w_frac: f32) -> f32 {
    slot_w * w_frac * TALISMAN_EXTENT_MUL
}

#[inline]
pub fn owned_talisman_tablet_extent(slot_w: f32) -> f32 {
    talisman_tablet_extent(slot_w, OWNED_TALISMAN_W_FRAC)
}

#[inline]
pub fn for_sale_talisman_tablet_extent(slot_w: f32) -> f32 {
    talisman_tablet_extent(slot_w, FOR_SALE_TALISMAN_W_FRAC)
}

#[inline]
pub fn ribbon_length_in_slot(slot_w: f32, slot_h: f32, w_frac: f32, h_frac: f32) -> f32 {
    ribbon_length_fitting_rect(slot_w * w_frac, slot_h * h_frac)
}

#[inline]
pub fn owned_ribbon_length(slot_w: f32, slot_h: f32) -> f32 {
    ribbon_length_in_slot(slot_w, slot_h, OWNED_RIBBON_W_FRAC, OWNED_RIBBON_H_FRAC)
}

#[inline]
pub fn for_sale_ribbon_length(slot_w: f32, slot_h: f32) -> f32 {
    ribbon_length_in_slot(
        slot_w,
        slot_h,
        FOR_SALE_RIBBON_W_FRAC,
        FOR_SALE_RIBBON_H_FRAC,
    )
}
