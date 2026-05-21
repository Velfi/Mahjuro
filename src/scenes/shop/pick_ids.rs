//! Stable `ShopHit::Dish(u32)` ids for the shop scene.
//!
//! Order: coin ([`PICK_COIN_DISH`]), journal ([`PICK_JOURNAL_BOOK`]), then
//! [`N_TILE_PACKS`] consecutive pack ids starting at [`PICK_TILE_PACK_BASE`],
//! then leave prop and reroll prop. GLB button nodes
//! (`exit_btn`, `restock_btn`) map to [`PICK_LEAVE_PROP`] / [`PICK_REROLL_PROP`] in
//! [`crate::render::picking`].

use crate::scenes::journal_transition::YAKU_JOURNAL_BOOK_PICK_ID;

pub const PICK_COIN_DISH: u32 = 2;
/// Shared with journal / gameplay book picks.
pub const PICK_JOURNAL_BOOK: u32 = YAKU_JOURNAL_BOOK_PICK_ID;

pub const PICK_TILE_PACK_BASE: u32 = 4;
pub const N_TILE_PACKS: usize = 2;

/// Leave / next-round prop — first id after the tile-pack block.
pub const PICK_LEAVE_PROP: u32 = PICK_TILE_PACK_BASE + N_TILE_PACKS as u32;
pub const PICK_REROLL_PROP: u32 = PICK_LEAVE_PROP + 1;
