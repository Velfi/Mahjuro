//! Focus graph and input handling for the strategic Wall screen.

use crate::game::event_bus::GameEvent;
use crate::game::wall_stats::{FaceKey, GRID_FACE_ORDER, WallStats};
use crate::render::draw_cmd::UiFrame;
use crate::render::theme::{ButtonState, ButtonVariant};
use crate::scenes::header_chrome::HeaderChromeMetrics;
use crate::scenes::{OverlayRequest, SceneTransition, UpdateCtx};
use crate::sfx_id::SfxId;
use crate::ui::focus_nav;
use crate::ui::input::UiAction;
use crate::ui::widget::{self, ButtonSpec};
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use super::StrategicWallScene;
use super::layout::{WallLayout, grid_cell_rect, text_line_h, view_toggle_rect};

const NAV_BASE: u32 = 0xE200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LedgerNav {
    Back,
    View,
    Summary(usize),
    Tile(usize),
}

impl LedgerNav {
    pub fn id(self) -> FocusId {
        FocusId(match self {
            Self::Back => NAV_BASE,
            Self::View => NAV_BASE + 10,
            Self::Summary(i) => NAV_BASE + 20 + i as u32,
            Self::Tile(i) => NAV_BASE + 40 + i as u32,
        })
    }
}

pub fn ledger_nav_from_id(id: FocusId) -> LedgerNav {
    if id == LedgerNav::Back.id() {
        LedgerNav::Back
    } else if id == LedgerNav::View.id() {
        LedgerNav::View
    } else if id.0 >= NAV_BASE + 40 {
        LedgerNav::Tile((id.0 - (NAV_BASE + 40)) as usize)
    } else if id.0 >= NAV_BASE + 20 {
        LedgerNav::Summary((id.0 - (NAV_BASE + 20)) as usize)
    } else {
        LedgerNav::Back
    }
}

pub struct WallFocusModel {
    pub tree: TreeState,
}

impl WallFocusModel {
    pub fn new() -> Self {
        let mut tree = TreeState::new();
        if let Some(i) = face_index(FaceKey {
            suit: crate::core::tile::Suit::Souzu,
            rank: 5,
        }) {
            tree.set_focus(LedgerNav::Tile(i).id());
        }
        Self { tree }
    }
}

pub fn face_index(face: FaceKey) -> Option<usize> {
    GRID_FACE_ORDER
        .iter()
        .position(|&(s, r)| s == face.suit && r == face.rank)
}

impl StrategicWallScene {
    pub fn focused_nav(&self) -> Option<LedgerNav> {
        self.focus.tree.focused().map(ledger_nav_from_id)
    }

    pub fn go_back(overlay_request: &mut Option<OverlayRequest>) -> SceneTransition {
        *overlay_request = Some(OverlayRequest::Pop);
        None
    }

    fn activate(&mut self, nav: LedgerNav, stats: &WallStats) -> bool {
        match nav {
            LedgerNav::Back => return true,
            LedgerNav::View => self.screen.view = self.screen.view.next(),
            LedgerNav::Summary(i) => {
                if let Some(hint) = stats.best_draws.get(i) {
                    self.screen.selected = hint.face;
                    if let Some(tile_i) = face_index(hint.face) {
                        self.focus.tree.set_focus(LedgerNav::Tile(tile_i).id());
                    }
                }
            }
            LedgerNav::Tile(i) => {
                if let Some(&(suit, rank)) = GRID_FACE_ORDER.get(i) {
                    self.screen.selected = FaceKey { suit, rank };
                }
            }
        }
        false
    }

    pub fn flat_items(
        &self,
        w: f32,
        layout: &WallLayout,
        stats: &WallStats,
    ) -> Vec<FlatItem<LedgerNav>> {
        let mut out = Vec::new();
        out.push(FlatItem::new(
            LedgerNav::Back.id(),
            HeaderChromeMetrics::from_window(w, layout.detail_y + layout.detail_h)
                .back_rect_left(),
            LedgerNav::Back,
        ));

        out.push(FlatItem::new(
            LedgerNav::View.id(),
            view_toggle_rect(w, layout),
            LedgerNav::View,
        ));

        let line = text_line_h(layout.caption_px);
        let section_line = text_line_h(layout.caption_px * 0.94);
        let mut section_top = layout.summary_y + layout.summary_pad();
        section_top += text_line_h(layout.caption_px * 1.02) + 8.0;
        section_top += (line + 2.0) * 2.0;
        section_top += 6.0 + 7.0;
        section_top += section_line + 4.0;
        section_top += (line + 2.0) * 5.0;
        section_top += 4.0 + 7.0;
        section_top += section_line + 4.0;
        for (i, _) in stats.best_draws.iter().enumerate().take(3) {
            out.push(FlatItem::new(
                LedgerNav::Summary(i).id(),
                [
                    layout.summary_x + 8.0,
                    section_top + i as f32 * (line * 2.0 + 2.0),
                    layout.summary_w - 16.0,
                    line * 2.0,
                ],
                LedgerNav::Summary(i),
            ));
        }

        for (idx, entry) in stats.entries.iter().enumerate() {
            if !self.screen.face_visible(entry) {
                continue;
            }
            if let Some(rect) = grid_cell_rect(layout, idx) {
                out.push(FlatItem::new(LedgerNav::Tile(idx).id(), rect, LedgerNav::Tile(idx)));
            }
        }

        out
    }

    fn sync_selected_from_focus(&mut self) {
        if let Some(LedgerNav::Tile(i)) = self.focused_nav() {
            if let Some(&(suit, rank)) = GRID_FACE_ORDER.get(i) {
                self.screen.selected = FaceKey { suit, rank };
            }
        }
    }

    pub fn handle_input(
        &mut self,
        ctx: UpdateCtx<'_>,
        layout: &WallLayout,
        stats: &WallStats,
    ) -> Option<SceneTransition> {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;

        for a in ctx.actions {
            if matches!(a, UiAction::Cancel | UiAction::Pause | UiAction::Help) {
                ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
                return Some(Self::go_back(ctx.overlay_request));
            }
        }

        for a in ctx.actions {
            match a {
                UiAction::InvertSelection => {
                    self.screen.view = self.screen.view.next();
                    ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
                }
                UiAction::Delete => {
                    self.screen.view = self.screen.view.prev();
                    ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
                }
                _ => {}
            }
        }

        let items = self.flat_items(w, layout, stats);
        let action = self.focus.tree.update_flat(
            &items,
            TreeInput {
                actions: ctx.actions,
                button_clicks: ctx.button_clicks,
                cursor_pos: ctx.cursor_pos,
                window: (w, h),
                input_mode: ctx.input_mode,
                scroll_lines: 0.0,
            },
        );
        if self.focus.tree.take_focus_changed() {
            ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
        }
        self.sync_selected_from_focus();

        match action {
            Some(LedgerNav::Back) => {
                ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
                return Some(Self::go_back(ctx.overlay_request));
            }
            Some(nav) => {
                if self.activate(nav, stats) {
                    ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
                    return Some(Self::go_back(ctx.overlay_request));
                }
                ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
            }
            None => {}
        }

        None
    }
}

pub fn push_back_button(frame: &mut UiFrame, tree: &TreeState, w: f32, h: f32) {
    let scale = (w.min(h)) / 600.0;
    let back = HeaderChromeMetrics::from_window(w, h).back_rect_left();
    let focused = tree.focused() == Some(LedgerNav::Back.id());
    let mut nav_quads = Vec::new();
    let mut nav_texts = Vec::new();
    let mut junk_buttons = Vec::new();
    widget::push_button(
        &mut nav_quads,
        &mut nav_texts,
        &mut junk_buttons,
        ButtonSpec {
            rect: back,
            label: "Back",
            variant: ButtonVariant::Default,
            state: if focused {
                ButtonState::Hover
            } else {
                ButtonState::Rest
            },
            action: UiAction::Confirm,
        },
    );
    if focused {
        focus_nav::push_focus_ring(back, scale, w, h, &mut nav_quads);
    }
    frame.quads(nav_quads);
    for label in nav_texts {
        frame.texts([label]);
    }
}
