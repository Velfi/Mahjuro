//! Focus graph and input handling for the strategic Wall screen.

use crate::game::event_bus::GameEvent;
use crate::game::wall_stats::{FaceKey, GRID_FACE_ORDER, WallStats};
use crate::render::draw_cmd::UiFrame;
use crate::render::theme::{ButtonState, ButtonVariant};
use crate::scenes::header_chrome::HeaderChromeMetrics;
use crate::scenes::{OverlayRequest, SceneTransition, UpdateCtx};
use crate::sfx_id::SfxId;
use crate::ui::focus_nav::{self, FocusDir};
use crate::ui::input::UiAction;
use crate::ui::widget::{self, ButtonSpec};
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use super::StrategicWallScene;
use super::layout::{GRID_ROWS, WallLayout, grid_cell_rect};

const NAV_BASE: u32 = 0xE200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LedgerNav {
    Back,
    Tile(usize),
}

impl LedgerNav {
    pub fn id(self) -> FocusId {
        FocusId(match self {
            Self::Back => NAV_BASE,
            Self::Tile(i) => NAV_BASE + 40 + i as u32,
        })
    }
}

pub fn ledger_nav_from_id(id: FocusId) -> LedgerNav {
    if id == LedgerNav::Back.id() {
        LedgerNav::Back
    } else if id.0 >= NAV_BASE + 40 {
        LedgerNav::Tile((id.0 - (NAV_BASE + 40)) as usize)
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

/// Explicit grid edges so sidebar chrome cannot merge suit rows during inference.
pub fn wall_ledger_nav_edges() -> Vec<(FocusId, FocusDir, FocusId)> {
    let mut edges = Vec::new();
    let mut edge = |from: LedgerNav, dir: FocusDir, to: LedgerNav| {
        edges.push((from.id(), dir, to.id()));
    };

    for (row_idx, &(start, count)) in GRID_ROWS.iter().enumerate() {
        for col in 0..count {
            let idx = start + col;
            let cur = LedgerNav::Tile(idx);
            if col + 1 < count {
                edge(cur, FocusDir::Right, LedgerNav::Tile(start + col + 1));
            }
            if row_idx > 0 {
                let (prev_start, prev_count) = GRID_ROWS[row_idx - 1];
                let target_col = col.min(prev_count - 1);
                edge(cur, FocusDir::Up, LedgerNav::Tile(prev_start + target_col));
            }
            if row_idx + 1 < GRID_ROWS.len() {
                let (next_start, next_count) = GRID_ROWS[row_idx + 1];
                let target_col = col.min(next_count - 1);
                edge(
                    cur,
                    FocusDir::Down,
                    LedgerNav::Tile(next_start + target_col),
                );
            }
        }
    }

    edge(LedgerNav::Back, FocusDir::Down, LedgerNav::Tile(0));
    edge(LedgerNav::Tile(0), FocusDir::Left, LedgerNav::Back);

    edges
}

impl StrategicWallScene {
    pub fn focused_nav(&self) -> Option<LedgerNav> {
        self.focus.tree.focused().map(ledger_nav_from_id)
    }

    pub fn go_back(overlay_request: &mut Option<OverlayRequest>) -> SceneTransition {
        *overlay_request = Some(OverlayRequest::Pop);
        None
    }

    fn activate(&mut self, nav: LedgerNav, _stats: &WallStats) -> bool {
        match nav {
            LedgerNav::Back => return true,
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
            HeaderChromeMetrics::from_window(w, layout.summary_y + layout.summary_h)
                .back_rect_left(),
            LedgerNav::Back,
        ));

        for (idx, entry) in stats.entries.iter().enumerate() {
            if !self.screen.face_visible(entry) {
                continue;
            }
            if let Some(rect) = grid_cell_rect(layout, idx) {
                out.push(FlatItem::new(
                    LedgerNav::Tile(idx).id(),
                    rect,
                    LedgerNav::Tile(idx),
                ));
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

        let items = self.flat_items(w, layout, stats);
        let edges = wall_ledger_nav_edges();
        let action = self.focus.tree.update_flat_with_edges(
            &items,
            TreeInput {
                actions: ctx.actions,
                button_clicks: ctx.button_clicks,
                cursor_pos: ctx.cursor_pos,
                window: (w, h),
                input_mode: ctx.input_mode,
                scroll_lines: 0.0,
            },
            &edges,
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

#[cfg(test)]
mod tests {
    use super::super::layout::{read_boost, wall_layout};
    use super::super::state::WallScreenState;
    use super::*;
    use crate::core::tile::Suit;
    use crate::game::wall_stats::{
        AbundanceState, ModifierBreakdown, TileLedgerEntry, TileLocationCounts, WallStats,
    };
    use crate::ui::input::{InputMode, UiAction};
    use crate::ui::widget_tree::TreeState;

    fn stub_stats() -> WallStats {
        let entries = GRID_FACE_ORDER
            .iter()
            .map(|&(suit, rank)| TileLedgerEntry {
                suit,
                rank,
                remaining: 4,
                seen: 0,
                total: 4,
                locations: TileLocationCounts {
                    in_wall: 4,
                    in_hand: 0,
                    played: 0,
                    discarded: 0,
                },
                draw_probability: 0.0,
                wall_share: 0.0,
                abundance: AbundanceState::Normal,
                modifiers: ModifierBreakdown::default(),
            })
            .collect();
        WallStats {
            entries,
            suit_summary: Default::default(),
            total_remaining: 136,
            total_wall: 136,
            most_common: Vec::new(),
            thin_exhausted: Vec::new(),
            abundant: Vec::new(),
            best_draws: Vec::new(),
            yaku_hints: Vec::new(),
            global_modifiers: ModifierBreakdown::default(),
        }
    }

    fn flat_items_at(w: f32, h: f32, stats: &WallStats) -> Vec<FlatItem<LedgerNav>> {
        let jr = read_boost(w, h);
        let layout = wall_layout(w, h, jr);
        StrategicWallScene {
            mode: crate::game::wall_ledger::WallLedgerMode::Live,
            screen: WallScreenState {
                selected: FaceKey {
                    suit: Suit::Souzu,
                    rank: 5,
                },
            },
            focus: WallFocusModel::new(),
            sidebar_scroll: crate::ui::smooth_scroll::SmoothScroll::new(),
        }
        .flat_items(w, &layout, stats)
    }

    #[test]
    fn down_from_souzu_row_reaches_pinzu_row() {
        let w = 1920.0;
        let h = 1080.0;
        let stats = stub_stats();
        let items = flat_items_at(w, h, &stats);
        let souzu5 = face_index(FaceKey {
            suit: Suit::Souzu,
            rank: 5,
        })
        .expect("souzu 5");
        let pinzu5 = face_index(FaceKey {
            suit: Suit::Pinzu,
            rank: 5,
        })
        .expect("pinzu 5");

        let mut tree = TreeState::new();
        tree.set_focus(LedgerNav::Tile(souzu5).id());
        let edges = wall_ledger_nav_edges();
        let _ = tree.update_flat_with_edges(
            &items,
            TreeInput {
                actions: &[UiAction::FocusDown],
                button_clicks: &[],
                cursor_pos: (0.0, 0.0),
                window: (w, h),
                input_mode: InputMode::Controller,
                scroll_lines: 0.0,
            },
            &edges,
        );
        let got = tree.focused().map(ledger_nav_from_id);
        assert_eq!(
            got,
            Some(LedgerNav::Tile(pinzu5)),
            "down from 5 Souzu should land on 5 Pinzu, got {got:?}"
        );
    }

    #[test]
    fn focus_nav_debug_snapshot_includes_grid_nodes() {
        let w = 1920.0;
        let h = 1080.0;
        let stats = stub_stats();
        let items = flat_items_at(w, h, &stats);
        let mut tree = TreeState::new();
        let edges = wall_ledger_nav_edges();
        let _ = tree.update_flat_with_edges(
            &items,
            TreeInput {
                actions: &[],
                button_clicks: &[],
                cursor_pos: (0.0, 0.0),
                window: (w, h),
                input_mode: InputMode::Controller,
                scroll_lines: 0.0,
            },
            &edges,
        );
        let snap = tree.focus_nav_debug_snapshot_flat(&items, |a| format!("{a:?}"));
        assert!(
            snap.nodes.len() >= 38,
            "expected grid + chrome nodes, got {}",
            snap.nodes.len()
        );
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
