//! Debug scene: stress-test rendering and picking with a flat tile grid + FPS.
//!
//! Entered from Debug → Tile Stress Lab…

use std::time::Instant;

use crate::core::deck::build_wall;
use crate::core::tile::Tile;
use crate::render::draw_cmd::UiFrame;
use crate::render::theme::{color, metrics, typography};
use crate::render::wgpu_renderer::{GpuInstance, PointLight, TextAlign, TextLabel};
use crate::ui::controller_hints::{
    HintStyle, push_screen_footer_hint, tile_stress_lab_footer_row,
};
use crate::ui::input::{InputMode, UiAction};
use crate::ui::widget_tree::{TreeInput, TreeState};

use super::tile_picker::{
    FlatTileStressConfig, STRESS_LAB_SCROLL_LINES_PX, camera_params,
    compute_flat_tile_stress_layout, footer_button_rects, wall_tiles_with_repeat,
};
use super::{BackgroundId, DrawCtx, SceneBehavior, SceneIntent, SceneTransition, UpdateCtx};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LabAction {
    TogglePick,
    Back,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum LabInputFocus {
    #[default]
    Controls,
    Tiles,
}

pub struct TileStressLabScene {
    has_suspended: bool,
    tree: TreeState,
    pickable: bool,
    tile_focus: usize,
    input_focus: LabInputFocus,
    hovered_tile: Option<usize>,
    fps_smoothed: f32,
    last_tick: Option<Instant>,
    wall_tiles: Vec<Tile>,
    display_tiles: Vec<Tile>,
    /// Repeat the full wall this many times (1 = 140 tiles).
    tile_repeat: usize,
    tile_scroll_y: f32,
}

impl TileStressLabScene {
    pub fn new(has_suspended: bool) -> Self {
        let wall_tiles = build_wall();
        let display_tiles = wall_tiles_with_repeat(&wall_tiles, 1);
        Self {
            has_suspended,
            tree: TreeState::default(),
            pickable: true,
            tile_focus: 0,
            input_focus: LabInputFocus::Tiles,
            hovered_tile: None,
            fps_smoothed: 60.0,
            last_tick: None,
            wall_tiles,
            display_tiles,
            tile_repeat: 1,
            tile_scroll_y: 0.0,
        }
    }

    fn refresh_display_tiles(&mut self) {
        self.display_tiles = wall_tiles_with_repeat(&self.wall_tiles, self.tile_repeat);
        self.tile_scroll_y = 0.0;
        self.tile_focus = 0;
    }

    fn go_back(&self, overlay_request: &mut Option<super::OverlayRequest>) -> SceneTransition {
        if self.has_suspended {
            *overlay_request = Some(super::OverlayRequest::Pop);
            None
        } else {
            Some(SceneIntent::MainMenu)
        }
    }

    fn apply_action(&mut self, action: LabAction) {
        match action {
            LabAction::TogglePick => {
                self.pickable = !self.pickable;
                if !self.pickable {
                    self.input_focus = LabInputFocus::Controls;
                }
            }
            LabAction::Back => {}
        }
    }

    fn tick_fps(&mut self) {
        let now = Instant::now();
        if let Some(prev) = self.last_tick.replace(now) {
            let dt = now.duration_since(prev).as_secs_f32().max(1e-6);
            let instant = 1.0 / dt;
            self.fps_smoothed = self.fps_smoothed * 0.9 + instant * 0.1;
        }
    }

    fn layout_for(
        &self,
        w: f32,
        h: f32,
        face_aspect: f32,
        hovered: Option<usize>,
    ) -> super::tile_picker::FlatTileStressLayout<LabAction> {
        let rects = footer_button_rects(w, h, 2);
        let footer = [
            (LabAction::TogglePick, rects[0]),
            (LabAction::Back, rects[1]),
        ];
        compute_flat_tile_stress_layout(
            w,
            h,
            FlatTileStressConfig {
                tiles: &self.display_tiles,
                face_aspect,
                scroll_y: self.tile_scroll_y,
                pickable: self.pickable,
                hovered_pick: hovered,
                footer_actions: &footer,
            },
        )
    }
}

impl SceneBehavior for TileStressLabScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        self.tick_fps();

        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let face_aspect = crate::persistence::load_settings()
            .tile_preset
            .face_long_ratio();
        let hover_for_layout = if self.pickable
            && matches!(ctx.input_mode, InputMode::Controller | InputMode::Keyboard)
            && self.input_focus == LabInputFocus::Tiles
        {
            Some(self.tile_focus)
        } else if self.pickable && ctx.input_mode == InputMode::Cursor {
            ctx.picked_hand_tile
        } else {
            None
        };
        let layout = self.layout_for(w, h, face_aspect, hover_for_layout);

        if !self.pickable {
            self.input_focus = LabInputFocus::Controls;
        }
        if layout.visible_count == 0 {
            self.tile_focus = 0;
        } else {
            self.tile_focus %= layout.visible_count;
        }

        let manual_tile_nav = self.pickable
            && matches!(ctx.input_mode, InputMode::Controller | InputMode::Keyboard)
            && self.input_focus == LabInputFocus::Tiles
            && layout.visible_count > 0;

        let mut tree_actions: Vec<UiAction> = Vec::new();
        for &a in ctx.actions {
            match a {
                UiAction::Cancel | UiAction::Pause => {
                    return self.go_back(ctx.overlay_request);
                }
                UiAction::InvertSelection
                    if self.pickable
                        && matches!(ctx.input_mode, InputMode::Controller | InputMode::Keyboard) =>
                {
                    self.input_focus = match self.input_focus {
                        LabInputFocus::Controls => {
                            if layout.visible_count > 0 {
                                LabInputFocus::Tiles
                            } else {
                                LabInputFocus::Controls
                            }
                        }
                        LabInputFocus::Tiles => LabInputFocus::Controls,
                    };
                }
                UiAction::FocusNext | UiAction::FocusDown if manual_tile_nav => {
                    self.tile_focus = (self.tile_focus + 1) % layout.visible_count;
                }
                UiAction::FocusPrev | UiAction::FocusUp if manual_tile_nav => {
                    self.tile_focus =
                        (self.tile_focus + layout.visible_count - 1) % layout.visible_count;
                }
                UiAction::NavigateHudPrev => {
                    self.tile_repeat = self.tile_repeat.saturating_sub(10).max(1);
                    self.refresh_display_tiles();
                }
                UiAction::NavigateHudNext => {
                    self.tile_repeat = (self.tile_repeat + 10).min(200);
                    self.refresh_display_tiles();
                }
                UiAction::PageNext => {
                    self.tile_scroll_y = (self.tile_scroll_y
                        + layout.scroll.viewport[3] * 0.85)
                        .min(layout.scroll.max_scroll_y);
                }
                UiAction::PagePrev => {
                    self.tile_scroll_y =
                        (self.tile_scroll_y - layout.scroll.viewport[3] * 0.85).max(0.0);
                }
                UiAction::Confirm | UiAction::CommitDiscard if manual_tile_nav => {}
                other => tree_actions.push(other),
            }
        }

        if ctx.scroll_lines.abs() > f32::EPSILON {
            self.tile_scroll_y = (self.tile_scroll_y
                + ctx.scroll_lines * STRESS_LAB_SCROLL_LINES_PX)
                .clamp(0.0, layout.scroll.max_scroll_y);
        }

        if manual_tile_nav {
            self.hovered_tile = Some(self.tile_focus);
        } else if self.pickable && ctx.input_mode == InputMode::Cursor {
            self.hovered_tile = ctx.picked_hand_tile;
        } else {
            self.hovered_tile = None;
        }

        let input = TreeInput {
            actions: &tree_actions,
            button_clicks: ctx.button_clicks,
            cursor_pos: ctx.cursor_pos,
            window: (w, h),
            input_mode: ctx.input_mode,
            scroll_lines: ctx.scroll_lines,
        };
        let fired = self.tree.update_flat(&layout.flat_items, input);
        match fired {
            Some(LabAction::Back) => self.go_back(ctx.overlay_request),
            Some(action) => {
                self.apply_action(action);
                None
            }
            None => None,
        }
    }

    fn draw_frame(&self, mut ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let scale = metrics::scene_scale(w, h);
        let focused = self.tree.focused();

        let face_aspect = ctx.tile_preset.face_long_ratio();
        let layout = self.layout_for(w, h, face_aspect, self.hovered_tile);

        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);
        frame.camera_override = Some(camera_params(h));

        frame.scene_lighting.push_smooth(PointLight {
            pos: [w * 0.5, h * 0.38, h * 1.35],
            radius: h * 2.9,
            color: color::rgb(color::PARCHMENT),
            intensity: 1.15,
        });

        let title_font = typography::size(typography::H20, h);
        let body_font = typography::size(typography::H42, h);
        let margin = (14.0 * scale).max(8.0);

        if !layout.placements.is_empty() {
            frame.showcase_tile_batch(layout.placements);
        }

        frame.text(TextLabel {
            rect: [margin, margin, w - margin * 2.0, title_font * 1.4],
            text: "Tile Stress Lab".into(),
            color: color::CHAMPAGNE,
            align: TextAlign::Left,
            font_px: Some(title_font),
            ..Default::default()
        });

        let pick_line = if self.pickable {
            match self.hovered_tile {
                Some(i) => format!("pick slot {i}"),
                None => "pick —".into(),
            }
        } else {
            "picking off".into()
        };
        frame.text(TextLabel {
            rect: [
                margin,
                margin + title_font * 1.5,
                w - margin * 2.0,
                body_font * 2.4,
            ],
            text: format!(
                "{} drawn  ·  ×{repeat} wall  ·  {pick_line}  ·  {:.0} FPS\n\
                 [ ] / LB RB ±10 tile count · scroll / PgUp PgDn · Z / L3 toggles footer vs tile focus.",
                layout.tile_count,
                self.fps_smoothed,
                repeat = self.tile_repeat,
            ),
            color: color::PARCHMENT,
            align: TextAlign::Left,
            font_px: Some(body_font),
            ..Default::default()
        });

        let header_font = (body_font * 0.92).max(11.0);
        for it in &layout.flat_items {
            let is_focused = focused == Some(it.id);
            frame.quad(GpuInstance {
                rect: it.rect,
                color: if is_focused {
                    color::alpha(color::BRASS, 0.92)
                } else {
                    color::alpha(color::WALNUT_INK, 0.94)
                },
                user: 0,
            });
            frame.text(TextLabel {
                rect: it.rect,
                text: footer_button_label(it.action, self.pickable),
                color: if is_focused {
                    color::WALNUT_DEEP
                } else {
                    color::CHAMPAGNE
                },
                align: TextAlign::Center,
                font_px: Some(header_font),
                ..Default::default()
            });
        }

        self.tree
            .register_flat_buttons(&layout.flat_items, &mut frame.buttons);

        ctx.stash_focus_nav_tree_flat(&self.tree, &layout.flat_items, |a| {
            footer_button_label(a, self.pickable)
        });

        push_screen_footer_hint(
            &mut frame,
            &ctx,
            tile_stress_lab_footer_row(
                ctx.input_mode,
                self.pickable,
                self.input_focus == LabInputFocus::Tiles,
                self.input_focus == LabInputFocus::Controls,
            ),
            HintStyle::standard(w, h),
        );
        frame.window_title = "Mahjuro — Tile Stress Lab".into();
        frame
    }

    fn has_blocking_overlay(&self) -> bool {
        true
    }
}

fn footer_button_label(action: LabAction, pickable: bool) -> String {
    match action {
        LabAction::TogglePick => {
            if pickable {
                "Pick: on".into()
            } else {
                "Pick: off".into()
            }
        }
        LabAction::Back => "Back".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_shows_full_wall() {
        let scene = TileStressLabScene::new(false);
        let layout = scene.layout_for(900.0, 500.0, 26.0 / 19.0, None);
        assert_eq!(layout.tile_count, 140);
        assert_eq!(layout.visible_count, 140);
    }

    #[test]
    fn tile_repeat_scales_wall() {
        let mut scene = TileStressLabScene::new(false);
        scene.tile_repeat = 3;
        scene.refresh_display_tiles();
        let layout = scene.layout_for(900.0, 500.0, 26.0 / 19.0, None);
        assert_eq!(layout.tile_count, 420);
    }
}
