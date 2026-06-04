//! Renderer-wide numeric limits and clamp helpers.

/// Maximum width/height for window-backed surfaces and derived HDR targets.
/// Caps bogus platform `Resized` values that can otherwise allocate tens of GB.
pub(crate) const MAX_RENDER_DIMENSION: u32 = 8192;

/// Scene/depth/bloom allocation size from window size and [`mahjuro_gfx_types::GraphicsMode::render_scale`].
/// Preserves the window aspect ratio when enforcing minimum dimensions.
pub(crate) fn scaled_render_size(
    window: crate::physical_size::PhysicalSize,
    scale: f32,
) -> crate::physical_size::PhysicalSize {
    let scale = scale.clamp(0.5, 1.0);
    let aspect = window.width as f32 / window.height.max(1) as f32;
    let mut w = ((window.width as f32) * scale).round() as u32;
    let mut h = ((window.height as f32) * scale).round() as u32;
    w = w.clamp(1, window.width);
    h = h.clamp(1, window.height);

    let min_w = mahjuro_gfx_types::MIN_RENDER_WIDTH.min(window.width);
    let min_h = mahjuro_gfx_types::MIN_RENDER_HEIGHT.min(window.height);
    if w < min_w {
        w = min_w;
        h = (w as f32 / aspect).round().clamp(1.0, window.height as f32) as u32;
    }
    if h < min_h {
        h = min_h;
        w = (h as f32 * aspect).round().clamp(1.0, window.width as f32) as u32;
    }

    if w != window.width || h != window.height {
        log::debug!(
            "internal render size {}×{} (window {}×{}, scale={scale})",
            w,
            h,
            window.width,
            window.height,
        );
    }
    crate::physical_size::PhysicalSize::new(w, h)
}

pub(crate) fn clamp_render_physical_size(
    size: crate::physical_size::PhysicalSize,
) -> crate::physical_size::PhysicalSize {
    let w = size.width.clamp(1, MAX_RENDER_DIMENSION);
    let h = size.height.clamp(1, MAX_RENDER_DIMENSION);
    if w != size.width || h != size.height {
        log::warn!(
            "clamping render size {}×{} to {}×{} (MAX_RENDER_DIMENSION={})",
            size.width,
            size.height,
            w,
            h,
            MAX_RENDER_DIMENSION
        );
    }
    crate::physical_size::PhysicalSize::new(w, h)
}

/// Maximum number of point lights uploaded each frame. Must match the array
/// length in tile_3d.wgsl.
pub const MAX_POINT_LIGHTS: usize = 16;

/// Maximum number of spotlights uploaded each frame. Must match the array
/// length in `tile_3d.wgsl` and `lit_mesh.wgsl`. Bound as render pipeline
/// group 3 for tiles and lit meshes (table / Object3d).
pub const MAX_SPOT_LIGHTS: usize = 8;

/// Maximum number of analytic tile occluders uploaded for the candle-pool
/// shadow tests in `lit_mesh.wgsl`. One per visible hand tile, conservatively
/// sized so the full hand fits.
pub const MAX_TILE_OCCLUDERS: usize = 16;

pub(crate) const MAX_SHOWCASE_TILE_SLOTS: usize = 160;

/// Frames an unused entry stays in `text_label_cache` before eviction.
pub(crate) const TEXT_CACHE_TTL_FRAMES: u64 = 120;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaled_render_size_075_at_1080p() {
        let window = crate::physical_size::PhysicalSize::new(1920, 1080);
        let rs = scaled_render_size(window, 0.75);
        assert_eq!(rs.width, 1440);
        assert_eq!(rs.height, 810);
    }

    #[test]
    fn scaled_render_size_min_clamp_preserves_aspect() {
        let window = crate::physical_size::PhysicalSize::new(1440, 900);
        let rs = scaled_render_size(window, 0.75);
        let window_aspect = window.width as f32 / window.height as f32;
        let rs_aspect = rs.width as f32 / rs.height as f32;
        assert!((rs_aspect - window_aspect).abs() < 0.02);
        assert!(rs.width <= window.width);
        assert!(rs.height <= window.height);
    }
}

// Tile-mesh local extents (after `normalize_mesh` in tile_glb.rs):
//   local X — long face axis  (extent ~1.000) → table-Z (front-back)
//   local Y — thickness        (extent ~0.424) → world Y (up off table)
//   local Z — short face axis  (extent ~0.734) → table-X (left-right)
pub(crate) const LOCAL_X_EXTENT: f32 = 1.000;
pub(crate) const LOCAL_Y_EXTENT: f32 = 0.424;
pub(crate) const LOCAL_Z_EXTENT: f32 = 0.734;

pub const MAIN_MENU_PICK_PLAY: u32 = 240;
pub const MAIN_MENU_PICK_OPTIONS: u32 = 241;
pub const MAIN_MENU_PICK_QUIT: u32 = 242;
/// Main-menu 3D moon (`MoonObject`) — toggles the "quit that" quip bubble.
pub const MAIN_MENU_PICK_MOON: u32 = 243;

/// Maximum number of physical relic placeholders rendered in one batch. Must
/// match the size of the `relic_instances` slot pool below; the renderer
/// silently truncates batches longer than this.
pub const MAX_RELIC_SLOTS: usize = 128;
/// Archive boss-tab cubbies + pedestal (one frame, all slots visible).
pub const MAX_ORDEAL_ICON_SLOTS: usize = 32;
/// Maximum number of zodiac/talisman ribbon *draw slots* per frame (across all
/// `ZodiacBatch` cmds). Each textured ribbon uses up to 3 slots (top/mid/bot
/// caps), so 16 logical ribbons × 3 = 48. Truncated silently.
pub const MAX_RIBBON_SLOTS: usize = 48;
/// Maximum number of talisman tablets rendered per frame (archive: 21 cubbies + pedestal).
pub const MAX_TALISMAN_SLOTS: usize = 24;

/// `talisman_slot_kind` values `>=` this bind memorial height + mask views.
pub const MEMORIAL_TALISMAN_TEXTURE_BASE: u8 = 128;
/// Maximum number of 3D moths (main-menu door light) rendered per frame.
/// Each live bug consumes one slot for body + two for live wings + two for
/// blur-fan surrogates (L/R). The blur-fan slot pools share this same size.
pub const MAX_BUG_SLOTS: usize = 8;
/// Maximum yen / flying coins using [`coin.glb`](../../../assets/3d/coin.glb) PBR meshes.
pub const MAX_COIN_GLTF_SLOTS: usize = 64;
/// Maximum number of material-preview orbs rendered per frame. Only the
/// material viewer debug scene uses these; 32 covers every `MaterialKind`
/// with room to grow.
pub const MAX_ORB_SLOTS: usize = 32;
/// Maximum number of yaku tablets per frame (5 visible + headroom).
pub const MAX_YAKU_TABLET_SLOTS: usize = 12;
/// Maximum number of wood action tablets per frame (cash-in).
pub const MAX_WOOD_TABLET_SLOTS: usize = 8;
/// Maximum number of leather books per frame (shop uses 1: journal).
pub const MAX_BOOK_SLOTS: usize = 2;
/// Maximum number of bowls per frame (gameplay uses 1: discard).
pub const MAX_BOWL_SLOTS: usize = 2;
/// Maximum number of bronze mirrors per frame (gameplay uses 1: play hand).
pub const MAX_MIRROR_SLOTS: usize = 2;
/// Maximum number of tally fans per frame (gameplay uses 2: draws + discards).
pub const MAX_TALLY_FAN_SLOTS: usize = 2;
/// Maximum total number of tally sticks rendered per frame across all fans.
/// Each fan emits `count` base sticks plus `count` tip-cap overlays, so this
/// bound is on the sum of both.
pub const MAX_TALLY_STICK_SLOTS: usize = 32;
/// Maximum number of facedown wall tiles drawn at the back of the table.
pub const MAX_WALL_TILE_SLOTS: usize = 80;
/// Maximum number of in-flight 3D extruded-glyph score popups. A single
/// cascade rarely fires more than 8-10 steps, so 32 is plenty for the
/// per-step popups plus the running-total readout that holds across the
/// final beat.
/// Score reel uses up to 2 × N_COLS slots (prev + current per spinning column)
/// plus popup labels. 48 gives headroom for reel overflow columns.
pub const MAX_EXTRUDED_GLYPH_SLOTS: usize = 80;
