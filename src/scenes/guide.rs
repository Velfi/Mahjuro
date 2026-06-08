//! Guide — dense in-game reference for tiles, melds, flowers, scoring, economy, and yaku.
//!
//! Paginated 3D-tile diagrams with glossary-style definitions. Scoring basics
//! on page 4, economy on page 5, Tanuki's Tips on page 6; yaku detail pages follow.
//!
//! Opened from the gameplay-table guide book, the in-run `Help` shortcut
//! (keyboard or controller Select / View / −), the tutorial summary, or
//! shop help. The previous scene is suspended by `App` and restored when
//! the player presses Back.

use crate::core::hand::{MeldKind, validate_selection};
use crate::core::memorial_talisman::MemorialTalismanKind;
use crate::core::progression::PlayerProgress;
use crate::core::relic::{RelicId, all_relic_defs, relic_visual};
use crate::core::tag::TagKind;
use crate::core::talisman::TalismanKind;
use crate::core::tile::{Suit, Tile};
use crate::core::tile_pack::{TilePackKind, PACK_ASPECT_W_OVER_H};
use crate::core::yaku::{YakuKind, detect_yaku_with_wind};
use crate::core::zodiac::ZodiacKind;
use crate::game::event_bus::GameEvent;
use crate::persistence::TilePreset;
use crate::render::decal::load_ui_font;
use crate::render::consumable_prop_scale::for_sale_talisman_tablet_extent;
use crate::render::draw_cmd::{
    CameraParams, DrawCmd, ImageQuad, ImageQuadSource, Object3d, Object3dKind, SceneLighting,
    ShowcaseTilePlacement, UiFrame, camera_facing_euler_xyz_rad,
};
use crate::render::table_transform::{
    compose_rotation_euler, mat4_to_euler_xyz_rad, rot_euler_xyz_rad,
};
use crate::render::world_space::{
    object3d_pos_triple_for_world_center, world_on_camera_ray_plane_z,
};
use crate::render::gameplay_glb;
use crate::render::showcase_tile_layout::{
    ShowcaseTileLabelGaps, showcase_tile_group_label_anchor, showcase_tile_merge_projected_group,
};
use crate::render::theme::{ButtonState, ButtonVariant, color, metrics, typography};
use crate::render::vocabulary_colors::{GlossaryMode, color_for_token, text_effect_for_glossary_tint};
use crate::render::wgpu_renderer::{GpuInstance, PointLight, TextAlign, TextBlockVerticalAlign, TextLabel};
use crate::ui::styled_text::push_keyword_label;
use crate::sfx_id::SfxId;
use crate::ui::clip::intersect_rect;
use crate::ui::styled_text;
use crate::ui::controller_hints::{
    HintStyle, guide_footer_row, push_screen_footer_hint, screen_footer_reserve,
};
use crate::ui::focus_nav;
use crate::ui::input::UiAction;
use crate::ui::smooth_scroll::SmoothScroll;
use crate::ui::chart_primitives::{ChartClip, push_yaku_pill, yaku_pill_width};
use crate::ui::temptation_icons::temptation_icon_source;
use crate::ui::widget::{self, wrap_text};
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use super::archive_career::{yaku_pill_face, yaku_pill_ink, yaku_pill_rim};
use super::economy_intro_copy;
use super::flowers_intro_copy;
use super::header_chrome::{HeaderChromeMetrics, HeaderTitleLayout};
use super::melds_intro_copy;
use super::scoring_intro_copy;
use super::tanuki_tips_intro_copy;
use super::tiles_intro_copy;
use super::yaku_intro_copy;
use super::{BackgroundId, DrawCtx, SceneBehavior, SceneTransition, UpdateCtx};

use glam::{Mat4, Quat, Vec3};

// ── Page indices ──────────────────────────────────────────────────────────
//
// Four fixed reference pages, a yaku intro page, scoring basics, Tanuki's
// Tips, then yaku detail pages from `PlayerProgress::available_yaku` (sorted
// lowest payout first; Kokushi Musō omitted until first cash-in).

const PAGE_TILES: usize = 0;
const PAGE_MELDS: usize = 1;
const PAGE_YAKU: usize = 2;
const PAGE_FLOWERS: usize = 3;
const PAGE_SCORING: usize = 4;
const PAGE_ECONOMY: usize = 5;
const PAGE_TANUKI_TIPS: usize = 6;
const YAKU_PAGE_START: usize = 7;
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

fn yaku_chunk_for_page(page: usize, progress: &PlayerProgress) -> Option<Vec<YakuKind>> {
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
enum GuideNav {
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
        let cam_scale = h / 1600.0;
        frame.camera_override = Some(CameraParams {
            eye: [0.0, -200.0 * cam_scale, 2040.0 * cam_scale],
            target: [0.0, -50.0 * cam_scale, 0.0],
            up: [0.0, 0.0, 1.0],
            fovy_deg: 45.0,
            clip_near: None,
            clip_far: None,
        });
        frame.showcase_render_hints.layout_use_ray_plane_z = true;

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
            draw_economy_page(
                &mut frame,
                &layout,
                w,
                h,
                &cam,
                content_top,
                content_floor,
            );
        } else if self.page == PAGE_TANUKI_TIPS {
            let (content_top, content_floor) = guide_content_band(
                w,
                h,
                layout.header_chrome().back,
                subtitle,
            );
            let tips_layout = tanuki_tips_scroll_layout(&layout, content_top, content_floor);
            self.tips_scroll
                .set_max(tips_layout.max_scroll_px.ceil() as u32);
            let scroll_px = self.tips_scroll.tick();
            draw_tanuki_tips_page(
                &mut frame,
                &layout,
                h,
                &tips_layout,
                scroll_px,
            );
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

// ── Page content ──────────────────────────────────────────────────────────

/// A labelled group of tiles forming one meld (or tile-category cluster).
pub(crate) struct TileGroup {
    pub label: &'static str,
    pub tiles: Vec<Tile>,
    /// Accent for titles and framed-panel tint.
    pub accent: [f32; 4],
    /// Second line under the label (tiles intro page).
    pub subtitle: Option<&'static str>,
    /// Draw a bordered panel behind this example cell (sequence comparison).
    pub framed: bool,
}

fn tile_group(label: &'static str, tiles: Vec<Tile>, accent: [f32; 4]) -> TileGroup {
    TileGroup {
        label,
        tiles,
        accent,
        subtitle: None,
        framed: false,
    }
}

fn tile_group_with_subtitle(
    label: &'static str,
    subtitle: &'static str,
    tiles: Vec<Tile>,
    accent: [f32; 4],
) -> TileGroup {
    TileGroup {
        label,
        tiles,
        accent,
        subtitle: Some(subtitle),
        framed: false,
    }
}

fn tile_group_framed(
    label: &'static str,
    subtitle: &'static str,
    tiles: Vec<Tile>,
    accent: [f32; 4],
) -> TileGroup {
    TileGroup {
        label,
        tiles,
        accent,
        subtitle: Some(subtitle),
        framed: true,
    }
}

/// Meld label positioned below a tile group in screen space.
struct MeldLabel {
    x: f32,
    y: f32,
    w: f32,
    underline_y: f32,
    text: String,
    color: [f32; 4],
}

const GUIDE_TILE_ROTATION: [f32; 3] = [0.0, 0.0, std::f32::consts::PI];
/// Per-tile yaw wobble for invalid guide examples (melds invalid-sequence pattern).
const GUIDE_INVALID_TILE_WOBBLE: [f32; 3] = [0.14, -0.11, 0.09];

fn guide_example_is_invalid(group: &TileGroup) -> bool {
    group.label.starts_with("Invalid")
}

fn guide_invalid_tile_rotation(tile_i: usize) -> [f32; 3] {
    let wobble = GUIDE_INVALID_TILE_WOBBLE
        [tile_i % GUIDE_INVALID_TILE_WOBBLE.len()];
    [
        GUIDE_TILE_ROTATION[0],
        GUIDE_TILE_ROTATION[1],
        GUIDE_TILE_ROTATION[2] + wobble,
    ]
}

/// Convenience tile constructor.
fn t(suit: Suit, rank: u8, id: u32) -> Tile {
    Tile::new(suit, rank, id)
}

fn suit_ranks(suit: Suit, id_base: u32) -> Vec<Tile> {
    (1..=9u8)
        .map(|rank| t(suit, rank, id_base + rank as u32 - 1))
        .collect()
}

/// Optional in-universe margin scrawl for a page. Rendered below the tile
/// area in faded italic to feel like a player's aside left on the guide.
fn page_graffiti(page: usize) -> Option<&'static str> {
    match page {
        PAGE_MELDS => Some(
            "\"What use is a single tile? The House has rejected everything I've tried.\"  \u{2014} Mastromonaco",
        ),
        PAGE_FLOWERS => Some(
            "\"A flower may mend a triplet or a sequence, yet never weds a stranger as a pair. why?\"  \u{2014} Nicole",
        ),
        _ => None,
    }
}

/// Returns `(title, tile groups)` for the given page index.
fn page_content(page: usize, progress: &PlayerProgress) -> (&'static str, Vec<TileGroup>) {
    match page {
        PAGE_TILES => (
            tiles_intro_copy::PAGE_TITLE,
            vec![
                tile_group_with_subtitle(
                    "Manzu",
                    "ranks 1–9",
                    suit_ranks(Suit::Manzu, 0),
                    Suit::Manzu.keyword_color(),
                ),
                tile_group_with_subtitle(
                    "Souzu",
                    "ranks 1–9",
                    suit_ranks(Suit::Souzu, 9),
                    Suit::Souzu.keyword_color(),
                ),
                tile_group_with_subtitle(
                    "Pinzu",
                    "ranks 1–9",
                    suit_ranks(Suit::Pinzu, 18),
                    Suit::Pinzu.keyword_color(),
                ),
                tile_group(
                    "Winds",
                    vec![
                        t(Suit::Wind, 1, 27),
                        t(Suit::Wind, 2, 28),
                        t(Suit::Wind, 3, 29),
                        t(Suit::Wind, 4, 30),
                    ],
                    Suit::Wind.keyword_color(),
                ),
                tile_group(
                    "Dragons",
                    vec![
                        t(Suit::Dragon, 1, 31),
                        t(Suit::Dragon, 2, 32),
                        t(Suit::Dragon, 3, 33),
                    ],
                    Suit::Dragon.keyword_color(),
                ),
                tile_group(
                    "Flowers",
                    vec![
                        t(Suit::Flower, 1, 34),
                        t(Suit::Flower, 2, 35),
                        t(Suit::Flower, 3, 36),
                        t(Suit::Flower, 4, 37),
                    ],
                    Suit::Flower.keyword_color(),
                ),
            ],
        ),
        PAGE_MELDS => (
            melds_intro_copy::PAGE_TITLE,
            vec![
                tile_group_with_subtitle(
                    "Pair",
                    "Two identical tiles",
                    vec![t(Suit::Souzu, 5, 0), t(Suit::Souzu, 5, 1)],
                    color::CHAMPAGNE,
                ),
                tile_group_with_subtitle(
                    "Sequence",
                    "One suit · numbers only",
                    vec![
                        t(Suit::Manzu, 4, 2),
                        t(Suit::Manzu, 5, 3),
                        t(Suit::Manzu, 6, 4),
                    ],
                    [0.35, 0.70, 0.85, 0.9],
                ),
                tile_group_with_subtitle(
                    "Triplet",
                    "Three of a kind",
                    vec![
                        t(Suit::Pinzu, 7, 5),
                        t(Suit::Pinzu, 7, 6),
                        t(Suit::Pinzu, 7, 7),
                    ],
                    color::GOLD,
                ),
                tile_group_with_subtitle(
                    "Kong",
                    "Four of a kind",
                    vec![
                        t(Suit::Wind, 1, 8),
                        t(Suit::Wind, 1, 9),
                        t(Suit::Wind, 1, 10),
                        t(Suit::Wind, 1, 11),
                    ],
                    [0.85, 0.65, 0.20, 0.9],
                ),
                tile_group_with_subtitle(
                    "Single",
                    "One tile",
                    vec![t(Suit::Manzu, 5, 12)],
                    color::STONE,
                ),
                tile_group_framed(
                    "Valid sequence",
                    "3-4-5 Manzu",
                    vec![
                        t(Suit::Manzu, 3, 16),
                        t(Suit::Manzu, 4, 17),
                        t(Suit::Manzu, 5, 18),
                    ],
                    [0.35, 0.70, 0.85, 0.9],
                ),
                tile_group_framed(
                    "Invalid sequence",
                    "3 Manzu / 4 Souzu / 5 Pinzu",
                    vec![
                        t(Suit::Manzu, 3, 19),
                        t(Suit::Souzu, 4, 20),
                        t(Suit::Pinzu, 5, 21),
                    ],
                    color::STONE,
                ),
            ],
        ),
        PAGE_YAKU => (
            yaku_intro_copy::PAGE_TITLE,
            vec![
                tile_group_with_subtitle(
                    "4 melds + 1 pair",
                    "a high-scoring structure",
                    vec![
                        t(Suit::Manzu, 2, 20),
                        t(Suit::Manzu, 3, 21),
                        t(Suit::Manzu, 4, 22),
                        t(Suit::Souzu, 5, 23),
                        t(Suit::Souzu, 6, 24),
                        t(Suit::Souzu, 7, 25),
                        t(Suit::Pinzu, 8, 26),
                        t(Suit::Pinzu, 8, 27),
                        t(Suit::Pinzu, 8, 28),
                        t(Suit::Pinzu, 3, 29),
                        t(Suit::Pinzu, 3, 30),
                        t(Suit::Pinzu, 3, 31),
                        t(Suit::Wind, 2, 32),
                        t(Suit::Wind, 2, 33),
                    ],
                    [0.35, 0.70, 0.85, 0.9],
                ),
                tile_group_with_subtitle(
                    "With a kong",
                    "Kongs fill one meld slot",
                    vec![
                        t(Suit::Manzu, 1, 40),
                        t(Suit::Manzu, 1, 41),
                        t(Suit::Manzu, 1, 42),
                        t(Suit::Manzu, 1, 43),
                        t(Suit::Souzu, 4, 44),
                        t(Suit::Souzu, 5, 45),
                        t(Suit::Souzu, 6, 46),
                        t(Suit::Pinzu, 7, 47),
                        t(Suit::Pinzu, 8, 48),
                        t(Suit::Pinzu, 9, 49),
                        t(Suit::Dragon, 1, 50),
                        t(Suit::Dragon, 1, 51),
                        t(Suit::Dragon, 1, 52),
                        t(Suit::Wind, 2, 53),
                        t(Suit::Wind, 2, 54),
                    ],
                    [0.85, 0.65, 0.20, 0.9],
                ),
                tile_group_with_subtitle(
                    "Tanyao · Chinitsu",
                    "one suit, ranks 2–8 only",
                    vec![
                        t(Suit::Souzu, 2, 60),
                        t(Suit::Souzu, 3, 61),
                        t(Suit::Souzu, 4, 62),
                        t(Suit::Souzu, 4, 63),
                        t(Suit::Souzu, 5, 64),
                        t(Suit::Souzu, 6, 65),
                        t(Suit::Souzu, 6, 66),
                        t(Suit::Souzu, 7, 67),
                        t(Suit::Souzu, 8, 68),
                        t(Suit::Souzu, 5, 69),
                        t(Suit::Souzu, 5, 70),
                        t(Suit::Souzu, 5, 71),
                    ],
                    [0.35, 0.75, 0.45, 0.9],
                ),
            ],
        ),
        PAGE_FLOWERS => {
            let flower_accent: [f32; 4] = [0.85, 0.55, 0.70, 0.9];
            (
                flowers_intro_copy::PAGE_TITLE,
                vec![
                    tile_group_with_subtitle(
                        "Sequence",
                        "4 · Flower · 6 Manzu",
                        vec![
                            t(Suit::Manzu, 4, 3),
                            t(Suit::Flower, 3, 4),
                            t(Suit::Manzu, 6, 5),
                        ],
                        flower_accent,
                    ),
                    tile_group_with_subtitle(
                        "Triplet",
                        "Three Flowers",
                        vec![
                            t(Suit::Flower, 1, 8),
                            t(Suit::Flower, 3, 9),
                            t(Suit::Flower, 4, 10),
                        ],
                        flower_accent,
                    ),
                    tile_group_framed(
                        "Valid pair",
                        "Two Flowers",
                        vec![t(Suit::Flower, 1, 16), t(Suit::Flower, 2, 17)],
                        flower_accent,
                    ),
                    tile_group_framed(
                        "Invalid pair",
                        "7 Pinzu / Flower",
                        vec![t(Suit::Pinzu, 7, 11), t(Suit::Flower, 1, 12)],
                        color::STONE,
                    ),
                    tile_group_framed(
                        "Valid triplet",
                        "7 · 7 · Flower",
                        vec![
                            t(Suit::Pinzu, 7, 18),
                            t(Suit::Pinzu, 7, 19),
                            t(Suit::Flower, 2, 20),
                        ],
                        flower_accent,
                    ),
                    tile_group_framed(
                        "Invalid triplet",
                        "4 Manzu / Flower / Flower",
                        vec![
                            t(Suit::Manzu, 4, 13),
                            t(Suit::Flower, 2, 14),
                            t(Suit::Flower, 3, 15),
                        ],
                        color::STONE,
                    ),
                ],
            )
        }
        PAGE_ECONOMY => (economy_intro_copy::PAGE_TITLE, vec![]),
        PAGE_SCORING => (
            scoring_intro_copy::PAGE_TITLE,
            vec![
                tile_group_with_subtitle(
                    "5-6-7 Pinzu",
                    "Selected meld",
                    vec![
                        t(Suit::Pinzu, 5, 0),
                        t(Suit::Pinzu, 6, 1),
                        t(Suit::Pinzu, 7, 2),
                    ],
                    Suit::Pinzu.keyword_color(),
                ),
                tile_group_with_subtitle(
                    "5 Pinzu",
                    "+5 chips",
                    vec![t(Suit::Pinzu, 5, 9)],
                    Suit::Pinzu.keyword_color(),
                ),
                tile_group_with_subtitle(
                    "Red Dragon",
                    "+12 chips",
                    vec![t(Suit::Dragon, 1, 10)],
                    Suit::Dragon.keyword_color(),
                ),
                tile_group_with_subtitle(
                    "Flower",
                    "+0 chips",
                    vec![t(Suit::Flower, 1, 11)],
                    Suit::Flower.keyword_color(),
                ),
                {
                    let mut tile = t(Suit::Pinzu, 1, 12);
                    tile.debuffed_visual = true;
                    tile_group_with_subtitle(
                        "Debuffed tile",
                        "+0 chips",
                        vec![tile],
                        color::STONE,
                    )
                },
            ],
        ),
        PAGE_TANUKI_TIPS => (tanuki_tips_intro_copy::PAGE_TITLE, vec![]),
        _ => {
            let title = match yaku_chunk_for_page(page, progress) {
                Some(chunk) if chunk.len() == 1 => chunk[0].name(),
                Some(_) => "Yaku Reference",
                None => "",
            };
            (title, vec![])
        }
    }
}

// ── Guide layout frame ────────────────────────────────────────────────────

/// Margins and reserved header / nav bands for every guide page.
struct GuideLayout {
    window_w: f32,
    window_h: f32,
    margin: f32,
    content_x: f32,
    content_w: f32,
    content_bottom: f32,
    header_btn_h: f32,
}

struct GuideHeaderChrome {
    back: [f32; 4],
    prev: [f32; 4],
    next: [f32; 4],
    page_counter: [f32; 4],
}

struct GuideNavHeader {
    copy_x: f32,
    title_y: f32,
    subtitle_y: f32,
    content_top: f32,
    title_font: f32,
    body_font: f32,
}

fn guide_copy_inset_x(w: f32) -> f32 {
    w * 0.055
}

fn page_nav_subtitle(page: usize) -> Option<&'static str> {
    match page {
        PAGE_TILES => Some(tiles_intro_copy::INTRO),
        PAGE_MELDS => Some(melds_intro_copy::PAGE_SUBTITLE),
        PAGE_YAKU => Some(yaku_intro_copy::PAGE_SUBTITLE),
        PAGE_FLOWERS => Some(flowers_intro_copy::PAGE_SUBTITLE),
        PAGE_SCORING => Some(scoring_intro_copy::SUBTITLE),
        PAGE_ECONOMY => Some(economy_intro_copy::SUBTITLE),
        _ => None,
    }
}

fn guide_nav_header(w: f32, h: f32, back: [f32; 4], subtitle: Option<&str>) -> GuideNavHeader {
    let scale = metrics::scene_scale(w, h);
    let title_font = typography::size(typography::H20, h);
    let body_font = typography::size(typography::H42, h);
    let jr = (w.min(h) / 720.0).clamp(1.0, 1.38);
    let title = HeaderTitleLayout::nav_row_aligned(
        back,
        guide_copy_inset_x(w),
        (18.0 * scale).max(10.0),
        title_font,
        jr,
    );
    let subtitle_w = w * 0.72;
    let divider_y = if let Some(sub) = subtitle {
        let subtitle_line_h = styled_text::ColoredLineBlock::measure(
            sub,
            subtitle_w,
            body_font,
            color::PARCHMENT,
            GlossaryMode::Prose,
        )
        .height();
        title.subtitle_y + subtitle_line_h
    } else {
        HeaderChromeMetrics::from_window(w, h).chrome_bottom()
    };
    GuideNavHeader {
        copy_x: title.copy_x,
        title_y: title.title_y,
        subtitle_y: title.subtitle_y,
        content_top: divider_y,
        title_font,
        body_font,
    }
}

impl GuideLayout {
    fn new(w: f32, h: f32) -> Self {
        let chrome = HeaderChromeMetrics::from_window(w, h);
        let content_bottom = h - screen_footer_reserve(w, h) - 12.0 * chrome.ui;
        Self {
            window_w: w,
            window_h: h,
            margin: chrome.margin,
            content_x: chrome.margin,
            content_w: w - chrome.margin * 2.0,
            content_bottom,
            header_btn_h: chrome.button_h,
        }
    }

    fn header_chrome(&self) -> GuideHeaderChrome {
        let chrome_metrics = HeaderChromeMetrics::from_window(self.window_w, self.window_h);
        let btn_h = self.header_btn_h;
        let btn_gap = 10.0 * (chrome_metrics.margin / 48.0);
        let row_y = self.margin;

        let back = chrome_metrics.back_rect_left();

        let arrow_w = btn_h * 1.12;
        let right_edge = self.window_w - chrome_metrics.margin;
        let next = [right_edge - arrow_w, row_y, arrow_w, btn_h];

        let counter_w = (112.0 * (chrome_metrics.margin / 48.0)).clamp(96.0, 140.0);
        let counter_x = next[0] - btn_gap - counter_w;
        let page_counter = [counter_x, row_y, counter_w, btn_h];

        let prev = [counter_x - btn_gap - arrow_w, row_y, arrow_w, btn_h];

        GuideHeaderChrome {
            back,
            prev,
            next,
            page_counter,
        }
    }
}

fn push_guide_panel_stroke(frame: &mut UiFrame, rect: [f32; 4], color: [f32; 4]) {
    let [x, y, w, h] = rect;
    let t = 1.0;
    frame.quad(GpuInstance {
        rect: [x, y, w, t],
        color,
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: [x, y + h - t, w, t],
        color,
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: [x, y, t, h],
        color,
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: [x + w - t, y, t, h],
        color,
        user: 0,
    });
}

/// Header rule below the nav title band. Returns top of the content band.
fn push_guide_chrome(frame: &mut UiFrame, layout: &GuideLayout, divider_y: f32) -> f32 {
    let jr = (layout.window_w.min(layout.window_h) / 720.0).clamp(1.0, 1.38);
    frame.quad(GpuInstance {
        rect: [layout.content_x, divider_y, layout.content_w, 1.0],
        color: color::alpha(color::STONE, 0.45),
        user: 0,
    });

    divider_y + 1.0 + (18.0 * jr).max(14.0)
}

fn push_guide_header_nav(
    frame: &mut UiFrame,
    layout: &GuideLayout,
    tree: &TreeState,
    page: usize,
    pages: usize,
    scale: f32,
    w: f32,
    h: f32,
    page_title: &str,
    nav_header: &GuideNavHeader,
    subtitle: Option<&str>,
) {
    let prev_enabled = page > 0;
    let next_enabled = page + 1 < pages;
    let chrome = layout.header_chrome();
    let items = [
        FlatItem::new(GuideNav::Back.id(), chrome.back, GuideNav::Back),
        FlatItem::new(GuideNav::Prev.id(), chrome.prev, GuideNav::Prev),
        FlatItem::new(GuideNav::Next.id(), chrome.next, GuideNav::Next),
    ];
    let mut nav_quads = Vec::new();
    let mut nav_texts = Vec::new();
    let mut junk_buttons = Vec::new();
    for item in &items {
        let focused = tree.focused() == Some(item.id);
        let (label, variant, state) = match item.action {
            GuideNav::Prev => {
                let state = if !prev_enabled {
                    ButtonState::Disabled
                } else if focused {
                    ButtonState::Hover
                } else {
                    ButtonState::Rest
                };
                ("◀️", ButtonVariant::Default, state)
            }
            GuideNav::Back => (
                "Back",
                ButtonVariant::Default,
                if focused {
                    ButtonState::Hover
                } else {
                    ButtonState::Rest
                },
            ),
            GuideNav::Next => {
                let state = if !next_enabled {
                    ButtonState::Disabled
                } else if focused {
                    ButtonState::Hover
                } else {
                    ButtonState::Rest
                };
                ("▶️", ButtonVariant::Default, state)
            }
        };
        widget::push_button(
            &mut nav_quads,
            &mut nav_texts,
            &mut junk_buttons,
            widget::ButtonSpec {
                rect: item.rect,
                label,
                variant,
                state,
                action: UiAction::Confirm,
            },
        );
        if focused {
            focus_nav::push_focus_ring(item.rect, scale, w, h, &mut nav_quads);
        }
    }
    frame.quads(nav_quads);
    for label in nav_texts {
        frame.text(label);
    }
    let counter_font = typography::size(typography::H28, h);
    let counter_line_h = styled_text::colored_row_line_step(counter_font);
    let btn_y = chrome.prev[1];
    let btn_h = chrome.prev[3];
    let counter_rect = [
        chrome.page_counter[0],
        btn_y + (btn_h - counter_line_h) * 0.5,
        chrome.page_counter[2],
        counter_line_h,
    ];
    frame.text(TextLabel {
        rect: counter_rect,
        text: format!("{} / {}", page + 1, pages),
        color: color::UMBER,
        align: TextAlign::Center,
        font_px: Some(counter_font),
        bold: true,
        ..Default::default()
    });
    junk_buttons.clear();
    tree.register_flat_buttons(&items, &mut frame.buttons);

    frame.text(TextLabel {
        rect: [
            nav_header.copy_x,
            nav_header.title_y,
            w * 0.55,
            nav_header.title_font * 1.15,
        ],
        text: page_title.into(),
        color: color::CHAMPAGNE,
        align: TextAlign::Left,
        font_px: Some(nav_header.title_font),
        bold: true,
        ..Default::default()
    });
    if let Some(sub) = subtitle {
        let mut subtitle_labels = Vec::new();
        styled_text::push_colored_line_left(
            &mut subtitle_labels,
            nav_header.copy_x,
            nav_header.subtitle_y,
            w * 0.72,
            nav_header.body_font,
            sub,
            color::PARCHMENT,
            GlossaryMode::Prose,
        );
        frame.texts(subtitle_labels);
    }
}

/// Long-form copy for a yaku page (Guide only; journal uses [`yaku_page`] rule text).
pub(crate) struct YakuGuideDetail {
    pub rule: &'static str,
    pub requires: &'static str,
    pub breaks_if: &'static str,
}

pub(crate) fn yaku_guide_detail(yk: YakuKind, kokushi_discovered: bool) -> YakuGuideDetail {
    match yk {
        YakuKind::ChickenHand => YakuGuideDetail {
            rule: "A valid hand that triggers no other yaku.",
            requires: "Valid hand · no other yaku",
            breaks_if: "Any other yaku is triggered.",
        },
        YakuKind::Tanyao => YakuGuideDetail {
            rule: "All tiles are simples. A simple is a number tile ranked 2–8.",
            requires: "Simples only",
            breaks_if: "The hand contains a terminal, wind, or dragon.",
        },
        YakuKind::Yakuhai => YakuGuideDetail {
            rule: "A triplet or kong of any dragon, or of the current round wind.",
            requires: "Dragon triplet/kong OR round-wind triplet/kong",
            breaks_if: "The honor group is only a pair, or the wind is not the current round wind.",
        },
        YakuKind::Toitoi => YakuGuideDetail {
            rule: "All melds are triplets or kongs. No sequences allowed.",
            requires: "Triplets/kongs only",
            breaks_if: "The hand contains any sequence.",
        },
        YakuKind::Honitsu => YakuGuideDetail {
            rule: "One number suit plus honors only. No other number suits.",
            requires: "One number suit · honors allowed",
            breaks_if: "A second number suit appears.",
        },
        YakuKind::Iipeikou => YakuGuideDetail {
            rule: "Two identical sequences in the same suit on a full 14-tile hand.",
            requires: "Full hand · two identical sequences · same suit",
            breaks_if: "The matching sequences use different suits, or the hand is not a full hand.",
        },
        YakuKind::Junchan => YakuGuideDetail {
            rule: "All tiles are terminals or honors. Every meld contains a terminal or honor.",
            requires: "Terminal-touching melds · no middle-only melds",
            breaks_if: "A meld has no terminal or honor.",
        },
        YakuKind::Honroutou => YakuGuideDetail {
            rule: "Every tile is a terminal or honor.",
            requires: "Terminals · honors · no simples",
            breaks_if: "Any rank 2–8 tile appears.",
        },
        YakuKind::FullHand => YakuGuideDetail {
            rule: "A complete 14-tile hand made from 4 melds and 1 pair. Seven pairs does not count.",
            requires: "4 melds · 1 pair",
            breaks_if: if kokushi_discovered {
                "The hand is seven pairs, thirteen-orphans style, incomplete, or built from the wrong group sizes."
            } else {
                "The hand is seven pairs, incomplete, or built from the wrong group sizes."
            },
        },
        YakuKind::Chinitsu => YakuGuideDetail {
            rule: "All tiles come from a single number suit. No honors.",
            requires: "One number suit only · no honors",
            breaks_if: "Any honor appears, or any second number suit appears.",
        },
        YakuKind::SanshokuDoujun => YakuGuideDetail {
            rule: "The same numerical sequence appears in all three number suits.",
            requires: "One matching sequence in Manzu · Souzu · Pinzu",
            breaks_if: "The sequences use different ranks, or one number suit is missing.",
        },
        YakuKind::Ittsu => YakuGuideDetail {
            rule: "A 1–9 straight in one suit: three sequences covering 1-2-3, 4-5-6, and 7-8-9.",
            requires: "Same suit · 1-2-3 · 4-5-6 · 7-8-9",
            breaks_if: "The sequences are split across suits, or one of the three sequence ranges is missing.",
        },
        YakuKind::Chiitoitsu => YakuGuideDetail {
            rule: "Seven distinct pairs. This is an alternate hand shape and does not use melds.",
            requires: "7 distinct pairs",
            breaks_if: "A pair type is repeated, or the hand is scored as normal melds.",
        },
        YakuKind::KokushiMusou => YakuGuideDetail {
            rule: "One of each terminal and honor, plus one duplicate. This creates twelve singles and one pair.",
            requires: "All terminals · all honors · one duplicate",
            breaks_if: "Any required terminal or honor is missing, or the duplicate is not one of those tile types.",
        },
        YakuKind::Chanta => YakuGuideDetail {
            rule: "Every meld contains a terminal (1 or 9) or an honor. The pair may be a simple 2–8 tile.",
            requires: "Each meld touches a terminal or honor",
            breaks_if: "Any meld is made only of ranks 2–8 with no terminal or honor.",
        },
        YakuKind::Ryanpeikou => YakuGuideDetail {
            rule: "Two different sequences, each duplicated, in the same number suit on a full 14-tile hand.",
            requires: "Full hand · two pairs of identical sequences · one suit",
            breaks_if: "Only one duplicated sequence, or duplicates are split across suits.",
        },
        YakuKind::SanshokuDoukou => YakuGuideDetail {
            rule: "The same rank triplet or kong appears in Manzu, Souzu, and Pinzu.",
            requires: "Matching rank · triplet/kong in all three number suits",
            breaks_if: "A number suit is missing that rank, or the groups are sequences instead of triplets.",
        },
        YakuKind::Pinfu => YakuGuideDetail {
            rule: "A full hand of four sequences and a pair ranked 2–8 in a number suit (no honor pair).",
            requires: "Full hand · four sequences · simple number pair",
            breaks_if: "Any triplet/kong appears, or the pair is a wind or dragon.",
        },
    }
}

fn draw_yaku_guide_page(
    frame: &mut UiFrame,
    progress: &PlayerProgress,
    w: f32,
    h: f32,
    scale: f32,
    yaku: &[YakuKind],
    body_top: f32,
    content_floor: f32,
    cam: &CameraParams,
) {
    let kokushi_discovered = progress.kokushi_musou_discovered();
    if yaku.is_empty() {
        return;
    }
    let n = yaku.len();
    let band_gap = h * 0.014;
    let band_h = (content_floor - body_top - band_gap * (n.saturating_sub(1)) as f32) / n as f32;

    for (i, &yk) in yaku.iter().enumerate() {
        let band_top = body_top + i as f32 * (band_h + band_gap);
        let band_bottom = band_top + band_h;
        let detail = yaku_guide_detail(yk, kokushi_discovered);
        let (_, groups) = yaku_page(yk);
        draw_yaku_entry(
            frame,
            w,
            h,
            scale,
            yk,
            &detail,
            &groups,
            band_top,
            band_bottom,
            cam,
            n > 1,
        );
        if i + 1 < n {
            let div_y = band_bottom + band_gap * 0.35;
            frame.quad(GpuInstance {
                rect: [w * 0.06, div_y, w * 0.88, (1.5 * scale).max(1.0)],
                color: color::alpha(color::STONE, 0.35),
                user: 0,
            });
        }
    }
}

fn draw_yaku_entry(
    frame: &mut UiFrame,
    w: f32,
    h: f32,
    scale: f32,
    yk: YakuKind,
    detail: &YakuGuideDetail,
    groups: &[TileGroup],
    band_top: f32,
    band_bottom: f32,
    cam: &CameraParams,
    compact: bool,
) {
    let name_font = typography::size(
        if compact {
            typography::H24
        } else {
            typography::H20
        },
        h,
    );
    let stats_font = typography::size(typography::H42, h);
    let body_font = typography::size(typography::H36, h);
    let label_font = typography::size(typography::H28, h);
    let pad = w * 0.05;
    let inner_w = w - pad * 2.0;

    let name_h = name_font * (if compact { 1.12 } else { 1.22 });
    frame.text(TextLabel {
        rect: [pad, band_top, inner_w, name_h],
        text: yk.name().into(),
        color: color::CHAMPAGNE,
        align: TextAlign::Left,
        font_px: Some(name_font),
        bold: true,
        ..Default::default()
    });

    let stats = format!("+{} mult · +{} chips", yk.mult_bonus(), yk.chip_bonus());
    let stats_y = band_top + name_h + h * 0.002;
    let stats_h = push_dense_text_lines(
        frame,
        [pad, stats_y, inner_w, 0.0],
        &stats,
        stats_font,
        color::alpha(color::CHAMPAGNE, 0.82),
        1.18,
    );

    let rule_y = stats_y + stats_h + h * 0.004;
    let rule_h = push_dense_text(
        frame,
        [pad, rule_y, inner_w, 0.0],
        detail.rule,
        body_font,
        color::PARCHMENT,
    );

    let cols_top = rule_y + rule_h + h * (if compact { 0.006 } else { 0.012 });
    let col_gap = w * 0.02;
    let text_col_w = inner_w * 0.32;
    let tile_col_w = inner_w - text_col_w - col_gap;
    let breaks_reserve = body_font * (if compact { 1.35 } else { 1.55 });
    let col_h = (band_bottom - cols_top - breaks_reserve - h * 0.008).max(h * 0.12);

    let requires_label = format!("Requires: {}", detail.requires);
    push_dense_text(
        frame,
        [pad, cols_top, text_col_w, col_h],
        &requires_label,
        label_font,
        color::STONE,
    );

    let tile_x = pad + text_col_w + col_gap;
    let tile_center_y = cols_top + col_h * 0.48;
    let group_refs: Vec<&TileGroup> = groups.iter().collect();
    let max_tile = if compact { h * 0.17 } else { h * 0.24 };
    let (placements, labels) = layout_tile_groups_with_max(
        cam,
        &group_refs,
        w,
        h,
        tile_center_y,
        Some([tile_x, tile_x + tile_col_w]),
        max_tile,
        0.98,
    );
    if !placements.is_empty() {
        frame.cmds.push(DrawCmd::ShowcaseTileBatch(placements.into()));
    }
    push_tile_group_labels(frame, &labels, h, scale, false);

    let breaks_y = band_bottom - breaks_reserve;
    let breaks_text = format!("Breaks if: {}", detail.breaks_if);
    push_dense_text(
        frame,
        [pad, breaks_y, inner_w, 0.0],
        &breaks_text,
        body_font,
        color::STONE,
    );
}

fn push_dense_text(
    frame: &mut UiFrame,
    rect: [f32; 4],
    text: &str,
    font_px: f32,
    color: [f32; 4],
) -> f32 {
    push_dense_text_lines(
        frame,
        rect,
        text,
        font_px,
        color,
        widget::PLAIN_TEXT_LINE_STEP_MUL,
    )
}

fn push_dense_text_lines(
    frame: &mut UiFrame,
    rect: [f32; 4],
    text: &str,
    font_px: f32,
    color: [f32; 4],
    line_mul: f32,
) -> f32 {
    push_dense_text_lines_aligned(frame, rect, text, font_px, color, line_mul, TextAlign::Left)
}

fn push_dense_text_lines_aligned(
    frame: &mut UiFrame,
    rect: [f32; 4],
    text: &str,
    font_px: f32,
    color: [f32; 4],
    line_mul: f32,
    align: TextAlign,
) -> f32 {
    let line_h = font_px * line_mul;
    let wrapped = styled_text::wrap_colored_text_multiline(
        text,
        rect[2],
        font_px / 0.99,
        color,
        false,
        GlossaryMode::Prose,
    );
    let block_h = line_h * wrapped.len().max(1) as f32;
    let Some(font) = load_ui_font() else {
        let wrapped = wrap_text(text, rect[2], font_px / 0.99);
        frame.text(TextLabel {
            rect: [rect[0], rect[1], rect[2], block_h],
            text: wrapped.join("\n"),
            color,
            align,
            font_px: Some(font_px),
            ..Default::default()
        });
        return block_h;
    };

    for (row, chunks) in wrapped.iter().enumerate() {
        let line_y = rect[1] + row as f32 * line_h;
        let measured: f32 = chunks
            .iter()
            .map(|(s, _)| {
                s.chars()
                    .map(|ch| font.metrics(ch, font_px).advance_width)
                    .sum::<f32>()
            })
            .sum();
        let mut cx = match align {
            TextAlign::Left => rect[0],
            TextAlign::Center => rect[0] + (rect[2] - measured) * 0.5,
            TextAlign::Right => rect[0] + rect[2] - measured,
        };
        for (s, c) in chunks {
            let piece_w = s
                .chars()
                .map(|ch| font.metrics(ch, font_px).advance_width)
                .sum::<f32>()
                .max(1.0);
            let mut chunk_labels = Vec::new();
            push_keyword_label(
                &mut chunk_labels,
                TextLabel {
                    rect: [cx, line_y, piece_w, line_h],
                    text: s.clone(),
                    color: *c,
                    font_px: Some(font_px),
                    align: TextAlign::Left,
                    text_effect: text_effect_for_glossary_tint(*c),
                    ..Default::default()
                },
                color,
                true,
            );
            for lbl in chunk_labels {
                frame.text(lbl);
            }
            cx += piece_w;
        }
    }
    block_h
}

/// Build tile groups for a yaku example hand. The rule string matches [`yaku_guide_detail`]
/// for journal / reference UIs.
pub(crate) fn yaku_page(yk: YakuKind) -> (&'static str, Vec<TileGroup>) {
    let detail = yaku_guide_detail(yk, true);
    let seq_color: [f32; 4] = [0.35, 0.70, 0.85, 0.9];
    let trip_color: [f32; 4] = color::GOLD;
    let pair_color: [f32; 4] = color::CHAMPAGNE;
    let single_color: [f32; 4] = [0.78, 0.74, 0.58, 0.9];
    let _kong_color: [f32; 4] = [0.85, 0.65, 0.20, 0.9];

    let groups = match yk {
        YakuKind::Tanyao => meld_groups(&[
            (
                "Sequence",
                MeldKind::Sequence,
                Suit::Manzu,
                &[2, 3, 4],
                seq_color,
            ),
            (
                "Sequence",
                MeldKind::Sequence,
                Suit::Souzu,
                &[5, 6, 7],
                seq_color,
            ),
            (
                "Triplet",
                MeldKind::Triplet,
                Suit::Pinzu,
                &[8, 8, 8],
                trip_color,
            ),
            ("Pair", MeldKind::Pair, Suit::Manzu, &[5, 5], pair_color),
        ]),
        YakuKind::Toitoi => meld_groups(&[
            (
                "Triplet",
                MeldKind::Triplet,
                Suit::Manzu,
                &[1, 1, 1],
                trip_color,
            ),
            (
                "Triplet",
                MeldKind::Triplet,
                Suit::Souzu,
                &[5, 5, 5],
                trip_color,
            ),
            (
                "Triplet",
                MeldKind::Triplet,
                Suit::Pinzu,
                &[9, 9, 9],
                trip_color,
            ),
            ("Pair", MeldKind::Pair, Suit::Wind, &[1, 1], pair_color),
        ]),
        YakuKind::Honroutou => meld_groups(&[
            (
                "Triplet",
                MeldKind::Triplet,
                Suit::Manzu,
                &[1, 1, 1],
                trip_color,
            ),
            (
                "Triplet",
                MeldKind::Triplet,
                Suit::Souzu,
                &[9, 9, 9],
                trip_color,
            ),
            (
                "Triplet",
                MeldKind::Triplet,
                Suit::Wind,
                &[1, 1, 1],
                trip_color,
            ),
            ("Pair", MeldKind::Pair, Suit::Pinzu, &[1, 1], pair_color),
        ]),
        YakuKind::Iipeikou => meld_groups(&[
            (
                "Sequence",
                MeldKind::Sequence,
                Suit::Souzu,
                &[1, 2, 3],
                seq_color,
            ),
            (
                "Sequence",
                MeldKind::Sequence,
                Suit::Souzu,
                &[1, 2, 3],
                seq_color,
            ),
            (
                "Triplet",
                MeldKind::Triplet,
                Suit::Pinzu,
                &[4, 4, 4],
                trip_color,
            ),
            (
                "Triplet",
                MeldKind::Triplet,
                Suit::Manzu,
                &[5, 5, 5],
                trip_color,
            ),
            ("Pair", MeldKind::Pair, Suit::Wind, &[1, 1], pair_color),
        ]),
        YakuKind::FullHand => meld_groups(&[
            (
                "Sequence",
                MeldKind::Sequence,
                Suit::Manzu,
                &[1, 2, 3],
                seq_color,
            ),
            (
                "Sequence",
                MeldKind::Sequence,
                Suit::Souzu,
                &[4, 5, 6],
                seq_color,
            ),
            (
                "Triplet",
                MeldKind::Triplet,
                Suit::Pinzu,
                &[7, 7, 7],
                trip_color,
            ),
            (
                "Sequence",
                MeldKind::Sequence,
                Suit::Manzu,
                &[7, 8, 9],
                seq_color,
            ),
            ("Pair", MeldKind::Pair, Suit::Dragon, &[1, 1], pair_color),
        ]),
        YakuKind::Chinitsu => meld_groups(&[
            (
                "Sequence",
                MeldKind::Sequence,
                Suit::Souzu,
                &[1, 2, 3],
                seq_color,
            ),
            (
                "Sequence",
                MeldKind::Sequence,
                Suit::Souzu,
                &[4, 5, 6],
                seq_color,
            ),
            (
                "Triplet",
                MeldKind::Triplet,
                Suit::Souzu,
                &[7, 7, 7],
                trip_color,
            ),
            ("Pair", MeldKind::Pair, Suit::Souzu, &[9, 9], pair_color),
        ]),
        YakuKind::SanshokuDoujun => meld_groups(&[
            (
                "Sequence",
                MeldKind::Sequence,
                Suit::Manzu,
                &[4, 5, 6],
                seq_color,
            ),
            (
                "Sequence",
                MeldKind::Sequence,
                Suit::Souzu,
                &[4, 5, 6],
                seq_color,
            ),
            (
                "Sequence",
                MeldKind::Sequence,
                Suit::Pinzu,
                &[4, 5, 6],
                seq_color,
            ),
            ("Pair", MeldKind::Pair, Suit::Wind, &[1, 1], pair_color),
        ]),
        YakuKind::Junchan => meld_groups(&[
            (
                "Triplet",
                MeldKind::Triplet,
                Suit::Manzu,
                &[1, 1, 1],
                trip_color,
            ),
            (
                "Triplet",
                MeldKind::Triplet,
                Suit::Manzu,
                &[9, 9, 9],
                trip_color,
            ),
            (
                "Triplet",
                MeldKind::Triplet,
                Suit::Pinzu,
                &[1, 1, 1],
                trip_color,
            ),
            (
                "Triplet",
                MeldKind::Triplet,
                Suit::Dragon,
                &[2, 2, 2],
                trip_color,
            ),
            ("Pair", MeldKind::Pair, Suit::Souzu, &[9, 9], pair_color),
        ]),
        YakuKind::Ittsu => meld_groups(&[
            (
                "Sequence",
                MeldKind::Sequence,
                Suit::Souzu,
                &[1, 2, 3],
                seq_color,
            ),
            (
                "Sequence",
                MeldKind::Sequence,
                Suit::Souzu,
                &[4, 5, 6],
                seq_color,
            ),
            (
                "Sequence",
                MeldKind::Sequence,
                Suit::Souzu,
                &[7, 8, 9],
                seq_color,
            ),
            ("Pair", MeldKind::Pair, Suit::Manzu, &[5, 5], pair_color),
        ]),
        YakuKind::Honitsu => meld_groups(&[
            (
                "Sequence",
                MeldKind::Sequence,
                Suit::Souzu,
                &[2, 3, 4],
                seq_color,
            ),
            (
                "Sequence",
                MeldKind::Sequence,
                Suit::Souzu,
                &[6, 7, 8],
                seq_color,
            ),
            (
                "Triplet",
                MeldKind::Triplet,
                Suit::Wind,
                &[1, 1, 1],
                trip_color,
            ),
            ("Pair", MeldKind::Pair, Suit::Souzu, &[9, 9], pair_color),
        ]),
        YakuKind::Yakuhai => meld_groups(&[
            (
                "Triplet",
                MeldKind::Triplet,
                Suit::Dragon,
                &[1, 1, 1],
                trip_color,
            ),
            (
                "Sequence",
                MeldKind::Sequence,
                Suit::Manzu,
                &[2, 3, 4],
                seq_color,
            ),
            ("Pair", MeldKind::Pair, Suit::Souzu, &[5, 5], pair_color),
        ]),
        YakuKind::Chiitoitsu => meld_groups(&[
            ("Pair", MeldKind::Pair, Suit::Manzu, &[1, 1], pair_color),
            ("Pair", MeldKind::Pair, Suit::Manzu, &[3, 3], pair_color),
            ("Pair", MeldKind::Pair, Suit::Souzu, &[5, 5], pair_color),
            ("Pair", MeldKind::Pair, Suit::Souzu, &[7, 7], pair_color),
            ("Pair", MeldKind::Pair, Suit::Pinzu, &[2, 2], pair_color),
            ("Pair", MeldKind::Pair, Suit::Pinzu, &[4, 4], pair_color),
            ("Pair", MeldKind::Pair, Suit::Wind, &[1, 1], pair_color),
        ]),
        YakuKind::KokushiMusou => meld_groups(&[
            ("Pair", MeldKind::Pair, Suit::Manzu, &[1, 1], pair_color),
            ("Single", MeldKind::Single, Suit::Manzu, &[9], single_color),
            ("Single", MeldKind::Single, Suit::Souzu, &[1], single_color),
            ("Single", MeldKind::Single, Suit::Souzu, &[9], single_color),
            ("Single", MeldKind::Single, Suit::Pinzu, &[1], single_color),
            ("Single", MeldKind::Single, Suit::Pinzu, &[9], single_color),
            ("Single", MeldKind::Single, Suit::Wind, &[1], single_color),
            ("Single", MeldKind::Single, Suit::Wind, &[2], single_color),
            ("Single", MeldKind::Single, Suit::Wind, &[3], single_color),
            ("Single", MeldKind::Single, Suit::Wind, &[4], single_color),
            ("Single", MeldKind::Single, Suit::Dragon, &[1], single_color),
            ("Single", MeldKind::Single, Suit::Dragon, &[2], single_color),
            ("Single", MeldKind::Single, Suit::Dragon, &[3], single_color),
        ]),
        YakuKind::Chanta => meld_groups(&[
            (
                "Sequence",
                MeldKind::Sequence,
                Suit::Manzu,
                &[1, 2, 3],
                seq_color,
            ),
            (
                "Sequence",
                MeldKind::Sequence,
                Suit::Souzu,
                &[7, 8, 9],
                seq_color,
            ),
            (
                "Triplet",
                MeldKind::Triplet,
                Suit::Wind,
                &[1, 1, 1],
                trip_color,
            ),
            (
                "Triplet",
                MeldKind::Triplet,
                Suit::Pinzu,
                &[9, 9, 9],
                trip_color,
            ),
            ("Pair", MeldKind::Pair, Suit::Manzu, &[5, 5], pair_color),
        ]),
        YakuKind::Ryanpeikou => meld_groups(&[
            (
                "Sequence",
                MeldKind::Sequence,
                Suit::Manzu,
                &[2, 3, 4],
                seq_color,
            ),
            (
                "Sequence",
                MeldKind::Sequence,
                Suit::Manzu,
                &[2, 3, 4],
                seq_color,
            ),
            (
                "Sequence",
                MeldKind::Sequence,
                Suit::Manzu,
                &[5, 6, 7],
                seq_color,
            ),
            (
                "Sequence",
                MeldKind::Sequence,
                Suit::Manzu,
                &[5, 6, 7],
                seq_color,
            ),
            ("Pair", MeldKind::Pair, Suit::Souzu, &[8, 8], pair_color),
        ]),
        YakuKind::SanshokuDoukou => meld_groups(&[
            (
                "Triplet",
                MeldKind::Triplet,
                Suit::Manzu,
                &[4, 4, 4],
                trip_color,
            ),
            (
                "Triplet",
                MeldKind::Triplet,
                Suit::Souzu,
                &[4, 4, 4],
                trip_color,
            ),
            (
                "Triplet",
                MeldKind::Triplet,
                Suit::Pinzu,
                &[4, 4, 4],
                trip_color,
            ),
            (
                "Sequence",
                MeldKind::Sequence,
                Suit::Manzu,
                &[2, 3, 4],
                seq_color,
            ),
            ("Pair", MeldKind::Pair, Suit::Wind, &[1, 1], pair_color),
        ]),
        YakuKind::Pinfu => meld_groups(&[
            (
                "Sequence",
                MeldKind::Sequence,
                Suit::Manzu,
                &[2, 3, 4],
                seq_color,
            ),
            (
                "Sequence",
                MeldKind::Sequence,
                Suit::Manzu,
                &[5, 6, 7],
                seq_color,
            ),
            (
                "Sequence",
                MeldKind::Sequence,
                Suit::Souzu,
                &[3, 4, 5],
                seq_color,
            ),
            (
                "Sequence",
                MeldKind::Sequence,
                Suit::Pinzu,
                &[6, 7, 8],
                seq_color,
            ),
            ("Pair", MeldKind::Pair, Suit::Manzu, &[5, 5], pair_color),
        ]),
        YakuKind::ChickenHand => meld_groups(&[
            (
                "Sequence",
                MeldKind::Sequence,
                Suit::Manzu,
                &[1, 2, 3],
                seq_color,
            ),
            (
                "Sequence",
                MeldKind::Sequence,
                Suit::Souzu,
                &[4, 5, 6],
                seq_color,
            ),
            (
                "Triplet",
                MeldKind::Triplet,
                Suit::Pinzu,
                &[3, 3, 3],
                trip_color,
            ),
            ("Pair", MeldKind::Pair, Suit::Wind, &[2, 2], pair_color),
        ]),
    };
    (detail.rule, groups)
}

/// `(label, kind, suit, ranks, accent)` descriptor for a single meld row.
type MeldSpec = (&'static str, MeldKind, Suit, &'static [u8], [f32; 4]);

/// Build `TileGroup`s from a compact descriptor list. Assigns sequential tile
/// ids across all groups so the renderer treats each tile as unique.
fn meld_groups(specs: &[MeldSpec]) -> Vec<TileGroup> {
    let mut id_counter: u32 = 0;
    specs
        .iter()
        .map(|&(label, _kind, suit, ranks, accent)| {
            let tiles = ranks
                .iter()
                .map(|&r| {
                    let tile = t(suit, r, id_counter);
                    id_counter += 1;
                    tile
                })
                .collect();
            TileGroup {
                label,
                tiles,
                accent,
                subtitle: None,
                framed: false,
            }
        })
        .collect()
}

// ── Scoring page (page 4) ─────────────────────────────────────────────────

const SCORING_FLOW_MELD: usize = 0;
const SCORING_CHIP_GROUPS: &[usize] = &[1, 2, 3, 4];
const SCORING_STRUCTURE_SLOT_COUNT: usize = 6;
const SCORING_STRUCTURE_FILLED: usize = 3;

fn draw_scoring_page(
    frame: &mut UiFrame,
    ctx: &DrawCtx<'_>,
    layout: &GuideLayout,
    _progress: &PlayerProgress,
    w: f32,
    h: f32,
    _scale: f32,
    groups: &[TileGroup],
    content_top: f32,
    content_floor: f32,
) {
    let gap = 10.0;
    let pad = 10.0;
    let (flow_tile_max, values_tile_max) = scoring_guide_tile_caps(w, h);
    let body_font = typography::size(typography::H36, h);
    let section_font = typography::size(typography::H28, h);
    let small_font = typography::size(typography::H42, h);

    let x = layout.content_x;
    let full_w = layout.content_w;
    let mut y = content_top;
    let usable = (content_floor - y).max(1.0);
    let flow_h = usable * 0.50;
    let bottom_h = usable - flow_h - gap;

    let flow_outer = [x, y, full_w, flow_h];
    let cash_in_visual = scoring_flow_cash_in_visual_rect(
        flow_outer,
        section_font,
        body_font,
        small_font,
        pad,
        flow_tile_max,
    );
    let glb_cash_in = push_scoring_gameplay_cash_in_env(frame, ctx, w, h, cash_in_visual);

    let flow_content = scoring_panel_open(
        frame,
        flow_outer,
        scoring_intro_copy::SECTION_FLOW,
        section_font,
        ScoringPanelStyle::Diagram,
    );
    push_scoring_flow_panel(
        frame,
        groups,
        flow_content,
        h,
        flow_tile_max,
        body_font,
        small_font,
        pad,
        glb_cash_in,
    );
    y += flow_h + gap;

    let panel_gap = gap;
    let panel_w = (full_w - panel_gap * 2.0) / 3.0;
    let tiles_content = scoring_panel_open(
        frame,
        [x, y, panel_w, bottom_h],
        &scoring_section_title(1, scoring_intro_copy::SECTION_TILE_VALUES),
        section_font,
        ScoringPanelStyle::Cards,
    );
    push_scoring_tile_values_panel(
        frame,
        groups,
        tiles_content,
        values_tile_max,
        small_font,
        body_font,
        pad,
    );

    let yaku_content = scoring_panel_open(
        frame,
        [x + panel_w + panel_gap, y, panel_w, bottom_h],
        &scoring_section_title(2, scoring_intro_copy::SECTION_YAKU_RELICS),
        section_font,
        ScoringPanelStyle::Cards,
    );
    push_scoring_yaku_relics_panel(frame, yaku_content, small_font, body_font, pad);

    push_scoring_final_score_panel(
        frame,
        [x + (panel_w + panel_gap) * 2.0, y, panel_w, bottom_h],
        w,
        h,
        &scoring_section_title(3, scoring_intro_copy::SECTION_FINAL_SCORE),
        section_font,
        body_font,
        small_font,
        pad,
    );
}

// ── Economy page (page 5) ─────────────────────────────────────────────────

const ECONOMY_ITEM_COLS: usize = 3;
const ECONOMY_ITEM_ROWS: usize = 2;
const ECONOMY_ICON_COL_FRAC: f32 = 0.26;

#[derive(Clone, Copy)]
enum EconomyItemExample {
    Relic(RelicId),
    Zodiac(ZodiacKind),
    Talisman(TalismanKind),
    TilePack(TilePackKind),
    Memorial(MemorialTalismanKind),
    Temptation(TagKind),
}

const ECONOMY_ITEM_EXAMPLES: [EconomyItemExample; 6] = [
    EconomyItemExample::Relic(RelicId::GoldIdol),
    EconomyItemExample::Zodiac(ZodiacKind::Dog),
    EconomyItemExample::Talisman(TalismanKind::Pearl),
    EconomyItemExample::TilePack(TilePackKind::Flowers),
    EconomyItemExample::Memorial(MemorialTalismanKind::Hoarder),
    EconomyItemExample::Temptation(TagKind::GoldIngot),
];

fn draw_economy_page(
    frame: &mut UiFrame,
    layout: &GuideLayout,
    w: f32,
    h: f32,
    cam: &CameraParams,
    content_top: f32,
    content_floor: f32,
) {
    let gap = 10.0;
    let pad = 10.0;
    let body_font = typography::size(typography::H36, h);
    let section_font = typography::size(typography::H28, h);
    let small_font = typography::size(typography::H42, h);
    let micro_font = typography::size(typography::H45, h);
    let x = layout.content_x;
    let full_w = layout.content_w;
    let mut y = content_top;
    let usable = (content_floor - y).max(1.0);
    let top_row_h = usable * 0.50;
    let items_h = usable - top_row_h - gap;

    let panel_gap = gap;
    let flow_header_h = section_font * 1.0 + 8.0;
    let flow_content_h = (top_row_h - flow_header_h - 8.0).max(1.0);
    let ring = economy_flow_ring_layout(h, small_font, pad, f32::MAX, flow_content_h);
    let flow_w = economy_flow_panel_width(full_w, panel_gap, ring.ring_w);
    let rules_w = full_w - panel_gap - flow_w;

    let flow_content = scoring_panel_open(
        frame,
        [x, y, flow_w, top_row_h],
        economy_intro_copy::SECTION_BETWEEN_CHAMBERS,
        section_font,
        ScoringPanelStyle::Diagram,
    );
    draw_between_chambers_band(
        frame,
        flow_content,
        h,
        body_font,
        small_font,
        micro_font,
        pad,
    );

    let rules_content = scoring_panel_open(
        frame,
        [x + flow_w + panel_gap, y, rules_w, top_row_h],
        economy_intro_copy::SECTION_ECONOMY_RULES,
        section_font,
        ScoringPanelStyle::Cards,
    );
    draw_economy_rules_band(frame, rules_content, body_font, micro_font, pad);
    y += top_row_h + gap;

    push_economy_item_cards(
        frame,
        layout,
        w,
        h,
        cam,
        [x, y, full_w, items_h.max(1.0)],
        small_font,
        body_font,
        pad,
        gap,
    );
}

fn zodiac_icon_asset(kind: ZodiacKind) -> &'static str {
    match kind {
        ZodiacKind::Mouse => "textures/zodiacs/zodiac_mouse.png",
        ZodiacKind::Rat => "textures/zodiacs/zodiac_rat.png",
        ZodiacKind::Ox => "textures/zodiacs/zodiac_ox.png",
        ZodiacKind::Tiger => "textures/zodiacs/zodiac_tiger.png",
        ZodiacKind::Rabbit => "textures/zodiacs/zodiac_rabbit.png",
        ZodiacKind::Dragon => "textures/zodiacs/zodiac_dragon.png",
        ZodiacKind::Snake => "textures/zodiacs/zodiac_snake.png",
        ZodiacKind::Horse => "textures/zodiacs/zodiac_horse.png",
        ZodiacKind::Goat => "textures/zodiacs/zodiac_goat.png",
        ZodiacKind::Monkey => "textures/zodiacs/zodiac_monkey.png",
        ZodiacKind::Rooster => "textures/zodiacs/zodiac_rooster.png",
        ZodiacKind::Dog => "textures/zodiacs/zodiac_dog.png",
        ZodiacKind::Pig => "textures/zodiacs/zodiac_pig.png",
        ZodiacKind::Qilin => "textures/zodiacs/zodiac_qilin.png",
        ZodiacKind::Phoenix => "textures/zodiacs/zodiac_phoenix.png",
        ZodiacKind::Crane => "textures/zodiacs/zodiac_crane.png",
    }
}

fn economy_relic_extents(relic_id: RelicId, icon_span: f32) -> ([f32; 3], f32) {
    let visual = relic_visual(relic_id);
    let base = icon_span * 0.32;
    let seed = (relic_id as u32).wrapping_mul(2654435761) ^ 0x9E3779B9;
    let r0 = ((seed >> 8) & 0xFF) as f32 / 255.0;
    let r2 = ((seed >> 24) & 0xFF) as f32 / 255.0;
    let face = base * (0.65 + r0 * 0.45);
    let thick = base * (0.04 + r2 * 0.02);
    ([face * 2.0, thick * 2.0, face * 2.0], visual.ui_tilt_x_deg)
}

fn push_economy_icon_image(
    frame: &mut UiFrame,
    icon_rect: [f32; 4],
    path: &'static str,
    width_over_height: f32,
) {
    let [ix, iy, iw, ih] = icon_rect;
    let icon_cx = ix + iw * 0.5;
    let icon_cy = iy + ih * 0.5;
    let max_w = iw * 0.86;
    let max_h = ih * 0.88;
    let (quad_w, quad_h) = if width_over_height >= max_w / max_h {
        let w = max_w;
        (w, w / width_over_height)
    } else {
        let h = max_h;
        (h * width_over_height, h)
    };
    frame.image_quads(vec![ImageQuad {
        inst: GpuInstance {
            rect: [
                icon_cx - quad_w * 0.5,
                icon_cy - quad_h * 0.5,
                quad_w,
                quad_h,
            ],
            color: [1.0, 1.0, 1.0, 0.98],
            user: 0,
        },
        source: ImageQuadSource::Asset { path },
        clip_rect: Some(icon_rect),
    }]);
}

fn economy_icon_object3d_pos(
    w: f32,
    h: f32,
    cam: &CameraParams,
    icon_cx: f32,
    icon_cy: f32,
) -> [f32; 3] {
    let world = world_on_camera_ray_plane_z(w, h, cam, icon_cx, icon_cy, 0.0);
    object3d_pos_triple_for_world_center(w, h, world)
}

/// Orients a mesh face toward the guide camera (oblique views need more than pitch-only).
fn economy_icon_face_camera_rotation(
    w: f32,
    h: f32,
    cam: &CameraParams,
    icon_cx: f32,
    icon_cy: f32,
    local_normal: Vec3,
) -> [f32; 3] {
    let center = world_on_camera_ray_plane_z(w, h, cam, icon_cx, icon_cy, 0.0);
    let eye = Vec3::from_array(cam.eye);
    let mut toward_camera = eye - center;
    if toward_camera.length_squared() < 1e-8 {
        return camera_facing_euler_xyz_rad(cam.eye, cam.target);
    }
    toward_camera = toward_camera.normalize();
    let normal = local_normal.normalize();
    if toward_camera.dot(normal).abs() > 0.97 {
        return camera_facing_euler_xyz_rad(cam.eye, cam.target);
    }
    let q = Quat::from_rotation_arc(normal, toward_camera);
    mat4_to_euler_xyz_rad(Mat4::from_quat(q.normalize()))
}

fn economy_card_body_font(
    available_h: f32,
    inner_w: f32,
    lines: &[&str],
    start_font: f32,
    min_font: f32,
    row_gap: f32,
) -> f32 {
    let mut font = start_font;
    loop {
        let mut needed = 0.0f32;
        for line in lines {
            let wrapped = styled_text::wrap_colored_text_multiline(
                line,
                inner_w,
                font / 0.99,
                color::PARCHMENT,
                true,
                GlossaryMode::Prose,
            );
            needed += styled_text::colored_wrapped_rows_height(&wrapped, font) + row_gap;
        }
        if needed <= available_h || font <= min_font {
            return font;
        }
        font *= 0.94;
    }
}

fn push_economy_item_example(
    frame: &mut UiFrame,
    w: f32,
    h: f32,
    cam: &CameraParams,
    example: EconomyItemExample,
    icon_rect: [f32; 4],
    card_index: usize,
) {
    let [ix, iy, iw, ih] = icon_rect;
    let icon_cx = ix + iw * 0.5;
    let icon_cy = iy + ih * 0.5;
    let icon_span = iw.min(ih);
    let pos = economy_icon_object3d_pos(w, h, cam, icon_cx, icon_cy);
    let anim_id = card_index as u64;

    match example {
        EconomyItemExample::Relic(relic_id) => {
            let (extents, _) = economy_relic_extents(relic_id, icon_span);
            let rarity = all_relic_defs()
                .iter()
                .find(|d| d.id == relic_id)
                .map(|d| d.rarity)
                .unwrap_or(crate::core::relic::Rarity::Common);
            frame.object3d(Object3d {
                pos,
                extents,
                rotation: economy_icon_face_camera_rotation(
                    w,
                    h,
                    cam,
                    icon_cx,
                    icon_cy,
                    Vec3::Y,
                ),
                color: color::rarity(rarity.tier()),
                kind: Object3dKind::Relic {
                    relic_id,
                    glow: 0.12,
                    silhouette: false,
                    debuffed: false,
                },
                hover_target: 0.0,
                anim_id,
            });
        }
        EconomyItemExample::Zodiac(kind) => {
            push_economy_icon_image(
                frame,
                icon_rect,
                zodiac_icon_asset(kind),
                1.0 / 3.0,
            );
        }
        EconomyItemExample::Talisman(kind) => {
            let accent = kind.accent_color();
            let tablet = for_sale_talisman_tablet_extent(icon_span * 0.88);
            let face_base = crate::render::talisman_mesh::talisman_face_camera_rotation(14.0);
            let toward = economy_icon_face_camera_rotation(
                w,
                h,
                cam,
                icon_cx,
                icon_cy,
                Vec3::NEG_Y,
            );
            frame.object3d(Object3d {
                pos,
                extents: crate::render::talisman_mesh::talisman_object_extents(tablet),
                rotation: compose_rotation_euler(
                    rot_euler_xyz_rad(face_base[0], face_base[1], face_base[2]),
                    [
                        toward[0].to_degrees(),
                        toward[1].to_degrees(),
                        toward[2].to_degrees(),
                    ],
                ),
                color: [
                    (accent[0] * 1.15).min(1.0),
                    (accent[1] * 1.15).min(1.0),
                    (accent[2] * 1.15).min(1.0),
                    1.0,
                ],
                kind: Object3dKind::Talisman { kind },
                hover_target: 0.0,
                anim_id,
            });
        }
        EconomyItemExample::TilePack(kind) => {
            let pack_h = ih * 0.68;
            let pack_w = pack_h * PACK_ASPECT_W_OVER_H;
            let pack_t = pack_h * 0.11;
            frame.object3d(Object3d {
                pos,
                extents: [pack_w, pack_t, pack_h],
                rotation: economy_icon_face_camera_rotation(
                    w,
                    h,
                    cam,
                    icon_cx,
                    icon_cy,
                    Vec3::NEG_Y,
                ),
                color: kind.foil_tint(),
                kind: Object3dKind::Pack {
                    kind,
                    pick_id: None,
                },
                hover_target: 0.0,
                anim_id,
            });
        }
        EconomyItemExample::Memorial(kind) => {
            let accent = kind.accent_color();
            let tablet = for_sale_talisman_tablet_extent(icon_span * 0.88);
            let face_base = crate::render::talisman_mesh::talisman_face_camera_rotation(14.0);
            let toward = economy_icon_face_camera_rotation(
                w,
                h,
                cam,
                icon_cx,
                icon_cy,
                Vec3::NEG_Y,
            );
            frame.object3d(Object3d {
                pos,
                extents: crate::render::talisman_mesh::talisman_object_extents(tablet),
                rotation: compose_rotation_euler(
                    rot_euler_xyz_rad(face_base[0], face_base[1], face_base[2]),
                    [
                        toward[0].to_degrees(),
                        toward[1].to_degrees(),
                        toward[2].to_degrees(),
                    ],
                ),
                color: [
                    (accent[0] * 1.15).min(1.0),
                    (accent[1] * 1.15).min(1.0),
                    (accent[2] * 1.15).min(1.0),
                    1.0,
                ],
                kind: Object3dKind::MemorialTalisman { kind },
                hover_target: 0.0,
                anim_id,
            });
        }
        EconomyItemExample::Temptation(tag) => {
            let max_side = icon_span * 0.72;
            frame.image_quads(vec![ImageQuad {
                inst: GpuInstance {
                    rect: [
                        icon_cx - max_side * 0.5,
                        icon_cy - max_side * 0.5,
                        max_side,
                        max_side,
                    ],
                    color: [1.0, 1.0, 1.0, 0.98],
                    user: 0,
                },
                source: temptation_icon_source(tag),
                clip_rect: Some(icon_rect),
            }]);
        }
    }
}

fn economy_measure_text_width(text: &str, font_px: f32) -> f32 {
    let font_px = font_px.max(8.0);
    if let Some(font) = load_ui_font() {
        text.chars()
            .map(|ch| font.metrics(ch, font_px).advance_width)
            .sum()
    } else {
        font_px * 0.52 * text.chars().count().max(1) as f32
    }
}

fn draw_economy_panel_header(
    frame: &mut UiFrame,
    x: f32,
    y: f32,
    w: f32,
    row_h: f32,
    text: &str,
    font: f32,
) {
    frame.text(TextLabel {
        rect: [x, y, w, row_h],
        text: text.into(),
        color: color::alpha(color::BRASS, 0.92),
        align: TextAlign::Left,
        font_px: Some(font),
        bold: true,
        ..Default::default()
    });
}

fn draw_dot_leader_row(
    frame: &mut UiFrame,
    x: f32,
    y: f32,
    row_w: f32,
    row_h: f32,
    label: &str,
    value: &str,
    font: f32,
    value_col_w: f32,
    label_color: [f32; 4],
    value_color: [f32; 4],
) {
    let label_w = economy_measure_text_width(label, font).max(1.0);
    let gap = 4.0;
    let dot_char_w = economy_measure_text_width(".", font).max(1.0);
    let value_x = x + row_w - value_col_w;
    let dots_x = x + label_w + gap;
    let dots_w = (value_x - gap - dots_x).max(0.0);

    frame.text(TextLabel {
        rect: [x, y, label_w.min(row_w), row_h],
        text: label.into(),
        color: label_color,
        align: TextAlign::Left,
        font_px: Some(font),
        ..Default::default()
    });
    if dots_w >= dot_char_w {
        let dot_count = (dots_w / dot_char_w).floor() as usize;
        frame.text(TextLabel {
            rect: [dots_x, y, dots_w, row_h],
            text: ".".repeat(dot_count.max(1)),
            color: color::alpha(color::UMBER, 0.72),
            align: TextAlign::Right,
            font_px: Some(font),
            ..Default::default()
        });
    }
    frame.text(TextLabel {
        rect: [value_x, y, value_col_w, row_h],
        text: value.into(),
        color: value_color,
        align: TextAlign::Right,
        font_px: Some(font),
        mono: true,
        ..Default::default()
    });
}

fn draw_earning_note_row(
    frame: &mut UiFrame,
    x: f32,
    y: f32,
    w: f32,
    row_h: f32,
    label: &str,
    line: &str,
    font: f32,
    label_color: [f32; 4],
    line_color: [f32; 4],
) -> f32 {
    frame.text(TextLabel {
        rect: [x, y, w, row_h],
        text: label.into(),
        color: label_color,
        align: TextAlign::Left,
        font_px: Some(font),
        ..Default::default()
    });
    let indent = font * 0.35;
    frame.text(TextLabel {
        rect: [x + indent, y + row_h * 0.92, w - indent, row_h],
        text: line.into(),
        color: line_color,
        align: TextAlign::Left,
        font_px: Some(font * 0.96),
        ..Default::default()
    });
    row_h * 1.85
}

fn economy_flow_block_inner_pad(pad: f32) -> f32 {
    pad * 0.85
}

fn economy_flow_block_line_gap(pad: f32) -> f32 {
    pad * 0.22
}

fn economy_flow_font_line_height(font_px: f32, italic: bool) -> f32 {
    let base = load_ui_font()
        .and_then(|font| font.horizontal_line_metrics(font_px))
        .map(|lm| lm.new_line_size)
        .unwrap_or(font_px * 1.25);
    if italic {
        base * 1.10
    } else {
        base
    }
}

fn economy_flow_wrap_line_h(font_px: f32) -> f32 {
    font_px / 0.99
}

fn economy_flow_wrapped_line_count(text: &str, text_w: f32, font_px: f32) -> usize {
    wrap_text(text, text_w.max(1.0), economy_flow_wrap_line_h(font_px))
        .len()
        .max(1)
}

fn economy_flow_wrapped_text(text: &str, text_w: f32, font_px: f32) -> String {
    wrap_text(text, text_w.max(1.0), economy_flow_wrap_line_h(font_px)).join("\n")
}

fn economy_flow_badge_size(label_font: f32) -> f32 {
    label_font * 1.05
}

fn economy_flow_header_metrics(
    step: &economy_intro_copy::FlowStep,
    label_font: f32,
    pad: f32,
    block_w: f32,
) -> (f32, f32, f32, f32) {
    let inner_pad = economy_flow_block_inner_pad(pad);
    let label_font_px = label_font * 0.86;
    let badge = economy_flow_badge_size(label_font);
    let badge_gap = inner_pad * 0.55;
    let text_w = (block_w - inner_pad * 2.0).max(1.0);
    let title_w = (text_w - badge - badge_gap).max(1.0);
    let title_lines =
        economy_flow_wrapped_line_count(step.label, title_w, label_font_px).max(1) as f32;
    let title_h = economy_flow_font_line_height(label_font_px, false) * title_lines;
    let header_h = title_h.max(badge);
    (badge, badge_gap, title_w, header_h)
}

fn economy_flow_block_height_at_width(
    step: &economy_intro_copy::FlowStep,
    label_font: f32,
    line_font: f32,
    pad: f32,
    block_w: f32,
) -> f32 {
    let inner_pad = economy_flow_block_inner_pad(pad);
    let line_font_px = line_font * 0.92;
    let text_w = (block_w - inner_pad * 2.0).max(1.0);
    let (_, _, _, header_h) = economy_flow_header_metrics(step, label_font, pad, block_w);
    let body_line_h = economy_flow_font_line_height(line_font_px, true);
    let body_lines = economy_flow_wrapped_line_count(step.line, text_w, line_font_px).max(2) as f32;
    inner_pad
        + header_h
        + economy_flow_block_line_gap(pad)
        + body_line_h * body_lines
        + inner_pad
        + pad * 0.18
}

fn economy_flow_block_natural_width(
    step: &economy_intro_copy::FlowStep,
    label_font: f32,
    line_font: f32,
    pad: f32,
) -> f32 {
    let inner_pad = economy_flow_block_inner_pad(pad);
    let label_font_px = label_font * 0.86;
    let line_font_px = line_font * 0.92;
    let badge = economy_flow_badge_size(label_font);
    let badge_gap = inner_pad * 0.55;
    let label_w = economy_measure_text_width(step.label, label_font_px);
    let line_w = economy_measure_text_width(step.line, line_font_px);
    let header_w = inner_pad + badge + badge_gap + label_w + inner_pad;
    let body_w = inner_pad * 2.0 + line_w;
    header_w.max(body_w).max(120.0)
}

fn draw_economy_flow_block(
    frame: &mut UiFrame,
    rect: [f32; 4],
    step: &economy_intro_copy::FlowStep,
    label_font: f32,
    line_font: f32,
    pad: f32,
) {
    let [x, y, w, h] = rect;
    frame.quad(GpuInstance {
        rect,
        color: color::alpha(color::WALNUT_SOFT, 0.52),
        user: 0,
    });
    push_guide_panel_stroke(frame, rect, color::alpha(color::BRASS, 0.42));

    let inner_pad = economy_flow_block_inner_pad(pad);
    let text_w = (w - inner_pad * 2.0).max(1.0);
    let label_font_px = label_font * 0.86;
    let (badge, badge_gap, title_w, header_h) =
        economy_flow_header_metrics(step, label_font, pad, w);
    let title_y = y + inner_pad;
    let badge_rect = [x + inner_pad, title_y, badge, badge];
    frame.quad(GpuInstance {
        rect: badge_rect,
        color: color::alpha(color::WALNUT_DEEP, 0.94),
        user: 0,
    });
    push_guide_panel_stroke(frame, badge_rect, color::alpha(color::BRASS, 0.38));
    frame.text(TextLabel {
        rect: badge_rect,
        text: step.num.to_string(),
        color: color::CHAMPAGNE,
        align: TextAlign::Center,
        font_px: Some(label_font * 0.82),
        bold: true,
        ..Default::default()
    });

    let title_x = badge_rect[0] + badge + badge_gap;
    let title_h = economy_flow_font_line_height(label_font_px, false)
        * economy_flow_wrapped_line_count(step.label, title_w, label_font_px).max(1) as f32;
    frame.text(TextLabel {
        rect: [title_x, title_y, title_w, header_h.max(title_h)],
        text: economy_flow_wrapped_text(step.label, title_w, label_font_px),
        color: color::CHAMPAGNE,
        align: TextAlign::Left,
        block_vertical_align: TextBlockVerticalAlign::Top,
        font_px: Some(label_font_px),
        bold: true,
        ..Default::default()
    });

    let line_font_px = line_font * 0.92;
    let line_y = title_y + header_h + economy_flow_block_line_gap(pad);
    let body_line_h = economy_flow_font_line_height(line_font_px, true);
    let body_lines =
        economy_flow_wrapped_line_count(step.line, text_w, line_font_px).max(2) as f32;
    let body_content_h = body_line_h * body_lines;
    let line_h = (y + h - inner_pad - line_y).max(body_content_h);
    frame.text(TextLabel {
        rect: [x + inner_pad, line_y, text_w, line_h],
        text: economy_flow_wrapped_text(step.line, text_w, line_font_px),
        color: color::alpha(color::STONE, 0.88),
        align: TextAlign::Left,
        block_vertical_align: TextBlockVerticalAlign::Top,
        font_px: Some(line_font_px),
        italic: true,
        ..Default::default()
    });
}

fn economy_flow_ring_block_sizes(label_font: f32, line_font: f32, pad: f32) -> [f32; 2] {
    let block_w = economy_intro_copy::FLOW_STEPS
        .iter()
        .map(|step| economy_flow_block_natural_width(step, label_font, line_font, pad))
        .fold(0.0f32, f32::max)
        .max(1.0);
    let block_h = economy_intro_copy::FLOW_STEPS
        .iter()
        .map(|step| economy_flow_block_height_at_width(step, label_font, line_font, pad, block_w))
        .fold(0.0f32, f32::max)
        .max(1.0);
    [block_w, block_h]
}

struct EconomyFlowRingLayout {
    label_font: f32,
    line_font: f32,
    block_w: f32,
    block_h: f32,
    ring_w: f32,
    ring_h: f32,
    arrow_font: f32,
    h_gutter: f32,
    v_gutter: f32,
}

fn economy_flow_ring_layout(
    window_h: f32,
    caption_font: f32,
    pad: f32,
    max_w: f32,
    max_h: f32,
) -> EconomyFlowRingLayout {
    let arrow_font = typography::size(typography::H36, window_h).max(caption_font * 0.85);
    let h_gutter = arrow_font * 1.15;
    let v_gutter = arrow_font * 1.10;
    let mut label_font = typography::size(typography::H36, window_h).max(caption_font * 0.92);
    let mut line_font = caption_font * 0.84;
    let [mut block_w, mut block_h] = economy_flow_ring_block_sizes(label_font, line_font, pad);
    let mut ring_w = block_w * 2.0 + h_gutter;
    let mut ring_h = block_h * 2.0 + v_gutter;
    if ring_w > max_w || ring_h > max_h {
        let scale = (max_w / ring_w).min(max_h / ring_h).min(1.0);
        label_font *= scale;
        line_font *= scale;
        [block_w, block_h] = economy_flow_ring_block_sizes(label_font, line_font, pad);
        ring_w = block_w * 2.0 + h_gutter;
        ring_h = block_h * 2.0 + v_gutter;
    }
    EconomyFlowRingLayout {
        label_font,
        line_font,
        block_w,
        block_h,
        ring_w,
        ring_h,
        arrow_font,
        h_gutter,
        v_gutter,
    }
}

fn economy_flow_panel_width(
    full_w: f32,
    panel_gap: f32,
    ring_w: f32,
) -> f32 {
    let available = (full_w - panel_gap).max(1.0);
    let chrome_w = 32.0;
    let min_w = available * 0.28;
    let max_w = available * 0.50;
    (ring_w + chrome_w).clamp(min_w, max_w)
}

fn push_economy_flow_ring_arrow(frame: &mut UiFrame, rect: [f32; 4], glyph: &str, font: f32) {
    let [x, y, w, h] = rect;
    let side = font * 1.05;
    let cx = x + w * 0.5;
    let cy = y + h * 0.5;
    frame.text(TextLabel {
        rect: [cx - side * 0.5, cy - side * 0.5, side, side],
        text: glyph.into(),
        color: color::alpha(color::CHAMPAGNE, 0.90),
        align: TextAlign::Center,
        font_px: Some(font),
        ..Default::default()
    });
}

fn draw_between_chambers_band(
    frame: &mut UiFrame,
    content: [f32; 4],
    window_h: f32,
    _body_font: f32,
    caption_font: f32,
    _micro_font: f32,
    pad: f32,
) {
    let [cx, cy, cw, ch] = content;
    let ring = economy_flow_ring_layout(window_h, caption_font, pad, cw, ch);

    let origin_x = cx + (cw - ring.ring_w) * 0.5;
    let origin_y = cy + (ch - ring.ring_h) * 0.5;
    let x0 = origin_x;
    let x1 = origin_x + ring.block_w + ring.h_gutter;
    let y0 = origin_y;
    let y1 = origin_y + ring.block_h + ring.v_gutter;

    let block_rects = [
        [x0, y0, ring.block_w, ring.block_h],
        [x1, y0, ring.block_w, ring.block_h],
        [x1, y1, ring.block_w, ring.block_h],
        [x0, y1, ring.block_w, ring.block_h],
    ];

    push_economy_flow_ring_arrow(
        frame,
        [x0 + ring.block_w, y0, ring.h_gutter, ring.block_h],
        "\u{27a1}",
        ring.arrow_font,
    );
    push_economy_flow_ring_arrow(
        frame,
        [x1, y0 + ring.block_h, ring.block_w, ring.v_gutter],
        "\u{2b07}",
        ring.arrow_font,
    );
    push_economy_flow_ring_arrow(
        frame,
        [x0 + ring.block_w, y1, ring.h_gutter, ring.block_h],
        "\u{2b05}",
        ring.arrow_font,
    );
    push_economy_flow_ring_arrow(
        frame,
        [x0, y0 + ring.block_h, ring.block_w, ring.v_gutter],
        "\u{2b06}",
        ring.arrow_font,
    );

    for (step, block_rect) in economy_intro_copy::FLOW_STEPS
        .iter()
        .zip(block_rects)
    {
        draw_economy_flow_block(
            frame,
            block_rect,
            step,
            ring.label_font,
            ring.line_font,
            pad,
        );
    }
}

fn draw_skip_steps_column(
    frame: &mut UiFrame,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    font: f32,
    pad: f32,
    body_color: [f32; 4],
) {
    let steps = economy_intro_copy::SKIP_PATH_STEPS;
    let lines = economy_intro_copy::SKIP_LINES;
    let n = steps.len().min(lines.len());
    if n == 0 {
        return;
    }

    let step_color = color::alpha(color::CHAMPAGNE, 0.92);
    let label_font = font * 1.02;
    let line_font = font * 0.96;
    let label_h = label_font * 1.12;
    let line_gap = pad * 0.14;
    let block_h = (h / n as f32).max(label_h + line_font * 1.2 + line_gap);

    for i in 0..n {
        let block_y = y + i as f32 * block_h;
        frame.text(TextLabel {
            rect: [x, block_y, w, label_h],
            text: steps[i].into(),
            color: step_color,
            align: TextAlign::Left,
            font_px: Some(label_font),
            bold: true,
            ..Default::default()
        });
        frame.text(TextLabel {
            rect: [x, block_y + label_h + line_gap, w, block_h - label_h - line_gap],
            text: lines[i].into(),
            color: body_color,
            align: TextAlign::Left,
            font_px: Some(line_font),
            ..Default::default()
        });
    }
}

fn draw_economy_rules_band(
    frame: &mut UiFrame,
    content: [f32; 4],
    caption_font: f32,
    micro_font: f32,
    pad: f32,
) {
    let [cx, cy, cw, ch] = content;
    let inner_pad = pad * 0.6;
    let mut body_font = caption_font;
    let mut row_h = body_font * 1.12;
    let header_h = micro_font * 1.02;
    let body_color = color::alpha(color::PARCHMENT, 0.90);
    let yen_color = color::keyword::GOLD;
    let top_y = cy + inner_pad * 0.35;
    let bottom_y = cy + ch - inner_pad;
    let col_gap = pad * 0.48;
    let col_w = ((cw - inner_pad * 2.0 - col_gap * 2.0) / 3.0).max(1.0);

    let earning_x = cx + inner_pad;
    let store_x = earning_x + col_w + col_gap;
    let skip_x = store_x + col_w + col_gap;
    let body_h = bottom_y - top_y;
    let earn_body_h = body_h - header_h - pad * 0.14;
    let earn_units = economy_intro_copy::EARNING_CLEAR_ROWS.len() as f32
        + economy_intro_copy::EARNING_NOTE_ROWS.len() as f32 * 1.85;
    let earn_needed = earn_units * row_h + pad * 0.06;
    if earn_body_h > earn_needed {
        row_h *= (earn_body_h / earn_needed).min(1.28);
        body_font *= (earn_body_h / earn_needed).sqrt().min(1.12);
    }

    for div_x in [store_x - col_gap * 0.5, skip_x - col_gap * 0.5] {
        frame.quad(GpuInstance {
            rect: [div_x - 0.5, top_y, 1.0, body_h],
            color: color::alpha(color::UMBER, 0.36),
            user: 0,
        });
    }

    // Panel 1 — Earning Yen
    let earn_pad = pad * 0.35;
    let earn_x = earning_x + earn_pad;
    let earn_w = (col_w - earn_pad * 2.0).max(1.0);
    draw_economy_panel_header(
        frame,
        earn_x,
        top_y,
        earn_w,
        header_h,
        economy_intro_copy::SECTION_EARNING,
        micro_font,
    );
    let mut ey = top_y + header_h + pad * 0.14;
    let earning_value_col_w = economy_intro_copy::EARNING_CLEAR_ROWS
        .iter()
        .map(|(_, value)| economy_measure_text_width(value, body_font))
        .fold(0.0f32, f32::max)
        .max(1.0);
    for (label, value) in economy_intro_copy::EARNING_CLEAR_ROWS {
        if ey + row_h > bottom_y {
            break;
        }
        draw_dot_leader_row(
            frame,
            earn_x,
            ey,
            earn_w,
            row_h,
            label,
            value,
            body_font,
            earning_value_col_w,
            body_color,
            yen_color,
        );
        ey += row_h;
    }
    ey += pad * 0.06;
    for note in economy_intro_copy::EARNING_NOTE_ROWS {
        if ey + row_h > bottom_y {
            break;
        }
        let line_color = if note.label == "Interest" {
            yen_color
        } else {
            body_color
        };
        ey += draw_earning_note_row(
            frame,
            earn_x,
            ey,
            earn_w,
            row_h,
            note.label,
            note.line,
            body_font,
            body_color,
            line_color,
        );
    }

    // Panel 2 — The Storeroom
    let store_pad = pad * 0.35;
    let store_inner_x = store_x + store_pad;
    let store_inner_w = (col_w - store_pad * 2.0).max(1.0);
    draw_economy_panel_header(
        frame,
        store_inner_x,
        top_y,
        store_inner_w,
        header_h,
        economy_intro_copy::SECTION_STOREROOM,
        micro_font,
    );
    let footer_h = row_h * 1.05;
    let store_body_top = top_y + header_h + pad * 0.14;
    let store_body_h = (bottom_y - footer_h - pad * 0.08 - store_body_top).max(row_h);
    let store_line_count = economy_intro_copy::STOREROOM_LINES.len() as f32;
    let store_row_h = if store_body_h > store_line_count * row_h {
        store_body_h / store_line_count
    } else {
        row_h
    };
    let mut sty = store_body_top;
    for line in economy_intro_copy::STOREROOM_LINES {
        if sty + store_row_h > bottom_y - footer_h {
            break;
        }
        frame.text(TextLabel {
            rect: [store_inner_x, sty, store_inner_w, store_row_h],
            text: (*line).into(),
            color: body_color,
            align: TextAlign::Left,
            font_px: Some(body_font),
            ..Default::default()
        });
        sty += store_row_h;
    }
    let footer_y = bottom_y - footer_h;
    if footer_y > sty + pad * 0.08 {
        frame.quad(GpuInstance {
            rect: [store_inner_x, footer_y - pad * 0.08, store_inner_w, 1.0],
            color: color::alpha(color::UMBER, 0.34),
            user: 0,
        });
    }
    frame.text(TextLabel {
        rect: [store_inner_x, footer_y, store_inner_w, footer_h],
        text: economy_intro_copy::STOREROOM_CAPACITY_FOOTER.into(),
        color: color::alpha(color::PARCHMENT, 0.86),
        align: TextAlign::Left,
        font_px: Some(body_font * 0.94),
        ..Default::default()
    });

    // Panel 3 — Skipping
    let skip_pad = pad * 0.35;
    let skip_inner_x = skip_x + skip_pad;
    let skip_inner_w = (col_w - skip_pad * 2.0).max(1.0);
    draw_economy_panel_header(
        frame,
        skip_inner_x,
        top_y,
        skip_inner_w,
        header_h,
        economy_intro_copy::SECTION_SKIPPING,
        micro_font,
    );
    let skip_body_top = top_y + header_h + pad * 0.14;
    let skip_body_h = bottom_y - skip_body_top;
    draw_skip_steps_column(
        frame,
        skip_inner_x,
        skip_body_top,
        skip_inner_w,
        skip_body_h,
        body_font,
        pad,
        body_color,
    );
}

fn economy_item_title_color(card_index: usize) -> [f32; 4] {
    match card_index {
        0 => color::BRASS,
        1 => color::keyword::FLOWER,
        2 => color::keyword::SEASON,
        3 => color::keyword::TRIGGER,
        4 => color::JADE,
        _ => color::AMBER,
    }
}

fn economy_item_role_label(card_index: usize) -> &'static str {
    match card_index {
        0 => "PAST PLAYER'S POWER",
        1 => "YAKUS REWARD MORE",
        2 => "REMAKE YOUR TILES",
        3 => "BUILD THE WALL",
        4 => "YOUR SAD REMAINS",
        _ => "MAKE A CHOICE",
    }
}

fn push_economy_item_cards(
    frame: &mut UiFrame,
    layout: &GuideLayout,
    w: f32,
    h: f32,
    cam: &CameraParams,
    outer: [f32; 4],
    small_font: f32,
    title_font: f32,
    pad: f32,
    gap: f32,
) {
    let [_x, y, full_w, full_h] = outer;
    let cell_w = (full_w - gap * (ECONOMY_ITEM_COLS - 1) as f32) / ECONOMY_ITEM_COLS as f32;
    let cell_h = (full_h - gap * (ECONOMY_ITEM_ROWS - 1) as f32) / ECONOMY_ITEM_ROWS as f32;
    let stroke = color::alpha(color::BRASS, 0.32);
    let fill = color::alpha(color::WALNUT_RAISED, 0.28);
    let body_color = color::alpha(color::PARCHMENT, 0.90);
    let role_font = small_font * 0.78;
    let content_x = layout.content_x;
    let icon_col_w = cell_w * ECONOMY_ICON_COL_FRAC;
    let text_pad = pad * 0.75;
    let text_x_offset = icon_col_w + text_pad;

    for (i, card) in economy_intro_copy::ITEMS.iter().enumerate() {
        let col = i % ECONOMY_ITEM_COLS;
        let row = i / ECONOMY_ITEM_COLS;
        let cx = content_x + col as f32 * (cell_w + gap);
        let cy = y + row as f32 * (cell_h + gap);
        let rect = [cx, cy, cell_w, cell_h];

        frame.quad(GpuInstance {
            rect,
            color: fill,
            user: 0,
        });
        push_guide_panel_stroke(frame, rect, stroke);
        push_guide_panel_stroke(
            frame,
            [
                rect[0] + 2.0,
                rect[1] + 2.0,
                rect[2] - 4.0,
                rect[3] - 4.0,
            ],
            color::alpha(color::BRASS, 0.14),
        );

        let icon_rect = [
            cx + pad,
            cy + pad,
            (icon_col_w - pad * 1.25).max(1.0),
            (cell_h - pad * 2.0).max(1.0),
        ];
        push_economy_item_example(
            frame,
            w,
            h,
            cam,
            ECONOMY_ITEM_EXAMPLES[i],
            icon_rect,
            i,
        );

        let text_x = cx + text_x_offset;
        let inner_w = (cell_w - text_x_offset - pad).max(1.0);
        let title_color = economy_item_title_color(i);
        let title_h = title_font * 1.08;
        let role_h = role_font * 1.05;
        let text_clip = intersect_rect(
            [text_x, cy + pad, inner_w, cell_h - pad * 2.0],
            rect,
        )
        .unwrap_or(rect);
        let mut title_label = TextLabel {
            rect: [text_clip[0], text_clip[1], text_clip[2], title_h],
            text: card.title.to_uppercase(),
            color: title_color,
            align: TextAlign::Left,
            font_px: Some(title_font * 0.96),
            bold: true,
            ..Default::default()
        };
        title_label.clip_rect = Some(text_clip);
        frame.text(title_label);
        let mut role_label = TextLabel {
            rect: [
                text_clip[0],
                text_clip[1] + title_h,
                text_clip[2],
                role_h,
            ],
            text: economy_item_role_label(i).into(),
            color: color::alpha(color::STONE, 0.78),
            align: TextAlign::Left,
            font_px: Some(role_font),
            bold: true,
            ..Default::default()
        };
        role_label.clip_rect = Some(text_clip);
        frame.text(role_label);

        let body_top = cy + pad + title_h + role_h + pad * 0.12;
        let body_available = (cy + cell_h - pad - body_top).max(1.0);
        let row_gap = pad * 0.14;
        let min_font = typography::size(typography::H45, h) * 0.92;
        let body_font = economy_card_body_font(
            body_available,
            inner_w,
            card.lines,
            small_font,
            min_font,
            row_gap,
        );
        let mut line_y = body_top;
        let bottom = cy + cell_h - pad;
        for line in card.lines {
            if line_y >= bottom {
                break;
            }
            let wrapped = styled_text::wrap_colored_text_multiline(
                line,
                inner_w,
                body_font / 0.99,
                body_color,
                true,
                GlossaryMode::Prose,
            );
            let block_h = styled_text::colored_wrapped_rows_height(&wrapped, body_font);
            if line_y + block_h > bottom {
                break;
            }
            let mut labels = Vec::new();
            styled_text::push_colored_rows_left(
                &mut labels,
                styled_text::ColoredRowsLayout {
                    text_left: text_x,
                    top_y: line_y,
                    inner_w,
                    line_h: body_font,
                    fallback_plain: line,
                    fallback_color: body_color,
                    italic: false,
                    glossary: GlossaryMode::Prose,
                },
                &wrapped,
            );
            for label in &mut labels {
                label.clip_rect = Some(text_clip);
            }
            frame.texts(labels);
            line_y += block_h + row_gap;
        }
    }
}

// ── Tanuki's Tips page (page 6) ───────────────────────────────────────────

struct TanukiTipsScrollLayout {
    viewport: [f32; 4],
    cell_w: f32,
    cell_h: f32,
    gap: f32,
    pad: f32,
    rows: usize,
    max_scroll_px: f32,
    wheel_step_px: f32,
}

/// Content band top/bottom for a guide page (matches [`push_guide_chrome`] without drawing).
fn guide_content_band(w: f32, h: f32, back: [f32; 4], subtitle: Option<&str>) -> (f32, f32) {
    let layout = GuideLayout::new(w, h);
    let nav_header = guide_nav_header(w, h, back, subtitle);
    let jr = (w.min(h) / 720.0).clamp(1.0, 1.38);
    let content_top = nav_header.content_top + 1.0 + (18.0 * jr).max(14.0);
    (content_top, layout.content_bottom)
}

fn tanuki_tips_scroll_layout(
    layout: &GuideLayout,
    content_top: f32,
    content_floor: f32,
) -> TanukiTipsScrollLayout {
    const ROWS: usize = 2;
    const VISIBLE_COLS: f32 = 2.35;
    let scale = metrics::scene_scale(layout.window_w, layout.window_h);
    let gap = (16.0 * scale).max(12.0);
    let pad = (14.0 * scale).max(10.0);
    let scroll_track_reserve = (14.0 * scale).max(10.0);
    let x = layout.content_x;
    let full_w = layout.content_w;
    let usable_h = (content_floor - content_top).max(1.0);
    let grid_h = (usable_h - scroll_track_reserve).max(1.0);
    let cell_h = ((grid_h - gap) / ROWS as f32).max(1.0);
    let tip_count = tanuki_tips_intro_copy::TIPS.len();
    let cols = tip_count.div_ceil(ROWS).max(1);
    let min_cell_w = (260.0 * scale).min(full_w * 0.24);
    let fill_cell_w =
        (full_w - gap * (cols.saturating_sub(1)) as f32) / cols as f32;
    let scroll_cell_w = (full_w - gap * (VISIBLE_COLS - 1.0)) / VISIBLE_COLS;
    let total_fill_w = cols as f32 * fill_cell_w + cols.saturating_sub(1) as f32 * gap;
    let cell_w = if total_fill_w <= full_w {
        fill_cell_w.max(min_cell_w)
    } else {
        scroll_cell_w.max(min_cell_w)
    };
    let total_w = cols as f32 * cell_w + cols.saturating_sub(1) as f32 * gap;
    let max_scroll_px = (total_w - full_w).max(0.0);
    TanukiTipsScrollLayout {
        viewport: [x, content_top, full_w, usable_h],
        cell_w,
        cell_h,
        gap,
        pad,
        rows: ROWS,
        max_scroll_px,
        wheel_step_px: (cell_w * 0.22).clamp(48.0 * scale, 120.0 * scale),
    }
}

fn draw_tanuki_tips_page(
    frame: &mut UiFrame,
    layout: &GuideLayout,
    h: f32,
    tips_layout: &TanukiTipsScrollLayout,
    scroll_px: f32,
) {
    let TanukiTipsScrollLayout {
        viewport,
        cell_w,
        cell_h,
        gap,
        pad,
        rows,
        ..
    } = *tips_layout;
    let quote_color = color::CHAMPAGNE;
    let stroke = color::alpha(color::STONE, 0.32);
    let fill = color::alpha(color::WALNUT_RAISED, 0.22);
    let [vx, vy, vw, vh] = viewport;
    let content_x = layout.content_x;

    for (i, tip) in tanuki_tips_intro_copy::TIPS.iter().enumerate() {
        let col = i / rows;
        let row = i % rows;
        let cx = content_x + col as f32 * (cell_w + gap) - scroll_px;
        let cy = vy + row as f32 * (cell_h + gap);
        let rect = [cx, cy, cell_w, cell_h];
        let Some(clipped_panel) = intersect_rect(rect, viewport) else {
            continue;
        };

        let inner_w = (cell_w - pad * 2.0).max(1.0);
        let inner_h = (cell_h - pad * 2.0).max(1.0);
        let text_clip = intersect_rect([cx + pad, cy + pad, inner_w, inner_h], viewport)
            .unwrap_or(viewport);
        let quote_text = tanuki_tips_intro_copy::quoted(tip);

        frame.quad(GpuInstance {
            rect: clipped_panel,
            color: fill,
            user: 0,
        });
        push_guide_panel_stroke(frame, clipped_panel, stroke);

        let quote_area_h = inner_h;
        let mut font = typography::size(typography::H36, h);
        let min_font = typography::size(typography::H42, h);
        let wrapped = loop {
            let lines = styled_text::wrap_colored_text_multiline(
                &quote_text,
                inner_w,
                font / 0.99,
                quote_color,
                true,
                GlossaryMode::Prose,
            );
            let block_h = styled_text::colored_wrapped_rows_height(&lines, font);
            if block_h <= quote_area_h || font <= min_font {
                break lines;
            }
            font *= 0.94;
        };
        let quote_top = cy + pad;

        let mut labels = Vec::new();
        styled_text::push_colored_rows_left(
            &mut labels,
            styled_text::ColoredRowsLayout {
                text_left: text_clip[0],
                top_y: quote_top,
                inner_w: text_clip[2],
                line_h: font,
                fallback_plain: &quote_text,
                fallback_color: quote_color,
                italic: true,
                glossary: GlossaryMode::Prose,
            },
            &wrapped,
        );
        for label in &mut labels {
            label.clip_rect = Some(text_clip);
        }
        frame.texts(labels);
    }

    if tips_layout.max_scroll_px > 0.5 {
        let track_h = 4.0;
        let track_y = vy + vh - track_h - 6.0;
        let track = [vx, track_y, vw, track_h];
        frame.quad(GpuInstance {
            rect: track,
            color: color::alpha(color::STONE, 0.28),
            user: 0,
        });
        let thumb_w = (vw * (vw / (vw + tips_layout.max_scroll_px))).clamp(48.0, vw);
        let thumb_travel = (vw - thumb_w).max(0.0);
        let thumb_x = vx + thumb_travel * (scroll_px / tips_layout.max_scroll_px);
        frame.quad(GpuInstance {
            rect: [thumb_x, track_y, thumb_w, track_h],
            color: color::alpha(color::BRASS, 0.72),
            user: 0,
        });
    }
}

fn scoring_section_title(index: u8, title: &str) -> String {
    format!("{index}. {title}")
}

#[derive(Clone, Copy)]
enum ScoringPanelStyle {
    Diagram,
    Cards,
    Formula,
}

/// Draw panel chrome and return the inner content rect.
fn scoring_panel_open(
    frame: &mut UiFrame,
    rect: [f32; 4],
    title: &str,
    section_font: f32,
    style: ScoringPanelStyle,
) -> [f32; 4] {
    let [x, y, w, h] = rect;
    let header_h = section_font * 1.0 + 8.0;
    let inset = 8.0;
    let (fill, stroke, header_fill, title_color) = match style {
        ScoringPanelStyle::Diagram => (
            color::alpha(color::WALNUT_DEEP, 0.78),
            color::alpha(color::BRASS, 0.42),
            color::alpha(color::WALNUT_RAISED, 0.82),
            color::CHAMPAGNE,
        ),
        ScoringPanelStyle::Cards => (
            color::alpha(color::WALNUT_DEEP, 0.72),
            color::alpha(color::STONE, 0.38),
            color::alpha(color::WALNUT_RAISED, 0.88),
            color::alpha(color::BRASS, 0.95),
        ),
        ScoringPanelStyle::Formula => (
            color::alpha(color::WALNUT_DEEP, 0.94),
            color::alpha(color::GOLD, 0.62),
            color::alpha(color::WALNUT_SOFT, 0.96),
            color::GOLD,
        ),
    };
    frame.quad(GpuInstance {
        rect,
        color: fill,
        user: 0,
    });
    push_scoring_panel_stroke(frame, rect, stroke, style);
    frame.quad(GpuInstance {
        rect: [x, y, w, header_h],
        color: header_fill,
        user: 0,
    });
    let rule_color = match style {
        ScoringPanelStyle::Formula => color::alpha(color::GOLD, 0.75),
        _ => color::alpha(color::GOLD, 0.55),
    };
    frame.quad(GpuInstance {
        rect: [x, y + header_h - 1.0, w, 1.5],
        color: rule_color,
        user: 0,
    });
    frame.text(TextLabel {
        rect: [x + inset, y + 4.0, w - inset * 2.0, header_h - 6.0],
        text: title.into(),
        color: title_color,
        align: TextAlign::Left,
        font_px: Some(section_font * 0.94),
        bold: true,
        ..Default::default()
    });
    [
        x + inset,
        y + header_h + 6.0,
        w - inset * 2.0,
        (h - header_h - 8.0).max(1.0),
    ]
}

/// Fit showcase tiles inside a cell without clipping panel chrome.
fn scoring_tile_size_for_cell(cell: [f32; 4], tile_count: usize, max_px: f32) -> f32 {
    let [_, _, cw, ch] = cell;
    let n = tile_count.max(1) as f32;
    let by_width = cw / (n * 0.58 + 0.30);
    let by_height = ch * 0.68;
    max_px.min(by_width).min(by_height)
}

fn push_scoring_panel_stroke(
    frame: &mut UiFrame,
    rect: [f32; 4],
    stroke: [f32; 4],
    style: ScoringPanelStyle,
) {
    push_guide_panel_stroke(frame, rect, stroke);
    if matches!(style, ScoringPanelStyle::Formula) {
        let [x, y, w, h] = rect;
        let inset = 2.0;
        push_guide_panel_stroke(
            frame,
            [x + inset, y + inset, w - inset * 2.0, h - inset * 2.0],
            color::alpha(color::GOLD, 0.22),
        );
    }
}

const SCORING_FLOW_ARROW_ASPECT: f32 = 210.0 / 150.0;
const SCORING_FLOW_STAGES: usize = 4;
const SCORING_FLOW_CASH_IN_STAGE: usize = 2;

fn scoring_guide_tile_caps(w: f32, h: f32) -> (f32, f32) {
    let scale = metrics::scene_scale(w, h);
    let flow = (52.0 * scale).max(h * 0.065);
    let values = (56.0 * scale).max(h * 0.070);
    (flow, values)
}

fn scoring_flow_ui_scale(content_h: f32) -> f32 {
    (content_h / 420.0).clamp(1.0, 2.8)
}

/// Shared lane geometry for the scoring flow diagram (titles, graphics, arrows).
struct ScoringFlowDiagramLayout {
    cx: f32,
    cy: f32,
    cw: f32,
    diagram_h: f32,
    title_h: f32,
    caption_h: f32,
    visual_h: f32,
    lane_w: f32,
    lane_gap: f32,
    arrow_slot_w: f32,
    lane_pad_x: f32,
    graphic_row_h: f32,
    graphic_row_y: f32,
    flow_tile: f32,
    reminder_h: f32,
    arrow_h_max: f32,
}

impl ScoringFlowDiagramLayout {
    fn new(
        content: [f32; 4],
        body_font: f32,
        caption_font: f32,
        pad: f32,
        tile_max: f32,
    ) -> Self {
        let [cx, cy, cw, ch] = content;
        let reminder_font = caption_font * 0.96;
        let reminder_h = reminder_font * 1.28;
        let diagram_h = (ch - reminder_h - pad * 1.35).max(1.0);
        let title_h = body_font * 1.02;
        let caption_h = caption_font * 1.12;
        let visual_top = cy + title_h + caption_h + pad * 0.35;
        let visual_h = (cy + diagram_h - visual_top - pad * 0.25).max(1.0);
        let ui_scale = scoring_flow_ui_scale(ch);
        let arrow_h_max = 38.0 * ui_scale;
        let arrow_h = (visual_h * 0.22).clamp(24.0, arrow_h_max);
        let arrow_img_w = arrow_h * SCORING_FLOW_ARROW_ASPECT;
        let arrow_slot_w = arrow_img_w + pad * 0.55;
        let lane_gap = pad * 0.40;
        let lane_w = (cw - lane_gap * (SCORING_FLOW_STAGES - 1) as f32
            - arrow_slot_w * (SCORING_FLOW_STAGES - 1) as f32)
            / SCORING_FLOW_STAGES as f32;
        let lane_pad_x = pad * 0.30;
        let lane_inner_w = (lane_w - lane_pad_x * 2.0).max(1.0);
        let probe = [0.0, 0.0, lane_inner_w, visual_h];
        let flow_tile = scoring_tile_size_for_cell(probe, SCORING_STRUCTURE_SLOT_COUNT, tile_max)
            .min(scoring_tile_size_for_cell(probe, 3, tile_max));
        let graphic_row_h = (flow_tile * 1.18).min(visual_h * 0.72);
        let graphic_row_y = visual_top + (visual_h - graphic_row_h) * 0.5;
        Self {
            cx,
            cy,
            cw,
            diagram_h,
            title_h,
            caption_h,
            visual_h,
            lane_w,
            lane_gap,
            arrow_slot_w,
            lane_pad_x,
            graphic_row_h,
            graphic_row_y,
            flow_tile,
            reminder_h,
            arrow_h_max,
        }
    }

    fn from_flow_outer(
        flow_outer: [f32; 4],
        section_font: f32,
        body_font: f32,
        caption_font: f32,
        pad: f32,
        tile_max: f32,
    ) -> Self {
        Self::new(
            scoring_flow_inner_content_rect(flow_outer, section_font),
            body_font,
            caption_font,
            pad,
            tile_max,
        )
    }

    fn lane_x(&self, stage: usize) -> f32 {
        self.cx + stage as f32 * (self.lane_w + self.lane_gap + self.arrow_slot_w)
    }

    fn lane_graphic_row(&self, stage: usize) -> [f32; 4] {
        let lane_x = self.lane_x(stage);
        [
            lane_x + self.lane_pad_x,
            self.graphic_row_y,
            (self.lane_w - self.lane_pad_x * 2.0).max(1.0),
            self.graphic_row_h,
        ]
    }

    fn arrow_rect(&self, stage: usize) -> [f32; 4] {
        let lane_x = self.lane_x(stage);
        let arrow_h = (self.visual_h * 0.22).clamp(24.0, self.arrow_h_max);
        let arrow_w = arrow_h * SCORING_FLOW_ARROW_ASPECT;
        let arrow_x =
            lane_x + self.lane_w + (self.lane_gap + self.arrow_slot_w) * 0.5 - arrow_w * 0.5;
        let arrow_y = self.graphic_row_y + self.graphic_row_h * 0.5 - arrow_h * 0.5;
        [arrow_x, arrow_y, arrow_w, arrow_h]
    }
}

/// Graphic-row target for the authored cash-in mesh (step 3).
fn scoring_flow_cash_in_visual_rect(
    flow_outer: [f32; 4],
    section_font: f32,
    body_font: f32,
    caption_font: f32,
    pad: f32,
    tile_max: f32,
) -> [f32; 4] {
    let layout = ScoringFlowDiagramLayout::from_flow_outer(
        flow_outer,
        section_font,
        body_font,
        caption_font,
        pad,
        tile_max,
    );
    layout.lane_graphic_row(SCORING_FLOW_CASH_IN_STAGE)
}

fn scoring_flow_inner_content_rect(flow_outer: [f32; 4], section_font: f32) -> [f32; 4] {
    let [x, y, w, h] = flow_outer;
    let header_h = section_font * 1.0 + 8.0;
    let inset = 8.0;
    [
        x + inset,
        y + header_h + 6.0,
        (w - inset * 2.0).max(1.0),
        (h - header_h - 8.0).max(1.0),
    ]
}

fn push_scoring_gameplay_cash_in_env(
    frame: &mut UiFrame,
    ctx: &DrawCtx<'_>,
    w: f32,
    h: f32,
    cash_in_visual: [f32; 4],
) -> bool {
    let env_h = ctx.room_gltf_height_scale.max(0.01);
    let Some(cam) =
        gameplay_glb::gameplay_cash_in_camera_for_screen_rect_if_present(w, h, env_h, cash_in_visual)
    else {
        return false;
    };

    // Keep the guide camera for showcase tiles; cash-in glTF draws via overlay camera.
    frame.gameplay_cash_in_overlay_camera = Some(cam);
    frame.gameplay_env_cash_in_only = true;
    frame.gameplay_cash_in_button_visible = true;
    frame.gameplay_cash_in_glow = 0.35;

    let room_glb_lights = gameplay_glb::gameplay_glb_has_embedded_lights();
    let mut overlay_lighting = SceneLighting::default();
    overlay_lighting.embedded_gltf_punctual = room_glb_lights;
    overlay_lighting.room_glb_brdf = room_glb_lights;
    if room_glb_lights {
        let tune = ctx.room_env_for("gameplay").0;
        let (punctual, nodes) = crate::render::room_gltf_punctual::tagged_to_scene_punctual(
            gameplay_glb::gameplay_embedded_point_lights_runtime_tagged(
                w,
                h,
                env_h,
                &tune,
                0.0,
                1.0,
                ctx.flame_tuning.candle_flicker_amp,
            ),
        );
        overlay_lighting.punctual = punctual;
        overlay_lighting.punctual_gltf_nodes = nodes;
        overlay_lighting.set_gltf_embedded_spot_lights(
            gameplay_glb::gameplay_embedded_spot_lights_runtime(w, h, env_h, &tune),
        );
    }
    frame.gameplay_cash_in_overlay_lighting = Some(overlay_lighting);
    true
}

fn push_scoring_flow_panel(
    frame: &mut UiFrame,
    groups: &[TileGroup],
    content: [f32; 4],
    window_h: f32,
    tile_max: f32,
    body_font: f32,
    caption_font: f32,
    pad: f32,
    glb_cash_in: bool,
) {
    let flow = ScoringFlowDiagramLayout::new(content, body_font, caption_font, pad, tile_max);
    let mut placements = Vec::new();
    let mut flow_arrows = Vec::new();

    let steps = [
        (
            scoring_intro_copy::FLOW_STEP_SELECT,
            scoring_intro_copy::FLOW_SELECT_CAPTION,
        ),
        (
            scoring_intro_copy::FLOW_STEP_PLAY,
            scoring_intro_copy::FLOW_PLAY_CAPTION,
        ),
        (
            scoring_intro_copy::FLOW_STEP_CASH_IN,
            scoring_intro_copy::FLOW_CASH_IN_CAPTION,
        ),
        (
            scoring_intro_copy::FLOW_STEP_SCORE,
            scoring_intro_copy::FLOW_SCORE_CAPTION,
        ),
    ];

    for (i, (title, caption)) in steps.iter().enumerate() {
        let lane_x = flow.lane_x(i);
        let lane = [lane_x, flow.cy, flow.lane_w, flow.diagram_h];
        frame.text(TextLabel {
            rect: [lane[0], lane[1], lane[2], flow.title_h],
            text: title.to_string(),
            color: color::CHAMPAGNE,
            align: TextAlign::Center,
            font_px: Some(body_font * 0.92),
            bold: true,
            ..Default::default()
        });
        frame.text(TextLabel {
            rect: [lane[0], lane[1] + flow.title_h, lane[2], flow.caption_h],
            text: caption.to_string(),
            color: color::alpha(color::PARCHMENT, 0.88),
            align: TextAlign::Center,
            font_px: Some(caption_font * 0.90),
            ..Default::default()
        });

        let graphic = flow.lane_graphic_row(i);
        match i {
            0 => {
                placements.extend(layout_scoring_group_tiles(
                    groups,
                    SCORING_FLOW_MELD,
                    graphic,
                    flow.flow_tile,
                    0.50,
                    false,
                ));
            }
            1 => {
                let Some(group) = groups.get(SCORING_FLOW_MELD) else {
                    continue;
                };
                push_scoring_structure_slots(
                    frame,
                    &mut placements,
                    &group.tiles,
                    graphic,
                    flow.flow_tile,
                );
            }
            2 => {
                if !glb_cash_in {
                    push_scoring_cash_in_plaque(frame, graphic, body_font);
                }
            }
            3 => {
                let eq_font = typography::size(typography::H24, window_h).max(body_font * 0.98);
                push_scoring_formula_colored(
                    frame,
                    graphic,
                    scoring_intro_copy::FLOW_SCORE_FORMULA,
                    eq_font,
                );
            }
            _ => {}
        }

        if i + 1 < SCORING_FLOW_STAGES {
            let arrow_rect = flow.arrow_rect(i);
            flow_arrows.push(ImageQuad {
                inst: GpuInstance {
                    rect: arrow_rect,
                    color: [1.0, 1.0, 1.0, 0.95],
                    user: 0,
                },
                source: ImageQuadSource::Asset {
                    path: scoring_intro_copy::FLOW_ARROW_ASSET,
                },
                clip_rect: None,
            });
        }
    }

    if !flow_arrows.is_empty() {
        frame.image_quads(flow_arrows);
    }

    if glb_cash_in {
        frame.gameplay_environment();
    }

    if !placements.is_empty() {
        frame.cmds.push(DrawCmd::ShowcaseTileBatch(placements.into()));
    }

    frame.text(TextLabel {
        rect: [
            flow.cx,
            flow.cy + flow.diagram_h + pad * 0.35,
            flow.cw,
            flow.reminder_h,
        ],
        text: scoring_intro_copy::FLOW_REMINDER.into(),
        color: color::PARCHMENT,
        align: TextAlign::Center,
        font_px: Some(caption_font * 0.96),
        ..Default::default()
    });
}

fn push_scoring_structure_slots(
    frame: &mut UiFrame,
    placements: &mut Vec<ShowcaseTilePlacement>,
    tiles: &[Tile],
    rect: [f32; 4],
    tile_size: f32,
) {
    let gap = 4.0;
    let slot_w =
        (rect[2] - gap * (SCORING_STRUCTURE_SLOT_COUNT.saturating_sub(1)) as f32)
            / SCORING_STRUCTURE_SLOT_COUNT as f32;
    let slot_h = rect[3];
    for slot_i in 0..SCORING_STRUCTURE_SLOT_COUNT {
        let slot = [
            rect[0] + slot_i as f32 * (slot_w + gap),
            rect[1],
            slot_w,
            slot_h,
        ];
        if slot_i < SCORING_STRUCTURE_FILLED {
            let Some(tile) = tiles.get(slot_i) else {
                push_structure_empty_slot(frame, slot);
                continue;
            };
            let size = scoring_tile_size_for_cell(slot, 1, tile_size);
            placements.extend(layout_tiles_in_cell(
                std::slice::from_ref(tile),
                slot,
                size,
                0.50,
                false,
            ));
        } else {
            push_structure_empty_slot(frame, slot);
        }
    }
}

fn push_structure_empty_slot(frame: &mut UiFrame, rect: [f32; 4]) {
    let inset = 2.0;
    let inner = [
        rect[0] + inset,
        rect[1] + inset,
        (rect[2] - inset * 2.0).max(1.0),
        (rect[3] - inset * 2.0).max(1.0),
    ];
    frame.quad(GpuInstance {
        rect: inner,
        color: color::alpha(color::WALNUT_DEEP, 0.40),
        user: 0,
    });
    push_guide_panel_stroke(frame, inner, color::alpha(color::STONE, 0.40));
}

fn push_scoring_cash_in_plaque(frame: &mut UiFrame, rect: [f32; 4], body_font: f32) {
    let btn_h = (rect[3] * 0.36).clamp(body_font * 1.20, rect[3] * 0.44);
    let btn_w = (rect[2] * 0.78).clamp(body_font * 3.0, rect[2] * 0.88);
    let btn = [
        rect[0] + (rect[2] - btn_w) * 0.5,
        rect[1] + (rect[3] - btn_h) * 0.5,
        btn_w,
        btn_h,
    ];
    let mut quads = Vec::new();
    let mut labels = Vec::new();
    widget::push_button(
        &mut quads,
        &mut labels,
        &mut Vec::new(),
        widget::ButtonSpec {
            rect: btn,
            label: scoring_intro_copy::FLOW_CASH_IN_BUTTON,
            variant: ButtonVariant::Primary,
            state: ButtonState::Rest,
            action: UiAction::Confirm,
        },
    );
    frame.quads(quads);
    for label in labels {
        frame.text(label);
    }
}

fn push_scoring_formula_colored(
    frame: &mut UiFrame,
    rect: [f32; 4],
    text: &str,
    font_px: f32,
) {
    let mut labels = Vec::new();
    let line_h = font_px;
    let drawn_h = styled_text::push_colored_line_left(
        &mut labels,
        rect[0],
        rect[1] + (rect[3] - line_h * styled_text::COLORED_ROW_LINE_STEP_MUL) * 0.5,
        rect[2],
        line_h,
        text,
        color::CHAMPAGNE,
        GlossaryMode::Prose,
    );
    let _ = drawn_h;
    frame.texts(labels);
}

fn push_scoring_tile_values_panel(
    frame: &mut UiFrame,
    groups: &[TileGroup],
    content: [f32; 4],
    tile_size: f32,
    caption_font: f32,
    body_font: f32,
    pad: f32,
) {
    let [cx, cy, cw, ch] = content;
    let caption_h = push_dense_text(
        frame,
        [cx, cy, cw, 0.0],
        scoring_intro_copy::TILE_VALUES_CAPTION,
        caption_font * 0.94,
        color::alpha(color::PARCHMENT, 0.88),
    );
    let examples_top = cy + caption_h + pad * 0.45;
    let examples_h = (cy + ch - examples_top - pad * 0.15).max(1.0);
    let col_count = SCORING_CHIP_GROUPS.len().max(1);
    let col_gap = pad * 0.65;
    let col_w = (cw - col_gap * (col_count.saturating_sub(1)) as f32) / col_count as f32;
    let value_font = body_font * 0.96;
    let name_h = caption_font * 1.02;
    let value_h = value_font * 1.10;
    let text_h = name_h + value_h + pad * 0.25;
    let tile_h = (examples_h - text_h).max(1.0);
    let mut placements = Vec::new();

    for (i, &gi) in SCORING_CHIP_GROUPS.iter().enumerate() {
        let Some(group) = groups.get(gi) else {
            continue;
        };
        let col_x = cx + i as f32 * (col_w + col_gap);
        let tile_area = [
            col_x + pad * 0.10,
            examples_top,
            col_w - pad * 0.20,
            tile_h,
        ];
        let tile_px = scoring_tile_size_for_cell(tile_area, 1, tile_size);
        placements.extend(layout_scoring_group_tiles(
            groups, gi, tile_area, tile_px, 0.50, false,
        ));
        let name_y = examples_top + tile_h + pad * 0.18;
        frame.text(TextLabel {
            rect: [col_x, name_y, col_w, name_h],
            text: group.label.into(),
            color: color::CHAMPAGNE,
            align: TextAlign::Center,
            font_px: Some(caption_font * 0.90),
            bold: true,
            ..Default::default()
        });
        if let Some(chips) = group.subtitle {
            frame.text(TextLabel {
                rect: [col_x, name_y + name_h, col_w, value_h],
                text: chips.into(),
                color: color::alpha(color::BRASS, 0.95),
                align: TextAlign::Center,
                font_px: Some(value_font),
                bold: true,
                ..Default::default()
            });
        }
    }
    if !placements.is_empty() {
        frame.cmds.push(DrawCmd::ShowcaseTileBatch(placements.into()));
    }
}

fn push_scoring_yaku_relics_panel(
    frame: &mut UiFrame,
    content: [f32; 4],
    caption_font: f32,
    body_font: f32,
    pad: f32,
) {
    let [x, y, w, h] = content;
    let intro_font = caption_font * 0.90;
    let mut cursor = y + pad * 0.15;
    for line in [
        scoring_intro_copy::YAKU_RELICS_INTRO,
        scoring_intro_copy::YAKU_RELICS_CASH_IN,
        scoring_intro_copy::YAKU_RELICS_RELICS,
    ] {
        let mut labels = Vec::new();
        let drawn = styled_text::push_colored_line_left(
            &mut labels,
            x,
            cursor,
            w,
            intro_font,
            line,
            color::alpha(color::PARCHMENT, 0.90),
            GlossaryMode::Prose,
        );
        frame.texts(labels);
        cursor += drawn + pad * 0.18;
    }

    let table_top = cursor + pad * 0.25;
    let table_h = (y + h - table_top).max(1.0);
    let header_h = body_font * 1.02;
    let row_h = ((table_h - header_h) / 4.0).max(body_font * 1.05);
    let col_example_w = w * 0.40;
    let col_num_w = (w - col_example_w) * 0.5;
    let header_y = table_top;
    frame.text(TextLabel {
        rect: [x, header_y, col_example_w, header_h],
        text: scoring_intro_copy::YAKU_TABLE_HEADER_EXAMPLE.into(),
        color: color::CHAMPAGNE,
        align: TextAlign::Left,
        font_px: Some(body_font * 0.92),
        bold: true,
        ..Default::default()
    });
    frame.text(TextLabel {
        rect: [x + col_example_w, header_y, col_num_w, header_h],
        text: scoring_intro_copy::YAKU_TABLE_HEADER_CHIPS.into(),
        color: color::keyword::CHIPS,
        align: TextAlign::Right,
        font_px: Some(body_font * 0.92),
        bold: true,
        ..Default::default()
    });
    frame.text(TextLabel {
        rect: [x + col_example_w + col_num_w, header_y, col_num_w, header_h],
        text: scoring_intro_copy::YAKU_TABLE_HEADER_MULT.into(),
        color: color::keyword::MULT,
        align: TextAlign::Right,
        font_px: Some(body_font * 0.92),
        bold: true,
        ..Default::default()
    });
    frame.quad(GpuInstance {
        rect: [x, header_y + header_h - 1.0, w, 1.0],
        color: color::alpha(color::BRASS, 0.42),
        user: 0,
    });

    let rows: [(&str, String, String); 3] = [
        (
            YakuKind::Tanyao.name(),
            format!("+{} chips", YakuKind::Tanyao.chip_bonus()),
            format!("+{:.1} mult", YakuKind::Tanyao.mult_bonus()),
        ),
        (
            YakuKind::Yakuhai.name(),
            format!("+{} chips", YakuKind::Yakuhai.chip_bonus()),
            format!("+{:.1} mult", YakuKind::Yakuhai.mult_bonus()),
        ),
        (
            scoring_intro_copy::YAKU_TABLE_RELIC_ROW,
            format!("+{} chips", scoring_intro_copy::RELIC_EXAMPLE_CHIPS),
            format!("+{:.1} mult", scoring_intro_copy::RELIC_EXAMPLE_MULT),
        ),
    ];

    let mut row_y = header_y + header_h;
    for (name, chips, mult) in rows {
        frame.text(TextLabel {
            rect: [x, row_y, col_example_w, row_h],
            text: name.into(),
            color: color::PARCHMENT,
            align: TextAlign::Left,
            font_px: Some(caption_font * 0.88),
            ..Default::default()
        });
        frame.text(TextLabel {
            rect: [x + col_example_w, row_y, col_num_w, row_h],
            text: chips,
            color: color::keyword::CHIPS,
            align: TextAlign::Right,
            font_px: Some(caption_font * 0.88),
            ..Default::default()
        });
        frame.text(TextLabel {
            rect: [x + col_example_w + col_num_w, row_y, col_num_w, row_h],
            text: mult,
            color: color::keyword::MULT,
            align: TextAlign::Right,
            font_px: Some(caption_font * 0.88),
            ..Default::default()
        });
        row_y += row_h;
    }
}

fn push_scoring_final_score_panel(
    frame: &mut UiFrame,
    rect: [f32; 4],
    _window_w: f32,
    window_h: f32,
    title: &str,
    section_font: f32,
    body_font: f32,
    caption_font: f32,
    pad: f32,
) {
    let content = scoring_panel_open(
        frame,
        rect,
        title,
        section_font,
        ScoringPanelStyle::Formula,
    );
    let [x, y, w, h] = content;
    let eq_font = typography::size(typography::H24, window_h).max(body_font * 1.08);
    let detail_font = caption_font * 0.90;
    let eq_h = h * 0.24;
    let detail_h = h * 0.34;
    let example_h = h * 0.18;

    push_scoring_panel_background(
        frame,
        [x, y + 1.0, w, eq_h - 2.0],
        color::alpha(color::GOLD, 0.12),
        color::alpha(color::GOLD, 0.50),
    );
    push_scoring_formula_colored(
        frame,
        [x, y, w, eq_h],
        scoring_intro_copy::FINAL_EQUATION,
        eq_font,
    );

    let detail_y = y + eq_h;
    let detail_line_h = detail_h * 0.5;
    for (i, line) in [
        scoring_intro_copy::FINAL_CHIPS_LINE,
        scoring_intro_copy::FINAL_MULT_LINE,
    ]
    .iter()
    .enumerate()
    {
        let mut labels = Vec::new();
        let _ = styled_text::push_colored_line_left(
            &mut labels,
            x,
            detail_y + detail_line_h * i as f32,
            w,
            detail_font,
            line,
            color::alpha(color::PARCHMENT, 0.92),
            GlossaryMode::Prose,
        );
        frame.texts(labels);
    }

    let example_y = detail_y + detail_h;
    let mut example_labels = Vec::new();
    let _ = styled_text::push_colored_line_left(
        &mut example_labels,
        x,
        example_y + (example_h - detail_font * styled_text::COLORED_ROW_LINE_STEP_MUL) * 0.5,
        w,
        detail_font * 1.04,
        scoring_intro_copy::FINAL_EXAMPLE,
        color::alpha(color::BRASS, 0.95),
        GlossaryMode::Prose,
    );
    frame.texts(example_labels);

    let _ = pad;
}

fn layout_scoring_group_tiles(
    groups: &[TileGroup],
    group_index: usize,
    cell: [f32; 4],
    tile_size: f32,
    y_center: f32,
    align_start: bool,
) -> Vec<ShowcaseTilePlacement> {
    let Some(group) = groups.get(group_index) else {
        return Vec::new();
    };
    layout_tiles_in_cell(&group.tiles, cell, tile_size, y_center, align_start)
}

fn layout_tiles_in_cell(
    tiles: &[Tile],
    cell: [f32; 4],
    tile_size: f32,
    y_center: f32,
    align_start: bool,
) -> Vec<ShowcaseTilePlacement> {
    let [cx, cy, cw, ch] = cell;
    let n = tiles.len().max(1);
    let size = tile_size.min(cw / (n as f32 * 0.5 + 0.12)).min(ch * 0.72);
    let row_w = size * n as f32;
    let start_x = if align_start {
        cx + size * 0.22
    } else {
        cx + (cw - row_w) * 0.5
    };
    let center_y = cy + ch * y_center.clamp(0.25, 0.75);
    tiles
        .iter()
        .enumerate()
        .map(|(i, tile)| ShowcaseTilePlacement {
            tile: *tile,
            center_pos: [start_x + size * (i as f32 + 0.5), center_y, 0.0],
            rotation: GUIDE_TILE_ROTATION,
            scale: 1.0,
            size_px: size,
            brightness: 1.0,
            opacity: 1.0,
            selected: false,
            hovered: false,
            outline: false,
            glow: false,
            glow_color: None,
            outline_sel: None,
            pick_id: None,
            overlay_rect_group: None,
        })
        .collect()
}

fn push_scoring_panel_background(
    frame: &mut UiFrame,
    rect: [f32; 4],
    fill: [f32; 4],
    stroke: [f32; 4],
) {
    frame.quad(GpuInstance {
        rect,
        color: fill,
        user: 0,
    });
    push_guide_panel_stroke(frame, rect, stroke);
}

// ── Tiles intro page (page 0) ─────────────────────────────────────────────

fn draw_tiles_page(
    frame: &mut UiFrame,
    layout: &GuideLayout,
    w: f32,
    h: f32,
    scale: f32,
    groups: &[TileGroup],
    cam: &CameraParams,
    content_top: f32,
    content_floor: f32,
) {
    let left_w = layout.content_w * 0.38;
    let gutter = layout.content_w * 0.02;
    let right_w = layout.content_w - left_w - gutter;
    let left_x = layout.content_x;
    let right_x = left_x + left_w + gutter;
    let columns_bottom = content_floor - h * 0.006;

    push_tiles_left_cards(
        frame,
        left_x,
        left_w,
        content_top,
        columns_bottom,
        h,
        typography::size(typography::H42, h),
        1.12,
    );

    let (placements, labels, panels) =
        layout_tiles_page_grid(cam, groups, right_x, right_w, w, h, content_top, columns_bottom);
    push_tiles_example_panels(frame, groups, &panels);
    if !placements.is_empty() {
        frame.cmds.push(DrawCmd::ShowcaseTileBatch(placements.into()));
    }
    push_tiles_example_labels(frame, groups, &labels, h, scale);
}

// ── Melds page (page 1) ───────────────────────────────────────────────────

const MELDS_EXAMPLE_ROWS: &[&[usize]] = &[&[0, 1], &[2, 3, 4], &[5, 6]];
const MELDS_ROW_WEIGHTS: [f32; 3] = [0.28, 0.38, 0.34];

fn draw_melds_page(
    frame: &mut UiFrame,
    layout: &GuideLayout,
    _progress: &PlayerProgress,
    w: f32,
    h: f32,
    scale: f32,
    groups: &[TileGroup],
    cam: &CameraParams,
    content_top: f32,
    content_floor: f32,
) {
    let body_font = typography::size(typography::H42, h);
    let line_mul = 1.12;
    let left_w = layout.content_w * 0.38;
    let gutter = layout.content_w * 0.02;
    let right_w = layout.content_w - left_w - gutter;
    let left_x = layout.content_x;
    let right_x = left_x + left_w + gutter;
    let columns_bottom = content_floor - h * 0.006;

    push_melds_left_cards(
        frame,
        left_x,
        left_w,
        content_top,
        columns_bottom - h * 0.14,
        h,
        body_font,
        line_mul,
    );
    if let Some(scrawl) = page_graffiti(PAGE_MELDS) {
        push_flowers_margin_scrawl(frame, left_x, left_w, columns_bottom, h, scrawl);
    }

    let (placements, labels, panels, _cells) = layout_guide_example_grid(
        cam,
        groups,
        right_x,
        right_w,
        w,
        h,
        content_top,
        columns_bottom,
        MELDS_EXAMPLE_ROWS,
        &MELDS_ROW_WEIGHTS,
        0.0,
        GuideExampleCellLayout::default(),
    );
    push_tiles_example_panels(frame, groups, &panels);
    if !placements.is_empty() {
        frame.cmds.push(DrawCmd::ShowcaseTileBatch(placements.into()));
    }
    push_tiles_example_labels(frame, groups, &labels, h, scale);
}

// ── Yaku intro page (page 2) ──────────────────────────────────────────────

const YAKU_EXAMPLE_ROWS: &[&[usize]] = &[&[0], &[1], &[2]];
const YAKU_ROW_WEIGHTS: [f32; 3] = [1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0];
/// Guide yaku intro tablets are drawn larger than the live HUD for readability.
const GUIDE_YAKU_TABLET_SCALE: f32 = 1.45;
/// Yaku intro page: left prose column share (remainder is example tiles).
const YAKU_INTRO_LEFT_COL_FRAC: f32 = 0.34;

fn guide_yaku_tablet_metrics(w: f32, h: f32, tablet_count: usize) -> (f32, f32) {
    let (row_h, pill_w) =
        crate::render::gameplay_glb::gameplay_yaku_tablet_ui_metrics(w, h, tablet_count);
    (
        row_h * GUIDE_YAKU_TABLET_SCALE,
        pill_w * GUIDE_YAKU_TABLET_SCALE,
    )
}

fn guide_yaku_tablet_reserve(w: f32, h: f32) -> f32 {
    let (row_h, _) = guide_yaku_tablet_metrics(w, h, 2);
    row_h + (h * 0.012).clamp(6.0, 12.0)
}

/// After tile *i*, insert extra horizontal gap before the next meld begins.
fn guide_example_meld_breaks_after(tiles: &[Tile]) -> Vec<bool> {
    let n = tiles.len();
    let mut breaks = vec![false; n];
    if n < 2 {
        return breaks;
    }
    let Some(sets) = validate_selection(tiles) else {
        return breaks;
    };
    let mut id_to_meld = std::collections::HashMap::with_capacity(n);
    for (mi, set) in sets.iter().enumerate() {
        for &id in &set.tile_ids {
            id_to_meld.insert(id, mi);
        }
    }
    for i in 0..n - 1 {
        if let (Some(a), Some(b)) = (
            id_to_meld.get(&tiles[i].id),
            id_to_meld.get(&tiles[i + 1].id),
        ) {
            breaks[i] = a != b;
        }
    }
    breaks
}

fn guide_example_meld_gap_count(tiles: &[Tile]) -> usize {
    guide_example_meld_breaks_after(tiles)
        .iter()
        .filter(|&&b| b)
        .count()
}

fn guide_example_meld_gap_px(window_h: f32, tile_px: f32) -> f32 {
    (tile_px * 0.28).clamp((window_h * 0.012).max(10.0), 32.0)
}

fn guide_example_row_width(tile_px: f32, tiles: &[Tile], meld_gap: f32) -> f32 {
    let n = tiles.len();
    if n == 0 {
        return 0.0;
    }
    tile_px * n as f32 + meld_gap * guide_example_meld_gap_count(tiles) as f32
}

/// Fixed overhead per yaku intro example row (title + gap + tablet band, no subtitle).
fn guide_yaku_example_row_overhead(h: f32, tablet_row_reserve: f32) -> f32 {
    let pad = 4.0;
    let title_h = typography::size(typography::H28, h) * 1.05;
    let label_tile_gap = (h * 0.012).clamp(8.0, 14.0);
    pad * 2.0 + title_h + label_tile_gap + tablet_row_reserve
}

/// One tile size for all yaku intro rows so examples read at the same scale.
fn guide_yaku_shared_tile_px(
    col_w: f32,
    h: f32,
    usable_h: f32,
    groups: &[TileGroup],
    rows: &[&[usize]],
    row_weights: &[f32],
    tablet_row_reserve: f32,
) -> f32 {
    let overhead = guide_yaku_example_row_overhead(h, tablet_row_reserve);
    let row_gap = 3.0;
    let weight_sum: f32 = row_weights.iter().sum();
    let tile_cap = h * 0.070;
    let floor = (h * 0.044).clamp(38.0, 52.0);
    let mut min_px = f32::MAX;
    let mut tightest_fit = f32::MAX;
    for (row_i, indices) in rows.iter().enumerate() {
        let row_weight = row_weights.get(row_i).copied().unwrap_or(1.0);
        let row_h = usable_h * (row_weight / weight_sum) - row_gap * 0.5;
        let tile_area_h = (row_h - overhead).max(20.0);
        tightest_fit = tightest_fit.min(tile_area_h * 0.88);
        for &gi in indices.iter() {
            if gi >= groups.len() {
                continue;
            }
            let n = groups[gi].tiles.len().max(1) as f32;
            let meld_gap_est = (h * 0.014).clamp(10.0, 22.0);
            let meld_gaps = guide_example_meld_gap_count(&groups[gi].tiles) as f32;
            let px = ((col_w - meld_gap_est * meld_gaps) / (n + 0.15))
                .min(tile_area_h * 0.88)
                .min(tile_cap);
            min_px = min_px.min(px);
        }
    }
    if min_px == f32::MAX {
        tightest_fit.min(floor).max(24.0)
    } else {
        min_px.max(floor.min(tightest_fit)).min(tightest_fit).max(24.0)
    }
}

fn draw_yaku_intro_page(
    frame: &mut UiFrame,
    layout: &GuideLayout,
    w: f32,
    h: f32,
    scale: f32,
    groups: &[TileGroup],
    cam: &CameraParams,
    content_top: f32,
    content_floor: f32,
) {
    let body_font = typography::size(typography::H42, h);
    let line_mul = 1.12;
    let left_w = layout.content_w * YAKU_INTRO_LEFT_COL_FRAC;
    let gutter = layout.content_w * 0.02;
    let right_w = layout.content_w - left_w - gutter;
    let left_x = layout.content_x;
    let right_x = left_x + left_w + gutter;
    let columns_bottom = content_floor - h * 0.006;

    push_yaku_left_cards(
        frame,
        left_x,
        left_w,
        content_top,
        columns_bottom,
        h,
        body_font,
        line_mul,
    );

    let tablet_row_h = guide_yaku_tablet_reserve(w, h);
    let usable_h = (columns_bottom - content_top).max(1.0);
    let shared_tile_px = guide_yaku_shared_tile_px(
        right_w,
        h,
        usable_h,
        groups,
        YAKU_EXAMPLE_ROWS,
        &YAKU_ROW_WEIGHTS,
        tablet_row_h,
    );
    let yaku_cell_layout = GuideExampleCellLayout {
        fixed_tile_px: Some(shared_tile_px),
        compact_headers: true,
        tile_height_cap: 0.070,
    };
    let (placements, labels, _panels, cells) = layout_guide_example_grid(
        cam,
        groups,
        right_x,
        right_w,
        w,
        h,
        content_top,
        columns_bottom,
        YAKU_EXAMPLE_ROWS,
        &YAKU_ROW_WEIGHTS,
        tablet_row_h,
        yaku_cell_layout,
    );
    if !placements.is_empty() {
        frame.cmds.push(DrawCmd::ShowcaseTileBatch(placements.into()));
    }
    push_tiles_example_labels(frame, groups, &labels, h, scale);
    for cell in &cells {
        let Some(group) = groups.get(cell.group_index) else {
            continue;
        };
        let yaku = example_structure_yaku(&group.tiles);
        push_guide_yaku_tablets(frame, cell.rect, cell.tiles_bottom, &yaku, w, h);
    }
}

fn push_yaku_left_cards(
    frame: &mut UiFrame,
    x: f32,
    w: f32,
    top: f32,
    bottom: f32,
    h: f32,
    body_font: f32,
    line_mul: f32,
) {
    let section_font = typography::size(typography::H28, h);
    let pad = 10.0;
    let inner_w = (w - pad * 2.0).max(1.0);
    let sections: &[(&str, &[&str])] = &[
        (
            yaku_intro_copy::SECTION_STRUCTURE,
            yaku_intro_copy::STRUCTURE_LINES,
        ),
        (
            yaku_intro_copy::SECTION_CASH_IN,
            yaku_intro_copy::CASH_IN_LINES,
        ),
        (
            tiles_intro_copy::SECTION_RANK_TERMS,
            tiles_intro_copy::RANK_TERM_LINES,
        ),
    ];
    push_guide_left_panels(
        frame,
        x,
        w,
        top,
        bottom,
        h,
        body_font,
        line_mul,
        section_font,
        pad,
        inner_w,
        &sections,
    );
}

// ── Flowers page (page 3) ───────────────────────────────────────────────────

const FLOWERS_EXAMPLE_ROWS: &[&[usize]] = &[&[0, 1], &[3, 2], &[5, 4]];
const FLOWERS_ROW_WEIGHTS: [f32; 3] = [0.36, 0.36, 0.28];

fn draw_flowers_page(
    frame: &mut UiFrame,
    layout: &GuideLayout,
    w: f32,
    h: f32,
    scale: f32,
    groups: &[TileGroup],
    cam: &CameraParams,
    content_top: f32,
    content_floor: f32,
) {
    let body_font = typography::size(typography::H42, h);
    let line_mul = 1.12;
    let left_w = layout.content_w * 0.38;
    let gutter = layout.content_w * 0.02;
    let right_w = layout.content_w - left_w - gutter;
    let left_x = layout.content_x;
    let right_x = left_x + left_w + gutter;
    let columns_bottom = content_floor - h * 0.006;

    push_flowers_left_cards(
        frame,
        left_x,
        left_w,
        content_top,
        columns_bottom,
        h,
        body_font,
        line_mul,
    );
    if let Some(scrawl) = page_graffiti(PAGE_FLOWERS) {
        push_flowers_margin_scrawl(frame, left_x, left_w, columns_bottom, h, scrawl);
    }

    let (placements, labels, panels, _cells) = layout_guide_example_grid(
        cam,
        groups,
        right_x,
        right_w,
        w,
        h,
        content_top,
        columns_bottom,
        FLOWERS_EXAMPLE_ROWS,
        &FLOWERS_ROW_WEIGHTS,
        0.0,
        GuideExampleCellLayout::default(),
    );
    push_tiles_example_panels(frame, groups, &panels);
    if !placements.is_empty() {
        frame.cmds.push(DrawCmd::ShowcaseTileBatch(placements.into()));
    }
    push_tiles_example_labels(frame, groups, &labels, h, scale);
}

fn push_flowers_left_cards(
    frame: &mut UiFrame,
    x: f32,
    w: f32,
    top: f32,
    bottom: f32,
    h: f32,
    body_font: f32,
    line_mul: f32,
) {
    let section_font = typography::size(typography::H28, h);
    let pad = 10.0;
    let inner_w = (w - pad * 2.0).max(1.0);
    let sections: [(&str, &[&str]); 2] = [
        (
            flowers_intro_copy::SECTION_ALLOWED,
            flowers_intro_copy::ALLOWED_LINES,
        ),
        (
            flowers_intro_copy::SECTION_NOT_ALLOWED,
            flowers_intro_copy::NOT_ALLOWED_LINES,
        ),
    ];
    push_guide_left_panels(
        frame,
        x,
        w,
        top,
        bottom - h * 0.14,
        h,
        body_font,
        line_mul,
        section_font,
        pad,
        inner_w,
        &sections,
    );
}

fn push_flowers_margin_scrawl(
    frame: &mut UiFrame,
    x: f32,
    w: f32,
    bottom: f32,
    h: f32,
    text: &str,
) {
    let font = typography::size(typography::H42, h);
    let pad = 10.0;
    let inner_w = (w - pad * 2.0).max(1.0);
    let default = color::alpha(color::STONE, 0.72);
    let wrapped = styled_text::wrap_colored_text_multiline(
        text,
        inner_w,
        font / 0.99,
        default,
        true,
        GlossaryMode::Prose,
    );
    let line_h = font;
    let block_h = styled_text::colored_wrapped_rows_height(&wrapped, line_h);
    let y = bottom - block_h - h * 0.008;
    let mut labels = Vec::new();
    styled_text::push_colored_rows_left(
        &mut labels,
        styled_text::ColoredRowsLayout {
            text_left: x + pad,
            top_y: y,
            inner_w,
            line_h,
            fallback_plain: text,
            fallback_color: default,
            italic: true,
            glossary: GlossaryMode::Prose,
        },
        &wrapped,
    );
    frame.texts(labels);
}

fn push_melds_left_cards(
    frame: &mut UiFrame,
    x: f32,
    w: f32,
    top: f32,
    bottom: f32,
    h: f32,
    body_font: f32,
    line_mul: f32,
) {
    let section_font = typography::size(typography::H28, h);
    let pad = 10.0;
    let inner_w = (w - pad * 2.0).max(1.0);
    let sections: [(&str, &[&str]); 1] = [(
        melds_intro_copy::SECTION_SEQUENCE_RULES,
        melds_intro_copy::SEQUENCE_RULE_LINES,
    )];
    push_guide_left_panels(
        frame,
        x,
        w,
        top,
        bottom,
        h,
        body_font,
        line_mul,
        section_font,
        pad,
        inner_w,
        &sections,
    );
}

fn tiles_section_panel_height(
    heading: &str,
    lines: &[&str],
    inner_w: f32,
    section_font: f32,
    body_font: f32,
    line_mul: f32,
    pad: f32,
) -> f32 {
    let heading_h = widget::plain_text_block_height(heading, inner_w, section_font, line_mul);
    let body_line_h = body_font * line_mul;
    let body_h = styled_text::colored_lines_block_height(
        lines,
        inner_w,
        body_line_h,
        color::PARCHMENT,
        GlossaryMode::Panel,
    );
    pad + heading_h + 6.0 + body_h + pad
}

fn push_tiles_left_cards(
    frame: &mut UiFrame,
    x: f32,
    w: f32,
    top: f32,
    bottom: f32,
    h: f32,
    body_font: f32,
    line_mul: f32,
) {
    let section_font = typography::size(typography::H28, h);
    let pad = 10.0;
    let inner_w = (w - pad * 2.0).max(1.0);
    let sections: &[(&str, &[&str])] = &[
        (
            tiles_intro_copy::SECTION_NUMBER_SUITS,
            tiles_intro_copy::NUMBER_SUIT_LINES,
        ),
        (
            tiles_intro_copy::SECTION_HONOR_SUITS,
            tiles_intro_copy::HONOR_LINES,
        ),
        (
            tiles_intro_copy::SECTION_FLOWERS,
            tiles_intro_copy::FLOWER_LINES,
        ),
    ];
    push_guide_left_panels(
        frame,
        x,
        w,
        top,
        bottom,
        h,
        body_font,
        line_mul,
        section_font,
        pad,
        inner_w,
        sections,
    );
}

fn push_guide_left_panels(
    frame: &mut UiFrame,
    x: f32,
    w: f32,
    top: f32,
    bottom: f32,
    _h: f32,
    body_font: f32,
    line_mul: f32,
    section_font: f32,
    pad: f32,
    inner_w: f32,
    sections: &[(&str, &[&str])],
) {
    let available = (bottom - top).max(1.0);
    let min_gap = 4.0;

    let mut eff_line_mul = line_mul;
    let mut heights: Vec<f32> = sections
        .iter()
        .map(|(heading, lines)| {
            tiles_section_panel_height(
                heading,
                lines,
                inner_w,
                section_font,
                body_font,
                eff_line_mul,
                pad,
            )
        })
        .collect();
    let mut total_natural: f32 =
        heights.iter().sum::<f32>() + min_gap * (sections.len().saturating_sub(1)) as f32;
    if total_natural > available && total_natural > 0.0 {
        eff_line_mul = line_mul * (available / total_natural) * 0.97;
        heights = sections
            .iter()
            .map(|(heading, lines)| {
                tiles_section_panel_height(
                    heading,
                    lines,
                    inner_w,
                    section_font,
                    body_font,
                    eff_line_mul,
                    pad,
                )
            })
            .collect();
        total_natural =
            heights.iter().sum::<f32>() + min_gap * (sections.len().saturating_sub(1)) as f32;
    }
    let section_gap = if sections.len() > 1 && total_natural <= available {
        ((available - total_natural) / (sections.len() - 1) as f32).min(10.0)
    } else {
        min_gap
    };

    let mut y = top;
    let stroke = color::alpha(color::STONE, 0.32);
    let fill = color::alpha(color::WALNUT_RAISED, 0.22);

    for (idx, ((heading, lines), &panel_h)) in sections.iter().zip(heights.iter()).enumerate() {
        frame.quad(GpuInstance {
            rect: [x, y, w, panel_h],
            color: fill,
            user: 0,
        });
        push_guide_panel_stroke(frame, [x, y, w, panel_h], stroke);

        let mut cursor = y + pad;
        cursor += push_dense_text_lines(
            frame,
            [x + pad, cursor, inner_w, 0.0],
            heading,
            section_font,
            color::CHAMPAGNE,
            eff_line_mul,
        );
        cursor += 6.0;

        let body_line_h = body_font * eff_line_mul;
        let mut labels = Vec::new();
        for line in *lines {
            let line_h = styled_text::push_colored_line_left(
                &mut labels,
                x + pad,
                cursor,
                inner_w,
                body_line_h,
                line,
                color::PARCHMENT,
                GlossaryMode::Panel,
            );
            cursor += line_h;
        }
        for label in labels {
            frame.text(label);
        }

        y += panel_h;
        if idx + 1 < sections.len() {
            y += section_gap;
        }
    }
}

/// Label anchors for a tile example cell (title above tiles).
struct TilesExampleLabel {
    title_rect: [f32; 4],
    title: &'static str,
    subtitle: Option<&'static str>,
}

/// Layout metadata for one guide example cell (tile group + optional yaku tablets).
struct GuideExampleCell {
    group_index: usize,
    rect: [f32; 4],
    tiles_bottom: f32,
}

/// Per-page tuning for [`layout_guide_example_grid`] cells.
#[derive(Clone, Copy)]
struct GuideExampleCellLayout {
    fixed_tile_px: Option<f32>,
    /// Drop subtitle from header reserve and label draw (yaku intro page).
    compact_headers: bool,
    tile_height_cap: f32,
}

impl Default for GuideExampleCellLayout {
    fn default() -> Self {
        Self {
            fixed_tile_px: None,
            compact_headers: false,
            tile_height_cap: 0.082,
        }
    }
}

/// Yaku scored by a complete example structure (excluding chicken hand).
fn example_structure_yaku(tiles: &[Tile]) -> Vec<YakuKind> {
    let Some(sets) = validate_selection(tiles) else {
        return Vec::new();
    };
    let mut detected: Vec<_> = detect_yaku_with_wind(tiles, &sets, None, None, None)
        .into_iter()
        .filter(|y| *y != YakuKind::ChickenHand)
        .collect();
    detected.sort_by(YakuKind::cmp_by_base_score);
    detected
}

fn push_guide_yaku_tablets(
    frame: &mut UiFrame,
    cell: [f32; 4],
    tiles_bottom: f32,
    yaku: &[YakuKind],
    w: f32,
    h: f32,
) {
    if yaku.is_empty() {
        return;
    }
    let (row_h, pill_max_w) = guide_yaku_tablet_metrics(w, h, yaku.len());
    let caption_px = (row_h * 0.36).clamp(24.0, 44.0);
    let gap = 6.0 * ((w.min(h)) / 600.0).max(0.85) * GUIDE_YAKU_TABLET_SCALE;
    let pad = 4.0;
    let clip = ChartClip {
        top: cell[1],
        bottom: cell[1] + cell[3],
    };
    let row_y = tiles_bottom + (h * 0.008).clamp(4.0, 10.0);
    let pill_ws: Vec<f32> = yaku
        .iter()
        .map(|yk| yaku_pill_width(yk.name(), caption_px, row_h).min(pill_max_w))
        .collect();
    let mut x = cell[0] + pad;
    let mut squircles = Vec::new();
    let mut labels = Vec::new();
    let face = yaku_pill_face();
    let ink = yaku_pill_ink();
    let rim = yaku_pill_rim();
    for (&yk, &pill_w) in yaku.iter().zip(pill_ws.iter()) {
        let drawn_w = push_yaku_pill(
            &mut squircles,
            &mut labels,
            clip,
            x,
            row_y,
            row_h,
            yk.name(),
            pill_w,
            face,
            ink,
            rim,
            caption_px,
        );
        x += drawn_w + gap;
    }
    frame.squircle_quads(squircles);
    for label in labels {
        frame.text(label);
    }
}

fn push_guide_tile_placements(
    placements: &mut Vec<ShowcaseTilePlacement>,
    tiles: &[Tile],
    start_x: f32,
    center_y: f32,
    tile_size: f32,
    tile_gap: f32,
) {
    let mut cursor_x = start_x;
    for tile in tiles {
        let px = cursor_x + tile_size * 0.5;
        placements.push(ShowcaseTilePlacement {
            tile: *tile,
            center_pos: [px, center_y, 0.0],
            rotation: GUIDE_TILE_ROTATION,
            scale: 1.0,
            size_px: tile_size,
            brightness: 1.0,
            opacity: 1.0,
            selected: false,
            hovered: false,
            outline: false,
            glow: false,
            glow_color: None,
            outline_sel: None,
            pick_id: None,
            overlay_rect_group: None,
        });
        cursor_x += tile_size + tile_gap;
    }
}

/// Tiles intro page — label column left, tiles fill the rest; row heights track tile size.
fn layout_tiles_page_grid(
    _cam: &CameraParams,
    groups: &[TileGroup],
    col_x: f32,
    col_w: f32,
    _window_w: f32,
    window_h: f32,
    top: f32,
    bottom: f32,
) -> (
    Vec<ShowcaseTilePlacement>,
    Vec<TilesExampleLabel>,
    Vec<(usize, [f32; 4])>,
) {
    let available_h = (bottom - top).max(1.0);
    let row_gap = 6.0;
    let tile_gap = 3.0;
    let pad = 3.0;
    let label_col_w = (col_w * 0.15).clamp(90.0, 160.0);
    let tile_span_w = (col_w - label_col_w).max(1.0);

    let title_font = typography::size(typography::H28, window_h);
    let sub_font = typography::size(typography::H45, window_h);
    let title_h = title_font * 1.05;
    let sub_line_h = sub_font * 1.02;
    let side_label_h = title_h
        + styled_text::colored_line_block_height(
            "ranks 1–9",
            label_col_w,
            sub_line_h,
            color::PARCHMENT,
            GlossaryMode::Prose,
        );

    let width_for = |span: f32, count: usize| -> f32 {
        let gaps = tile_gap * count.saturating_sub(1) as f32;
        ((span - gaps) / count.max(1) as f32).max(1.0)
    };
    let width_limit = width_for(tile_span_w, 9);

    let tiles_page_total_h = |tile_size: f32| -> f32 {
        let subtitled_row_h = tile_size.max(side_label_h) + pad * 2.0;
        let title_only_row_h = tile_size.max(title_h) + pad * 2.0;
        subtitled_row_h * 3.0 + title_only_row_h * 3.0 + row_gap * 5.0
    };

    let mut tile_size = width_limit;
    let height_budget = available_h * 0.94;
    for _ in 0..40 {
        if tiles_page_total_h(tile_size) <= height_budget {
            break;
        }
        tile_size *= 0.94;
    }
    tile_size = (tile_size * 0.88).max(24.0);

    let subtitled_row_h = tile_size.max(side_label_h) + pad * 2.0;
    let title_only_row_h = tile_size.max(title_h) + pad * 2.0;
    let row_heights: Vec<f32> = groups
        .iter()
        .take(6)
        .map(|group| {
            if group.subtitle.is_some() {
                subtitled_row_h
            } else {
                title_only_row_h
            }
        })
        .collect();
    let row_count = row_heights.len().max(1);
    let total_row_h: f32 = row_heights.iter().sum();
    let even_gap = ((available_h - total_row_h) / (row_count + 1) as f32).max(row_gap);
    let tile_center_in_row = |row_top: f32, row_h: f32| {
        row_top + pad + (row_h - pad * 2.0 - tile_size) * 0.5 + tile_size * 0.5
    };

    let mut placements = Vec::new();
    let mut labels = Vec::new();
    let mut row_y = top + even_gap;
    let tile_start_x = col_x + label_col_w;

    for (group, &row_h) in groups.iter().take(6).zip(row_heights.iter()) {
        labels.push(TilesExampleLabel {
            title_rect: [col_x + pad, row_y + pad, label_col_w - pad, title_h],
            title: group.label,
            subtitle: group.subtitle,
        });
        push_guide_tile_placements(
            &mut placements,
            &group.tiles,
            tile_start_x,
            tile_center_in_row(row_y, row_h),
            tile_size,
            tile_gap,
        );
        row_y += row_h + even_gap;
    }

    (placements, labels, vec![])
}

fn layout_guide_example_grid(
    cam: &CameraParams,
    groups: &[TileGroup],
    col_x: f32,
    col_w: f32,
    window_w: f32,
    window_h: f32,
    top: f32,
    bottom: f32,
    rows: &[&[usize]],
    row_weights: &[f32],
    tablet_row_reserve: f32,
    cell_layout: GuideExampleCellLayout,
) -> (
    Vec<ShowcaseTilePlacement>,
    Vec<TilesExampleLabel>,
    Vec<(usize, [f32; 4])>,
    Vec<GuideExampleCell>,
) {
    let usable_h = (bottom - top).max(1.0);
    let row_gap = 3.0;
    let weight_sum: f32 = row_weights.iter().sum();
    let mut placements = Vec::new();
    let mut labels = Vec::new();
    let mut panels = Vec::new();
    let mut cells = Vec::new();
    let mut row_y = top;

    for (row_i, indices) in rows.iter().enumerate() {
        let row_weight = row_weights.get(row_i).copied().unwrap_or(1.0);
        let row_h = usable_h * (row_weight / weight_sum) - row_gap * 0.5;
        let cell_ws = tiles_row_cell_widths(indices, groups, col_w, row_gap);
        let mut cell_x = col_x;
        for (col_i, &gi) in indices.iter().enumerate() {
            if gi >= groups.len() {
                continue;
            }
            let cw = cell_ws[col_i];
            let cell = [cell_x, row_y, cw, row_h];
            if groups[gi].framed {
                panels.push((gi, cell));
            }
            let (p, l, tiles_bottom) = layout_tile_group_cell(
                cam,
                &groups[gi],
                cell,
                window_w,
                window_h,
                tablet_row_reserve,
                cell_layout,
            );
            placements.extend(p);
            if let Some(lbl) = l {
                labels.push(lbl);
            }
            cells.push(GuideExampleCell {
                group_index: gi,
                rect: cell,
                tiles_bottom,
            });
            cell_x += cw + row_gap;
        }
        row_y += row_h + row_gap;
    }

    (placements, labels, panels, cells)
}

fn tiles_row_cell_widths(
    indices: &[usize],
    groups: &[TileGroup],
    col_w: f32,
    gap: f32,
) -> Vec<f32> {
    let n = indices.len().max(1);
    let usable = (col_w - gap * (n.saturating_sub(1)) as f32).max(1.0);
    let weights: Vec<f32> = indices
        .iter()
        .map(|&i| {
            let tiles = groups.get(i).map(|g| g.tiles.len()).unwrap_or(1);
            tiles as f32 + if tiles >= 4 { 0.6 } else { 0.35 }
        })
        .collect();
    let sum: f32 = weights.iter().sum();
    weights.iter().map(|w| usable * w / sum).collect()
}

fn layout_tile_group_cell(
    _cam: &CameraParams,
    group: &TileGroup,
    cell: [f32; 4],
    _window_w: f32,
    window_h: f32,
    tablet_row_reserve: f32,
    cell_layout: GuideExampleCellLayout,
) -> (Vec<ShowcaseTilePlacement>, Option<TilesExampleLabel>, f32) {
    let [cx, cy, cw, ch] = cell;
    let pad = if group.framed {
        12.0
    } else {
        4.0
    };
    let title_font = typography::size(typography::H28, window_h);
    let sub_font = typography::size(typography::H45, window_h);
    let inner_w = (cw - pad * 2.0).max(1.0);
    let title_h = title_font * 1.05;
    let sub_line_h = sub_font * 1.02;
    let sub_h = if cell_layout.compact_headers {
        0.0
    } else {
        group
            .subtitle
            .map(|sub| {
                styled_text::colored_line_block_height(
                    sub,
                    inner_w,
                    sub_line_h,
                    color::PARCHMENT,
                    GlossaryMode::Prose,
                )
            })
            .unwrap_or(0.0)
    };
    let label_tile_gap = (window_h * 0.012).clamp(8.0, 14.0);
    let tile_area_top = cy + pad + title_h + sub_h + label_tile_gap;
    let tile_area_h = (cy + ch - pad - tile_area_top - tablet_row_reserve).max(20.0);

    let n = group.tiles.len().max(1);
    let invalid = guide_example_is_invalid(group);
    let max_tile = cell_layout
        .fixed_tile_px
        .map(|px| px.min(tile_area_h * 0.88))
        .unwrap_or_else(|| {
            (cw / (n as f32 + 0.35 + if invalid { 0.25 } else { 0.0 }))
                .min(tile_area_h * 0.88)
                .min(window_h * cell_layout.tile_height_cap)
                .max(24.0)
        });
    let inter_tile_gap = if invalid { max_tile * 0.12 } else { 0.0 };
    let meld_gap = if invalid {
        0.0
    } else {
        guide_example_meld_gap_px(window_h, max_tile)
    };
    let meld_breaks = if invalid {
        vec![false; n]
    } else {
        guide_example_meld_breaks_after(&group.tiles)
    };
    let tile_center_y = (tile_area_top + max_tile * 0.5).min(cy + ch - pad - max_tile * 0.5);
    let group_w = if invalid {
        max_tile * n as f32 + inter_tile_gap * (n.saturating_sub(1) as f32)
    } else {
        guide_example_row_width(max_tile, &group.tiles, meld_gap)
    };
    let start_x = if tablet_row_reserve > 0.0 {
        cx + pad
    } else {
        cx + (cw - group_w) * 0.5
    };
    let mut placements = Vec::with_capacity(n);
    let mut cursor_x = start_x;

    for (tile_i, tile) in group.tiles.iter().enumerate() {
        let px = cursor_x + max_tile * 0.5;
        placements.push(ShowcaseTilePlacement {
            tile: *tile,
            center_pos: [px, tile_center_y, 0.0],
            rotation: if invalid {
                guide_invalid_tile_rotation(tile_i)
            } else {
                GUIDE_TILE_ROTATION
            },
            scale: 1.0,
            size_px: max_tile,
            brightness: 1.0,
            opacity: 1.0,
            selected: false,
            hovered: false,
            outline: false,
            glow: false,
            glow_color: None,
            outline_sel: None,
            pick_id: None,
            overlay_rect_group: None,
        });
        cursor_x += max_tile;
        if invalid {
            cursor_x += inter_tile_gap;
        } else if meld_breaks.get(tile_i).copied().unwrap_or(false) {
            cursor_x += meld_gap;
        }
    }

    let label = TilesExampleLabel {
        title_rect: [cx + pad, cy + pad, cw - pad * 2.0, title_h],
        title: group.label,
        subtitle: if cell_layout.compact_headers {
            None
        } else {
            group.subtitle
        },
    };

    let tiles_bottom = tile_center_y + max_tile * 0.5;
    (placements, Some(label), tiles_bottom)
}

fn push_tiles_example_panels(
    frame: &mut UiFrame,
    groups: &[TileGroup],
    panels: &[(usize, [f32; 4])],
) {
    for &(gi, rect) in panels {
        let Some(group) = groups.get(gi) else {
            continue;
        };
        let fill = color::alpha(group.accent, 0.10);
        let stroke = color::alpha(group.accent, 0.45);
        frame.quad(GpuInstance {
            rect,
            color: fill,
            user: 0,
        });
        push_guide_panel_stroke(frame, rect, stroke);
    }
}

fn push_tiles_example_labels(
    frame: &mut UiFrame,
    _groups: &[TileGroup],
    labels: &[TilesExampleLabel],
    h: f32,
    _scale: f32,
) {
    let title_font = typography::size(typography::H28, h);
    let sub_font = typography::size(typography::H45, h);
    for lbl in labels {
        let title_color = color_for_token(lbl.title, color::CHAMPAGNE, GlossaryMode::Prose);
        frame.text(TextLabel {
            rect: lbl.title_rect,
            text: lbl.title.into(),
            color: title_color,
            align: TextAlign::Left,
            font_px: Some(title_font),
            ..Default::default()
        });
        if let Some(sub) = lbl.subtitle {
            let sub_y = lbl.title_rect[1] + title_font * 1.05;
            let mut labels = Vec::new();
            styled_text::push_colored_line_left(
                &mut labels,
                lbl.title_rect[0],
                sub_y,
                lbl.title_rect[2],
                sub_font * 1.02,
                sub,
                color::PARCHMENT,
                GlossaryMode::Prose,
            );
            for label in labels {
                frame.text(label);
            }
        }
    }
}

fn push_tile_group_labels(
    frame: &mut UiFrame,
    labels: &[MeldLabel],
    h: f32,
    scale: f32,
    wrap_long_labels: bool,
) {
    let label_font = typography::size(typography::H42, h);
    let default = color::PARCHMENT;
    for ml in labels {
        let underline_h = (3.0 * scale).max(2.0);
        frame.quad(GpuInstance {
            rect: [ml.x, ml.underline_y, ml.w, underline_h],
            color: ml.color,
            user: 0,
        });
        let mut text_labels = Vec::new();
        if wrap_long_labels {
            let wrapped = styled_text::wrap_colored_text_multiline(
                &ml.text,
                ml.w,
                label_font / 0.99,
                default,
                false,
                GlossaryMode::Prose,
            );
            styled_text::push_colored_rows_in_width(
                &mut text_labels,
                styled_text::ColoredRowsLayout {
                    text_left: ml.x,
                    top_y: ml.y,
                    inner_w: ml.w,
                    line_h: label_font,
                    fallback_plain: &ml.text,
                    fallback_color: default,
                    italic: false,
                    glossary: GlossaryMode::Prose,
                },
                &wrapped,
                TextAlign::Center,
            );
        } else {
            styled_text::push_colored_line_clipped(
                &mut text_labels,
                [ml.x, ml.y, ml.w, label_font * 1.4],
                None,
                &ml.text,
                default,
                label_font,
                TextAlign::Center,
                false,
                GlossaryMode::Prose,
            );
        }
        for label in text_labels {
            frame.text(label);
        }
    }
}

// ── Tile layout ───────────────────────────────────────────────────────────

fn layout_tile_groups_with_max(
    cam: &CameraParams,
    groups: &[&TileGroup],
    window_w: f32,
    window_h: f32,
    center_y: f32,
    x_span: Option<[f32; 2]>,
    max_tile: f32,
    width_fill: f32,
) -> (Vec<ShowcaseTilePlacement>, Vec<MeldLabel>) {
    if groups.is_empty() {
        return (vec![], vec![]);
    }

    let total_tiles: usize = groups.iter().map(|g| g.tiles.len()).sum();
    let num_gaps = groups.len().saturating_sub(1);

    let (layout_w, layout_origin) = match x_span {
        Some([x0, x1]) => (x1 - x0, x0),
        None => (window_w, 0.0),
    };

    // Compute tile size to fill `width_fill` of the layout span, capped for readability.
    let gap_equiv = num_gaps as f32 * 0.6; // gap = 0.6 tile widths
    let tile_size = ((layout_w * width_fill) / (total_tiles as f32 + gap_equiv))
        .min(max_tile)
        .max(30.0);
    let gap = tile_size * 0.6;

    let total_w = total_tiles as f32 * tile_size + num_gaps as f32 * gap;
    let start_x = layout_origin + (layout_w - total_w) * 0.5;

    let scale = (window_w.min(window_h)) / 600.0;
    let label_gaps = ShowcaseTileLabelGaps {
        underline_gap: (8.0 * scale).max(5.0),
        underline_h: (3.0 * scale).max(2.0),
        label_text_gap: (5.0 * scale).max(3.0),
    };

    let mut placements = Vec::with_capacity(total_tiles);
    let mut labels = Vec::new();
    let mut cursor_x = start_x;

    for group in groups {
        let group_start_x = cursor_x;
        let mut centers_xy = Vec::with_capacity(group.tiles.len());

        for tile in &group.tiles {
            let px = cursor_x + tile_size * 0.5;
            centers_xy.push([px, center_y]);
            placements.push(ShowcaseTilePlacement {
                tile: *tile,
                center_pos: [px, center_y, 0.0],
                rotation: GUIDE_TILE_ROTATION,
                scale: 1.0,
                size_px: tile_size,
                brightness: 1.0,
                opacity: 1.0,
                selected: false,
                hovered: false,
                outline: false,
                glow: false,
                glow_color: None,
                    outline_sel: None,
                pick_id: None,
                overlay_rect_group: None,
            });
            cursor_x += tile_size;
        }

        let group_w = cursor_x - group_start_x;
        let bounds = showcase_tile_merge_projected_group(
            cam,
            window_w,
            window_h,
            TilePreset::Chinese,
            GUIDE_TILE_ROTATION,
            1.0,
            tile_size,
            0.0,
            &centers_xy,
        );
        let anchor = showcase_tile_group_label_anchor(bounds, label_gaps);
        labels.push(MeldLabel {
            x: group_start_x,
            y: anchor.label_y,
            w: group_w,
            underline_y: anchor.underline_y,
            text: group.label.to_string(),
            color: group.accent,
        });

        cursor_x += gap;
    }

    (placements, labels)
}

/// Short structure-shape description for each yaku
pub(crate) fn yaku_shape_text(yk: YakuKind) -> &'static str {
    match yk {
        YakuKind::Tanyao => "All tiles 2\u{2013}8, no honors/terminals",
        YakuKind::Toitoi => "All triplets/kongs, no sequences",
        YakuKind::FullHand => "Complete 14-tile hand: 4+4+4+4+2 (4 melds + 1 pair)",
        YakuKind::Yakuhai => "Triplet of any dragon or round wind",
        YakuKind::Iipeikou => "Two identical sequences on a full hand",
        YakuKind::Junchan => "All 1/9/honors; each meld has a terminal or honor",
        YakuKind::SanshokuDoujun => "Same sequence in all 3 suits",
        YakuKind::Ittsu => "1\u{2013}9 straight in one suit",
        YakuKind::Honitsu => "One number suit + honors only",
        YakuKind::Chinitsu => "All one number suit, no honors",
        YakuKind::Honroutou => "Only 1s, 9s, and honors",
        YakuKind::Chiitoitsu => "Seven distinct pairs",
        YakuKind::KokushiMusou => {
            "One of each 1/9 and honor, plus one extra copy of any of those tiles"
        }
        YakuKind::ChickenHand => "Legal hand with no yaku",
        YakuKind::Chanta => "Every meld has a terminal or honor; pair may be simple",
        YakuKind::Ryanpeikou => "Two duplicated sequences in one suit on a full hand",
        YakuKind::SanshokuDoukou => "Same-rank triplet in all three number suits",
        YakuKind::Pinfu => "Four sequences + 2–8 pair on a full hand",
    }
}

#[cfg(test)]
mod tests {
    use crate::core::hand::validate_selection;
    use crate::core::tile::{Suit, Tile};
    use crate::core::yaku::{YakuKind, detect_yaku_with_wind};
    use crate::scenes::guide::{example_structure_yaku, yaku_page};

    fn tile(suit: Suit, rank: u8, id: u32) -> Tile {
        Tile::new(suit, rank, id)
    }

    /// Yaku intro page example hands must decompose and score at least one yaku
    /// so the bone tablets under each structure are non-empty.
    #[test]
    fn yaku_intro_examples_score_yaku_for_tablets() {
        let full_hand = vec![
            tile(Suit::Manzu, 2, 20),
            tile(Suit::Manzu, 3, 21),
            tile(Suit::Manzu, 4, 22),
            tile(Suit::Souzu, 5, 23),
            tile(Suit::Souzu, 6, 24),
            tile(Suit::Souzu, 7, 25),
            tile(Suit::Pinzu, 8, 26),
            tile(Suit::Pinzu, 8, 27),
            tile(Suit::Pinzu, 8, 28),
            tile(Suit::Pinzu, 3, 29),
            tile(Suit::Pinzu, 3, 30),
            tile(Suit::Pinzu, 3, 31),
            tile(Suit::Wind, 2, 32),
            tile(Suit::Wind, 2, 33),
        ];
        let with_kong = vec![
            tile(Suit::Manzu, 1, 40),
            tile(Suit::Manzu, 1, 41),
            tile(Suit::Manzu, 1, 42),
            tile(Suit::Manzu, 1, 43),
            tile(Suit::Souzu, 4, 44),
            tile(Suit::Souzu, 5, 45),
            tile(Suit::Souzu, 6, 46),
            tile(Suit::Pinzu, 7, 47),
            tile(Suit::Pinzu, 8, 48),
            tile(Suit::Pinzu, 9, 49),
            tile(Suit::Dragon, 1, 50),
            tile(Suit::Dragon, 1, 51),
            tile(Suit::Dragon, 1, 52),
            tile(Suit::Wind, 2, 53),
            tile(Suit::Wind, 2, 54),
        ];
        let chinitsu = vec![
            tile(Suit::Souzu, 2, 60),
            tile(Suit::Souzu, 3, 61),
            tile(Suit::Souzu, 4, 62),
            tile(Suit::Souzu, 4, 63),
            tile(Suit::Souzu, 5, 64),
            tile(Suit::Souzu, 6, 65),
            tile(Suit::Souzu, 6, 66),
            tile(Suit::Souzu, 7, 67),
            tile(Suit::Souzu, 8, 68),
            tile(Suit::Souzu, 5, 69),
            tile(Suit::Souzu, 5, 70),
            tile(Suit::Souzu, 5, 71),
        ];
        for (name, hand) in [
            ("full hand", &full_hand),
            ("with kong", &with_kong),
            ("chinitsu", &chinitsu),
        ] {
            assert!(
                validate_selection(hand).is_some(),
                "{name}: example hand must decompose"
            );
            let yaku = example_structure_yaku(hand);
            assert!(
                !yaku.is_empty(),
                "{name}: expected at least one yaku tablet, got none"
            );
        }
        assert!(example_structure_yaku(&full_hand).contains(&YakuKind::FullHand));
        assert!(example_structure_yaku(&with_kong).contains(&YakuKind::Yakuhai));
        assert!(example_structure_yaku(&chinitsu).contains(&YakuKind::Chinitsu));
        assert!(example_structure_yaku(&chinitsu).contains(&YakuKind::Tanyao));
        assert!(!example_structure_yaku(&chinitsu).contains(&YakuKind::FullHand));
    }

    /// Every `yaku_page()` canonical hand must actually score as its named
    /// yaku in the real detector. The yaku journal draws these hands as
    /// teaching examples — if one drifts out of sync with the scorer we'd
    /// be teaching a lie, so the test locks the data to the implementation.
    ///
    /// Chicken Hand is skipped: the canonical example is a partial hand for
    /// display; full-run chicken is injected at structure cash-in when no
    /// unlocked yaku applies.
    #[test]
    fn every_yaku_page_hand_scores_as_its_yaku() {
        for &yk in YakuKind::all() {
            if yk == YakuKind::ChickenHand {
                continue;
            }
            let (_desc, groups) = yaku_page(yk);
            let tiles: Vec<_> = groups
                .iter()
                .flat_map(|g| g.tiles.iter().copied())
                .collect();
            let sets = validate_selection(&tiles).unwrap_or_else(|| {
                panic!(
                    "{:?}: yaku_page hand failed to decompose into sets: {:?}",
                    yk, tiles
                )
            });
            // Yakuhai needs a round wind hint for wind triplets to count.
            // The Yakuhai example in yaku_page uses a dragon triplet, which
            // counts regardless, so round_wind=None is still correct.
            let detected = detect_yaku_with_wind(&tiles, &sets, None, None, None);
            assert!(
                detected.contains(&yk),
                "{:?}: canonical hand did not score as {:?}. detected={:?}",
                yk,
                yk,
                detected,
            );
        }
    }
}
