use crate::core::tile::Suit;

/// A relic icon to draw as a textured quad at a screen-space rect.
/// (Struct retained for draw payloads; fields unused since glossary relic hovers were removed.)
#[allow(dead_code)]
pub struct RelicIcon {
    /// Position in screen pixels: [x, y, w, h].
    pub rect: [f32; 4],
    /// Which relic image to display.
    pub relic_id: crate::core::relic::RelicId,
}

/// Horizontal alignment of text inside its rect.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum TextAlign {
    Left,
    #[default]
    Center,

    Right,
}

/// A rasterized text label to draw over a screen-space rect.
///
/// `font_px = None` falls back to the legacy auto-shrink behaviour where
/// `rasterize_label` picks `min(rect.h * 0.55, rect.w * 1.5 / chars)`. Set
/// `font_px = Some(px)` to pin the font size — this is what `push_text_block`
/// uses to keep every wrapped line of a paragraph at a consistent size.
///
/// `text` may contain `\n` for explicit line breaks; the rasteriser
/// stacks lines vertically and applies the chosen alignment to each line.
///
/// When `bold`, `italic`, or `underline` is set (and `flavor_spans` is `None`),
/// the renderer uses the multi-face CPU raster path. [`text_effect`](crate::render::text_effect::TextEffectId)
/// is applied in the fragment shader and does not change the raster cache key.
pub struct TextLabel {
    /// Position in screen pixels: [x, y, w, h].
    pub rect: [f32; 4],
    /// Text to render. May contain `\n` for hard line breaks.
    pub text: String,
    /// Colour for the text glyphs (default: white).
    pub color: [f32; 4],
    /// Pinned font size in pixels. If `None`, the renderer auto-sizes from
    /// the rect's dimensions (legacy behaviour).
    pub font_px: Option<f32>,
    /// Horizontal alignment within the rect.
    pub align: TextAlign,
    /// Legacy flag from the removed glossary-hover pass; kept so call sites
    /// stay stable.
    #[allow(dead_code)]
    pub no_glossary: bool,
    /// Horizontal scroll offset in pixels (for marquee-style text).
    /// Shifts the rasterised text leftward by this many pixels so the
    /// caller can animate it for overflow text.  Default 0.0.
    pub scroll_offset: f32,
    /// Relic inspect flavor only: when non-empty, rasterized with mixed
    /// regular/italic and faux-bold; `text` is not drawn (cache uses
    /// [`crate::core::relic::flavor_spans_cache_key`]).
    pub flavor_spans: Option<&'static [crate::core::relic::RelicFlavorSpan]>,
    /// Faux-bold / italic face when `flavor_spans` is `None`.
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    /// Fragment shader preset (rainbow, pulse, …). Ignored for image quads.
    pub text_effect: crate::render::text_effect::TextEffectId,
    /// Clockwise quarter-turns applied when drawing (0 = 0°, 1 = 90°, …).
    /// When odd, the label is rasterized with swapped width/height so auto-fit
    /// fills the on-screen rect after rotation.
    pub rotation_quarters: u8,
    /// Added to the single-line raster baseline (negative nudges glyphs up in
    /// the texture short axis). Used for optical centering (e.g. hallway PLAY).
    pub baseline_shift_px: f32,
}

impl Default for TextLabel {
    fn default() -> Self {
        Self {
            rect: [0.0; 4],
            text: String::new(),
            color: [1.0; 4],
            font_px: None,
            align: TextAlign::Center,
            no_glossary: false,
            scroll_offset: 0.0,
            flavor_spans: None,
            bold: false,
            italic: false,
            underline: false,
            text_effect: crate::render::text_effect::TextEffectId::Flat,
            rotation_quarters: 0,
            baseline_shift_px: 0.0,
        }
    }
}

/// Outer cache key for a rasterized text label: the "shape" of the label —
/// font kind, integer-quantized font_px, rect width/height in px, alignment,
/// integer-quantized scroll offset. Cheap to hash, no allocation. The inner
/// map is keyed on the text string itself, so hit-path lookups can borrow
/// `&str` and avoid allocating a `String` to probe with.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct TextLabelShapeKey {
    /// Font kind: `false` = ui_font, `true` = emoji-fallback path.
    pub emoji_path: bool,
    /// Relic flavor span raster path (italic + faux-bold).
    pub flavor_spans: bool,
    /// Bold / italic / underline raster path (non-flavor).
    pub inline_face_bits: u8,
    /// Font size in px, quantized to int. `None` means auto-size from rect.
    pub font_px: Option<u32>,
    /// Rasterized texture width in px (rect.w clamped to [1, 16384]).
    pub width_px: u32,
    /// Rasterized texture height in px (rect.h clamped to [1, 16384]).
    pub height_px: u32,
    pub align: TextAlign,
    /// Scroll offset in px, quantized to int. Distinct values still rasterize
    /// distinct entries; identical-offset frames collide.
    pub scroll_offset_px: i32,
    pub rotation_quarters: u8,
    /// `baseline_shift_px * 8`, rounded — cache key for raster vertical nudge.
    pub baseline_shift_q: i16,
}

/// Cached GPU resources for a rasterized text label. The texture is owned
/// here; the `bind_group` is cloned out into per-frame `TextDraw`s.
pub(crate) struct CachedTextLabel {
    #[allow(dead_code)]
    pub tex: wgpu::Texture,
    pub bind_group: wgpu::BindGroup,
    /// Last frame on which this entry was used, used for TTL eviction.
    pub last_used: u64,
}

/// GPU resources for a single hand tile.
///
/// Each tile has its own uniform buffer (updated every frame with the per-tile
/// model matrix) and bind group (holds the tile's rasterised decal texture).
/// Storing them per-tile means all 14 `write_buffer` calls target distinct
/// GPU allocations, so every tile's matrix is visible when the command buffer
/// executes — no dynamic-offset trickery required.
#[allow(dead_code)]
pub(crate) struct HandTileGpu {
    /// Written every frame with view_proj + model + base_color_factor.
    pub uniform_buffer: wgpu::Buffer,
    /// Per-tile shadow caster uniform (light_view_proj * model). Written
    /// every frame in lockstep with `uniform_buffer` and consumed by the
    /// shadow pre-pass via `shadow_bind_group`.
    pub shadow_uniform_buffer: wgpu::Buffer,
    pub shadow_bind_group: wgpu::BindGroup,
    /// Cached to skip re-rasterisation when the tile hasn't changed.
    pub tile_id: (Suit, u8, Option<crate::core::tile::TileEnhancement>, bool),
}

/// GPU resources for a showcase tile (pack celebration, hand strip, choose-tiles grid, etc.).
pub(crate) struct ShowcaseTileGpu {
    /// Sub-rect within `showcase_decal_atlas` for this tile identity (see `tile_3d.wgsl`).
    pub decal_atlas_uv: [f32; 4],
    pub uniform_buffer: wgpu::Buffer,
    pub bind_groups: Vec<wgpu::BindGroup>,
    pub shadow_uniform_buffer: wgpu::Buffer,
    pub shadow_bind_group: wgpu::BindGroup,
    /// Last-uploaded shadow caster uniform — skips writes + shadow depth pass when static.
    pub cached_shadow_caster: crate::render::lit_mesh::ShadowCasterUniform,
    /// Cache key to skip re-rasterisation when the tile hasn't changed.
    pub tile_id: (Suit, u8, Option<crate::core::tile::TileEnhancement>, bool),
}

/// GPU uniforms + bind groups for the imported [`Shop.glb`](../../assets/3d/Shop.glb) environment mesh.
/// Uses the same tile textured pipeline as hand tiles; vertices are already in world space (`model = I`).
pub(crate) struct ShopEnvironmentGpu {
    pub uniform_buffer: wgpu::Buffer,
    /// Shared by all room primitives; pick-blind uploads [`crate::render::hallway_glb::HallwayDistortion`].
    pub distortion_buffer: wgpu::Buffer,
    pub bind_groups: Vec<wgpu::BindGroup>,
    /// Archive room only: CPU-updated decal atlas at `@group(0) @binding(3)`.
    pub archive_sign_decal_texture: Option<wgpu::Texture>,
}

pub(crate) struct TileFaceOverlayGpu {
    pub _texture: wgpu::Texture,
    pub bind_group: wgpu::BindGroup,
}

/// A tile animating away from the hand. Two-phase trajectory: an
/// **Arcing** phase that throws the tile from its hand slot down into the
/// discard river, followed by a **Drifting** phase where the tile rides
/// the current along the channel and fades out. The split is what reads
/// as "throw the tile away" vs the previous fly-off-the-table arc.
pub(crate) struct DepartingTile {
    /// Visual identity for rendering.
    pub symbol: String,
    pub suit_emoji: String,
    pub suit_color: [f32; 4],
    /// Screen-space rect at the moment of departure (top-left + size).
    pub start_rect: [f32; 4],
    /// Splash point — center of the river rect at spawn time, with a
    /// small per-tile jitter so multiple tiles land at slightly different
    /// spots instead of stacking pixel-perfect.
    pub river_target: (f32, f32),
    /// Pixel direction the tile drifts after splashing. Currently
    /// hard-coded to +X (the river flows left-to-right in screen space).
    /// Stored as a unit vector so the render path can change the river
    /// orientation without touching the simulation.
    pub drift_dir: (f32, f32),
    /// Per-tile drift speed in pixels/sec.
    pub drift_speed: f32,
    /// Phase 1 duration (seconds) — how long the arc-into-river takes.
    pub arc_dur: f32,
    /// Phase 2 duration (seconds) — how long the tile drifts before
    /// fading out. The total visible lifetime is `arc_dur + drift_dur`.
    pub drift_dur: f32,
    /// Seconds elapsed since departure started.
    pub elapsed: f32,
    /// Total lifetime convenience field — equals `arc_dur + drift_dur`.
    /// Kept so the existing `retain(|t| t.elapsed < t.lifetime)` cull and
    /// the gameplay-scene refill timer (which reads
    /// `cascade_tuning.depart_lifetime_ms`) keep working.
    pub lifetime: f32,
}
