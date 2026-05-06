//! Procedural **tile body** shading for `tile_3d.wgsl`. Orthogonal to
//! [`crate::persistence::TileMaterial`] (gameplay bonuses / labels): the renderer
//! resolves a [`TileBodyShaderKind`] from material so gameplay bonuses and UI
//! stay aligned with the chosen [`crate::persistence::TileMaterial`].
//!
//! `tile_3d.wgsl` also recognizes [`TEXTURED_BASE_MAP_BODY_KIND`] for imported meshes
//! (e.g. [`Shop.glb`](../../../assets/Shop.glb)) that should sample the bound base-color
//! texture instead of procedural tile bodies.

use crate::persistence::TileMaterial;

/// `base_color_factor.w` for [`crate::render::shop_glb`] — sample bound base color, **no**
/// projected tile decal (shop geometry shares the tile pipeline).
pub const TEXTURED_BASE_MAP_BODY_KIND: f32 = 4.0;

/// `base_color_factor.w` when [`crate::render::tile_glb::load_glb_tile_from_bytes`] produced
/// mesh primitives: sample each primitive's albedo and still project the mahjong face decal on
/// the front face (multi-material `Tile.glb` exports).
pub const TEXTURED_TILE_GAMEPLAY_BODY_KIND: f32 = 5.0;

/// Values passed to the GPU as `base_color_factor.w` (float equals discriminant).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TileBodyShaderKind {
    Bamboo = 0,
    Plastic = 1,
    TortoiseShell = 2,
}

impl TileBodyShaderKind {
    #[inline]
    pub fn id(self) -> f32 {
        self as u8 as f32
    }

    pub fn resolve(tile_material: TileMaterial) -> Self {
        match tile_material {
            TileMaterial::Bamboo => Self::Bamboo,
            TileMaterial::Plastic => Self::Plastic,
            TileMaterial::TortoiseShell => Self::TortoiseShell,
        }
    }
}
