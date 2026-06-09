use super::uniforms::Tile3dInstance;

/// Horizontal alignment of text inside its rect.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum TextAlign {
    Left,
    #[default]
    Center,

    Right,
}

/// Vertical placement of a multi-line block inside its rect.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum TextBlockVerticalAlign {
    /// Bottom of the rect (relic flavor band, bottom captions).
    #[default]
    Bottom,
    /// Top of the rect (stairway flavor at the top of the screen).
    Top,
}

/// A rasterized text label to draw over a screen-space rect.
///
/// `font_px = None` falls back to the legacy auto-shrink behaviour where
/// `rasterize_label` picks `min(rect.h * 0.55, rect.w * 1.5 / chars)`. Set
/// `font_px = Some(px)` to pin the maximum font size — the rasterizer shrinks
/// uniformly (down to the readable floor) when the string would overflow the
/// rect's width or height.
///
/// `text` may contain `\n` for explicit line breaks; the rasteriser
/// stacks lines vertically and applies the chosen alignment to each line.
///
/// When `bold`, `italic`, or `underline` is set (and `flavor_spans` is `None`),
/// the renderer uses the multi-face CPU raster path. [`text_effect`](crate::text_effect::TextEffectId)
/// is applied in the fragment shader and does not change the raster cache key.
#[derive(Clone)]
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
    /// Vertical placement for multi-line blocks (flavor spans and `\n` text).
    pub block_vertical_align: TextBlockVerticalAlign,
    /// Horizontal scroll offset in pixels (for marquee-style text).
    /// Shifts the rasterised text leftward by this many pixels so the
    /// caller can animate it for overflow text.  Default 0.0.
    pub scroll_offset: f32,
    /// Relic inspect flavor only: when non-empty, rasterized with mixed
    /// regular/italic and faux-bold; `text` is not drawn (cache uses
    /// [`mahjuro_core::core::relic::flavor_spans_cache_key`]).
    pub flavor_spans: Option<&'static [mahjuro_core::core::relic::RelicFlavorSpan]>,
    /// Faux-bold / italic face when `flavor_spans` is `None`.
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    /// Fragment shader preset (rainbow, pulse, …). Ignored for image quads.
    pub text_effect: crate::text_effect::TextEffectId,
    /// Clockwise quarter-turns applied when drawing (0 = 0°, 1 = 90°, …).
    /// When odd, the label is rasterized with swapped width/height so auto-fit
    /// fills the on-screen rect after rotation.
    pub rotation_quarters: u8,
    /// Added to the single-line raster baseline (negative nudges glyphs up in
    /// the texture short axis). Used for optical centering (e.g. hallway PLAY).
    pub baseline_shift_px: f32,
    /// Optional clip rectangle in screen pixels `[x, y, w, h]`.
    /// When set, the renderer applies a scissor rect for this label draw.
    pub clip_rect: Option<[f32; 4]>,
    /// Use [`crate::decal::load_mono_font`] (Xanh Mono) instead of Instrument Serif.
    pub mono: bool,
}

impl Default for TextLabel {
    fn default() -> Self {
        Self {
            rect: [0.0; 4],
            text: String::new(),
            color: [1.0; 4],
            font_px: None,
            align: TextAlign::Center,
            block_vertical_align: TextBlockVerticalAlign::Bottom,
            scroll_offset: 0.0,
            flavor_spans: None,
            bold: false,
            italic: false,
            underline: false,
            text_effect: crate::text_effect::TextEffectId::Flat,
            rotation_quarters: 0,
            baseline_shift_px: 0.0,
            clip_rect: None,
            mono: false,
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
    /// Xanh Mono tabular face instead of Instrument Serif.
    pub mono: bool,
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
    pub block_vertical_align: TextBlockVerticalAlign,
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
    pub _tex: wgpu::Texture,
    pub bind_group: wgpu::BindGroup,
    /// Last frame on which this entry was used, used for TTL eviction.
    pub last_used: u64,
}

/// Per-frame coin payload merged into the shared tile instance buffer.
pub(crate) struct Coin3dDrawState {
    pub instance: Tile3dInstance,
    pub casts_shadow: bool,
}

/// GPU uniforms + bind groups for imported room GLB environment meshes.
/// Uses the same tile textured pipeline as hand tiles; vertices are already in world space (`model = I`).
pub(crate) struct ShopEnvironmentGpu {
    /// Per-primitive camera uniforms; `uniform_buffers[i]` matches `bind_groups[i]`.
    pub uniform_buffers: Vec<wgpu::Buffer>,
    /// Shared by all room primitives; pick-blind uploads [`crate::hallway_glb::HallwayDistortion`].
    pub distortion_buffer: wgpu::Buffer,
    /// Per-primitive `(light_view_proj, model)` for [`shaders/shadow.wgsl`].
    pub shadow_uniform_buffers: Vec<wgpu::Buffer>,
    pub shadow_bind_groups: Vec<wgpu::BindGroup>,
    /// Same `distortion_buffer` as `room_glb` @binding(8) — shadow VS group 1 (warp disabled when zeroed).
    pub shadow_warp_bind_group: wgpu::BindGroup,
    pub bind_groups: Vec<wgpu::BindGroup>,
    /// Archive browse boards (`sign_description_*`): CPU-updated decal at `@group(0) @binding(3)`.
    pub archive_sign_decal_texture: Option<wgpu::Texture>,
    /// Pixel size of [`Self::archive_sign_decal_texture`] (fixed at first GPU upload).
    pub archive_sign_decal_size: Option<(u32, u32)>,
    /// Archive item inspect (`inspect_plaque`): separate atlas sized to that mesh aspect.
    pub archive_inspect_plaque_decal_texture: Option<wgpu::Texture>,
    /// Pixel size of [`Self::archive_inspect_plaque_decal_texture`].
    pub archive_inspect_plaque_decal_size: Option<(u32, u32)>,
}

pub(crate) struct TileFaceOverlayGpu {
    pub _texture: wgpu::Texture,
    pub bind_group: wgpu::BindGroup,
}
