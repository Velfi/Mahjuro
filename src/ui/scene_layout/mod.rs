//! Serializable scene-layout position data.
//!
//! Every manually-placeable object is a [`Placement`] with a single
//! consistent coordinate system:
//!
//! - `nx`, `ny` — normalized window fractions (0 = left/top, 1 = right/bottom)
//! - `lift_mm` — physical millimeters above the felt, converted to world units
//!   via [`crate::ui::layout::LayoutResult::mm`]
//! - `rx_deg`, `ry_deg`, `rz_deg` — rotation in degrees (Z → Y → X order)
//!
//! Anchor-relative placements (plaque, coin pile, hand strip, yaku tablet,
//! action-bar bowl/mirror) still use the same units — the scene interprets
//! their `nx`/`ny` as fractional *offsets* against a Cassowary-derived anchor
//! rather than absolute screen positions, but the unit system is identical.
//!
//! ## Save / load
//!
//! Positions load from JSON in the app's config directory at startup.
//! Missing files or fields fall back to compiled-in [`Default`] values, so
//! shipping requires no JSON files.
//!
//! ## Arrange mode
//!
//! Both `ShopPositions` and `GameplayPositions` implement
//! [`crate::ui::placement::ArrangeTarget`]; the generic
//! [`crate::ui::placement::apply_arrange`] handler nudges any registered
//! placement by name. The debug menu discovers placements by iterating
//! the known names.

mod collection;
mod fs;
mod gameplay;
mod shop;
mod start_screen;
mod tutorial;

#[allow(unused_imports)]
pub use collection::{
    COLLECTION_HIERARCHY, CollectionField, CollectionPositions, load_collection_positions,
    sanitize_collection_positions, save_collection_positions,
};
pub use gameplay::{
    GAMEPLAY_HIERARCHY, GameplayField, GameplayPositions, load_gameplay_positions,
    sanitize_gameplay_positions, save_gameplay_positions,
};
pub use shop::{
    CANONICAL_WINDOW_W, HFRAC_TO_MM, SHOP_HIERARCHY, ShopField, ShopPositions, load_shop_positions,
    sanitize_shop_positions, save_shop_positions,
};
#[allow(unused_imports)]
pub use start_screen::{
    START_SCREEN_HIERARCHY, StartScreenField, StartScreenPositions, load_start_screen_positions,
    sanitize_start_screen_positions, save_start_screen_positions,
};
#[allow(unused_imports)]
pub use tutorial::{
    TUTORIAL_HIERARCHY, TutorialField, TutorialPositions, load_tutorial_positions,
    sanitize_tutorial_positions, save_tutorial_positions,
};

#[cfg(test)]
mod tests;
