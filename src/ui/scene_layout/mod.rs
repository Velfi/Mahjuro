//! Scene-layout position data (compiled defaults in Rust).
//!
//! Every manually-placeable object is a [`Placement`] with a single
//! consistent coordinate system:
//!
//! - `nx`, `ny` — normalized window fractions (0 = left/top, 1 = right/bottom)
//! - `lift_mm` — physical millimeters above the felt, converted to world units
//!   via [`crate::ui::layout::LayoutResult::mm`]
//! - `rx_deg`, `ry_deg`, `rz_deg` — rotation in degrees (Z → Y → X order)

mod collection;
mod gameplay;
mod main_menu_exterior;
mod rotations;
mod shop;
mod tile_select;
mod tutorial;

pub use collection::CollectionPositions;
pub use gameplay::GameplayPositions;
pub use main_menu_exterior::MainMenuExteriorPositions;
pub use shop::ShopPositions;
pub use tile_select::TileSelectPositions;
pub use tutorial::TutorialPositions;
