//! The Wall — strategic wall supply overlay (live gameplay or shop preview).

mod data;
mod draw;
mod focus;
mod layout;
mod sidebar_scroll;
mod state;

use crate::core::tile::Suit;
use crate::game::wall_ledger::WallLedgerMode;
use crate::game::wall_stats::{FaceKey, selected_tile_details};
use crate::render::draw_cmd::UiFrame;
use crate::ui::smooth_scroll::SmoothScroll;

use super::{DrawCtx, SceneBehavior, SceneTransition, UpdateCtx};
use data::{build_frame_context, groups_by_face};
use draw::draw_strategic_frame;
use focus::WallFocusModel;
use layout::{read_boost, wall_layout};
use sidebar_scroll::{
    point_in_rect, sidebar_scroll_layout, sidebar_scrollbar, sidebar_scroll_y_from_cursor,
    SidebarScrollLayout,
};
use state::WallScreenState;

pub struct StrategicWallScene {
    pub mode: WallLedgerMode,
    pub screen: WallScreenState,
    pub focus: WallFocusModel,
    pub sidebar_scroll: SmoothScroll,
    dragging_scrollbar: bool,
    scroll_drag_grab_y: f32,
    prev_mouse_down: bool,
}

impl StrategicWallScene {
    fn with_mode(mode: WallLedgerMode) -> Self {
        Self {
            mode,
            screen: WallScreenState {
                selected: FaceKey {
                    suit: Suit::Souzu,
                    rank: 5,
                },
            },
            focus: WallFocusModel::new(),
            sidebar_scroll: SmoothScroll::new(),
            dragging_scrollbar: false,
            scroll_drag_grab_y: 0.0,
            prev_mouse_down: false,
        }
    }
}

pub enum WallLedgerScene {
    Strategic(StrategicWallScene),
}

impl WallLedgerScene {
    pub fn live() -> Self {
        Self::Strategic(StrategicWallScene::with_mode(WallLedgerMode::Live))
    }

    pub fn shop_preview() -> Self {
        Self::Strategic(StrategicWallScene::with_mode(WallLedgerMode::ShopPreview))
    }
}

impl SceneBehavior for WallLedgerScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        match self {
            Self::Strategic(scene) => {
                let w = ctx.layout.window_w;
                let h = ctx.layout.window_h;
                let jr = read_boost(w, h);
                let frame_ctx = build_frame_context(ctx.run, scene.mode);
                let layout = wall_layout(w, h, jr);
                let groups = groups_by_face(&frame_ctx.ledger);
                let group = groups
                    .get(&(scene.screen.selected.suit, scene.screen.selected.rank))
                    .copied();
                let selected_details = selected_tile_details(
                    &frame_ctx.stats,
                    scene.screen.selected,
                    &ctx.run.tile_debuffs,
                    group,
                );
                let scroll_layout = sidebar_scroll_layout(
                    &layout,
                    &frame_ctx.stats,
                    selected_details.as_ref(),
                    scene.mode,
                );
                scene
                    .sidebar_scroll
                    .set_max(scroll_layout.max_scroll_px.ceil() as u32);
                update_sidebar_scrollbar(scene, &ctx, &scroll_layout, &layout);
                if ctx.scroll_lines.abs() > 0.001
                    && scroll_layout.max_scroll_px > 0.0
                    && !scene.dragging_scrollbar
                {
                    scene
                        .sidebar_scroll
                        .scroll_by(ctx.scroll_lines * scroll_layout.wheel_step_px);
                }
                if let Some(transition) =
                    scene.handle_input(ctx, &layout, &frame_ctx.stats, &scroll_layout)
                {
                    return transition;
                }
                None
            }
        }
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        match self {
            Self::Strategic(scene) => draw_strategic_frame(scene, ctx),
        }
    }
}

fn update_sidebar_scrollbar(
    scene: &mut StrategicWallScene,
    ctx: &UpdateCtx<'_>,
    scroll_layout: &SidebarScrollLayout,
    layout: &layout::WallLayout,
) {
    let max_scroll = scroll_layout.max_scroll_px;
    let scroll_y = scene.sidebar_scroll.target();
    let sb = sidebar_scrollbar(
        layout,
        scroll_layout.clip,
        scroll_layout.content_h,
        scroll_y,
        max_scroll,
    );
    let mouse_down = ctx.mouse_left_down;
    let mouse_click = mouse_down && !scene.prev_mouse_down;
    if let Some(sb) = sb {
        if scene.dragging_scrollbar && mouse_down {
            let y = sidebar_scroll_y_from_cursor(
                ctx.cursor_pos.1,
                scene.scroll_drag_grab_y,
                &sb,
                max_scroll,
            );
            scene.sidebar_scroll.jump(y);
        } else if mouse_click {
            let (mx, my) = ctx.cursor_pos;
            if point_in_rect(mx, my, sb.thumb) {
                scene.dragging_scrollbar = true;
                scene.scroll_drag_grab_y = my - sb.thumb[1];
                let y = sidebar_scroll_y_from_cursor(my, scene.scroll_drag_grab_y, &sb, max_scroll);
                scene.sidebar_scroll.jump(y);
            } else if point_in_rect(mx, my, sb.hit_track) {
                scene.dragging_scrollbar = true;
                scene.scroll_drag_grab_y = sb.thumb[3] * 0.5;
                let y = sidebar_scroll_y_from_cursor(my, scene.scroll_drag_grab_y, &sb, max_scroll);
                scene.sidebar_scroll.jump(y);
            }
        }
    } else {
        scene.dragging_scrollbar = false;
    }
    if !mouse_down {
        scene.dragging_scrollbar = false;
    }
    scene.prev_mouse_down = mouse_down;
}

#[cfg(test)]
mod tests {
    use crate::game::wall_stats::GRID_FACE_ORDER;

    #[test]
    fn grid_has_thirty_eight_faces() {
        assert_eq!(GRID_FACE_ORDER.len(), 38);
    }
}
