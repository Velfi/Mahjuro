//! Guide — dense in-game reference for tiles, melds, flowers, scoring, economy, and yaku.
//!
//! Paginated 3D-tile diagrams with glossary-style definitions. Scoring basics
//! on page 4, economy on page 5, Tanuki's Tips on page 6; yaku detail pages follow.
//!
//! Opened from the gameplay-table guide book, the in-run `Help` shortcut
//! (keyboard or controller Select / View / −), the tutorial summary, or
//! shop help. The previous scene is suspended by `App` and restored when
//! the player presses Back.

use crate::core::progression::PlayerProgress;
use crate::core::yaku::YakuKind;
use crate::game::event_bus::GameEvent;
use crate::render::doc_tile_camera::doc_tile_camera;
use crate::render::draw_cmd::UiFrame;
use crate::render::theme::color;
use crate::render::wgpu_renderer::PointLight;
use crate::sfx_id::SfxId;
use crate::ui::controller_hints::{HintStyle, guide_footer_row, push_screen_footer_hint};
use crate::ui::input::UiAction;
use crate::ui::smooth_scroll::SmoothScroll;
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use super::{BackgroundId, DrawCtx, SceneBehavior, SceneTransition, UpdateCtx};

// ── Page indices ──────────────────────────────────────────────────────────
//
// Four fixed reference pages, a yaku intro page, scoring basics, Tanuki's
// Tips, then yaku detail pages from `PlayerProgress::available_yaku` (sorted
// lowest payout first; Kokushi Musō omitted until first cash-in).

pub(super) const PAGE_TILES: usize = 0;
pub(super) const PAGE_MELDS: usize = 1;
pub(super) const PAGE_YAKU: usize = 2;
pub(super) const PAGE_FLOWERS: usize = 3;
pub(super) const PAGE_SCORING: usize = 4;
pub(super) const PAGE_ECONOMY: usize = 5;
pub(super) const PAGE_TANUKI_TIPS: usize = 6;
pub(super) const YAKU_PAGE_START: usize = 7;
/// How many yaku entries to stack on one guide page when they fit.
fn yaku_needs_solo_guide_page(yk: YakuKind) -> bool {
    matches!(yk, YakuKind::Chiitoitsu | YakuKind::KokushiMusou)
}

/// Split visible yaku into guide pages (pairs of entries; chiitoitsu / kokushi solo).
fn yaku_guide_chunks(yaku: &[YakuKind]) -> Vec<Vec<YakuKind>> {
    let mut chunks: Vec<Vec<YakuKind>> = Vec::new();
    let mut i = 0;
    while i < yaku.len() {
        let yk = yaku[i];
        if yaku_needs_solo_guide_page(yk) {
            chunks.push(vec![yk]);
            i += 1;
            continue;
        }
        if i + 1 < yaku.len() && !yaku_needs_solo_guide_page(yaku[i + 1]) {
            chunks.push(vec![yk, yaku[i + 1]]);
            i += 2;
        } else {
            chunks.push(vec![yk]);
            i += 1;
        }
    }
    chunks
}

fn total_pages(progress: &PlayerProgress) -> usize {
    YAKU_PAGE_START + yaku_guide_chunks(&progress.available_yaku()).len()
}

pub(super) fn yaku_chunk_for_page(page: usize, progress: &PlayerProgress) -> Option<Vec<YakuKind>> {
    if page < YAKU_PAGE_START {
        return None;
    }
    let idx = page - YAKU_PAGE_START;
    yaku_guide_chunks(&progress.available_yaku())
        .get(idx)
        .cloned()
}

// ── Navigation ────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GuideNav {
    Prev,
    Back,
    Next,
}

impl GuideNav {
    fn id(self) -> FocusId {
        FocusId(0xD000 + self as u32)
    }
}

// ── Scene ─────────────────────────────────────────────────────────────────

pub struct GuideScene {
    page: usize,
    tree: TreeState,
    tips_scroll: SmoothScroll,
}

impl Default for GuideScene {
    fn default() -> Self {
        Self::new()
    }
}

impl GuideScene {
    pub fn new() -> Self {
        Self::with_page(0)
    }

    pub fn with_page(page: usize) -> Self {
        Self {
            page,
            tree: TreeState::new(),
            tips_scroll: SmoothScroll::new(),
        }
    }

    /// Guide page index for economy / storeroom reference.
    pub const ECONOMY_PAGE: usize = PAGE_ECONOMY;

    fn reset_tips_scroll(&self) {
        self.tips_scroll.jump(0.0);
    }

    #[cfg(feature = "game")]
    pub(crate) fn is_tanuki_tips_page(&self) -> bool {
        self.page == PAGE_TANUKI_TIPS
    }

    fn flat_items(&self, w: f32, h: f32) -> Vec<FlatItem<GuideNav>> {
        let layout = GuideLayout::new(w, h);
        let chrome = layout.header_chrome();
        vec![
            FlatItem::new(GuideNav::Back.id(), chrome.back, GuideNav::Back),
            FlatItem::new(GuideNav::Prev.id(), chrome.prev, GuideNav::Prev),
            FlatItem::new(GuideNav::Next.id(), chrome.next, GuideNav::Next),
        ]
    }

    fn go_back(&self, overlay_request: &mut Option<super::OverlayRequest>) -> SceneTransition {
        *overlay_request = Some(super::OverlayRequest::Pop);
        None
    }
}

impl SceneBehavior for GuideScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        let pages = total_pages(ctx.progress);

        for a in ctx.actions {
            if matches!(a, UiAction::Cancel | UiAction::Pause | UiAction::Help) {
                return self.go_back(ctx.overlay_request);
            }
        }

        for a in ctx.actions {
            match a {
                UiAction::TabPrev | UiAction::PagePrev => {
                    if self.page > 0 {
                        ctx.bus.push(GameEvent::UiSound(SfxId::TileClick));
                        self.page -= 1;
                        self.reset_tips_scroll();
                    } else {
                        ctx.bus.push(GameEvent::UiSound(SfxId::InvalidAction));
                    }
                }
                UiAction::TabNext | UiAction::PageNext => {
                    if self.page + 1 < pages {
                        ctx.bus.push(GameEvent::UiSound(SfxId::TileClick));
                        self.page += 1;
                        self.reset_tips_scroll();
                    } else {
                        ctx.bus.push(GameEvent::UiSound(SfxId::InvalidAction));
                    }
                }
                _ => {}
            }
        }

        if self.page == PAGE_TANUKI_TIPS {
            let w = ctx.layout.window_w;
            let h = ctx.layout.window_h;
            let layout = GuideLayout::new(w, h);
            let (content_top, content_floor) = guide_content_band(
                w,
                h,
                layout.header_chrome().back,
                page_nav_subtitle(self.page),
            );
            let tips_layout = tanuki_tips_scroll_layout(&layout, content_top, content_floor);
            self.tips_scroll
                .set_max(tips_layout.max_scroll_px.ceil() as u32);

            if ctx.scroll_lines.abs() > 0.001 && tips_layout.max_scroll_px > 0.0 {
                self.tips_scroll
                    .scroll_by(ctx.scroll_lines * tips_layout.wheel_step_px);
            }

            let card_step = tips_layout.cell_w + tips_layout.gap;
            for a in ctx.actions {
                match a {
                    UiAction::FocusNext if tips_layout.max_scroll_px > 0.0 => {
                        self.tips_scroll.scroll_by(card_step);
                    }
                    UiAction::FocusPrev if tips_layout.max_scroll_px > 0.0 => {
                        self.tips_scroll.scroll_by(-card_step);
                    }
                    _ => {}
                }
            }
        }

        let items = self.flat_items(ctx.layout.window_w, ctx.layout.window_h);
        let action = self.tree.update_flat(
            &items,
            TreeInput {
                actions: ctx.actions,
                button_clicks: ctx.button_clicks,
                cursor_pos: ctx.cursor_pos,
                window: (ctx.layout.window_w, ctx.layout.window_h),
                input_mode: ctx.input_mode,
                scroll_lines: 0.0,
            },
        );
        if self.tree.take_focus_changed() {
            ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
        }

        match action {
            Some(GuideNav::Prev) if self.page > 0 => {
                ctx.bus.push(GameEvent::UiSound(SfxId::TileClick));
                self.page -= 1;
                self.reset_tips_scroll();
                None
            }
            Some(GuideNav::Prev) => {
                ctx.bus.push(GameEvent::UiSound(SfxId::InvalidAction));
                None
            }
            Some(GuideNav::Next) if self.page + 1 < pages => {
                ctx.bus.push(GameEvent::UiSound(SfxId::TileClick));
                self.page += 1;
                self.reset_tips_scroll();
                None
            }
            Some(GuideNav::Next) => {
                ctx.bus.push(GameEvent::UiSound(SfxId::InvalidAction));
                None
            }
            Some(GuideNav::Back) => {
                ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
                self.go_back(ctx.overlay_request)
            }
            None => None,
        }
    }

    fn draw_frame(&self, mut ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let scale = (w.min(h)) / 600.0;
        let progress = ctx.progress;

        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);

        // ── Camera ────────────────────────────────────────────────
        frame.camera_override = Some(doc_tile_camera(h));
        frame.showcase_render_hints.layout_use_ray_plane_z = true;
        frame.showcase_render_hints.doc_tile_no_shadow = true;

        // ── Lights ────────────────────────────────────────────────
        // Match the Yaku Journal: one soft, high, wide-radius fill. Multiple
        // overlapping lights at high intensity caused harsh specular streaks on
        // tile faces against this scene's black backdrop (same issue journal fixed).
        frame.scene_lighting.push_smooth(PointLight {
            pos: [w * 0.5, h * 0.38, h * 1.35],
            radius: h * 2.9,
            color: color::rgb(color::PARCHMENT),
            intensity: 1.15,
        });

        // ── Chrome + page content ─────────────────────────────────
        let pages = total_pages(progress);
        let layout = GuideLayout::new(w, h);
        let (page_title, groups) = page_content(self.page, progress);
        let subtitle = page_nav_subtitle(self.page);
        let nav_header = guide_nav_header(w, h, layout.header_chrome().back, subtitle);
        let content_top = push_guide_chrome(&mut frame, &layout, nav_header.content_top);
        push_guide_header_nav(
            &mut frame,
            &layout,
            &self.tree,
            self.page,
            pages,
            scale,
            w,
            h,
            page_title,
            &nav_header,
            subtitle,
        );
        let content_floor = layout.content_bottom;
        let cam = frame.camera_override.expect("guide camera");

        if self.page == PAGE_TILES {
            draw_tiles_page(
                &mut frame,
                &layout,
                w,
                h,
                scale,
                &groups,
                &cam,
                content_top,
                content_floor,
            );
        } else if self.page == PAGE_MELDS {
            draw_melds_page(
                &mut frame,
                &layout,
                progress,
                w,
                h,
                scale,
                &groups,
                &cam,
                content_top,
                content_floor,
            );
        } else if self.page == PAGE_YAKU {
            draw_yaku_intro_page(
                &mut frame,
                &layout,
                w,
                h,
                scale,
                &groups,
                &cam,
                content_top,
                content_floor,
            );
        } else if self.page == PAGE_FLOWERS {
            draw_flowers_page(
                &mut frame,
                &layout,
                w,
                h,
                scale,
                &groups,
                &cam,
                content_top,
                content_floor,
            );
        } else if self.page == PAGE_SCORING {
            draw_scoring_page(
                &mut frame,
                &ctx,
                &layout,
                progress,
                w,
                h,
                scale,
                &groups,
                content_top,
                content_floor,
            );
        } else if self.page == PAGE_ECONOMY {
            draw_economy_page(&mut frame, &layout, w, h, &cam, content_top, content_floor);
        } else if self.page == PAGE_TANUKI_TIPS {
            let (content_top, content_floor) =
                guide_content_band(w, h, layout.header_chrome().back, subtitle);
            let tips_layout = tanuki_tips_scroll_layout(&layout, content_top, content_floor);
            self.tips_scroll
                .set_max(tips_layout.max_scroll_px.ceil() as u32);
            let scroll_px = self.tips_scroll.tick();
            draw_tanuki_tips_page(&mut frame, &layout, h, &tips_layout, scroll_px);
        } else if let Some(chunk) = yaku_chunk_for_page(self.page, progress) {
            draw_yaku_guide_page(
                &mut frame,
                progress,
                w,
                h,
                scale,
                &chunk,
                content_top,
                content_floor,
                &cam,
            );
        }

        push_screen_footer_hint(
            &mut frame,
            &ctx,
            guide_footer_row(ctx.input_mode),
            HintStyle::standard(w, h),
        );

        frame.window_title = "Mahjuro \u{2014} Guide".into();
        let items = self.flat_items(w, h);
        ctx.stash_focus_nav_tree_flat(&self.tree, &items, |a| format!("{a:?}"));
        frame
    }
}

#[allow(unused_imports)]
mod content;
#[allow(unused_imports)]
mod economy;
#[allow(unused_imports)]
mod economy_flow;
#[allow(unused_imports)]
mod economy_rules;
#[allow(unused_imports)]
mod example_grid;
#[allow(unused_imports)]
mod flowers_page;
#[allow(unused_imports)]
mod layout;
#[allow(unused_imports)]
mod melds_page;
#[allow(unused_imports)]
mod page_panels;
#[allow(unused_imports)]
mod scoring_diagram;
#[allow(unused_imports)]
mod scoring_page;
#[allow(unused_imports)]
mod scoring_panels;
#[allow(unused_imports)]
mod tanuki_tips;
#[allow(unused_imports)]
mod tile_layout;
#[allow(unused_imports)]
mod tiles_page;
#[allow(unused_imports)]
mod yaku_detail;
#[allow(unused_imports)]
mod yaku_intro_page;
#[allow(unused_imports)]
mod yaku_page;

#[cfg(test)]
mod tests;

pub(crate) use content::{TileGroup, page_content};
pub(crate) use layout::{
    GuideLayout, GuideNavHeader, guide_nav_header, page_nav_subtitle, push_guide_chrome,
    push_guide_header_nav,
};
#[allow(unused_imports)]
pub(crate) use page_panels::example_structure_yaku;
pub use scoring_diagram::push_gameplay_cash_in_overlay;
pub(crate) use scoring_panels::draw_tutorial_scoring_diagram;
pub(crate) use tile_layout::yaku_shape_text;
pub(crate) use yaku_detail::{dense_text_block_height, push_dense_text, yaku_guide_detail};
pub(crate) use yaku_page::yaku_page;

use economy::draw_economy_page;
use flowers_page::draw_flowers_page;
use melds_page::draw_melds_page;
use scoring_page::draw_scoring_page;
use tanuki_tips::{draw_tanuki_tips_page, guide_content_band, tanuki_tips_scroll_layout};
use tiles_page::draw_tiles_page;
use yaku_detail::draw_yaku_guide_page;
use yaku_intro_page::draw_yaku_intro_page;
