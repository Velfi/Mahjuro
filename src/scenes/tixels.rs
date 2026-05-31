//! Mahjuro — Tixels
//!
//! Experimental scene: render a source image as a field of tile-shaped pixels
//! ("tixels") using an 8x8 Bayer matrix and simple procedural tile marks.

use std::time::Instant;
use std::{
    path::{Path, PathBuf},
    process::Command,
};

use crate::core::tile::{Suit, Tile};
use crate::render::draw_cmd::{TileFaceQuad, UiFrame};
use crate::render::theme::{ButtonState, ButtonVariant, color, metrics, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use crate::ui::input::UiAction;
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use super::main_menu::MainMenuScene;
use super::{BackgroundId, DrawCtx, Scene, SceneBehavior, SceneTransition, UpdateCtx};

type Color = [f32; 4];

const MAX_GRID_W: u32 = 256;
const MAX_GRID_H: u32 = 144;
const RESOLUTION_PRESETS: &[(u32, u32)] = &[
    (32, 18),
    (48, 27),
    (64, 36),
    (80, 45),
    (96, 54),
    (112, 63),
    (128, 72),
    (144, 81),
    (160, 90),
    (176, 99),
    (192, 108),
    (224, 126),
    (256, 144),
];

#[derive(Clone, Copy, Debug)]
struct TixelSettings {
    grid_width: u32,
    grid_height: u32,
    tile_size: f32,
    gap: f32,
    use_bayer_dither: bool,
    use_image_color: bool,
    brightness_bias: f32,
    contrast: f32,
    dither_strength: f32,
}

impl Default for TixelSettings {
    fn default() -> Self {
        Self {
            grid_width: 128,
            grid_height: 72,
            tile_size: 8.0,
            gap: 1.0,
            use_bayer_dither: true,
            use_image_color: true,
            brightness_bias: 0.0,
            contrast: 1.1,
            dither_strength: 0.18,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Tixel {
    grid_x: u32,
    grid_y: u32,
    source_color: Color,
    brightness: f32,
    bayer_threshold: f32,
    tile_variant: TixelTileVariant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TixelTileVariant {
    Shadow,
    Blank,
    Dot,
    Bamboo,
    Character,
    Wind,
    Dragon,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TixelsAction {
    LoadImage,
    ResolutionDown,
    ResolutionUp,
    TileDown,
    TileUp,
    ToggleBayer,
    ToggleColor,
    BrightnessDown,
    BrightnessUp,
    ContrastDown,
    ContrastUp,
    DitherDown,
    DitherUp,
    Reset,
    Back,
}

impl TixelsAction {
    fn id(self) -> FocusId {
        FocusId(self as u32 + 1)
    }
}

struct SourceImage {
    width: u32,
    height: u32,
    pixels: Vec<Color>,
}

impl SourceImage {
    fn sample(&self, u: f32, v: f32) -> Color {
        let x = (u.clamp(0.0, 0.9999) * self.width as f32) as u32;
        let y = (v.clamp(0.0, 0.9999) * self.height as f32) as u32;
        let idx = (y * self.width + x) as usize;
        self.pixels
            .get(idx)
            .copied()
            .unwrap_or([0.0, 0.0, 0.0, 1.0])
    }
}

#[derive(Clone, Copy)]
struct TixelsLayout {
    left_panel: [f32; 4],
    preview_outer: [f32; 4],
    preview_inner: [f32; 4],
    right_panel: [f32; 4],
    bottom_strip: [f32; 4],
}

pub struct TixelsScene {
    has_suspended: bool,
    tree: TreeState,
    start_time: Instant,
    settings: TixelSettings,
    source_image: Option<SourceImage>,
    source_label: String,
    status_line: String,
    cache_dirty: bool,
    cached_preview_rect: [f32; 4],
    tixels: Vec<Tixel>,
    shadow_quads: Vec<GpuInstance>,
    bevel_quads: Vec<GpuInstance>,
    mark_quads: Vec<GpuInstance>,
    tile_squircles: Vec<GpuInstance>,
    mark_squircles: Vec<GpuInstance>,
    tile_face_quads: Vec<TileFaceQuad>,
    variant_counts: [u32; 7],
    unique_variants: usize,
    render_ms: f32,
    density: f32,
}

impl TixelsScene {
    pub fn new(has_suspended: bool) -> Self {
        Self {
            has_suspended,
            tree: TreeState::default(),
            start_time: Instant::now(),
            settings: TixelSettings::default(),
            source_image: None,
            source_label: "No image loaded".to_string(),
            status_line: "Load an image to begin".to_string(),
            cache_dirty: true,
            cached_preview_rect: [0.0; 4],
            tixels: Vec::new(),
            shadow_quads: Vec::new(),
            bevel_quads: Vec::new(),
            mark_quads: Vec::new(),
            tile_squircles: Vec::new(),
            mark_squircles: Vec::new(),
            tile_face_quads: Vec::new(),
            variant_counts: [0; 7],
            unique_variants: 0,
            render_ms: 0.0,
            density: 0.0,
        }
    }

    fn go_back(&self, overlay_request: &mut Option<super::OverlayRequest>) -> SceneTransition {
        if self.has_suspended {
            *overlay_request = Some(super::OverlayRequest::Pop);
            None
        } else {
            Some(Scene::MainMenu(MainMenuScene::new()))
        }
    }

    fn scene_layout(w: f32, h: f32) -> TixelsLayout {
        let scale = metrics::scene_scale(w, h);
        let margin = (20.0 * scale).max(14.0);
        let header_h = h * 0.125;
        let footer_h = h * 0.115;
        let body_top = margin + header_h;
        let body_bottom = h - margin - footer_h;
        let body_h = (body_bottom - body_top).max(100.0);
        let side_w = (w * 0.185).clamp(280.0, 380.0);
        let gutter = (14.0 * scale).max(8.0);
        let left_panel = [margin, body_top, side_w, body_h];
        let right_panel = [w - margin - side_w, body_top, side_w, body_h];
        let preview_outer = [
            left_panel[0] + left_panel[2] + gutter,
            body_top,
            w - (margin * 2.0 + side_w * 2.0 + gutter * 2.0),
            body_h,
        ];
        let preview_inner_pad = (30.0 * scale).clamp(24.0, 36.0);
        let preview_inner = [
            preview_outer[0] + preview_inner_pad,
            preview_outer[1] + preview_inner_pad,
            (preview_outer[2] - preview_inner_pad * 2.0).max(20.0),
            (preview_outer[3] - preview_inner_pad * 2.0).max(20.0),
        ];
        let bottom_strip = [margin, h - margin - footer_h, w - margin * 2.0, footer_h];
        TixelsLayout {
            left_panel,
            preview_outer,
            preview_inner,
            right_panel,
            bottom_strip,
        }
    }

    fn controls(layout: &TixelsLayout) -> Vec<FlatItem<TixelsAction>> {
        let [x, y, w, h] = layout.left_panel;
        let row_h = h * 0.054;
        let row_gap = h * 0.014;
        let btn_w = (w * 0.16).clamp(28.0, 48.0);
        let full_w = w * 0.92;
        let left = x + (w - full_w) * 0.5;
        let mut yy = y + h * 0.21;
        let mut items = Vec::with_capacity(15);

        let full = |top: f32| [left, top, full_w, row_h];
        let split = |top: f32| {
            let inner = full_w - btn_w * 2.0 - 8.0;
            (
                [left, top, btn_w, row_h],
                [left + btn_w + 4.0, top, inner, row_h],
                [left + btn_w + 4.0 + inner + 4.0, top, btn_w, row_h],
            )
        };

        items.push(FlatItem::new(
            TixelsAction::LoadImage.id(),
            full(yy),
            TixelsAction::LoadImage,
        ));
        yy += row_h + row_gap;

        let (res_dec, _, res_inc) = split(yy);
        items.push(FlatItem::new(
            TixelsAction::ResolutionDown.id(),
            res_dec,
            TixelsAction::ResolutionDown,
        ));
        items.push(FlatItem::new(
            TixelsAction::ResolutionUp.id(),
            res_inc,
            TixelsAction::ResolutionUp,
        ));
        yy += row_h + row_gap;

        let (tile_dec, _, tile_inc) = split(yy);
        items.push(FlatItem::new(
            TixelsAction::TileDown.id(),
            tile_dec,
            TixelsAction::TileDown,
        ));
        items.push(FlatItem::new(
            TixelsAction::TileUp.id(),
            tile_inc,
            TixelsAction::TileUp,
        ));
        yy += row_h + row_gap;

        items.push(FlatItem::new(
            TixelsAction::ToggleBayer.id(),
            full(yy),
            TixelsAction::ToggleBayer,
        ));
        yy += row_h + row_gap;

        items.push(FlatItem::new(
            TixelsAction::ToggleColor.id(),
            full(yy),
            TixelsAction::ToggleColor,
        ));
        yy += row_h + row_gap;

        let (b_dec, _, b_inc) = split(yy);
        items.push(FlatItem::new(
            TixelsAction::BrightnessDown.id(),
            b_dec,
            TixelsAction::BrightnessDown,
        ));
        items.push(FlatItem::new(
            TixelsAction::BrightnessUp.id(),
            b_inc,
            TixelsAction::BrightnessUp,
        ));
        yy += row_h + row_gap;

        let (c_dec, _, c_inc) = split(yy);
        items.push(FlatItem::new(
            TixelsAction::ContrastDown.id(),
            c_dec,
            TixelsAction::ContrastDown,
        ));
        items.push(FlatItem::new(
            TixelsAction::ContrastUp.id(),
            c_inc,
            TixelsAction::ContrastUp,
        ));
        yy += row_h + row_gap;

        let (d_dec, _, d_inc) = split(yy);
        items.push(FlatItem::new(
            TixelsAction::DitherDown.id(),
            d_dec,
            TixelsAction::DitherDown,
        ));
        items.push(FlatItem::new(
            TixelsAction::DitherUp.id(),
            d_inc,
            TixelsAction::DitherUp,
        ));

        let reset_y = y + h - row_h * 1.58;
        items.push(FlatItem::new(
            TixelsAction::Reset.id(),
            full(reset_y),
            TixelsAction::Reset,
        ));

        let [bx, by, bw, bh] = layout.bottom_strip;
        let back_w = (bw * 0.11).clamp(92.0, 124.0);
        items.push(FlatItem::new(
            TixelsAction::Back.id(),
            [bx + bw - back_w - 10.0, by + bh * 0.28, back_w, bh * 0.45],
            TixelsAction::Back,
        ));

        items
    }

    fn apply_action(
        &mut self,
        action: TixelsAction,
        overlay_request: &mut Option<super::OverlayRequest>,
    ) -> SceneTransition {
        match action {
            TixelsAction::LoadImage => {
                match pick_image_path() {
                    Ok(Some(path)) => match load_source_image_from_path(&path) {
                        Ok(source) => {
                            self.source_label = path
                                .file_name()
                                .and_then(|s| s.to_str())
                                .unwrap_or("custom_image")
                                .to_string();
                            self.source_image = Some(source);
                            self.status_line = format!("Loaded {}", self.source_label);
                            self.cache_dirty = true;
                        }
                        Err(err) => {
                            self.status_line = format!("Image decode failed ({err}).");
                            self.source_label = "No image loaded".to_string();
                            self.source_image = None;
                            self.cache_dirty = true;
                        }
                    },
                    Ok(None) => {
                        self.status_line = "Image load canceled".to_string();
                    }
                    Err(err) => {
                        self.status_line = format!("File picker unavailable ({err}).");
                        self.source_label = "No image loaded".to_string();
                        self.source_image = None;
                        self.cache_dirty = true;
                    }
                }
                None
            }
            TixelsAction::ResolutionDown => {
                self.step_resolution(-1);
                None
            }
            TixelsAction::ResolutionUp => {
                self.step_resolution(1);
                None
            }
            TixelsAction::TileDown => {
                self.settings.tile_size = (self.settings.tile_size - 1.0).clamp(4.0, 14.0);
                self.cache_dirty = true;
                None
            }
            TixelsAction::TileUp => {
                self.settings.tile_size = (self.settings.tile_size + 1.0).clamp(4.0, 14.0);
                self.cache_dirty = true;
                None
            }
            TixelsAction::ToggleBayer => {
                self.settings.use_bayer_dither = !self.settings.use_bayer_dither;
                self.cache_dirty = true;
                None
            }
            TixelsAction::ToggleColor => {
                self.settings.use_image_color = !self.settings.use_image_color;
                self.cache_dirty = true;
                None
            }
            TixelsAction::BrightnessDown => {
                self.settings.brightness_bias =
                    (self.settings.brightness_bias - 0.05).clamp(-0.5, 0.5);
                self.cache_dirty = true;
                None
            }
            TixelsAction::BrightnessUp => {
                self.settings.brightness_bias =
                    (self.settings.brightness_bias + 0.05).clamp(-0.5, 0.5);
                self.cache_dirty = true;
                None
            }
            TixelsAction::ContrastDown => {
                self.settings.contrast = (self.settings.contrast - 0.05).clamp(0.6, 1.8);
                self.cache_dirty = true;
                None
            }
            TixelsAction::ContrastUp => {
                self.settings.contrast = (self.settings.contrast + 0.05).clamp(0.6, 1.8);
                self.cache_dirty = true;
                None
            }
            TixelsAction::DitherDown => {
                self.settings.dither_strength =
                    (self.settings.dither_strength - 0.02).clamp(0.0, 0.50);
                self.cache_dirty = true;
                None
            }
            TixelsAction::DitherUp => {
                self.settings.dither_strength =
                    (self.settings.dither_strength + 0.02).clamp(0.0, 0.50);
                self.cache_dirty = true;
                None
            }
            TixelsAction::Reset => {
                self.settings = TixelSettings::default();
                self.status_line = "Settings reset".to_string();
                self.cache_dirty = true;
                None
            }
            TixelsAction::Back => self.go_back(overlay_request),
        }
    }

    fn step_resolution(&mut self, dir: i32) {
        let current = self.settings.grid_width.min(MAX_GRID_W);
        let default_idx = RESOLUTION_PRESETS
            .iter()
            .position(|&(w, h)| {
                w == TixelSettings::default().grid_width
                    && h == TixelSettings::default().grid_height
            })
            .unwrap_or(0) as i32;
        let mut idx = RESOLUTION_PRESETS
            .iter()
            .position(|&(w, h)| w == current && h == self.settings.grid_height)
            .unwrap_or(default_idx as usize) as i32;
        idx = (idx + dir).clamp(0, (RESOLUTION_PRESETS.len() - 1) as i32);
        let (w, h) = RESOLUTION_PRESETS[idx as usize];
        self.settings.grid_width = w;
        self.settings.grid_height = h;
        self.cache_dirty = true;
    }

    fn ensure_cache(&mut self, preview_rect: [f32; 4]) {
        let Some(source_image) = self.source_image.as_ref() else {
            self.cache_dirty = false;
            self.cached_preview_rect = preview_rect;
            self.tixels.clear();
            self.shadow_quads.clear();
            self.bevel_quads.clear();
            self.mark_quads.clear();
            self.tile_squircles.clear();
            self.mark_squircles.clear();
            self.tile_face_quads.clear();
            self.variant_counts = [0; 7];
            self.unique_variants = 0;
            self.render_ms = 0.0;
            self.density = 0.0;
            return;
        };
        if !self.cache_dirty && approx_rect_eq(self.cached_preview_rect, preview_rect) {
            return;
        }
        self.cache_dirty = false;
        self.cached_preview_rect = preview_rect;

        let start = Instant::now();
        self.tixels.clear();
        self.shadow_quads.clear();
        self.bevel_quads.clear();
        self.mark_quads.clear();
        self.tile_squircles.clear();
        self.mark_squircles.clear();
        self.tile_face_quads.clear();
        self.variant_counts = [0; 7];

        let grid_w = self.settings.grid_width.clamp(1, MAX_GRID_W);
        let grid_h = self.settings.grid_height.clamp(1, MAX_GRID_H);
        let count = (grid_w * grid_h) as usize;
        self.tixels
            .reserve(count.saturating_sub(self.tixels.capacity()));
        self.shadow_quads
            .reserve(count.saturating_sub(self.shadow_quads.capacity()));
        self.bevel_quads
            .reserve((count * 2).saturating_sub(self.bevel_quads.capacity()));
        self.tile_squircles
            .reserve(count.saturating_sub(self.tile_squircles.capacity()));
        self.tile_face_quads
            .reserve(count.saturating_sub(self.tile_face_quads.capacity()));

        let raw_step = self.settings.tile_size + self.settings.gap;
        let raw_w = grid_w as f32 * raw_step - self.settings.gap;
        let raw_h = grid_h as f32 * raw_step - self.settings.gap;
        let fit = (preview_rect[2] / raw_w)
            .min(preview_rect[3] / raw_h)
            .clamp(0.12, 5.0);
        let step = raw_step * fit;
        let tile_px = self.settings.tile_size * fit;
        let gap_px = (self.settings.gap * fit).clamp(0.4, tile_px * 0.20);
        let draw_w = grid_w as f32 * step - gap_px;
        let draw_h = grid_h as f32 * step - gap_px;
        let origin_x = preview_rect[0] + (preview_rect[2] - draw_w) * 0.5;
        let origin_y = preview_rect[1] + (preview_rect[3] - draw_h) * 0.5;

        // TODO: move tixel generation fully into a shader once source texture uniforms
        // and file picker/image upload flow are finalized.
        let grid_aspect = grid_w as f32 / grid_h as f32;
        for y in 0..grid_h {
            for x in 0..grid_w {
                let u = (x as f32 + 0.5) / grid_w as f32;
                let v = (y as f32 + 0.5) / grid_h as f32;
                let (src, in_bounds) =
                    sample_source_preserve_aspect(source_image, u, v, grid_aspect);
                let threshold = bayer_8x8(x, y);
                let adjusted = if in_bounds {
                    let base_brightness = luminance(src);
                    ((base_brightness - 0.5) * self.settings.contrast
                        + 0.5
                        + self.settings.brightness_bias)
                        .clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let dithered = if in_bounds && self.settings.use_bayer_dither {
                    (adjusted + (threshold - 0.5) * self.settings.dither_strength).clamp(0.0, 1.0)
                } else {
                    adjusted
                };
                let quantized = quantize_tixel_value(dithered);
                let variant = if in_bounds {
                    brightness_to_variant(quantized)
                } else {
                    TixelTileVariant::Shadow
                };
                self.variant_counts[variant_index(variant)] += 1;
                let tixel = Tixel {
                    grid_x: x,
                    grid_y: y,
                    source_color: src,
                    brightness: quantized,
                    bayer_threshold: threshold,
                    tile_variant: variant,
                };
                self.tixels.push(tixel);
            }
        }

        self.unique_variants = self.variant_counts.iter().filter(|&&n| n > 0).count();
        self.density = (grid_w * grid_h) as f32 / (MAX_GRID_W * MAX_GRID_H) as f32;

        for tixel in &self.tixels {
            let cell_x = origin_x + tixel.grid_x as f32 * step;
            let cell_y = origin_y + tixel.grid_y as f32 * step;
            let body_w = (tile_px * 0.82).max(1.0);
            let body_h = (tile_px * 0.96).max(1.0);
            let body_x = cell_x + (tile_px - body_w) * 0.5;
            let body_y = cell_y + (tile_px - body_h) * 0.5;
            let value = tixel.brightness.clamp(0.0, 1.0);
            let shadow_alpha = 0.30 - value * 0.11;
            let rim_alpha = 0.34 - value * 0.18;
            let shadow_off = (tile_px * 0.10).max(0.3);
            self.shadow_quads.push(GpuInstance {
                rect: [body_x + shadow_off, body_y + shadow_off, body_w, body_h],
                color: [0.0, 0.0, 0.0, shadow_alpha.clamp(0.10, 0.32)],
                user: 0,
            });
            self.shadow_quads.push(GpuInstance {
                rect: [body_x - 0.20, body_y - 0.20, body_w + 0.40, body_h + 0.40],
                color: [0.03, 0.02, 0.02, rim_alpha.clamp(0.12, 0.34)],
                user: 0,
            });

            let tile_color = tixel_color(
                tixel.source_color,
                tixel.brightness,
                self.settings.use_image_color,
                tixel.bayer_threshold,
            );
            self.tile_squircles.push(GpuInstance {
                rect: [body_x, body_y, body_w, body_h],
                color: tile_color,
                user: 0,
            });
            let top_bevel_alpha = (0.12 + value * 0.22).clamp(0.08, 0.34);
            let side_bevel_alpha = (0.09 + value * 0.14).clamp(0.07, 0.24);
            let bottom_bevel_alpha = (0.16 - value * 0.10).clamp(0.05, 0.18);

            self.bevel_quads.push(GpuInstance {
                rect: [
                    body_x + 0.30,
                    body_y + 0.20,
                    body_w - 0.60,
                    (body_h * 0.19).max(0.5),
                ],
                color: [1.0, 0.98, 0.93, top_bevel_alpha],
                user: 0,
            });
            self.bevel_quads.push(GpuInstance {
                rect: [
                    body_x + 0.3,
                    body_y + body_h * 0.81,
                    body_w - 0.6,
                    (body_h * 0.17).max(0.5),
                ],
                color: [0.06, 0.04, 0.03, bottom_bevel_alpha],
                user: 0,
            });
            self.bevel_quads.push(GpuInstance {
                rect: [
                    body_x + 0.2,
                    body_y + 0.2,
                    (body_w * 0.18).max(0.5),
                    body_h - 0.4,
                ],
                color: [0.98, 0.95, 0.90, side_bevel_alpha],
                user: 0,
            });
            self.bevel_quads.push(GpuInstance {
                rect: [
                    body_x + body_w * 0.82,
                    body_y + 0.2,
                    (body_w * 0.16).max(0.5),
                    body_h - 0.4,
                ],
                color: [
                    0.04,
                    0.03,
                    0.02,
                    (0.14 + (1.0 - value) * 0.12).clamp(0.08, 0.28),
                ],
                user: 0,
            });

            let decal_alpha = (0.16 + value * 0.72).clamp(0.14, 0.92);
            self.tile_face_quads.push(TileFaceQuad {
                tile: variant_demo_tile(tixel.tile_variant),
                inst: GpuInstance {
                    rect: [
                        body_x + body_w * 0.11,
                        body_y + body_h * 0.12,
                        body_w * 0.78,
                        body_h * 0.78,
                    ],
                    color: [1.0, 1.0, 1.0, decal_alpha],
                    user: 0,
                },
            });
        }

        self.render_ms = start.elapsed().as_secs_f32() * 1000.0;
    }
}

impl SceneBehavior for TixelsScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        for a in ctx.actions {
            match a {
                UiAction::Cancel | UiAction::Pause => return self.go_back(ctx.overlay_request),
                UiAction::TixelsLoadImage => {
                    if let Some(next) =
                        self.apply_action(TixelsAction::LoadImage, ctx.overlay_request)
                    {
                        return Some(next);
                    }
                }
                UiAction::TixelsResolutionDown => {
                    if let Some(next) =
                        self.apply_action(TixelsAction::ResolutionDown, ctx.overlay_request)
                    {
                        return Some(next);
                    }
                }
                UiAction::TixelsResolutionUp => {
                    if let Some(next) =
                        self.apply_action(TixelsAction::ResolutionUp, ctx.overlay_request)
                    {
                        return Some(next);
                    }
                }
                UiAction::TixelsTileDown => {
                    if let Some(next) =
                        self.apply_action(TixelsAction::TileDown, ctx.overlay_request)
                    {
                        return Some(next);
                    }
                }
                UiAction::TixelsTileUp => {
                    if let Some(next) = self.apply_action(TixelsAction::TileUp, ctx.overlay_request)
                    {
                        return Some(next);
                    }
                }
                UiAction::TixelsToggleBayer => {
                    if let Some(next) =
                        self.apply_action(TixelsAction::ToggleBayer, ctx.overlay_request)
                    {
                        return Some(next);
                    }
                }
                UiAction::TixelsToggleColor => {
                    if let Some(next) =
                        self.apply_action(TixelsAction::ToggleColor, ctx.overlay_request)
                    {
                        return Some(next);
                    }
                }
                UiAction::TixelsReset => {
                    if let Some(next) = self.apply_action(TixelsAction::Reset, ctx.overlay_request)
                    {
                        return Some(next);
                    }
                }
                _ => {}
            }
        }

        let layout = Self::scene_layout(ctx.layout.window_w, ctx.layout.window_h);
        let items = Self::controls(&layout);
        let fired = self.tree.update_flat(
            &items,
            TreeInput {
                actions: ctx.actions,
                button_clicks: ctx.button_clicks,
                cursor_pos: ctx.cursor_pos,
                window: (ctx.layout.window_w, ctx.layout.window_h),
                input_mode: ctx.input_mode,
                scroll_lines: ctx.scroll_lines,
            },
        );

        if let Some(action) = fired
            && let Some(next) = self.apply_action(action, ctx.overlay_request)
        {
            return Some(next);
        }

        self.ensure_cache(layout.preview_inner);
        None
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let t = self.start_time.elapsed().as_secs_f32();
        let layout = Self::scene_layout(w, h);
        let items = Self::controls(&layout);
        let focused = self.tree.focused();

        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);
        frame.golden_dust();
        draw_smoke_ambience(&mut frame, w, h, t);

        frame.quad(GpuInstance {
            rect: [0.0, 0.0, w, h],
            color: [0.03, 0.02, 0.07, 0.80],
            user: 0,
        });

        let title_font = typography::size(typography::H20, h);
        let subtitle_font = typography::size(typography::H36, h);
        frame.text(TextLabel {
            rect: [0.0, h * 0.025, w, title_font * 1.4],
            text: "Mahjuro — Tixels".into(),
            color: color::CHAMPAGNE,
            align: TextAlign::Center,
            font_px: Some(title_font),
            ..Default::default()
        });
        frame.text(TextLabel {
            rect: [0.0, h * 0.025 + title_font * 1.2, w, subtitle_font * 1.3],
            text: "Experimental Visualizer // Tile Shader R&D".into(),
            color: color::STONE,
            align: TextAlign::Center,
            font_px: Some(subtitle_font),
            ..Default::default()
        });

        draw_panel(
            &mut frame,
            layout.left_panel,
            [0.06, 0.05, 0.11, 0.92],
            color::BRASS,
        );
        draw_panel(
            &mut frame,
            layout.preview_outer,
            [0.08, 0.06, 0.13, 0.90],
            color::GOLD,
        );
        draw_preview_ornate_frame(&mut frame, layout.preview_outer, layout.preview_inner, h, t);
        draw_panel(
            &mut frame,
            layout.right_panel,
            [0.06, 0.05, 0.11, 0.92],
            color::BRASS,
        );
        draw_panel(
            &mut frame,
            layout.bottom_strip,
            [0.06, 0.05, 0.11, 0.95],
            color::BRASS,
        );

        frame.quads(self.shadow_quads.iter().copied());
        frame.squircle_quads(self.tile_squircles.iter().copied());
        frame.quads(self.bevel_quads.iter().copied());
        frame.tile_face_quads(self.tile_face_quads.iter().copied());
        frame.squircle_quads(self.mark_squircles.iter().copied());
        frame.quads(self.mark_quads.iter().copied());
        draw_preview_vignette(&mut frame, layout.preview_inner);

        draw_left_panel(
            &mut frame,
            &layout,
            &items,
            &self.settings,
            &self.source_label,
            h,
        );
        draw_right_panel(
            &mut frame,
            &layout,
            &self.settings,
            &self.source_label,
            self.source_image.as_ref(),
            self.tixels.len() as u32,
            self.unique_variants,
            self.density,
            self.render_ms,
            h,
        );
        draw_bottom_strip(&mut frame, &layout, &self.status_line, h);
        if self.source_image.is_none() {
            frame.text(TextLabel {
                rect: [
                    layout.preview_inner[0],
                    layout.preview_inner[1] + layout.preview_inner[3] * 0.45,
                    layout.preview_inner[2],
                    typography::size(typography::H36, h) * 1.4,
                ],
                text: "No source image loaded".into(),
                color: color::STONE,
                align: TextAlign::Center,
                font_px: Some(typography::size(typography::H36, h)),
                ..Default::default()
            });
        }

        for it in &items {
            draw_control_button(&mut frame, it, focused == Some(it.id));
        }
        self.tree.register_flat_buttons(&items, &mut frame.buttons);
        frame.window_title = "Mahjuro — Tixels".into();
        frame
    }
}

fn draw_panel(frame: &mut UiFrame, rect: [f32; 4], bg: [f32; 4], border: [f32; 4]) {
    frame.quad(GpuInstance {
        rect,
        color: bg,
        user: 0,
    });
    let t = (rect[3] * 0.012).clamp(1.0, 2.5);
    frame.quad(GpuInstance {
        rect: [rect[0], rect[1], rect[2], t],
        color: border,
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: [rect[0], rect[1] + rect[3] - t, rect[2], t],
        color: border,
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: [rect[0], rect[1], t, rect[3]],
        color: border,
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: [rect[0] + rect[2] - t, rect[1], t, rect[3]],
        color: border,
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: [
            rect[0] + t * 2.0,
            rect[1] + t * 2.0,
            rect[2] - t * 4.0,
            rect[3] - t * 4.0,
        ],
        color: [0.01, 0.01, 0.02, 0.09],
        user: 0,
    });
}

fn draw_smoke_ambience(frame: &mut UiFrame, w: f32, h: f32, t: f32) {
    for i in 0..10 {
        let p = i as f32 / 10.0;
        let drift = (t * 0.16 + p * 5.1).sin();
        let x = w * (0.08 + p * 0.86) + drift * w * 0.03;
        let y = h * (0.18 + ((p * 1.7 + t * 0.015).fract()) * 0.74);
        let rw = h * (0.13 + p * 0.05);
        let rh = rw * 0.56;
        frame.squircle_quads([GpuInstance {
            rect: [x - rw * 0.5, y - rh * 0.5, rw, rh],
            color: [0.46, 0.36, 0.19, 0.040],
            user: 0,
        }]);
    }

    let edge_w = w * 0.16;
    frame.quad(GpuInstance {
        rect: [0.0, 0.0, edge_w, h],
        color: [0.11, 0.09, 0.04, 0.07],
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: [w - edge_w, 0.0, edge_w, h],
        color: [0.11, 0.09, 0.04, 0.07],
        user: 0,
    });
}

fn draw_preview_ornate_frame(
    frame: &mut UiFrame,
    rect: [f32; 4],
    inner: [f32; 4],
    window_h: f32,
    t: f32,
) {
    let [x, y, w, h] = rect;
    let pulse = (t * 0.9).sin() * 0.5 + 0.5;
    let trim = [0.71, 0.59, 0.33, 0.24 + pulse * 0.10];
    let dark = [0.05, 0.04, 0.02, 0.65];

    frame.quad(GpuInstance {
        rect: [x + 4.0, y + 4.0, w - 8.0, h - 8.0],
        color: dark,
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: [x + 8.0, y + 8.0, w - 16.0, h - 16.0],
        color: [0.12, 0.10, 0.06, 0.22],
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: [x + 2.0, y + 2.0, w - 4.0, 2.0],
        color: trim,
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: [x + 2.0, y + h - 4.0, w - 4.0, 2.0],
        color: trim,
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: [x + 2.0, y + 4.0, 2.0, h - 8.0],
        color: trim,
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: [x + w - 4.0, y + 4.0, 2.0, h - 8.0],
        color: trim,
        user: 0,
    });

    let c = 10.0;
    for (cx, cy) in [
        (x + 3.0, y + 3.0),
        (x + w - 13.0, y + 3.0),
        (x + 3.0, y + h - 13.0),
        (x + w - 13.0, y + h - 13.0),
    ] {
        frame.squircle_quads([GpuInstance {
            rect: [cx, cy, c, c],
            color: [0.78, 0.66, 0.38, 0.55],
            user: 0,
        }]);
    }

    frame.text(TextLabel {
        rect: [
            inner[0],
            inner[1] - typography::size(typography::H42, window_h) * 1.1,
            inner[2],
            typography::size(typography::H42, window_h),
        ],
        text: "TIXEL PREVIEW".into(),
        color: color::CHAMPAGNE,
        align: TextAlign::Center,
        font_px: Some(typography::size(typography::H42, window_h).clamp(11.0, 18.0)),
        ..Default::default()
    });
}

fn draw_preview_vignette(frame: &mut UiFrame, rect: [f32; 4]) {
    let [x, y, w, h] = rect;
    let edge = (w.min(h) * 0.07).clamp(8.0, 20.0);
    frame.quad(GpuInstance {
        rect: [x, y, w, edge],
        color: [0.0, 0.0, 0.0, 0.18],
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: [x, y + h - edge, w, edge],
        color: [0.0, 0.0, 0.0, 0.18],
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: [x, y + edge, edge, h - edge * 2.0],
        color: [0.0, 0.0, 0.0, 0.12],
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: [x + w - edge, y + edge, edge, h - edge * 2.0],
        color: [0.0, 0.0, 0.0, 0.12],
        user: 0,
    });
}

fn draw_left_panel(
    frame: &mut UiFrame,
    layout: &TixelsLayout,
    items: &[FlatItem<TixelsAction>],
    settings: &TixelSettings,
    source: &str,
    window_h: f32,
) {
    let [x, y, w, h] = layout.left_panel;
    let heading_font = typography::size(typography::H36, window_h).clamp(13.0, 22.0);
    let value_font = typography::size(typography::H42, window_h).clamp(12.0, 19.0);
    let small_font = value_font * 0.88;

    frame.text(TextLabel {
        rect: [x + w * 0.06, y + h * 0.02, w * 0.88, heading_font * 1.2],
        text: "SETTINGS".into(),
        color: color::CHAMPAGNE,
        align: TextAlign::Left,
        font_px: Some(heading_font),
        ..Default::default()
    });

    let row_rect = |action: TixelsAction| -> [f32; 4] {
        items
            .iter()
            .find(|it| it.action == action)
            .map(|it| it.rect)
            .unwrap_or([x + w * 0.06, y + h * 0.2, w * 0.88, h * 0.06])
    };

    let load_row = row_rect(TixelsAction::LoadImage);
    frame.text(TextLabel {
        rect: [
            x + w * 0.07,
            load_row[1] - heading_font * 1.75,
            w * 0.86,
            heading_font,
        ],
        text: "INPUT".into(),
        color: color::CHAMPAGNE,
        align: TextAlign::Left,
        font_px: Some(heading_font * 0.96),
        ..Default::default()
    });
    frame.text(TextLabel {
        rect: [
            x + w * 0.08,
            load_row[1] - value_font * 1.04,
            w * 0.84,
            value_font,
        ],
        text: truncate_middle(source, 26),
        color: color::STONE,
        align: TextAlign::Left,
        font_px: Some(small_font),
        ..Default::default()
    });

    draw_setting_row(
        frame,
        row_rect(TixelsAction::ResolutionDown),
        "RESOLUTION",
        &format!("{} x {}", settings.grid_width, settings.grid_height),
        Some(&format!(
            "{} tixels",
            settings.grid_width * settings.grid_height
        )),
        heading_font,
        value_font,
    );
    draw_setting_row(
        frame,
        row_rect(TixelsAction::TileDown),
        "TILE SIZE",
        &format!("{:.1} px", settings.tile_size),
        None,
        heading_font,
        value_font,
    );
    draw_setting_row(
        frame,
        row_rect(TixelsAction::ToggleBayer),
        "BAYER",
        if settings.use_bayer_dither {
            "On"
        } else {
            "Off"
        },
        None,
        heading_font,
        value_font,
    );
    draw_setting_row(
        frame,
        row_rect(TixelsAction::ToggleColor),
        "COLOR",
        if settings.use_image_color {
            "On"
        } else {
            "Off"
        },
        None,
        heading_font,
        value_font,
    );
    draw_setting_row(
        frame,
        row_rect(TixelsAction::BrightnessDown),
        "BRIGHTNESS",
        &format!("{:+.2}", settings.brightness_bias),
        None,
        heading_font,
        value_font,
    );
    draw_setting_row(
        frame,
        row_rect(TixelsAction::ContrastDown),
        "CONTRAST",
        &format!("{:.2}", settings.contrast),
        None,
        heading_font,
        value_font,
    );
    draw_setting_row(
        frame,
        row_rect(TixelsAction::DitherDown),
        "DITHER",
        &format!("{:.2}", settings.dither_strength),
        None,
        heading_font,
        value_font,
    );
    let reset_row = row_rect(TixelsAction::Reset);
    frame.text(TextLabel {
        rect: [
            reset_row[0] + 2.0,
            reset_row[1] - heading_font * 0.88,
            reset_row[2],
            heading_font,
        ],
        text: "RESET".into(),
        color: color::CHAMPAGNE,
        align: TextAlign::Left,
        font_px: Some(heading_font * 0.90),
        ..Default::default()
    });
}

fn draw_setting_row(
    frame: &mut UiFrame,
    row_rect: [f32; 4],
    label: &str,
    value: &str,
    subvalue: Option<&str>,
    label_font: f32,
    value_font: f32,
) {
    let [x, y, w, h] = row_rect;
    frame.text(TextLabel {
        rect: [x + 2.0, y - label_font * 0.88, w - 4.0, label_font],
        text: label.to_string(),
        color: color::CHAMPAGNE,
        align: TextAlign::Left,
        font_px: Some(label_font * 0.90),
        ..Default::default()
    });
    frame.text(TextLabel {
        rect: [x + w * 0.28, y + h * 0.04, w * 0.44, h * 0.52],
        text: value.to_string(),
        color: color::PARCHMENT,
        align: TextAlign::Center,
        font_px: Some(value_font),
        ..Default::default()
    });
    if let Some(sub) = subvalue {
        frame.text(TextLabel {
            rect: [x + w * 0.25, y + h * 0.52, w * 0.50, h * 0.42],
            text: sub.to_string(),
            color: color::STONE,
            align: TextAlign::Center,
            font_px: Some((value_font * 0.80).max(10.5)),
            ..Default::default()
        });
    }
}

fn truncate_middle(s: &str, max_chars: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_chars {
        return s.to_string();
    }
    if max_chars < 7 {
        return "...".to_string();
    }
    let left = (max_chars - 3) / 2;
    let right = max_chars - 3 - left;
    let mut out = String::with_capacity(max_chars);
    for c in &chars[..left] {
        out.push(*c);
    }
    out.push_str("...");
    for c in &chars[chars.len().saturating_sub(right)..] {
        out.push(*c);
    }
    out
}

fn draw_section_heading(frame: &mut UiFrame, rect: [f32; 4], text: &str, font_px: f32) {
    frame.text(TextLabel {
        rect,
        text: text.to_string(),
        color: color::CHAMPAGNE,
        align: TextAlign::Left,
        font_px: Some(font_px),
        ..Default::default()
    });
}

fn draw_right_panel(
    frame: &mut UiFrame,
    layout: &TixelsLayout,
    settings: &TixelSettings,
    source_label: &str,
    source_image: Option<&SourceImage>,
    tixel_count: u32,
    unique_variants: usize,
    density: f32,
    render_ms: f32,
    window_h: f32,
) {
    let [x, y, w, h] = layout.right_panel;
    let heading_font = typography::size(typography::H36, window_h).clamp(13.0, 21.0);
    let body_font = typography::size(typography::H42, window_h).clamp(11.5, 18.5);

    draw_section_heading(
        frame,
        [x + w * 0.08, y + h * 0.02, w * 0.84, heading_font * 1.2],
        "TILE SET",
        heading_font,
    );
    let tile_w = (w * 0.22).clamp(34.0, 58.0);
    let tile_h = tile_w * 1.16;
    let labels = [
        ("Blank", TixelTileVariant::Blank),
        ("Dot", TixelTileVariant::Dot),
        ("Bamboo", TixelTileVariant::Bamboo),
        ("Char", TixelTileVariant::Character),
        ("Wind", TixelTileVariant::Wind),
        ("Dragon", TixelTileVariant::Dragon),
    ];
    for (i, (name, variant)) in labels.iter().enumerate() {
        let col = i % 3;
        let row = i / 3;
        let tx = x + w * 0.10 + col as f32 * (tile_w + 10.0);
        let ty = y + h * 0.08 + row as f32 * (tile_h + body_font * 1.3);
        draw_variant_sample_tile(frame, [tx, ty, tile_w, tile_h], *variant);
        frame.text(TextLabel {
            rect: [tx - 2.0, ty + tile_h + 2.0, tile_w + 4.0, body_font * 1.0],
            text: name.to_string(),
            color: color::STONE,
            align: TextAlign::Center,
            font_px: Some(body_font * 0.92),
            ..Default::default()
        });
    }

    draw_section_heading(
        frame,
        [x + w * 0.08, y + h * 0.42, w * 0.84, heading_font * 1.2],
        "PALETTE",
        heading_font,
    );
    if let Some(src) = source_image {
        let palette = sampled_palette(src);
        let sw = (w * 0.16).clamp(22.0, 36.0);
        for (i, col) in palette.iter().enumerate() {
            let col_i = i % 4;
            let row_i = i / 4;
            let sx = x + w * 0.10 + col_i as f32 * (sw + 8.0);
            let sy = y + h * 0.49 + row_i as f32 * (sw + 7.0);
            frame.squircle_quads([GpuInstance {
                rect: [sx, sy, sw, sw * 0.92],
                color: *col,
                user: 0,
            }]);
        }
    } else {
        frame.text(TextLabel {
            rect: [x + w * 0.10, y + h * 0.49, w * 0.82, body_font * 1.1],
            text: "No source image".into(),
            color: color::STONE,
            align: TextAlign::Left,
            font_px: Some(body_font),
            ..Default::default()
        });
    }

    draw_section_heading(
        frame,
        [x + w * 0.08, y + h * 0.67, w * 0.84, heading_font * 1.2],
        "STATS",
        heading_font,
    );
    let stats = format!(
        "Image: {}\nGrid: {} x {}\nTixels: {}\nVariants: {}\nBayer: {}\nColor: {}\nDensity: {:.0}%\nRender: {:.2} ms",
        truncate_middle(source_label, 24),
        settings.grid_width,
        settings.grid_height,
        tixel_count,
        unique_variants,
        if settings.use_bayer_dither {
            "On"
        } else {
            "Off"
        },
        if settings.use_image_color {
            "On"
        } else {
            "Off"
        },
        density * 100.0,
        render_ms,
    );
    frame.text(TextLabel {
        rect: [x + w * 0.10, y + h * 0.73, w * 0.82, h * 0.24],
        text: stats,
        color: color::PARCHMENT,
        align: TextAlign::Left,
        font_px: Some(body_font),
        ..Default::default()
    });
}

fn draw_bottom_strip(frame: &mut UiFrame, layout: &TixelsLayout, status_line: &str, window_h: f32) {
    let [x, y, w, h] = layout.bottom_strip;
    let body_font = typography::size(typography::H42, window_h).clamp(11.0, 18.0);
    let key_font = typography::size(typography::H36, window_h).clamp(12.0, 20.0);
    let prompt_h = h * 0.36;
    let py = y + h * 0.16;
    let bar_rect = [x + w * 0.012, y + h * 0.08, w * 0.976, h * 0.52];
    frame.quad(GpuInstance {
        rect: bar_rect,
        color: [0.06, 0.05, 0.08, 0.88],
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: [bar_rect[0], bar_rect[1], bar_rect[2], 1.6],
        color: [0.72, 0.60, 0.33, 0.50],
        user: 0,
    });

    let mut px = x + w * 0.02;
    let spacing = 4.0;
    let prompts = [
        ("O", "Load Image", 0.16),
        ("[ ]", "Resolution", 0.17),
        ("- =", "Tile Size", 0.17),
        ("D", "Bayer", 0.12),
        ("C", "Color", 0.11),
        ("R", "Reset", 0.12),
        ("Esc", "Back", 0.12),
    ];
    let weight_sum: f32 = prompts.iter().map(|(_, _, wt)| *wt).sum();
    let usable_w = w * 0.96 - spacing * (prompts.len().saturating_sub(1)) as f32;

    for (key, label, weight) in prompts {
        let total_w = usable_w * (weight / weight_sum);
        draw_key_prompt(
            frame,
            [px, py, total_w, prompt_h],
            key,
            label,
            key_font,
            body_font,
        );
        px += total_w + spacing;
    }

    frame.text(TextLabel {
        rect: [x + w * 0.02, y + h * 0.66, w * 0.96, h * 0.26],
        text: truncate_middle(status_line, 120),
        color: color::STONE,
        align: TextAlign::Left,
        font_px: Some((body_font * 0.92).max(10.5)),
        ..Default::default()
    });
}

fn draw_key_prompt(
    frame: &mut UiFrame,
    rect: [f32; 4],
    key: &str,
    label: &str,
    key_font: f32,
    body_font: f32,
) {
    let [x, y, w, h] = rect;
    frame.quad(GpuInstance {
        rect,
        color: [0.10, 0.09, 0.10, 0.92],
        user: 0,
    });
    let cap_w = (w * 0.36).clamp(26.0, 60.0);
    frame.squircle_quads([GpuInstance {
        rect: [x + 4.0, y + 4.0, cap_w - 8.0, h - 8.0],
        color: [0.31, 0.25, 0.15, 0.96],
        user: 0,
    }]);
    frame.text(TextLabel {
        rect: [x + 4.0, y + 3.0, cap_w - 8.0, h - 8.0],
        text: key.to_string(),
        color: color::CHAMPAGNE,
        align: TextAlign::Center,
        font_px: Some(key_font),
        ..Default::default()
    });
    frame.text(TextLabel {
        rect: [x + cap_w + 2.0, y, w - cap_w - 4.0, h],
        text: label.to_string(),
        color: color::PARCHMENT,
        align: TextAlign::Left,
        font_px: Some((body_font * 0.90).max(10.5)),
        ..Default::default()
    });
}

fn draw_control_button(frame: &mut UiFrame, item: &FlatItem<TixelsAction>, focused: bool) {
    let state = if focused {
        ButtonState::Hover
    } else {
        ButtonState::Rest
    };
    let mut colors = crate::render::theme::button_colors(ButtonVariant::Default, state);
    if item.action == TixelsAction::Back && !focused {
        colors.bg[3] *= 0.58;
        colors.text[3] *= 0.72;
    }
    frame.quad(GpuInstance {
        rect: item.rect,
        color: colors.bg,
        user: 0,
    });
    let label = match item.action {
        TixelsAction::LoadImage => "Load Image".to_string(),
        TixelsAction::ResolutionDown => "[".to_string(),
        TixelsAction::ResolutionUp => "]".to_string(),
        TixelsAction::TileDown => "-".to_string(),
        TixelsAction::TileUp => "=".to_string(),
        TixelsAction::ToggleBayer => String::new(),
        TixelsAction::ToggleColor => String::new(),
        TixelsAction::BrightnessDown => "-".to_string(),
        TixelsAction::BrightnessUp => "+".to_string(),
        TixelsAction::ContrastDown => "-".to_string(),
        TixelsAction::ContrastUp => "+".to_string(),
        TixelsAction::DitherDown => "-".to_string(),
        TixelsAction::DitherUp => "+".to_string(),
        TixelsAction::Reset => "Reset".to_string(),
        TixelsAction::Back => "Back".to_string(),
    };
    let font_px = (item.rect[3] * 0.44).clamp(9.5, 16.0);
    frame.text(TextLabel {
        rect: item.rect,
        text: label,
        color: colors.text,
        align: TextAlign::Center,
        font_px: Some(font_px),
        ..Default::default()
    });
}

fn pick_image_path() -> Result<Option<PathBuf>, String> {
    #[cfg(target_os = "macos")]
    {
        let script = r#"POSIX path of (choose file with prompt "Load image for Tixels" of type {"public.image"})"#;
        let output = Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()
            .map_err(|e| format!("failed to run osascript: {e}"))?;
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if path.is_empty() {
                Ok(None)
            } else {
                Ok(Some(PathBuf::from(path)))
            }
        } else {
            let err = String::from_utf8_lossy(&output.stderr);
            if err.contains("User canceled")
                || err.contains("-128")
                || err.to_ascii_lowercase().contains("cancel")
            {
                Ok(None)
            } else {
                Err(err.trim().to_string())
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        let script = r#"
Add-Type -AssemblyName System.Windows.Forms | Out-Null
$dlg = New-Object System.Windows.Forms.OpenFileDialog
$dlg.Title = 'Load image for Tixels'
$dlg.Multiselect = $false
$dlg.Filter = 'Image files|*.png;*.jpg;*.jpeg;*.bmp;*.gif;*.webp|All files|*.*'
if ($dlg.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {
    Write-Output $dlg.FileName
}
"#;
        let output = Command::new("powershell")
            .arg("-NoProfile")
            .arg("-STA")
            .arg("-Command")
            .arg(script)
            .output()
            .map_err(|e| format!("failed to run powershell file dialog: {e}"))?;
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if path.is_empty() {
                Ok(None)
            } else {
                Ok(Some(PathBuf::from(path)))
            }
        } else {
            let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
            if err.is_empty() { Ok(None) } else { Err(err) }
        }
    }
    #[cfg(target_os = "linux")]
    {
        let filters = "*.png *.jpg *.jpeg *.bmp *.gif *.webp *.PNG *.JPG *.JPEG *.BMP *.GIF *.WEBP";

        let zenity = Command::new("zenity")
            .arg("--file-selection")
            .arg("--title=Load image for Tixels")
            .arg(format!("--file-filter=Image files | {filters}"))
            .arg("--file-filter=All files | *")
            .output();
        if let Some(res) = parse_linux_dialog_result(zenity)? {
            return Ok(Some(res));
        }

        let kdialog = Command::new("kdialog")
            .arg("--getopenfilename")
            .arg(".")
            .arg("Images (*.png *.jpg *.jpeg *.bmp *.gif *.webp)")
            .output();
        if let Some(res) = parse_linux_dialog_result(kdialog)? {
            return Ok(Some(res));
        }

        let yad = Command::new("yad")
            .arg("--file")
            .arg("--title=Load image for Tixels")
            .arg(format!("--file-filter=Image files|{filters}"))
            .output();
        if let Some(res) = parse_linux_dialog_result(yad)? {
            return Ok(Some(res));
        }

        Err("no supported Linux picker found (tried zenity, kdialog, yad)".to_string())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        Err("native file picker not wired for this platform yet".to_string())
    }
}

#[cfg(target_os = "linux")]
fn parse_linux_dialog_result(
    result: std::io::Result<std::process::Output>,
) -> Result<Option<PathBuf>, String> {
    use std::io::ErrorKind;
    match result {
        Ok(output) => {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if path.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(PathBuf::from(path)))
                }
            } else {
                let code = output.status.code().unwrap_or(-1);
                // Common cancel codes for zenity/kdialog/yad.
                if matches!(code, 1 | 255) {
                    Ok(None)
                } else {
                    let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
                    if err.is_empty() { Ok(None) } else { Err(err) }
                }
            }
        }
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("failed to run picker command: {e}")),
    }
}

fn load_source_image_from_path(path: &Path) -> Result<SourceImage, String> {
    let reader = image::ImageReader::open(path)
        .map_err(|e| format!("open failed: {e}"))?
        .with_guessed_format()
        .map_err(|e| format!("format detection failed: {e}"))?;
    let decoded = reader.decode().map_err(|e| format!("decode failed: {e}"))?;

    let max_dim = 1024u32;
    let resized = if decoded.width() > max_dim || decoded.height() > max_dim {
        decoded.resize(max_dim, max_dim, image::imageops::FilterType::Triangle)
    } else {
        decoded
    };
    let rgba = resized.to_rgba8();
    let (w, h) = rgba.dimensions();
    let mut pixels = Vec::with_capacity((w * h) as usize);
    for px in rgba.pixels() {
        let [r, g, b, a] = px.0;
        pixels.push([
            r as f32 / 255.0,
            g as f32 / 255.0,
            b as f32 / 255.0,
            a as f32 / 255.0,
        ]);
    }
    Ok(SourceImage {
        width: w,
        height: h,
        pixels,
    })
}

fn bayer_8x8(x: u32, y: u32) -> f32 {
    const M: [[u8; 8]; 8] = [
        [0, 48, 12, 60, 3, 51, 15, 63],
        [32, 16, 44, 28, 35, 19, 47, 31],
        [8, 56, 4, 52, 11, 59, 7, 55],
        [40, 24, 36, 20, 43, 27, 39, 23],
        [2, 50, 14, 62, 1, 49, 13, 61],
        [34, 18, 46, 30, 33, 17, 45, 29],
        [10, 58, 6, 54, 9, 57, 5, 53],
        [42, 26, 38, 22, 41, 25, 37, 21],
    ];
    M[(y & 7) as usize][(x & 7) as usize] as f32 / 63.0
}

fn quantize_tixel_value(value: f32) -> f32 {
    match value {
        v if v < 0.12 => 0.05,
        v if v < 0.28 => 0.20,
        v if v < 0.45 => 0.38,
        v if v < 0.62 => 0.55,
        v if v < 0.78 => 0.72,
        _ => 0.92,
    }
}

fn brightness_to_variant(brightness: f32) -> TixelTileVariant {
    if brightness < 0.10 {
        TixelTileVariant::Shadow
    } else if brightness < 0.20 {
        TixelTileVariant::Blank
    } else if brightness < 0.36 {
        TixelTileVariant::Dot
    } else if brightness < 0.55 {
        TixelTileVariant::Bamboo
    } else if brightness < 0.75 {
        TixelTileVariant::Character
    } else if brightness < 0.90 {
        TixelTileVariant::Wind
    } else {
        TixelTileVariant::Dragon
    }
}

fn tixel_color(source: Color, brightness: f32, use_image_color: bool, threshold: f32) -> Color {
    let value = brightness.clamp(0.0, 1.0);
    let dark_bone = [0.12, 0.11, 0.10];
    let bright_bone = [0.93, 0.89, 0.81];
    let mut base = [
        dark_bone[0] + (bright_bone[0] - dark_bone[0]) * value,
        dark_bone[1] + (bright_bone[1] - dark_bone[1]) * value,
        dark_bone[2] + (bright_bone[2] - dark_bone[2]) * value,
    ];
    if use_image_color {
        let tint_mix = 0.25;
        base = [
            base[0] * (1.0 - tint_mix) + source[0] * tint_mix,
            base[1] * (1.0 - tint_mix) + source[1] * tint_mix,
            base[2] * (1.0 - tint_mix) + source[2] * tint_mix,
        ];
    }
    let dither_tone = 0.98 + (threshold - 0.5) * 0.08;
    [
        (base[0] * dither_tone).clamp(0.0, 1.0),
        (base[1] * dither_tone).clamp(0.0, 1.0),
        (base[2] * dither_tone).clamp(0.0, 1.0),
        1.0,
    ]
}

fn variant_index(v: TixelTileVariant) -> usize {
    match v {
        TixelTileVariant::Shadow => 0,
        TixelTileVariant::Blank => 1,
        TixelTileVariant::Dot => 2,
        TixelTileVariant::Bamboo => 3,
        TixelTileVariant::Character => 4,
        TixelTileVariant::Wind => 5,
        TixelTileVariant::Dragon => 6,
    }
}

fn variant_demo_tile(v: TixelTileVariant) -> Tile {
    match v {
        TixelTileVariant::Shadow => Tile::new(Suit::Dragon, 3, 0),
        TixelTileVariant::Blank => Tile::new(Suit::Wind, 1, 0),
        TixelTileVariant::Dot => Tile::new(Suit::Pinzu, 5, 0),
        TixelTileVariant::Bamboo => Tile::new(Suit::Souzu, 5, 0),
        TixelTileVariant::Character => Tile::new(Suit::Manzu, 5, 0),
        TixelTileVariant::Wind => Tile::new(Suit::Wind, 3, 0),
        TixelTileVariant::Dragon => Tile::new(Suit::Dragon, 1, 0),
    }
}

fn sampled_palette(source_image: &SourceImage) -> [Color; 8] {
    let mut out = [[0.0; 4]; 8];
    let sample_points = [
        (0.12, 0.18),
        (0.35, 0.18),
        (0.58, 0.18),
        (0.81, 0.18),
        (0.15, 0.58),
        (0.38, 0.58),
        (0.62, 0.58),
        (0.85, 0.58),
    ];
    for (i, item) in out.iter_mut().enumerate() {
        let (u, v) = sample_points[i];
        let mut c = source_image.sample(u, v);
        c[0] = c[0].clamp(0.05, 1.0);
        c[1] = c[1].clamp(0.05, 1.0);
        c[2] = c[2].clamp(0.05, 1.0);
        c[3] = 1.0;
        *item = c;
    }
    out.sort_by(|a, b| luminance(*a).total_cmp(&luminance(*b)));
    out
}

fn draw_variant_sample_tile(frame: &mut UiFrame, rect: [f32; 4], variant: TixelTileVariant) {
    let [x, y, w, h] = rect;
    let preview_value = match variant {
        TixelTileVariant::Shadow => 0.05,
        TixelTileVariant::Blank => 0.20,
        TixelTileVariant::Dot => 0.38,
        TixelTileVariant::Bamboo => 0.55,
        TixelTileVariant::Character => 0.72,
        TixelTileVariant::Wind => 0.82,
        TixelTileVariant::Dragon => 0.92,
    };
    let body = tixel_color([0.42, 0.34, 0.24, 1.0], preview_value, false, 0.5);
    frame.quad(GpuInstance {
        rect: [x + w * 0.08, y + h * 0.10, w * 0.92, h * 0.92],
        color: [0.0, 0.0, 0.0, 0.20],
        user: 0,
    });
    frame.squircle_quads([GpuInstance {
        rect,
        color: body,
        user: 0,
    }]);
    frame.quad(GpuInstance {
        rect: [x + 0.6, y + 0.6, w - 1.2, (h * 0.18).max(0.5)],
        color: [1.0, 0.98, 0.93, 0.26],
        user: 0,
    });
    let mark_rect = [x + w * 0.10, y + h * 0.10, w * 0.80, h * 0.80];
    match variant {
        TixelTileVariant::Shadow => frame.quad(GpuInstance {
            rect: [
                mark_rect[0] + mark_rect[2] * 0.18,
                mark_rect[1] + mark_rect[3] * 0.20,
                mark_rect[2] * 0.64,
                mark_rect[3] * 0.58,
            ],
            color: [0.08, 0.07, 0.09, 0.92],
            user: 0,
        }),
        _ => frame.tile_face_quads([TileFaceQuad {
            tile: variant_demo_tile(variant),
            inst: GpuInstance {
                rect: mark_rect,
                color: [1.0, 1.0, 1.0, 0.90],
                user: 0,
            },
        }]),
    }
}

fn luminance(c: Color) -> f32 {
    c[0] * 0.2126 + c[1] * 0.7152 + c[2] * 0.0722
}

fn sample_source_preserve_aspect(
    source: &SourceImage,
    u: f32,
    v: f32,
    target_aspect: f32,
) -> (Color, bool) {
    let source_aspect = source.width as f32 / source.height.max(1) as f32;
    let bar_color = [0.02, 0.02, 0.03, 1.0];

    if source_aspect > target_aspect {
        let visible_v = (target_aspect / source_aspect).clamp(0.0, 1.0);
        let half = visible_v * 0.5;
        let v0 = 0.5 - half;
        let v1 = 0.5 + half;
        if v < v0 || v > v1 {
            return (bar_color, false);
        }
        let remapped_v = ((v - v0) / (v1 - v0)).clamp(0.0, 0.9999);
        return (source.sample(u, remapped_v), true);
    }
    if source_aspect < target_aspect {
        let visible_u = (source_aspect / target_aspect).clamp(0.0, 1.0);
        let half = visible_u * 0.5;
        let u0 = 0.5 - half;
        let u1 = 0.5 + half;
        if u < u0 || u > u1 {
            return (bar_color, false);
        }
        let remapped_u = ((u - u0) / (u1 - u0)).clamp(0.0, 0.9999);
        return (source.sample(remapped_u, v), true);
    }

    (source.sample(u, v), true)
}

fn approx_rect_eq(a: [f32; 4], b: [f32; 4]) -> bool {
    (a[0] - b[0]).abs() < 0.5
        && (a[1] - b[1]).abs() < 0.5
        && (a[2] - b[2]).abs() < 0.5
        && (a[3] - b[3]).abs() < 0.5
}
