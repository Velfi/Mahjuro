//! Guide — dense in-game reference for tiles, melds, flowers, scoring, and yaku.
//!
//! Paginated 3D-tile diagrams with glossary-style definitions. Scoring basics
//! on page 4; yaku detail pages follow. Does not teach run flow, shop, relics,
//! bosses, or zodiac leveling beyond scoring references.
//!
//! Opened from the gameplay-table guide book, the in-run `Help` shortcut
//! (keyboard or controller Select / View / −), the tutorial summary, or
//! shop help. The previous scene is suspended by `App` and restored when
//! the player presses Back.

use crate::sfx_id::SfxId;
use crate::core::hand::MeldKind;
use crate::core::progression::PlayerProgress;
use crate::core::tile::{Suit, Tile};
use crate::core::yaku::YakuKind;
use crate::game::event_bus::GameEvent;
use crate::persistence::TilePreset;
use crate::render::decal::load_ui_font;
use crate::render::draw_cmd::{CameraParams, DrawCmd, ShowcaseTilePlacement, UiFrame};
use crate::render::showcase_tile_layout::{
    ShowcaseTileLabelGaps, showcase_tile_group_label_anchor, showcase_tile_merge_projected_group,
};
use crate::render::theme::{ButtonState, ButtonVariant, color, typography};
use crate::render::wgpu_renderer::{GpuInstance, PointLight, TextAlign, TextLabel};
use crate::ui::colored_keywords;
use crate::ui::controller_hints::{
    HintStyle, guide_footer_row, push_screen_footer_hint, screen_footer_reserve,
};
use crate::ui::focus_nav;
use crate::ui::input::UiAction;
use crate::ui::widget::{self, wrap_text};
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use super::flowers_intro_copy;
use super::melds_intro_copy;
use super::scoring_intro_copy;
use super::tiles_intro_copy;
use super::{BackgroundId, DrawCtx, SceneBehavior, SceneTransition, UpdateCtx};

// ── Page indices ──────────────────────────────────────────────────────────
//
// Four fixed reference pages, then yaku pages from `PlayerProgress::available_yaku`
// (sorted lowest payout first; Kokushi Musō omitted until first cash-in).

const PAGE_TILES: usize = 0;
const PAGE_MELDS: usize = 1;
const PAGE_FLOWERS: usize = 2;
const PAGE_SCORING: usize = 3;
const YAKU_PAGE_START: usize = 4;
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
        }
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
                None
            }
            Some(GuideNav::Prev) => {
                ctx.bus.push(GameEvent::UiSound(SfxId::InvalidAction));
                None
            }
            Some(GuideNav::Next) if self.page + 1 < pages => {
                ctx.bus.push(GameEvent::UiSound(SfxId::TileClick));
                self.page += 1;
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

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
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
        let (title, groups) = page_content(self.page, progress);
        let content_top = push_guide_chrome(&mut frame, &layout);
        push_guide_header_nav(
            &mut frame, &layout, &self.tree, self.page, pages, scale, w, h,
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
        } else if let Some(chunk) = yaku_chunk_for_page(self.page, progress) {
            let body_top = push_page_title(&mut frame, &layout, content_top, title, h);
            draw_yaku_guide_page(
                &mut frame,
                progress,
                w,
                h,
                scale,
                &chunk,
                body_top,
                content_floor,
                &cam,
            );
        }

        push_screen_footer_hint(
            &mut frame,
            &ctx,
            guide_footer_row(ctx.input_mode),
            HintStyle::standard(h),
        );

        frame.window_title = "Mahjuro \u{2014} Guide".into();
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

/// Convenience tile constructor.
fn t(suit: Suit, rank: u8, id: u32) -> Tile {
    Tile::new(suit, rank, id)
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
                    vec![
                        t(Suit::Manzu, 1, 0),
                        t(Suit::Manzu, 5, 1),
                        t(Suit::Manzu, 9, 2),
                    ],
                    Suit::Manzu.keyword_color(),
                ),
                tile_group_with_subtitle(
                    "Souzu",
                    "ranks 1–9",
                    vec![
                        t(Suit::Souzu, 1, 3),
                        t(Suit::Souzu, 5, 4),
                        t(Suit::Souzu, 9, 5),
                    ],
                    Suit::Souzu.keyword_color(),
                ),
                tile_group_with_subtitle(
                    "Pinzu",
                    "ranks 1–9",
                    vec![
                        t(Suit::Pinzu, 1, 6),
                        t(Suit::Pinzu, 5, 7),
                        t(Suit::Pinzu, 9, 8),
                    ],
                    Suit::Pinzu.keyword_color(),
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
                tile_group(
                    "Winds",
                    vec![
                        t(Suit::Wind, 1, 9),
                        t(Suit::Wind, 2, 10),
                        t(Suit::Wind, 3, 11),
                        t(Suit::Wind, 4, 12),
                    ],
                    Suit::Wind.keyword_color(),
                ),
                tile_group(
                    "Dragons",
                    vec![
                        t(Suit::Dragon, 1, 13),
                        t(Suit::Dragon, 2, 14),
                        t(Suit::Dragon, 3, 15),
                    ],
                    Suit::Dragon.keyword_color(),
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
            ],
        ),
        PAGE_FLOWERS => {
            let flower_accent: [f32; 4] = [0.85, 0.55, 0.70, 0.9];
            (
                flowers_intro_copy::PAGE_TITLE,
                vec![
                    tile_group_with_subtitle(
                        "Triplet",
                        "7 · 7 · Flower",
                        vec![
                            t(Suit::Pinzu, 7, 0),
                            t(Suit::Pinzu, 7, 1),
                            t(Suit::Flower, 2, 2),
                        ],
                        flower_accent,
                    ),
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
                        "Pair",
                        "Two Flowers",
                        vec![t(Suit::Flower, 1, 6), t(Suit::Flower, 2, 7)],
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
                ],
            )
        }
        PAGE_SCORING => {
            let seq_color: [f32; 4] = [0.35, 0.70, 0.85, 0.9];
            let trip_color: [f32; 4] = color::GOLD;
            (
                scoring_intro_copy::PAGE_TITLE,
                vec![
                    tile_group_with_subtitle(
                        "Hand",
                        "Play melds",
                        vec![
                            t(Suit::Pinzu, 5, 0),
                            t(Suit::Pinzu, 6, 1),
                            t(Suit::Pinzu, 7, 2),
                        ],
                        Suit::Pinzu.keyword_color(),
                    ),
                    tile_group_with_subtitle(
                        "Sequence",
                        "In Structure",
                        vec![
                            t(Suit::Manzu, 3, 3),
                            t(Suit::Manzu, 4, 4),
                            t(Suit::Manzu, 5, 5),
                        ],
                        seq_color,
                    ),
                    tile_group_with_subtitle(
                        "Triplet",
                        "In Structure",
                        vec![
                            t(Suit::Souzu, 7, 6),
                            t(Suit::Souzu, 7, 7),
                            t(Suit::Souzu, 7, 8),
                        ],
                        trip_color,
                    ),
                    tile_group_with_subtitle(
                        "5 pinzu",
                        "+5 chips",
                        vec![t(Suit::Pinzu, 5, 9)],
                        Suit::Pinzu.keyword_color(),
                    ),
                    tile_group_with_subtitle(
                        "Red dragon",
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
                        tile_group_with_subtitle("Debuffed", "+0 chips", vec![tile], color::STONE)
                    },
                ],
            )
        }
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
    margin: f32,
    content_x: f32,
    content_w: f32,
    content_top: f32,
    content_bottom: f32,
    header_btn_h: f32,
}

struct GuideHeaderChrome {
    back: [f32; 4],
    prev: [f32; 4],
    next: [f32; 4],
    page_counter: [f32; 4],
}

impl GuideLayout {
    fn new(w: f32, h: f32) -> Self {
        let ui = (w / 1920.0).min(h / 1080.0).clamp(0.55, 1.35);
        let margin = 48.0 * ui;
        let header_btn_h = (h * 0.052).clamp(44.0, 72.0);
        let content_top = margin + header_btn_h + 12.0 * ui;
        let content_bottom = h - screen_footer_reserve(h) - 12.0 * ui;
        Self {
            window_w: w,
            margin,
            content_x: margin,
            content_w: w - margin * 2.0,
            content_top,
            content_bottom,
            header_btn_h,
        }
    }

    fn header_chrome(&self) -> GuideHeaderChrome {
        let btn_h = self.header_btn_h;
        let btn_gap = 10.0 * (self.margin / 48.0);
        let row_y = self.margin;

        let back_w = (108.0 * (self.margin / 48.0)).clamp(88.0, 132.0);
        let back = [self.content_x, row_y, back_w, btn_h];

        let arrow_w = btn_h * 1.12;
        let right_edge = self.window_w - self.margin;
        let next = [right_edge - arrow_w, row_y, arrow_w, btn_h];

        let counter_w = (96.0 * (self.margin / 48.0)).clamp(80.0, 120.0);
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

/// Header rule below nav. Returns top of the content band.
fn push_guide_chrome(frame: &mut UiFrame, layout: &GuideLayout) -> f32 {
    let rule_y = layout.content_top - 2.0;
    frame.quad(GpuInstance {
        rect: [layout.content_x, rule_y, layout.content_w, 1.0],
        color: color::alpha(color::STONE, 0.45),
        user: 0,
    });

    layout.content_top + 4.0
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
    let counter_font = typography::size(typography::H42, h);
    frame.text(TextLabel {
        rect: chrome.page_counter,
        text: format!("{} / {}", page + 1, pages),
        color: color::UMBER,
        align: TextAlign::Center,
        font_px: Some(counter_font),
        bold: true,
        ..Default::default()
    });
    junk_buttons.clear();
    tree.register_flat_buttons(&items, &mut frame.buttons);
}

/// Page title anchored at the top of the content band (left-aligned).
fn push_page_title(
    frame: &mut UiFrame,
    layout: &GuideLayout,
    y: f32,
    title: &str,
    window_h: f32,
) -> f32 {
    let title_font = typography::size(typography::H20, window_h);
    let title_h = title_font * 1.32;
    frame.text(TextLabel {
        rect: [layout.content_x, y, layout.content_w, title_h],
        text: title.into(),
        color: color::CHAMPAGNE,
        align: TextAlign::Left,
        font_px: Some(title_font),
        ..Default::default()
    });
    y + title_h
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
        frame.cmds.push(DrawCmd::ShowcaseTileBatch(placements));
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
    let wrapped =
        colored_keywords::wrap_colored_text_multiline(text, rect[2], font_px / 0.99, color);
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
            frame.text(TextLabel {
                rect: [cx, line_y, piece_w, line_h],
                text: s.clone(),
                color: *c,
                font_px: Some(font_px),
                align: TextAlign::Left,
                ..Default::default()
            });
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

// ── Scoring page (page 3) ─────────────────────────────────────────────────

const SCORING_LOOP_HAND: usize = 0;
const SCORING_LOOP_STRUCTURE: &[usize] = &[1, 2];
const SCORING_CHIP_GROUPS: &[usize] = &[3, 4, 5, 6];

fn draw_scoring_page(
    frame: &mut UiFrame,
    layout: &GuideLayout,
    _progress: &PlayerProgress,
    _w: f32,
    h: f32,
    _scale: f32,
    groups: &[TileGroup],
    _cam: &CameraParams,
    content_top: f32,
    content_floor: f32,
) {
    let line_mul = 1.04;
    let gap = 4.0;
    let pad = 6.0;
    let tile_lg = (h * 0.164).clamp(112.0, 176.0);
    let body_font = typography::size(typography::H36, h);
    let tiles_font = typography::size(typography::H28, h);
    let section_font = typography::size(typography::H32, h);
    let small_font = typography::size(typography::H42, h);

    let summary_h = (body_font * line_mul + pad * 1.4).max(h * 0.040);
    let body_bottom = content_floor - summary_h - gap;

    let mut y = push_page_title(
        frame,
        layout,
        content_top,
        scoring_intro_copy::PAGE_TITLE,
        h,
    );
    y += push_dense_text_lines(
        frame,
        [layout.content_x, y, layout.content_w, 0.0],
        scoring_intro_copy::SUBTITLE,
        small_font,
        color::alpha(color::BRASS, 0.92),
        line_mul,
    ) + gap * 0.25;

    let usable = (body_bottom - y).max(1.0);
    let gaps_total = gap * 2.0;
    let content_h = (usable - gaps_total).max(1.0);
    let loop_h = content_h * 0.30;
    let tiles_h = content_h * 0.42;
    let bottom_h = content_h - loop_h - tiles_h;
    let x = layout.content_x;
    let full_w = layout.content_w;

    // ── 1. The loop ─────────────────────────────────────────────────────
    let loop_content = scoring_panel_open(
        frame,
        [x, y, full_w, loop_h],
        scoring_intro_copy::SECTION_LOOP,
        section_font,
        ScoringPanelStyle::Diagram,
    );
    push_scoring_loop_flow(
        frame,
        groups,
        loop_content,
        h,
        tile_lg,
        body_font,
        line_mul,
        pad,
    );
    y += loop_h + gap;

    // ── 2. Tiles & chips ────────────────────────────────────────────────
    let tiles_content = scoring_panel_open(
        frame,
        [x, y, full_w, tiles_h],
        scoring_intro_copy::SECTION_TILES,
        section_font,
        ScoringPanelStyle::Cards,
    );
    push_scoring_chip_cards(
        frame,
        groups,
        tiles_content,
        tile_lg,
        tiles_font,
        line_mul,
        pad,
    );
    y += tiles_h + gap;

    // ── 3. Yaku | 4. Your score ─────────────────────────────────────────
    let yaku_w = full_w * 0.42;
    let score_w = full_w - yaku_w - gap;
    push_scoring_yaku_panel(
        frame,
        [x, y, yaku_w, bottom_h],
        section_font,
        body_font,
        line_mul,
        pad,
    );
    push_scoring_final_panel(
        frame,
        [x + yaku_w + gap, y, score_w, bottom_h],
        h,
        section_font,
        body_font,
        pad,
    );
}

#[derive(Clone, Copy)]
enum ScoringPanelStyle {
    Diagram,
    Cards,
    Ledger,
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
    let inset = 6.0;
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
        ScoringPanelStyle::Ledger => (
            color::alpha(color::WALNUT_DEEP, 0.88),
            color::alpha(color::BRASS, 0.48),
            color::alpha(color::WALNUT_RAISED, 0.92),
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
        y + header_h + 4.0,
        w - inset * 2.0,
        (h - header_h - 6.0).max(1.0),
    ]
}

fn push_scoring_panel_stroke(
    frame: &mut UiFrame,
    rect: [f32; 4],
    stroke: [f32; 4],
    style: ScoringPanelStyle,
) {
    push_guide_panel_stroke(frame, rect, stroke);
    match style {
        ScoringPanelStyle::Formula => {
            let [x, y, w, h] = rect;
            let inset = 2.0;
            push_guide_panel_stroke(
                frame,
                [x + inset, y + inset, w - inset * 2.0, h - inset * 2.0],
                color::alpha(color::GOLD, 0.22),
            );
        }
        ScoringPanelStyle::Cards | ScoringPanelStyle::Ledger | ScoringPanelStyle::Diagram => {}
    }
}

fn push_scoring_loop_flow(
    frame: &mut UiFrame,
    groups: &[TileGroup],
    content: [f32; 4],
    window_h: f32,
    tile_size: f32,
    body_font: f32,
    line_mul: f32,
    pad: f32,
) {
    let [cx, cy, cw, ch] = content;
    let caption_h = push_dense_text_lines_aligned(
        frame,
        [cx, cy, cw, 0.0],
        scoring_intro_copy::LOOP_CAPTION,
        body_font,
        color::PARCHMENT,
        line_mul * 1.15,
        TextAlign::Center,
    );

    let diagram_top = cy + caption_h + 2.0;
    let diagram_h = (cy + ch - diagram_top).max(1.0);
    let arrow_w = body_font * 1.05;
    let lane_gap = pad * 0.25;
    let lane_w = (cw - lane_gap * 2.0 - arrow_w * 2.0) / 3.0;
    let mut placements = Vec::new();
    let label_h = body_font * line_mul;
    let tile_band_h = (diagram_h - label_h - 2.0).max(1.0);

    let lanes = [
        ("Play melds", false),
        ("Build a structure", false),
        ("Cash In", true),
    ];
    for (i, (label, is_payoff)) in lanes.iter().enumerate() {
        let lane_x = cx + i as f32 * (lane_w + lane_gap + arrow_w);
        let lane = [lane_x, diagram_top, lane_w, diagram_h];
        if *is_payoff {
            push_scoring_panel_background(
                frame,
                lane,
                color::alpha(color::GOLD, 0.12),
                color::alpha(color::GOLD, 0.55),
            );
        }
        if *is_payoff {
            frame.text(TextLabel {
                rect: [lane[0], lane[1], lane[2], label_h],
                text: (*label).into(),
                color: color::GOLD,
                align: TextAlign::Center,
                font_px: Some(body_font * 0.94),
                bold: true,
                ..Default::default()
            });
        } else {
            let _ = push_dense_text_lines_aligned(
                frame,
                [lane[0], lane[1], lane[2], 0.0],
                label,
                body_font * 0.94,
                color::CHAMPAGNE,
                label_h / (body_font * 0.94),
                TextAlign::Center,
            );
        }
        let tile_cell = [lane[0], lane[1] + label_h, lane[2], tile_band_h];
        let stage_tile = tile_size.min(tile_band_h * 0.92).min(tile_cell[2] * 0.90);
        match i {
            0 => {
                placements.extend(layout_scoring_group_tiles(
                    groups,
                    SCORING_LOOP_HAND,
                    tile_cell,
                    stage_tile,
                    0.55,
                    false,
                ));
            }
            1 => {
                placements.extend(layout_scoring_groups_row(
                    groups,
                    SCORING_LOOP_STRUCTURE,
                    tile_cell,
                    stage_tile,
                ));
            }
            2 => {
                frame.text(TextLabel {
                    rect: [
                        tile_cell[0],
                        tile_cell[1] + tile_cell[3] * 0.30,
                        tile_cell[2],
                        tile_cell[3] * 0.42,
                    ],
                    text: "score!".into(),
                    color: color::CHAMPAGNE,
                    align: TextAlign::Center,
                    font_px: Some(typography::size(typography::H24, window_h)),
                    bold: true,
                    ..Default::default()
                });
            }
            _ => {}
        }
        if i < 2 {
            let arrow_x = lane_x + lane_w + (lane_gap + arrow_w) * 0.5 - arrow_w * 0.5;
            frame.text(TextLabel {
                rect: [
                    arrow_x,
                    diagram_top + diagram_h * 0.40,
                    arrow_w,
                    diagram_h * 0.22,
                ],
                text: scoring_intro_copy::LOOP_ARROW.into(),
                color: color::alpha(color::GOLD, 0.90),
                align: TextAlign::Center,
                font_px: Some(body_font * 1.08),
                bold: true,
                ..Default::default()
            });
        }
    }
    if !placements.is_empty() {
        frame.cmds.push(DrawCmd::ShowcaseTileBatch(placements));
    }
}

fn push_scoring_chip_cards(
    frame: &mut UiFrame,
    groups: &[TileGroup],
    content: [f32; 4],
    tile_size: f32,
    text_font: f32,
    line_mul: f32,
    pad: f32,
) {
    let [cx, cy, cw, ch] = content;
    let intro_h = push_dense_text_lines_aligned(
        frame,
        [cx, cy, cw, 0.0],
        scoring_intro_copy::TILES_INTRO,
        text_font,
        color::PARCHMENT,
        line_mul * 1.05,
        TextAlign::Center,
    );

    let row_inset = (tile_size * 0.08).max(12.0);
    let ex_x = cx + row_inset;
    let ex_w = (cw - row_inset * 2.0).max(1.0);
    let ex_y = cy + intro_h + pad * 0.35;
    let ex_h = (cy + ch - ex_y).max(1.0);
    let n = SCORING_CHIP_GROUPS.len().max(1);
    let col_gap = pad * 0.5;
    let cell_w = (ex_w - col_gap * (n.saturating_sub(1)) as f32) / n as f32;
    let label_font = text_font * 1.22;
    let chips_font = text_font * 1.12;
    let label_line_h = label_font * line_mul;
    let chips_line_h = chips_font * line_mul;
    let label_chips_gap = label_font * line_mul * 0.14;
    let mut placements = Vec::new();

    for (i, &gi) in SCORING_CHIP_GROUPS.iter().enumerate() {
        let cell_x = ex_x + i as f32 * (cell_w + col_gap);
        let Some(group) = groups.get(gi) else {
            continue;
        };
        let tile_px = tile_size.min(ex_h * 0.88).min(cell_w * 0.50);
        // Reserve width for projected showcase mesh (face long edge + π rotation).
        let tile_col_w = tile_px * 1.42 + pad * 2.0;
        let text_gap = (tile_px * 0.05).max(8.0);
        let text_x = cell_x + tile_col_w + text_gap;
        let text_w = (cell_w - tile_col_w - text_gap).max(1.0);
        let cell_pad = if i == 0 { pad * 2.0 } else { pad * 0.5 };
        let tile_area = [
            cell_x + cell_pad,
            ex_y,
            (tile_col_w - cell_pad).max(1.0),
            ex_h,
        ];
        placements.extend(layout_scoring_group_tiles(
            groups, gi, tile_area, tile_px, 0.5, false,
        ));

        let text_block_h = match group.subtitle {
            Some(_) => label_line_h + label_chips_gap + chips_line_h,
            None => label_line_h,
        };
        let text_y = ex_y + (ex_h - text_block_h) * 0.5;
        let _ = push_dense_text_lines_aligned(
            frame,
            [text_x, text_y, text_w, 0.0],
            group.label,
            label_font,
            color::CHAMPAGNE,
            line_mul,
            TextAlign::Left,
        );
        if let Some(chips) = group.subtitle {
            let _ = push_dense_text_lines_aligned(
                frame,
                [text_x, text_y + label_line_h + label_chips_gap, text_w, 0.0],
                chips,
                chips_font,
                color::alpha(color::BRASS, 0.95),
                line_mul,
                TextAlign::Left,
            );
        }
    }
    if !placements.is_empty() {
        frame.cmds.push(DrawCmd::ShowcaseTileBatch(placements));
    }
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

fn layout_scoring_groups_row(
    groups: &[TileGroup],
    indices: &[usize],
    cell: [f32; 4],
    tile_size: f32,
) -> Vec<ShowcaseTilePlacement> {
    let [cx, cy, cw, ch] = cell;
    let n = indices.len().max(1);
    let slot_w = cw / n as f32;
    let mut out = Vec::new();
    for (i, &gi) in indices.iter().enumerate() {
        let Some(group) = groups.get(gi) else {
            continue;
        };
        let slot = [cx + slot_w * i as f32, cy, slot_w, ch];
        let n_tiles = group.tiles.len().max(1);
        let size = tile_size
            .min(slot_w / (n_tiles as f32 * 0.5 + 0.15))
            .min(ch * 0.95);
        out.extend(layout_tiles_in_cell(&group.tiles, slot, size, 0.5, false));
    }
    out
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
    let size = tile_size.min(cw / (n as f32 * 0.5 + 0.12)).min(ch * 0.94);
    let row_w = size * n as f32;
    // Left-aligned rows: offset center — 3D rotation extends past the naive half-width.
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
            selected: false,
            hovered: false,
            outline: false,
            glow: false,
            glow_color: None,
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

fn push_scoring_yaku_panel(
    frame: &mut UiFrame,
    rect: [f32; 4],
    section_font: f32,
    body_font: f32,
    line_mul: f32,
    pad: f32,
) {
    let content = scoring_panel_open(
        frame,
        rect,
        scoring_intro_copy::SECTION_YAKU,
        section_font,
        ScoringPanelStyle::Ledger,
    );
    let [x, y, w, h] = content;
    let intro_h = push_dense_text_lines(
        frame,
        [x, y, w, 0.0],
        scoring_intro_copy::YAKU_INTRO,
        body_font,
        color::PARCHMENT,
        line_mul * 1.15,
    );

    let table_top = y + intro_h + 4.0;
    let table_h = (y + h - table_top).max(1.0);
    let table_rows = scoring_intro_copy::YAKU_TABLE_ROWS.len() + 1;
    let row_h = table_h / table_rows as f32;
    let col_name_w = w * 0.44;
    let col_num_w = (w - col_name_w) * 0.5;
    let mut cursor = table_top;
    let (h0, h1, h2) = scoring_intro_copy::YAKU_TABLE_HEADER;
    for (col, text, cw) in [(0, h0, col_name_w), (1, h1, col_num_w), (2, h2, col_num_w)] {
        let ox = match col {
            0 => x,
            1 => x + col_name_w,
            _ => x + col_name_w + col_num_w,
        };
        frame.text(TextLabel {
            rect: [ox, cursor, cw, row_h],
            text: text.into(),
            color: if col == 1 || col == 2 {
                crate::render::vocabulary_colors::color_for_token(
                    text.trim_start_matches('+'),
                    color::CHAMPAGNE,
                )
            } else {
                color::CHAMPAGNE
            },
            align: if col == 0 {
                TextAlign::Left
            } else {
                TextAlign::Right
            },
            font_px: Some(body_font * 0.96),
            bold: true,
            ..Default::default()
        });
    }
    cursor += row_h;
    frame.quad(GpuInstance {
        rect: [x, cursor - 1.0, w, 1.0],
        color: color::alpha(color::BRASS, 0.45),
        user: 0,
    });

    for (name, mult, chips) in scoring_intro_copy::YAKU_TABLE_ROWS {
        frame.text(TextLabel {
            rect: [x, cursor, col_name_w, row_h],
            text: (*name).into(),
            color: color::PARCHMENT,
            align: TextAlign::Left,
            font_px: Some(body_font * 0.94),
            ..Default::default()
        });
        frame.text(TextLabel {
            rect: [x + col_name_w, cursor, col_num_w, row_h],
            text: (*mult).into(),
            color: color::keyword::MULT,
            align: TextAlign::Right,
            font_px: Some(body_font * 0.94),
            ..Default::default()
        });
        frame.text(TextLabel {
            rect: [x + col_name_w + col_num_w, cursor, col_num_w, row_h],
            text: (*chips).into(),
            color: color::keyword::CHIPS,
            align: TextAlign::Right,
            font_px: Some(body_font * 0.94),
            ..Default::default()
        });
        cursor += row_h;
    }
    let _ = pad;
}

fn push_scoring_final_panel(
    frame: &mut UiFrame,
    rect: [f32; 4],
    window_h: f32,
    section_font: f32,
    body_font: f32,
    pad: f32,
) {
    let content = scoring_panel_open(
        frame,
        rect,
        scoring_intro_copy::SECTION_SCORE,
        section_font,
        ScoringPanelStyle::Formula,
    );
    let [x, y, w, h] = content;
    let lines = [
        scoring_intro_copy::SCORE_INTRO,
        scoring_intro_copy::FINAL_EQUATION,
        scoring_intro_copy::SCORE_CHIPS_LINE,
        scoring_intro_copy::SCORE_MULT_LINE,
        scoring_intro_copy::SCORE_EXAMPLE,
    ];
    let eq_idx = 1usize;
    let row_h = h / lines.len() as f32;
    for (i, line) in lines.iter().enumerate() {
        let row_y = y + row_h * i as f32;
        let is_eq = i == eq_idx;
        if is_eq {
            push_scoring_panel_background(
                frame,
                [x, row_y + 2.0, w, row_h - 4.0],
                color::alpha(color::GOLD, 0.14),
                color::alpha(color::GOLD, 0.55),
            );
        }
        let font_px = if is_eq {
            typography::size(typography::H24, window_h).max(body_font * 1.12)
        } else {
            body_font * 0.96
        };
        let line_color = if is_eq {
            color::GOLD
        } else if i == lines.len() - 1 {
            color::alpha(color::BRASS, 0.95)
        } else {
            color::PARCHMENT
        };
        let _ = push_dense_text_lines_aligned(
            frame,
            [x + pad, row_y, w - pad * 2.0, 0.0],
            line,
            font_px,
            line_color,
            (row_h / font_px).max(0.9),
            if is_eq {
                TextAlign::Center
            } else {
                TextAlign::Left
            },
        );
    }
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
    let body_font = typography::size(typography::H42, h);
    let line_mul = 1.12;
    let mut y = push_page_title(frame, layout, content_top, tiles_intro_copy::PAGE_TITLE, h);
    for line in [
        tiles_intro_copy::INTRO_LINE_1,
        tiles_intro_copy::INTRO_LINE_2,
    ] {
        y += push_dense_text_lines(
            frame,
            [layout.content_x, y, layout.content_w, 0.0],
            line,
            body_font,
            color::PARCHMENT,
            line_mul,
        ) + h * 0.003;
    }
    y += h * 0.008;

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
        y,
        columns_bottom,
        h,
        body_font,
        line_mul,
    );

    let (placements, labels, panels) =
        layout_tiles_page_grid(cam, groups, right_x, right_w, w, h, y, columns_bottom);
    push_tiles_example_panels(frame, groups, &panels);
    if !placements.is_empty() {
        frame.cmds.push(DrawCmd::ShowcaseTileBatch(placements));
    }
    push_tiles_example_labels(frame, groups, &labels, h, scale);
}

// ── Melds page (page 1) ───────────────────────────────────────────────────

const MELDS_EXAMPLE_ROWS: &[&[usize]] = &[&[0, 1], &[2, 3, 4], &[5], &[6]];
const MELDS_ROW_WEIGHTS: [f32; 4] = [0.22, 0.30, 0.24, 0.24];

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
    let mut y = push_page_title(frame, layout, content_top, melds_intro_copy::PAGE_TITLE, h);
    for line in [
        melds_intro_copy::INTRO_LINE_1,
        melds_intro_copy::INTRO_LINE_2,
    ] {
        y += push_dense_text_lines(
            frame,
            [layout.content_x, y, layout.content_w, 0.0],
            line,
            body_font,
            color::PARCHMENT,
            line_mul,
        ) + h * 0.003;
    }
    y += h * 0.008;

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
        y,
        columns_bottom - h * 0.14,
        h,
        body_font,
        line_mul,
    );
    if let Some(scrawl) = page_graffiti(PAGE_MELDS) {
        push_flowers_margin_scrawl(frame, left_x, left_w, columns_bottom, h, scrawl);
    }

    let (placements, labels, _panels) = layout_guide_example_grid(
        cam,
        groups,
        right_x,
        right_w,
        w,
        h,
        y,
        columns_bottom,
        MELDS_EXAMPLE_ROWS,
        &MELDS_ROW_WEIGHTS,
    );
    if !placements.is_empty() {
        frame.cmds.push(DrawCmd::ShowcaseTileBatch(placements));
    }
    push_tiles_example_labels(frame, groups, &labels, h, scale);
}

// ── Flowers page (page 2) ───────────────────────────────────────────────────

const FLOWERS_EXAMPLE_ROWS: &[&[usize]] = &[&[0, 1], &[2, 3]];
const FLOWERS_ROW_WEIGHTS: [f32; 2] = [0.50, 0.50];

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
    let mut y = push_page_title(
        frame,
        layout,
        content_top,
        flowers_intro_copy::PAGE_TITLE,
        h,
    );
    for line in [
        flowers_intro_copy::INTRO_LINE_1,
        flowers_intro_copy::INTRO_LINE_2,
    ] {
        y += push_dense_text_lines(
            frame,
            [layout.content_x, y, layout.content_w, 0.0],
            line,
            body_font,
            color::PARCHMENT,
            line_mul,
        ) + h * 0.003;
    }
    y += h * 0.008;

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
        y,
        columns_bottom,
        h,
        body_font,
        line_mul,
    );
    if let Some(scrawl) = page_graffiti(PAGE_FLOWERS) {
        push_flowers_margin_scrawl(frame, left_x, left_w, columns_bottom, h, scrawl);
    }

    let (placements, labels, _panels) = layout_guide_example_grid(
        cam,
        groups,
        right_x,
        right_w,
        w,
        h,
        y,
        columns_bottom,
        FLOWERS_EXAMPLE_ROWS,
        &FLOWERS_ROW_WEIGHTS,
    );
    if !placements.is_empty() {
        frame.cmds.push(DrawCmd::ShowcaseTileBatch(placements));
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
    let wrapped =
        colored_keywords::wrap_colored_text_multiline(text, inner_w, font / 0.99, default);
    let line_h = font;
    let block_h = colored_keywords::colored_wrapped_rows_height(&wrapped, line_h);
    let y = bottom - block_h - h * 0.008;
    let mut labels = Vec::new();
    colored_keywords::push_colored_rows_left(
        &mut labels,
        colored_keywords::ColoredRowsLayout {
            text_left: x + pad,
            top_y: y,
            inner_w,
            line_h,
            fallback_plain: text,
            fallback_color: default,
        },
        &wrapped,
    );
    for mut label in labels {
        label.italic = true;
        frame.text(label);
    }
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
    let sections: [(&str, &[&str]); 2] = [
        (
            melds_intro_copy::SECTION_STRUCTURE,
            melds_intro_copy::STRUCTURE_LINES,
        ),
        (
            melds_intro_copy::SECTION_CASH_IN,
            melds_intro_copy::CASH_IN_LINES,
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
    let body_h =
        colored_keywords::colored_lines_block_height(lines, inner_w, body_line_h, color::PARCHMENT);
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
            tiles_intro_copy::SECTION_RANK_TERMS,
            tiles_intro_copy::RANK_TERM_LINES,
        ),
        (
            tiles_intro_copy::SECTION_SEQUENCE_RULES,
            tiles_intro_copy::SEQUENCE_RULE_LINES,
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
            let line_h = colored_keywords::push_colored_line_left(
                &mut labels,
                x + pad,
                cursor,
                inner_w,
                body_line_h,
                line,
                color::PARCHMENT,
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

/// Right-column rows: suits, then valid/invalid sequences, then winds/dragons.
const TILES_EXAMPLE_ROWS: &[&[usize]] = &[&[0, 1, 2], &[3, 4], &[5, 6]];
const TILES_ROW_WEIGHTS: [f32; 3] = [0.34, 0.33, 0.33];

fn layout_tiles_page_grid(
    cam: &CameraParams,
    groups: &[TileGroup],
    col_x: f32,
    col_w: f32,
    window_w: f32,
    window_h: f32,
    top: f32,
    bottom: f32,
) -> (
    Vec<ShowcaseTilePlacement>,
    Vec<TilesExampleLabel>,
    Vec<(usize, [f32; 4])>,
) {
    layout_guide_example_grid(
        cam,
        groups,
        col_x,
        col_w,
        window_w,
        window_h,
        top,
        bottom,
        TILES_EXAMPLE_ROWS,
        &TILES_ROW_WEIGHTS,
    )
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
) -> (
    Vec<ShowcaseTilePlacement>,
    Vec<TilesExampleLabel>,
    Vec<(usize, [f32; 4])>,
) {
    let usable_h = (bottom - top).max(1.0);
    let row_gap = 3.0;
    let weight_sum: f32 = row_weights.iter().sum();
    let mut placements = Vec::new();
    let mut labels = Vec::new();
    let mut panels = Vec::new();
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
            let (p, l) = layout_tile_group_cell(cam, &groups[gi], cell, window_w, window_h);
            placements.extend(p);
            if let Some(lbl) = l {
                labels.push(lbl);
            }
            cell_x += cw + row_gap;
        }
        row_y += row_h + row_gap;
    }

    (placements, labels, panels)
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
) -> (Vec<ShowcaseTilePlacement>, Option<TilesExampleLabel>) {
    let [cx, cy, cw, ch] = cell;
    let pad = 4.0;
    let title_font = typography::size(typography::H28, window_h);
    let sub_font = typography::size(typography::H45, window_h);
    let inner_w = (cw - pad * 2.0).max(1.0);
    let title_h = title_font * 1.05;
    let sub_line_h = sub_font * 1.02;
    let sub_h = group
        .subtitle
        .map(|sub| {
            colored_keywords::colored_line_block_height(sub, inner_w, sub_line_h, color::PARCHMENT)
        })
        .unwrap_or(0.0);
    let label_tile_gap = (window_h * 0.012).clamp(8.0, 14.0);
    let tile_area_top = cy + pad + title_h + sub_h + label_tile_gap;
    let tile_area_h = (cy + ch - pad - tile_area_top).max(20.0);

    let n = group.tiles.len().max(1);
    let max_tile = (cw / (n as f32 + 0.35))
        .min(tile_area_h * 0.88)
        .min(window_h * 0.082)
        .max(24.0);
    let tile_center_y = (tile_area_top + max_tile * 0.5).min(cy + ch - pad - max_tile * 0.5);
    let group_w = max_tile * n as f32;
    let start_x = cx + (cw - group_w) * 0.5;
    let mut centers_xy = Vec::with_capacity(n);
    let mut placements = Vec::with_capacity(n);
    let mut cursor_x = start_x;

    for tile in &group.tiles {
        let px = cursor_x + max_tile * 0.5;
        centers_xy.push([px, tile_center_y]);
        placements.push(ShowcaseTilePlacement {
            tile: *tile,
            center_pos: [px, tile_center_y, 0.0],
            rotation: GUIDE_TILE_ROTATION,
            scale: 1.0,
            size_px: max_tile,
            brightness: 1.0,
            selected: false,
            hovered: false,
            outline: false,
            glow: false,
            glow_color: None,
            pick_id: None,
            overlay_rect_group: None,
        });
        cursor_x += max_tile;
    }

    let label = TilesExampleLabel {
        title_rect: [cx + pad, cy + pad, cw - pad * 2.0, title_h],
        title: group.label,
        subtitle: group.subtitle,
    };

    (placements, Some(label))
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
    use crate::render::vocabulary_colors::color_for_token;

    let title_font = typography::size(typography::H28, h);
    let sub_font = typography::size(typography::H45, h);
    for lbl in labels {
        let title_color = color_for_token(lbl.title, color::CHAMPAGNE);
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
            colored_keywords::push_colored_line_left(
                &mut labels,
                lbl.title_rect[0],
                sub_y,
                lbl.title_rect[2],
                sub_font * 1.02,
                sub,
                color::PARCHMENT,
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
            let wrapped = colored_keywords::wrap_colored_text_multiline(
                &ml.text,
                ml.w,
                label_font / 0.99,
                default,
            );
            colored_keywords::push_colored_rows_in_width(
                &mut text_labels,
                colored_keywords::ColoredRowsLayout {
                    text_left: ml.x,
                    top_y: ml.y,
                    inner_w: ml.w,
                    line_h: label_font,
                    fallback_plain: &ml.text,
                    fallback_color: default,
                },
                &wrapped,
                TextAlign::Center,
            );
        } else {
            colored_keywords::push_colored_line_clipped(
                &mut text_labels,
                [ml.x, ml.y, ml.w, label_font * 1.4],
                None,
                &ml.text,
                default,
                label_font,
                TextAlign::Center,
                false,
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
                selected: false,
                hovered: false,
                outline: false,
                glow: false,
                glow_color: None,
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
    use crate::core::yaku::{YakuKind, detect_yaku_with_wind};
    use crate::scenes::guide::yaku_page;

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
