//! The Wall — strategic wall supply overlay (live gameplay or shop preview).

mod data;
mod draw;
mod focus;
mod layout;
mod state;

use crate::core::tile::Suit;
use crate::game::wall_ledger::WallLedgerMode;
use crate::game::wall_stats::{FaceKey, WallCountView};
use crate::render::draw_cmd::UiFrame;

use super::{DrawCtx, SceneBehavior, SceneTransition, UpdateCtx};
use data::build_frame_context;
use draw::draw_strategic_frame;
use focus::WallFocusModel;
use layout::{read_boost, wall_layout};
use state::WallScreenState;

pub struct StrategicWallScene {
    pub mode: WallLedgerMode,
    pub screen: WallScreenState,
    pub focus: WallFocusModel,
}

impl StrategicWallScene {
    fn with_mode(mode: WallLedgerMode) -> Self {
        Self {
            mode,
            screen: WallScreenState {
                view: WallCountView::Remaining,
                selected: FaceKey {
                    suit: Suit::Souzu,
                    rank: 5,
                },
            },
            focus: WallFocusModel::new(),
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
                let frame_ctx = build_frame_context(ctx.run, scene.mode, scene.screen.view);
                let layout = wall_layout(w, h, jr);
                if let Some(transition) = scene.handle_input(ctx, &layout, &frame_ctx.stats) {
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

#[cfg(test)]
mod tests {
    use crate::game::wall_stats::GRID_FACE_ORDER;

    #[test]
    fn grid_has_thirty_eight_faces() {
        assert_eq!(GRID_FACE_ORDER.len(), 38);
    }
}
