//! Body-shader kinds for `tile_3d.wgsl`. Tiles are always rendered from GLB
//! meshes now, so the renderer only distinguishes textured paths via
//! `tile_visual_params.w`:
//!
//! - [`TEXTURED_BASE_MAP_BODY_KIND`] for imported environment meshes
//!   (e.g. [`shop.glb`](../../../assets/3d/shop.glb)) that sample the bound
//!   base-color texture without projecting a tile decal.
//! - [`TEXTURED_TILE_GAMEPLAY_BODY_KIND`] for the gameplay `tile.glb` meshes
//!   that sample each primitive's albedo and project the mahjong face decal.

/// `tile_visual_params.w` for [`crate::room_glb`] — sample bound base color, **no**
/// projected tile decal (shop geometry shares the tile pipeline).
pub const TEXTURED_BASE_MAP_BODY_KIND: f32 = 4.0;

/// `tile_visual_params.w` for sampling each GLB primitive's albedo while still projecting the
/// mahjong face decal (`tile_3d.wgsl` kind 5). Every gameplay tile uses this kind.
pub const TEXTURED_TILE_GAMEPLAY_BODY_KIND: f32 = 5.0;
