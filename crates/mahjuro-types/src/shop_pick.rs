//! Stable `ShopHit::Dish(u32)` ids shared by the shop scene and renderer picking.

pub const YAKU_JOURNAL_BOOK_PICK_ID: u32 = 3;
pub const PICK_COIN_DISH: u32 = 2;
pub const PICK_JOURNAL_BOOK: u32 = YAKU_JOURNAL_BOOK_PICK_ID;
pub const PICK_TILE_PACK_BASE: u32 = 4;
pub const N_TILE_PACKS: usize = 2;
pub const PICK_LEAVE_PROP: u32 = PICK_TILE_PACK_BASE + N_TILE_PACKS as u32;
pub const PICK_REROLL_PROP: u32 = PICK_LEAVE_PROP + 1;
