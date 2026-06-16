//! Material Viewer — debug pushdown scene that renders one preview orb per
//! `MaterialKind`. The instance pool binds the 1×1 default relief texture, so
//! materials that would otherwise sample a per-asset heightmap render as their
//! base shading model with no displacement — the scene previews the material
//! itself, not any specific mesh's heightmap.
//!
//! Entered via the debug menu ("Material Viewer..."). Pops back to the
//! previous scene via `OverlayRequest::Pop` when the player presses Back/Escape.

use crate::render::doc_tile_camera::doc_tile_camera;
use crate::render::draw_cmd::{CameraParams, Object3d, Object3dKind, UiFrame};
use crate::render::lit_mesh::{MaterialKind, MaterialParams};
use crate::render::theme::{color, metrics, typography};
use crate::render::wgpu_renderer::{GpuInstance, PointLight, TextAlign, TextLabel};
use crate::ui::controller_hints::{HintStyle, back_scroll_footer_row, push_screen_footer_hint};
use crate::ui::input::{InputMode, UiAction};
use crate::ui::smooth_scroll::SmoothScroll;

use super::{
    BackgroundId, ButtonDef, DrawCtx, SceneBehavior, SceneIntent, SceneTransition, UpdateCtx,
};

const CLICK_BACK: u32 = 0xE001;
const GRID_COLS: usize = 3;
/// Row height at reference layout height 1600 — tall enough for large orbs + caption.
const ROW_H_REF: f32 = 340.0;
const ROW_GAP_REF: f32 = 36.0;
const SCROLL_WHEEL_PX: f32 = 52.0;

pub struct MaterialViewerScene {
    /// `true` when entered as an overlay from another scene. Controls whether
    /// Back pops the overlay stack or transitions to the start screen.
    has_suspended: bool,
    scroll: SmoothScroll,
}

impl MaterialViewerScene {
    pub fn new(has_suspended: bool) -> Self {
        Self {
            has_suspended,
            scroll: SmoothScroll::new(),
        }
    }

    fn go_back(&self, overlay_request: &mut Option<super::OverlayRequest>) -> SceneTransition {
        if self.has_suspended {
            *overlay_request = Some(super::OverlayRequest::Pop);
            None
        } else {
            Some(SceneIntent::MainMenu)
        }
    }

    fn sync_scroll(&self, layout: &MaterialGridLayout) {
        self.scroll
            .set_max(layout.max_scroll_y.round().max(0.0) as u32);
    }
}

impl SceneBehavior for MaterialViewerScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        for &cid in ctx.button_clicks {
            if cid == CLICK_BACK {
                return self.go_back(ctx.overlay_request);
            }
        }

        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let layout = compute_material_grid_layout(w, h, material_entries().len());
        self.sync_scroll(&layout);

        let (cx, cy) = ctx.cursor_pos;
        let wheel_over_grid = cursor_in_rect(cx, cy, layout.viewport);

        if ctx.scroll_lines.abs() > 0.001
            && (ctx.input_mode != InputMode::Cursor || wheel_over_grid)
        {
            self.scroll.scroll_by(ctx.scroll_lines * SCROLL_WHEEL_PX);
        }

        for action in ctx.actions {
            match action {
                UiAction::PageNext if layout.max_scroll_y > 0.0 => {
                    self.scroll.scroll_by(layout.viewport[3] * 0.85);
                }
                UiAction::PagePrev if layout.max_scroll_y > 0.0 => {
                    self.scroll.scroll_by(-layout.viewport[3] * 0.85);
                }
                UiAction::Cancel | UiAction::Pause => {
                    return self.go_back(ctx.overlay_request);
                }
                _ => {}
            }
        }

        None
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let scale = metrics::scene_scale(w, h);
        let entries = material_entries();
        let layout = compute_material_grid_layout(w, h, entries.len());
        self.sync_scroll(&layout);
        let scroll_y = self.scroll.tick();

        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);

        let cam = doc_tile_camera(h);
        frame.camera_override = Some(cam);
        frame.showcase_render_hints.layout_use_ray_plane_z = true;
        frame.showcase_render_hints.doc_tile_no_shadow = true;

        let title_font = typography::size(typography::H20, h);
        let title_h = title_font * 1.6;
        let title_y = h * 0.04;
        frame.text(TextLabel {
            rect: [0.0, title_y, w, title_h],
            text: "Material Viewer".into(),
            color: color::CHAMPAGNE,
            align: TextAlign::Center,
            font_px: Some(title_font),
            ..Default::default()
        });

        let viewport = layout.viewport;
        let clip = Some(viewport);

        frame.quad(GpuInstance {
            rect: viewport,
            color: color::alpha(color::WALNUT_DEEP, 0.55),
            user: 0,
        });

        frame.scene_lighting.push_smooth(PointLight {
            pos: [w * 0.5, h * 0.38, h * 1.35],
            radius: h * 2.9,
            color: color::rgb(color::PARCHMENT),
            intensity: 1.15,
        });

        let viewport_top = viewport[1];
        let viewport_bottom = viewport[1] + viewport[3];
        let mut orbs: Vec<Object3d> = Vec::new();
        let mut captions: Vec<TextLabel> = Vec::new();

        for (i, entry) in entries.iter().enumerate() {
            let col = i % GRID_COLS;
            let row = i / GRID_COLS;
            let cell_top = viewport_top + row as f32 * layout.row_step - scroll_y;
            let cell_bottom = cell_top + layout.row_h;
            if cell_bottom < viewport_top || cell_top > viewport_bottom {
                continue;
            }

            let cell_left = layout.grid_left + col as f32 * layout.cell_w;
            let cx = cell_left + layout.cell_w * 0.5;
            let label_y = cell_bottom - layout.label_h - layout.label_gap;
            let target_orb_bottom = label_y - layout.label_gap;
            let (orb_cy, orb_diameter) = bottom_aligned_material_orb(
                &cam,
                w,
                h,
                cx,
                cell_top + (6.0 * scale).max(4.0),
                target_orb_bottom,
                layout.cell_w * 0.92,
            );

            orbs.push(Object3d {
                pos: [cx, orb_cy, 0.0],
                extents: [orb_diameter, orb_diameter, orb_diameter],
                rotation: [0.0, 0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
                kind: Object3dKind::MaterialOrb {
                    material: entry.material,
                },
                hover_target: 0.0,
                anim_id: 0,
            });

            captions.push(TextLabel {
                rect: [cell_left, label_y, layout.cell_w, layout.label_h],
                text: entry.label.into(),
                color: color::PARCHMENT,
                align: TextAlign::Center,
                font_px: Some(layout.label_font),
                clip_rect: clip,
                ..Default::default()
            });
        }

        frame.object3d_batch(orbs);
        for caption in captions {
            frame.text(caption);
        }

        if layout.max_scroll_y > 0.5 {
            push_scrollbar(&mut frame, &layout, scroll_y);
        }

        let btn_font = typography::size(typography::H36, h);
        let btn_h = (44.0 * scale).max(32.0);
        let btn_w = (160.0 * scale).max(100.0);
        let btn_y = h * 0.91;
        let btn_x = (w - btn_w) * 0.5;
        frame.quad(GpuInstance {
            rect: [btn_x, btn_y, btn_w, btn_h],
            color: color::WALNUT_INK,
            user: 0,
        });
        frame.text(TextLabel {
            rect: [btn_x, btn_y, btn_w, btn_h],
            text: "Back".into(),
            color: color::CHAMPAGNE,
            align: TextAlign::Center,
            font_px: Some(btn_font),
            ..Default::default()
        });
        frame
            .buttons
            .push(ButtonDef::scene((btn_x, btn_y, btn_w, btn_h), CLICK_BACK));

        push_screen_footer_hint(
            &mut frame,
            &ctx,
            back_scroll_footer_row(ctx.input_mode),
            HintStyle::standard(w, h),
        );
        frame.window_title = "Mahjuro \u{2014} Material Viewer".into();
        frame
    }
}

#[derive(Clone, Copy, Debug)]
struct MaterialGridLayout {
    viewport: [f32; 4],
    grid_left: f32,
    cell_w: f32,
    row_h: f32,
    row_step: f32,
    label_h: f32,
    label_gap: f32,
    label_font: f32,
    content_h: f32,
    max_scroll_y: f32,
}

fn compute_material_grid_layout(w: f32, h: f32, entry_count: usize) -> MaterialGridLayout {
    let scale = (h / 1600.0).max(0.5);
    let title_y = h * 0.04;
    let title_h = typography::size(typography::H20, h) * 1.6;
    let viewport_top = title_y + title_h + h * 0.03;
    let viewport_bottom = h * 0.86;
    let viewport_h = (viewport_bottom - viewport_top).max(1.0);
    let grid_w = w * 0.88;
    let grid_left = (w - grid_w) * 0.5;
    let cell_w = grid_w / GRID_COLS as f32;

    let row_h = (ROW_H_REF * scale).max(200.0);
    let row_gap = (ROW_GAP_REF * scale).max(14.0);
    let row_step = row_h + row_gap;
    let label_font = typography::size(typography::H32, h);
    let label_h = label_font * 1.2;
    let label_gap = (5.0 * scale).max(3.0);

    let row_count = entry_count.div_ceil(GRID_COLS);
    let content_h = if row_count == 0 {
        0.0
    } else {
        row_count as f32 * row_step - row_gap
    };
    let max_scroll_y = (content_h - viewport_h).max(0.0);

    MaterialGridLayout {
        viewport: [grid_left, viewport_top, grid_w, viewport_h],
        grid_left,
        cell_w,
        row_h,
        row_step,
        label_h,
        label_gap,
        label_font,
        content_h,
        max_scroll_y,
    }
}

fn cursor_in_rect(px: f32, py: f32, rect: [f32; 4]) -> bool {
    px >= rect[0] && px <= rect[0] + rect[2] && py >= rect[1] && py <= rect[1] + rect[3]
}

fn push_scrollbar(frame: &mut UiFrame, layout: &MaterialGridLayout, scroll_y: f32) {
    let track_w = 6.0f32;
    let track_x = layout.viewport[0] + layout.viewport[2] + 8.0;
    let track = [track_x, layout.viewport[1], track_w, layout.viewport[3]];
    frame.quad(GpuInstance {
        rect: track,
        color: color::alpha(color::WALNUT_INK, 0.85),
        user: 0,
    });

    let max_scroll = layout.max_scroll_y.max(1.0);
    let thumb_h = (layout.viewport[3] * (layout.viewport[3] / layout.content_h.max(1.0)))
        .clamp(24.0, layout.viewport[3]);
    let travel = (layout.viewport[3] - thumb_h).max(0.0);
    let thumb_y = layout.viewport[1] + (scroll_y / max_scroll) * travel;
    frame.quad(GpuInstance {
        rect: [track_x, thumb_y, track_w, thumb_h],
        color: color::alpha(color::BRASS, 0.82),
        user: 0,
    });
}

/// Screen-space AABB for a material orb (matches GPU `Object3d` placement).
fn material_orb_screen_bounds(
    cam: &CameraParams,
    w: f32,
    h: f32,
    px: f32,
    py: f32,
    diameter: f32,
) -> (f32, f32, f32, f32) {
    use crate::render::table_transform::translate_rot_scale;
    use crate::render::world_space::layout_anchor_to_world;
    use glam::{Mat4, Vec3};

    let center = layout_anchor_to_world(w, h, Some(cam), px, py, 0.0, true);
    let model = translate_rot_scale(center, Mat4::IDENTITY, Vec3::splat(diameter));
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for &corner in &[
        Vec3::new(-0.5, -0.5, -0.5),
        Vec3::new(0.5, -0.5, -0.5),
        Vec3::new(-0.5, 0.5, -0.5),
        Vec3::new(0.5, 0.5, -0.5),
        Vec3::new(-0.5, -0.5, 0.5),
        Vec3::new(0.5, -0.5, 0.5),
        Vec3::new(-0.5, 0.5, 0.5),
        Vec3::new(0.5, 0.5, 0.5),
    ] {
        let (sx, sy) = cam.project_world_to_screen(w, h, model.transform_point3(corner));
        min_x = min_x.min(sx);
        min_y = min_y.min(sy);
        max_x = max_x.max(sx);
        max_y = max_y.max(sy);
    }
    (min_x, min_y, max_x, max_y)
}

/// Place the largest orb that fits cell width, bottom-aligned above the label.
fn bottom_aligned_material_orb(
    cam: &CameraParams,
    w: f32,
    h: f32,
    cx: f32,
    band_top: f32,
    target_bottom: f32,
    max_w: f32,
) -> (f32, f32) {
    const BAND_MARGIN: f32 = 2.0;
    let avail_h = (target_bottom - band_top - BAND_MARGIN).max(1.0);
    let target_px = max_w.min(avail_h) * 0.96;

    let cy_for_d = |d: f32| -> f32 {
        let mut cy = target_bottom;
        for _ in 0..16 {
            let (_, _, _, max_y) = material_orb_screen_bounds(cam, w, h, cx, cy, d);
            cy += target_bottom - max_y;
        }
        cy
    };

    let fits = |d: f32| {
        let cy = cy_for_d(d);
        let (min_x, min_y, max_x, max_y) = material_orb_screen_bounds(cam, w, h, cx, cy, d);
        let proj = (max_x - min_x).min(max_y - min_y);
        proj <= target_px && min_y >= band_top + BAND_MARGIN && max_y <= target_bottom + 0.75
    };

    let mut lo = 8.0f32;
    if !fits(lo) {
        return (cy_for_d(lo), lo);
    }
    let mut hi = lo * 2.0;
    while hi < 4_000_000.0 && fits(hi) {
        lo = hi;
        hi *= 2.0;
    }
    for _ in 0..32 {
        let mid = (lo + hi) * 0.5;
        if fits(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (cy_for_d(lo), lo)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::doc_tile_camera::doc_tile_camera;

    #[test]
    fn material_orb_fills_cell_at_three_columns() {
        let w = 2560.0;
        let h = 1600.0;
        let layout = compute_material_grid_layout(w, h, material_entries().len());
        let cam = doc_tile_camera(h);
        let cell_top = layout.viewport[1];
        let cell_bottom = cell_top + layout.row_h;
        let label_y = cell_bottom - layout.label_h - layout.label_gap;
        let target_orb_bottom = label_y - layout.label_gap;
        let band_top = cell_top + 6.0;
        let avail_h = target_orb_bottom - band_top;
        let target_px = (layout.cell_w * 0.92).min(avail_h) * 0.96;
        let cx = layout.grid_left + layout.cell_w * 0.5;
        let (cy, d) = bottom_aligned_material_orb(
            &cam,
            w,
            h,
            cx,
            band_top,
            target_orb_bottom,
            layout.cell_w * 0.92,
        );
        let (min_x, _, max_x, max_y) = material_orb_screen_bounds(&cam, w, h, cx, cy, d);
        let (_, min_y, _, _) = material_orb_screen_bounds(&cam, w, h, cx, cy, d);
        let proj = (max_x - min_x).min(max_y - min_y);
        assert!(
            proj > target_px * 0.88,
            "proj {proj} vs target_px {target_px} (d={d}, cy={cy}, avail_h={avail_h})"
        );
        assert!(min_y >= band_top);
    }
}

struct MaterialEntry {
    label: &'static str,
    material: MaterialParams,
}

/// One entry per `MaterialKind`. `base_color` and specular values are chosen
/// to read cleanly on a sphere — they don't have to match how the game uses
/// the material in context.
fn material_entries() -> Vec<MaterialEntry> {
    use MaterialKind::*;
    let mk = |kind: MaterialKind, rgb: [f32; 3], spec_s: f32, spec_p: f32| MaterialParams {
        kind,
        base_color: [rgb[0], rgb[1], rgb[2], 1.0],
        specular_strength: spec_s,
        specular_power: spec_p,
    };
    vec![
        MaterialEntry {
            label: "Plain",
            material: mk(Plain, [0.85, 0.85, 0.85], 0.2, 32.0),
        },
        MaterialEntry {
            label: "Wax",
            material: MaterialParams::wax(),
        },
        MaterialEntry {
            label: "Wick",
            material: MaterialParams::wick(),
        },
        MaterialEntry {
            label: "Lacquered Wood",
            material: MaterialParams::lacquered_wood(),
        },
        MaterialEntry {
            label: "Lacq. Wood Flat",
            material: mk(LacqueredWoodFlat, [1.0, 1.0, 1.0], 0.55, 96.0),
        },
        MaterialEntry {
            label: "Metal",
            material: mk(Metal, [0.85, 0.70, 0.40], 1.0, 96.0),
        },
        MaterialEntry {
            label: "Water",
            material: mk(Water, [0.20, 0.40, 0.65], 0.8, 64.0),
        },
        MaterialEntry {
            label: "Pack Wrap",
            material: mk(PackWrap, [0.85, 0.85, 0.90], 0.85, 96.0),
        },
        MaterialEntry {
            label: "Glass",
            material: mk(Glass, [1.00, 0.92, 0.60], 1.0, 128.0),
        },
        MaterialEntry {
            label: "Enamel",
            material: mk(Enamel, [0.85, 0.25, 0.30], 0.9, 96.0),
        },
        MaterialEntry {
            label: "Polychrome",
            material: mk(Polychrome, [0.80, 0.80, 0.85], 0.9, 96.0),
        },
        MaterialEntry {
            label: "Score Glyph",
            material: mk(Polychrome, [0.20, 0.55, 1.00], 0.9, 48.0),
        },
        MaterialEntry {
            label: "Porcelain",
            material: mk(Porcelain, color::rgb(color::PARCHMENT), 0.7, 128.0),
        },
        MaterialEntry {
            label: "Porcelain (Aged)",
            material: mk(Porcelain, color::rgb(color::PORCELAIN_AGED), 0.7, 128.0),
        },
        MaterialEntry {
            label: "Porcelain (Antique)",
            material: mk(Porcelain, [0.55, 0.52, 0.46], 0.7, 128.0),
        },
        MaterialEntry {
            label: "Brass",
            material: mk(Brass, [0.92, 0.78, 0.38], 1.0, 96.0),
        },
        MaterialEntry {
            label: "Leather",
            material: mk(Leather, [0.45, 0.12, 0.10], 0.35, 32.0),
        },
        MaterialEntry {
            label: "Chitin (Talisman)",
            material: mk(Chitin, [0.82, 0.55, 0.95], 0.90, 56.0),
        },
        MaterialEntry {
            label: "Unshaded",
            material: mk(Unshaded, [1.0, 1.0, 1.0], 0.0, 1.0),
        },
        MaterialEntry {
            label: "Bronze Mirror",
            material: mk(BronzeMirror, [0.72, 0.52, 0.28], 1.0, 128.0),
        },
        MaterialEntry {
            label: "Catalog Paper",
            material: mk(CatalogPaper, color::rgb(color::PARCHMENT), 0.25, 24.0),
        },
        MaterialEntry {
            label: "Emissive",
            material: MaterialParams::emissive_lamp([1.0, 0.85, 0.55], 0.65),
        },
    ]
}
