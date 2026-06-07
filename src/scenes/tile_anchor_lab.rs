//! Debug scene: projected tile bounds vs label anchors (guide / tutorial layout path).
//!
//! Entered from Debug → Labs → Tile Anchor Lab…

use crate::core::tile::{Suit, Tile};
use crate::persistence::TilePreset;
use crate::render::draw_cmd::{CameraParams, DrawCmd, ShowcaseTilePlacement, UiFrame};
use crate::render::showcase_tile_layout::{
    ShowcaseTileLabelGaps, showcase_tile_group_label_anchor, showcase_tile_merge_projected_group,
};
use crate::render::theme::{color, metrics, typography};
use crate::render::wgpu_renderer::{GpuInstance, PointLight, TextAlign, TextLabel};
use crate::ui::controller_hints::{HintStyle, back_footer_row, push_screen_footer_hint};
use crate::ui::input::UiAction;

use super::{BackgroundId, ButtonDef, DrawCtx, SceneBehavior, SceneIntent, SceneTransition, UpdateCtx};

const CLICK_BACK: u32 = 0xE010;
const TILE_ROTATION: [f32; 3] = [0.0, 0.0, std::f32::consts::PI];

struct TileGroupSpec {
    label: &'static str,
    accent: [f32; 4],
    tiles: &'static [(Suit, u8)],
}

const DEMO_GROUPS: &[TileGroupSpec] = &[
    TileGroupSpec {
        label: "Manzu",
        accent: Suit::Manzu.keyword_color(),
        tiles: &[(Suit::Manzu, 1), (Suit::Manzu, 5), (Suit::Manzu, 9)],
    },
    TileGroupSpec {
        label: "Souzu",
        accent: Suit::Souzu.keyword_color(),
        tiles: &[(Suit::Souzu, 1), (Suit::Souzu, 5), (Suit::Souzu, 9)],
    },
    TileGroupSpec {
        label: "Pinzu",
        accent: Suit::Pinzu.keyword_color(),
        tiles: &[(Suit::Pinzu, 1), (Suit::Pinzu, 5), (Suit::Pinzu, 9)],
    },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CameraPreset {
    Guide,
    Tutorial,
}

impl CameraPreset {
    fn label(self) -> &'static str {
        match self {
            Self::Guide => "Guide camera",
            Self::Tutorial => "Tutorial camera",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Guide => Self::Tutorial,
            Self::Tutorial => Self::Guide,
        }
    }

    fn params(self, h: f32) -> CameraParams {
        let cam_scale = h / 1600.0;
        match self {
            Self::Guide => CameraParams {
                eye: [0.0, -200.0 * cam_scale, 2040.0 * cam_scale],
                target: [0.0, -50.0 * cam_scale, 0.0],
                up: [0.0, 0.0, 1.0],
                fovy_deg: 45.0,
                clip_near: None,
                clip_far: None,
            },
            Self::Tutorial => CameraParams {
                eye: [0.0, -220.0 * cam_scale, 1960.0 * cam_scale],
                target: [0.0, -40.0 * cam_scale, 0.0],
                up: [0.0, 0.0, 1.0],
                fovy_deg: 45.0,
                clip_near: None,
                clip_far: None,
            },
        }
    }
}

struct GroupLayout {
    group_start_x: f32,
    group_w: f32,
    label_anchor: crate::render::showcase_tile_layout::ShowcaseTileGroupLabelAnchor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScreenCorner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl ScreenCorner {
    fn label(self) -> &'static str {
        match self {
            Self::TopLeft => "Top-left",
            Self::TopRight => "Top-right",
            Self::BottomLeft => "Bottom-left",
            Self::BottomRight => "Bottom-right",
        }
    }

    fn tile_center(self, w: f32, h: f32, tile_size: f32) -> (f32, f32) {
        let inset = w * 0.07 + tile_size * 0.5;
        let top_y = h * 0.115 + tile_size * 0.5;
        let bottom_y = h * 0.855 - tile_size * 0.5;
        match self {
            Self::TopLeft => (inset, top_y),
            Self::TopRight => (w - inset, top_y),
            Self::BottomLeft => (inset, bottom_y),
            Self::BottomRight => (w - inset, bottom_y),
        }
    }
}

const CORNER_PROBES: &[(ScreenCorner, Suit, u8)] = &[
    (ScreenCorner::TopLeft, Suit::Manzu, 5),
    (ScreenCorner::TopRight, Suit::Souzu, 5),
    (ScreenCorner::BottomLeft, Suit::Pinzu, 5),
    (ScreenCorner::BottomRight, Suit::Wind, 1),
];

pub struct TileAnchorLabScene {
    has_suspended: bool,
    camera: CameraPreset,
}

impl TileAnchorLabScene {
    pub fn new(has_suspended: bool) -> Self {
        Self {
            has_suspended,
            camera: CameraPreset::Guide,
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
}

impl SceneBehavior for TileAnchorLabScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        for &cid in ctx.button_clicks {
            if cid == CLICK_BACK {
                return self.go_back(ctx.overlay_request);
            }
        }
        for a in ctx.actions {
            match a {
                UiAction::Cancel | UiAction::Pause => {
                    return self.go_back(ctx.overlay_request);
                }
                UiAction::FocusNext | UiAction::FocusDown => {
                    self.camera = self.camera.next();
                }
                UiAction::FocusPrev | UiAction::FocusUp => {
                    self.camera = self.camera.next();
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
        let cam = self.camera.params(h);

        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);
        frame.camera_override = Some(cam);

        frame.scene_lighting.push_smooth(PointLight {
            pos: [w * 0.5, h * 0.38, h * 1.35],
            radius: h * 2.9,
            color: color::rgb(color::PARCHMENT),
            intensity: 1.15,
        });

        let title_font = typography::size(typography::H20, h);
        let title_h = title_font * 1.5;
        let body_font = typography::size(typography::H36, h);
        let label_font = typography::size(typography::H42, h);
        let label_line_h = label_font * 1.22;

        let title_y = h * 0.03;
        frame.text(TextLabel {
            rect: [0.0, title_y, w, title_h],
            text: "Tile Anchor Lab".into(),
            color: color::CHAMPAGNE,
            align: TextAlign::Center,
            font_px: Some(title_font),
            ..Default::default()
        });

        let legend_y = title_y + title_h + h * 0.008;
        frame.text(TextLabel {
            rect: [w * 0.06, legend_y, w * 0.88, body_font * 3.0],
            text: format!(
                "Green = projected tile AABB. Cyan underline + label use the same anchor as guide/tutorial.\n\
                 Center row = suit groups. Corners = off-center probes.\n\
                 Camera: {} (↑/↓ to cycle). Back / Esc to exit.",
                self.camera.label(),
            ),
            color: color::PARCHMENT,
            align: TextAlign::Center,
            font_px: Some(body_font),
            ..Default::default()
        });

        let label_gaps = ShowcaseTileLabelGaps {
            underline_gap: (8.0 * scale).max(5.0),
            underline_h: (3.0 * scale).max(2.0),
            label_text_gap: (5.0 * scale).max(3.0),
        };

        let tile_size = (h * 0.052).clamp(28.0, 72.0);
        let corner_tile_size = (tile_size * 0.78).clamp(24.0, 56.0);
        let gap = tile_size * 0.6;
        let total_tiles: usize = DEMO_GROUPS.iter().map(|g| g.tiles.len()).sum();
        let total_w =
            total_tiles as f32 * tile_size + (DEMO_GROUPS.len().saturating_sub(1) as f32) * gap;
        let start_x = (w - total_w) * 0.5;
        let row_center_y = h * 0.48;

        let mut placements = Vec::new();
        let mut next_id = 90_000u32;

        let (row_placements, groups) = layout_demo_row(
            &cam,
            w,
            h,
            start_x,
            row_center_y,
            tile_size,
            gap,
            label_gaps,
            &mut next_id,
        );
        placements.extend(row_placements);

        for (group, spec) in groups.iter().zip(DEMO_GROUPS.iter()) {
            draw_group_annotations(
                &mut frame,
                group,
                spec.label,
                spec.accent,
                label_gaps,
                label_font,
                label_line_h,
            );
        }

        frame.text(TextLabel {
            rect: [
                w * 0.04,
                h * 0.09,
                w * 0.92,
                crate::ui::styled_text::colored_row_line_step(body_font),
            ],
            text: "Corner probes (off-center)".into(),
            color: color::STONE,
            align: TextAlign::Left,
            font_px: Some(body_font),
            ..Default::default()
        });

        for &(corner, suit, rank) in CORNER_PROBES {
            let (center_x, center_y) = corner.tile_center(w, h, corner_tile_size);
            let tiles = [(suit, rank)];
            let (corner_placements, group) = layout_single_tile_group(
                &cam,
                w,
                h,
                center_x,
                center_y,
                corner_tile_size,
                label_gaps,
                &tiles,
                &mut next_id,
            );
            placements.extend(corner_placements);
            draw_corner_probe(
                &mut frame,
                &group,
                corner.label(),
                label_gaps,
                label_font,
                label_line_h,
            );
        }

        if !placements.is_empty() {
            frame.cmds.push(DrawCmd::ShowcaseTileBatch(placements.into()));
        }

        let btn_font = typography::size(typography::H36, h);
        let btn_h = (44.0 * scale).max(32.0);
        let btn_w = (160.0 * scale).max(100.0);
        let btn_y = h * 0.93;
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

        frame.window_title = "Mahjuro — Tile Anchor Lab".into();
        push_screen_footer_hint(
            &mut frame,
            &ctx,
            back_footer_row(ctx.input_mode),
            HintStyle::standard(w, h),
        );
        frame
    }
}

fn layout_demo_row(
    cam: &CameraParams,
    win_w: f32,
    win_h: f32,
    start_x: f32,
    center_y: f32,
    tile_size: f32,
    gap: f32,
    label_gaps: ShowcaseTileLabelGaps,
    next_id: &mut u32,
) -> (Vec<ShowcaseTilePlacement>, Vec<GroupLayout>) {
    let mut placements = Vec::new();
    let mut groups = Vec::new();
    let mut cursor_x = start_x;

    for spec in DEMO_GROUPS {
        let group_w = spec.tiles.len() as f32 * tile_size;
        let center_x = cursor_x + group_w * 0.5;
        let (group_placements, group) = layout_single_tile_group(
            cam, win_w, win_h, center_x, center_y, tile_size, label_gaps, spec.tiles, next_id,
        );
        placements.extend(group_placements);
        groups.push(group);
        cursor_x += group_w + gap;
    }

    (placements, groups)
}

fn layout_single_tile_group(
    cam: &CameraParams,
    win_w: f32,
    win_h: f32,
    center_x: f32,
    center_y: f32,
    tile_size: f32,
    label_gaps: ShowcaseTileLabelGaps,
    tiles: &[(Suit, u8)],
    next_id: &mut u32,
) -> (Vec<ShowcaseTilePlacement>, GroupLayout) {
    let group_w = tiles.len() as f32 * tile_size;
    let group_start_x = center_x - group_w * 0.5;
    let mut placements = Vec::new();
    let mut centers_xy = Vec::with_capacity(tiles.len());
    let mut cursor_x = group_start_x;

    for &(suit, rank) in tiles {
        let px = cursor_x + tile_size * 0.5;
        centers_xy.push([px, center_y]);
        placements.push(ShowcaseTilePlacement {
            tile: Tile::new(suit, rank, *next_id),
            center_pos: [px, center_y, 0.0],
            rotation: TILE_ROTATION,
            scale: 1.0,
            size_px: tile_size,
            brightness: 1.08,
            selected: false,
            hovered: false,
            outline: false,
            glow: false,
            glow_color: None,
                    outline_sel: None,
            pick_id: None,
            overlay_rect_group: None,
        });
        *next_id += 1;
        cursor_x += tile_size;
    }

    let bounds = showcase_tile_merge_projected_group(
        cam,
        win_w,
        win_h,
        TilePreset::Chinese,
        TILE_ROTATION,
        1.0,
        tile_size,
        0.0,
        &centers_xy,
    );
    let label_anchor = showcase_tile_group_label_anchor(bounds, label_gaps);

    let group = GroupLayout {
        group_start_x,
        group_w,
        label_anchor,
    };
    (placements, group)
}

fn draw_projected_bounds(
    bounds: crate::render::showcase_tile_layout::ShowcaseTileScreenBounds,
    frame: &mut UiFrame,
) {
    frame.quad(GpuInstance {
        rect: [bounds.min_x, bounds.min_y, bounds.width(), bounds.height()],
        color: color::alpha([0.25, 0.85, 0.45, 1.0], 0.22),
        user: 0,
    });
}

fn draw_group_annotations(
    frame: &mut UiFrame,
    group: &GroupLayout,
    label: &str,
    accent: [f32; 4],
    gaps: ShowcaseTileLabelGaps,
    label_font: f32,
    label_line_h: f32,
) {
    draw_projected_bounds(group.label_anchor.bounds, frame);
    frame.quad(GpuInstance {
        rect: [
            group.group_start_x,
            group.label_anchor.underline_y,
            group.group_w,
            gaps.underline_h,
        ],
        color: accent,
        user: 0,
    });
    frame.text(TextLabel {
        rect: [
            group.group_start_x,
            group.label_anchor.label_y,
            group.group_w,
            label_line_h,
        ],
        text: label.into(),
        color: color::PARCHMENT,
        align: TextAlign::Center,
        font_px: Some(label_font),
        ..Default::default()
    });
}

fn draw_corner_probe(
    frame: &mut UiFrame,
    group: &GroupLayout,
    corner_label: &str,
    gaps: ShowcaseTileLabelGaps,
    label_font: f32,
    label_line_h: f32,
) {
    draw_projected_bounds(group.label_anchor.bounds, frame);
    frame.quad(GpuInstance {
        rect: [
            group.group_start_x,
            group.label_anchor.underline_y,
            group.group_w,
            gaps.underline_h,
        ],
        color: [0.35, 0.70, 0.85, 0.95],
        user: 0,
    });
    frame.text(TextLabel {
        rect: [
            group.group_start_x,
            group.label_anchor.label_y,
            group.group_w,
            label_line_h,
        ],
        text: corner_label.into(),
        color: color::CHAMPAGNE,
        align: TextAlign::Center,
        font_px: Some(label_font),
        ..Default::default()
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::showcase_tile_layout::{
        ShowcaseTileProjectParams, showcase_tile_projected_bounds_px,
    };

    #[test]
    fn corner_projected_bounds_differ_from_screen_center() {
        let w = 1920.0;
        let h = 1080.0;
        let cam = CameraPreset::Guide.params(h);
        let tile_size = 48.0;
        let center_bounds = showcase_tile_projected_bounds_px(&ShowcaseTileProjectParams {
            win_w: w,
            win_h: h,
            cam: &cam,
            preset: TilePreset::Chinese,
            center_px: [w * 0.5, h * 0.5, 0.0],
            rotation_xyz_rad: TILE_ROTATION,
            placement_scale: 1.0,
            size_px: tile_size,
        });
        let (corner_x, corner_y) = ScreenCorner::TopLeft.tile_center(w, h, tile_size);
        let corner_bounds = showcase_tile_projected_bounds_px(&ShowcaseTileProjectParams {
            win_w: w,
            win_h: h,
            cam: &cam,
            preset: TilePreset::Chinese,
            center_px: [corner_x, corner_y, 0.0],
            rotation_xyz_rad: TILE_ROTATION,
            placement_scale: 1.0,
            size_px: tile_size,
        });
        assert!(
            (center_bounds.bottom() - corner_bounds.bottom()).abs() > 0.5
                || (center_bounds.width() - corner_bounds.width()).abs() > 0.5,
            "corner tile should project differently from center tile",
        );
    }
}
