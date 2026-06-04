//! Stable `ShopHit::Dish(u32)` ids for the shop scene.
//!
//! Canonical values live in [`mahjuro_types::shop_pick`]; GLB button nodes map to
//! leave/restock props in [`mahjuro_render::picking`].

pub use mahjuro_types::shop_pick::{
    N_TILE_PACKS, PICK_COIN_DISH, PICK_JOURNAL_BOOK, PICK_LEAVE_PROP, PICK_RESTOCK_PROP,
    PICK_TILE_PACK_BASE,
};
