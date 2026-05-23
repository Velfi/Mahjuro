//! Guide — visual onboarding scene teaching melds, tile categories, and yaku.
//!
//! Replaces the old text-only glossary overlay with a paginated 3D-tile diagram
//! that shows concrete tile examples: suits, basic melds, flowers, and
//! every yaku hand pattern.
//!
//! Opened from the pause menu (gameplay or shop), the tutorial summary, or
//! in-shop help. The previous scene is suspended by `App` and restored when
//! the player presses Back.

use crate::core::hand::MeldKind;
use crate::core::progression::PlayerProgress;
use crate::core::tile::{Suit, Tile};
use crate::core::yaku::YakuKind;
use crate::render::draw_cmd::{CameraParams, DrawCmd, ShowcaseTilePlacement, UiFrame};
use crate::render::theme::{color, typography};
use crate::render::wgpu_renderer::{GpuInstance, PointLight, TextAlign, TextLabel};
use crate::ui::input::UiAction;
use crate::ui::widget::wrap_text;

use super::{BackgroundId, ButtonDef, DrawCtx, SceneBehavior, SceneTransition, UpdateCtx};

// ── Page indices ──────────────────────────────────────────────────────────
//
// Intro material is split into three short pages so each topic gets room to
// breathe: tiles first, then basic melds, then a flower-focused page that
// walks through every legal way to use a flower wildcard.

const PAGE_TILES: usize = 0;
const PAGE_BASIC_MELDS: usize = 1;
const PAGE_FLOWERS: usize = 2;
const YAKU_PAGE_START: usize = 3;

fn total_pages(progress: &PlayerProgress) -> usize {
    YAKU_PAGE_START + progress.available_yaku().len()
}

// ── Button click IDs ──────────────────────────────────────────────────────

const CLICK_PREV: u32 = 0xD001;
const CLICK_NEXT: u32 = 0xD002;
const CLICK_BACK: u32 = 0xD003;

// ── Scene ─────────────────────────────────────────────────────────────────

pub struct GuideScene {
    page: usize,
}

impl GuideScene {
    pub fn new() -> Self {
        Self { page: 0 }
    }

    fn go_back(&self, overlay_request: &mut Option<super::OverlayRequest>) -> SceneTransition {
        *overlay_request = Some(super::OverlayRequest::Pop);
        None
    }
}

impl SceneBehavior for GuideScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        let pages = total_pages(ctx.progress);

        for &cid in ctx.button_clicks {
            match cid {
                CLICK_PREV => {
                    if self.page > 0 {
                        self.page -= 1;
                    }
                }
                CLICK_NEXT => {
                    if self.page + 1 < pages {
                        self.page += 1;
                    }
                }
                CLICK_BACK => {
                    return self.go_back(ctx.overlay_request);
                }
                _ => {}
            }
        }

        for a in ctx.actions {
            match a {
                UiAction::Cancel | UiAction::Pause => {
                    return self.go_back(ctx.overlay_request);
                }
                UiAction::FocusPrev | UiAction::FocusUp => {
                    if self.page > 0 {
                        self.page -= 1;
                    }
                }
                UiAction::FocusNext | UiAction::FocusDown => {
                    if self.page + 1 < pages {
                        self.page += 1;
                    }
                }
                _ => {}
            }
        }

        None
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

        // ── Page content ──────────────────────────────────────────
        let (title, description, groups) = page_content(self.page, progress);

        // Title
        let title_font = typography::size(typography::H20, h);
        let title_h = title_font * 1.6;
        let title_y = h * 0.06;
        frame.text(TextLabel {
            rect: [0.0, title_y, w, title_h],
            text: title.into(),
            color: color::CHAMPAGNE,
            align: TextAlign::Center,
            font_px: Some(title_font),
            ..Default::default()
        });

        // Body copy (structured on intro pages)
        let tile_area_y = if self.page == PAGE_TILES {
            let body_top = title_y + title_h + h * 0.008;
            push_tiles_page_text(&mut frame, w, h, body_top)
        } else if self.page == PAGE_BASIC_MELDS {
            let body_top = title_y + title_h + h * 0.008;
            push_basic_melds_intro(&mut frame, w, h, body_top)
        } else {
            let desc_font = typography::size(typography::H36, h);
            let desc_w = w * 0.8;
            let line_h = desc_font * 1.22;
            let wrapped = wrap_text(description, desc_w, desc_font / 0.99);
            let desc_h = line_h * wrapped.len().max(1) as f32;
            let desc_y = title_y + title_h + h * 0.01;
            frame.text(TextLabel {
                rect: [w * 0.1, desc_y, desc_w, desc_h],
                text: wrapped.join("\n"),
                color: color::PARCHMENT,
                align: TextAlign::Center,
                font_px: Some(desc_font),
                ..Default::default()
            });
            desc_y + desc_h + h * 0.02
        };

        // ── Tile placements ───────────────────────────────────────
        let tile_center_y = tile_area_y + (h * 0.72 - tile_area_y) * 0.30;
        if self.page == PAGE_BASIC_MELDS {
            let (placements, cards) = layout_basic_meld_cards(&groups, w, h, tile_area_y);
            if !placements.is_empty() {
                frame.cmds.push(DrawCmd::ShowcaseTileBatch(placements));
            }
            push_basic_meld_card_text(&mut frame, &cards, h, scale);
        } else {
            let (placements, labels) = if self.page == PAGE_TILES {
                layout_tiles_page_groups(&groups, w, h, tile_area_y)
            } else {
                let group_refs: Vec<&TileGroup> = groups.iter().collect();
                layout_tile_groups(&group_refs, w, h, tile_center_y)
            };

            if !placements.is_empty() {
                frame.cmds.push(DrawCmd::ShowcaseTileBatch(placements));
            }

            // Meld group labels below each group
            let label_font = typography::size(typography::H42, h);
            let label_line_h = label_font * 1.22;
            for ml in &labels {
                let underline_h = (3.0 * scale).max(2.0);
                let underline_y = ml.y - underline_h - 2.0 * scale;
                frame.quad(GpuInstance {
                    rect: [ml.x, underline_y, ml.w, underline_h],
                    color: ml.color,
                    user: 0,
                });
                let (label_text, label_h) = if self.page == PAGE_TILES {
                    let wrapped = wrap_text(&ml.text, ml.w, label_font / 0.99);
                    let lh = label_line_h * wrapped.len().max(1) as f32;
                    (wrapped.join("\n"), lh)
                } else {
                    (ml.text.clone(), label_font * 1.4)
                };
                frame.text(TextLabel {
                    rect: [ml.x, ml.y, ml.w, label_h],
                    text: label_text,
                    color: color::PARCHMENT,
                    align: TextAlign::Center,
                    font_px: Some(label_font),
                    ..Default::default()
                });
            }
        }

        // ── In-universe margin scrawl ─────────────────────────────
        if let Some(scrawl) = page_graffiti(self.page) {
            let scrawl_font = typography::size(typography::H42, h);
            let scrawl_w = w * 0.76;
            let scrawl_line_h = scrawl_font * 1.22;
            let wrapped = wrap_text(scrawl, scrawl_w, scrawl_font / 0.99);
            let scrawl_h = scrawl_line_h * wrapped.len().max(1) as f32;
            let scrawl_y = h * 0.755;
            frame.text(TextLabel {
                rect: [w * 0.12, scrawl_y, scrawl_w, scrawl_h],
                text: wrapped.join("\n"),
                color: color::STONE,
                align: TextAlign::Center,
                font_px: Some(scrawl_font),
                italic: true,
                ..Default::default()
            });
        }

        // ── Scoring info (yaku pages) ─────────────────────────────
        if self.page >= YAKU_PAGE_START {
            let yaku_idx = self.page - YAKU_PAGE_START;
            if let Some(&yk) = progress.available_yaku().get(yaku_idx) {
                let score_font = typography::size(typography::H36, h);
                let score_h = score_font * 1.5;
                let score_y = h * 0.78;
                if yaku_idx == 0 {
                    let hint_font = typography::size(typography::H42, h);
                    let hint_h = hint_font * 1.35;
                    frame.text(TextLabel {
                        rect: [w * 0.1, score_y - hint_h - h * 0.008, w * 0.8, hint_h],
                        text: "Chips raise your base score; Mult multiplies it.".into(),
                        color: color::STONE,
                        align: TextAlign::Center,
                        font_px: Some(hint_font),
                        ..Default::default()
                    });
                }
                let score_text =
                    format!("+{} mult  /  +{} chips", yk.mult_bonus(), yk.chip_bonus(),);
                frame.text(TextLabel {
                    rect: [0.0, score_y, w, score_h],
                    text: score_text,
                    color: color::GOLD,
                    align: TextAlign::Center,
                    font_px: Some(score_font),
                    ..Default::default()
                });
            }
        }

        // ── Page indicator ────────────────────────────────────────
        let pages = total_pages(progress);
        let page_font = typography::size(typography::H42, h);
        let page_h = page_font * 1.4;
        let page_y = h * 0.84;
        frame.text(TextLabel {
            rect: [0.0, page_y, w, page_h],
            text: format!("{} / {}", self.page + 1, pages),
            color: color::UMBER,
            align: TextAlign::Center,
            font_px: Some(page_font),
            ..Default::default()
        });

        // ── Navigation buttons ────────────────────────────────────
        let btn_font = typography::size(typography::H36, h);
        let btn_h = (44.0 * scale).max(32.0);
        let btn_w = (140.0 * scale).max(90.0);
        let btn_y = h * 0.89;
        let btn_gap = 16.0 * scale;
        let total_btn_w = btn_w * 3.0 + btn_gap * 2.0;
        let btn_start_x = (w - total_btn_w) * 0.5;

        // Prev
        let prev_x = btn_start_x;
        let prev_enabled = self.page > 0;
        let prev_color = if prev_enabled {
            color::WALNUT_INK
        } else {
            color::alpha(color::WALNUT_INK, 0.5)
        };
        frame.quad(GpuInstance {
            rect: [prev_x, btn_y, btn_w, btn_h],
            color: prev_color,
            user: 0,
        });
        frame.text(TextLabel {
            rect: [prev_x, btn_y, btn_w, btn_h],
            text: "< Prev".into(),
            color: if prev_enabled {
                color::CHAMPAGNE
            } else {
                color::UMBER
            },
            align: TextAlign::Center,
            font_px: Some(btn_font),
            ..Default::default()
        });
        if prev_enabled {
            frame
                .buttons
                .push(ButtonDef::scene((prev_x, btn_y, btn_w, btn_h), CLICK_PREV));
        }

        // Back
        let back_x = prev_x + btn_w + btn_gap;
        frame.quad(GpuInstance {
            rect: [back_x, btn_y, btn_w, btn_h],
            color: color::WALNUT_INK,
            user: 0,
        });
        frame.text(TextLabel {
            rect: [back_x, btn_y, btn_w, btn_h],
            text: "Back".into(),
            color: color::CHAMPAGNE,
            align: TextAlign::Center,
            font_px: Some(btn_font),
            ..Default::default()
        });
        frame
            .buttons
            .push(ButtonDef::scene((back_x, btn_y, btn_w, btn_h), CLICK_BACK));

        // Next
        let next_x = back_x + btn_w + btn_gap;
        let next_enabled = self.page + 1 < pages;
        let next_color = if next_enabled {
            color::WALNUT_INK
        } else {
            color::alpha(color::WALNUT_INK, 0.5)
        };
        frame.quad(GpuInstance {
            rect: [next_x, btn_y, btn_w, btn_h],
            color: next_color,
            user: 0,
        });
        frame.text(TextLabel {
            rect: [next_x, btn_y, btn_w, btn_h],
            text: "Next >".into(),
            color: if next_enabled {
                color::CHAMPAGNE
            } else {
                color::UMBER
            },
            align: TextAlign::Center,
            font_px: Some(btn_font),
            ..Default::default()
        });
        if next_enabled {
            frame
                .buttons
                .push(ButtonDef::scene((next_x, btn_y, btn_w, btn_h), CLICK_NEXT));
        }

        frame.window_title = "Mahjuro \u{2014} Guide".into();
        frame
    }
}

// ── Page content ──────────────────────────────────────────────────────────

/// A labelled group of tiles forming one meld (or tile-category cluster).
pub(crate) struct TileGroup {
    pub label: &'static str,
    pub tiles: Vec<Tile>,
    /// Accent color for the underline bar.
    pub accent: [f32; 4],
    /// Short definition for card-style pages (e.g. basic melds).
    pub definition: Option<&'static str>,
    /// Optional rule note below the tile example.
    pub note: Option<&'static str>,
}

fn tile_group(label: &'static str, tiles: Vec<Tile>, accent: [f32; 4]) -> TileGroup {
    TileGroup {
        label,
        tiles,
        accent,
        definition: None,
        note: None,
    }
}

fn meld_card(
    label: &'static str,
    definition: &'static str,
    note: Option<&'static str>,
    tiles: Vec<Tile>,
    accent: [f32; 4],
) -> TileGroup {
    TileGroup {
        label,
        tiles,
        accent,
        definition: Some(definition),
        note,
    }
}

/// Meld label positioned below a tile group in screen space.
struct MeldLabel {
    x: f32,
    y: f32,
    w: f32,
    text: String,
    color: [f32; 4],
}

/// Convenience tile constructor.
fn t(suit: Suit, rank: u8, id: u32) -> Tile {
    Tile::new(suit, rank, id)
}

/// Optional in-universe margin scrawl for a page. Rendered below the tile
/// area in faded italic to feel like a player's aside left on the guide.
fn page_graffiti(page: usize) -> Option<&'static str> {
    match page {
        PAGE_FLOWERS => Some(
            "scribbled in the margin: \"a flower may close a triplet, mend a sequence \u{2014} yet never weds a stranger as a pair. why?\"  \u{2014} Nicole",
        ),
        _ => None,
    }
}

/// Returns `(title, description, groups)` for the given page index.
fn page_content(
    page: usize,
    progress: &PlayerProgress,
) -> (&'static str, &'static str, Vec<TileGroup>) {
    match page {
        PAGE_TILES => (
            "The Tiles",
            "",
            vec![
                tile_group(
                    "Manzu",
                    vec![
                        t(Suit::Manzu, 1, 0),
                        t(Suit::Manzu, 5, 1),
                        t(Suit::Manzu, 9, 2),
                    ],
                    Suit::Manzu.keyword_color(),
                ),
                tile_group(
                    "Souzu",
                    vec![
                        t(Suit::Souzu, 1, 3),
                        t(Suit::Souzu, 5, 4),
                        t(Suit::Souzu, 9, 5),
                    ],
                    Suit::Souzu.keyword_color(),
                ),
                tile_group(
                    "Pinzu",
                    vec![
                        t(Suit::Pinzu, 1, 6),
                        t(Suit::Pinzu, 5, 7),
                        t(Suit::Pinzu, 9, 8),
                    ],
                    Suit::Pinzu.keyword_color(),
                ),
                tile_group(
                    "3-4-5 Manzu = valid",
                    vec![
                        t(Suit::Manzu, 3, 16),
                        t(Suit::Manzu, 4, 17),
                        t(Suit::Manzu, 5, 18),
                    ],
                    [0.35, 0.70, 0.85, 0.9],
                ),
                tile_group(
                    "3 Manzu / 4 Souzu / 5 Pinzu = invalid",
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
        PAGE_BASIC_MELDS => (
            "Basic Melds",
            "",
            vec![
                meld_card(
                    "Pair",
                    "Two identical tiles.",
                    None,
                    vec![t(Suit::Souzu, 5, 0), t(Suit::Souzu, 5, 1)],
                    color::CHAMPAGNE,
                ),
                meld_card(
                    "Sequence",
                    "Three consecutive number tiles in one suit.",
                    Some("Numbers only.\nSame suit only.\nHonors cannot sequence."),
                    vec![
                        t(Suit::Manzu, 4, 2),
                        t(Suit::Manzu, 5, 3),
                        t(Suit::Manzu, 6, 4),
                    ],
                    [0.35, 0.70, 0.85, 0.9],
                ),
                meld_card(
                    "Triplet",
                    "Three identical tiles.",
                    None,
                    vec![
                        t(Suit::Pinzu, 7, 5),
                        t(Suit::Pinzu, 7, 6),
                        t(Suit::Pinzu, 7, 7),
                    ],
                    color::GOLD,
                ),
                meld_card(
                    "Kong",
                    "Four identical tiles.",
                    None,
                    vec![
                        t(Suit::Wind, 1, 8),
                        t(Suit::Wind, 1, 9),
                        t(Suit::Wind, 1, 10),
                        t(Suit::Wind, 1, 11),
                    ],
                    [0.85, 0.65, 0.20, 0.9],
                ),
            ],
        ),
        PAGE_FLOWERS => {
            let flower_accent: [f32; 4] = [0.85, 0.55, 0.70, 0.9];
            (
                "Flowers",
                "Flowers are wildcards. One flower can stand in for the missing tile of a triplet or sequence (max one per meld). Two flowers form a pair and three form a triplet, regardless of rank. A flower cannot pair with a regular tile.",
                vec![
                    tile_group(
                        "Fills a triplet",
                        vec![
                            t(Suit::Pinzu, 7, 0),
                            t(Suit::Pinzu, 7, 1),
                            t(Suit::Flower, 2, 2),
                        ],
                        flower_accent,
                    ),
                    tile_group(
                        "Fills a sequence",
                        vec![
                            t(Suit::Manzu, 4, 3),
                            t(Suit::Flower, 3, 4),
                            t(Suit::Manzu, 6, 5),
                        ],
                        flower_accent,
                    ),
                    tile_group(
                        "Two flowers \u{2192} pair",
                        vec![t(Suit::Flower, 1, 6), t(Suit::Flower, 2, 7)],
                        flower_accent,
                    ),
                    tile_group(
                        "Three flowers \u{2192} triplet",
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
        _ => {
            // Yaku pages (Kokushi Musō page is omitted until first cash-in).
            let yaku_idx = page - YAKU_PAGE_START;
            let visible = progress.available_yaku();
            if let Some(&yk) = visible.get(yaku_idx) {
                let (desc, groups) = yaku_page(yk);
                (yk.name(), desc, groups)
            } else {
                ("", "", vec![])
            }
        }
    }
}

/// Build tile groups for a yaku example hand.
pub(crate) fn yaku_page(yk: YakuKind) -> (&'static str, Vec<TileGroup>) {
    let seq_color: [f32; 4] = [0.35, 0.70, 0.85, 0.9];
    let trip_color: [f32; 4] = color::GOLD;
    let pair_color: [f32; 4] = color::CHAMPAGNE;
    let single_color: [f32; 4] = [0.78, 0.74, 0.58, 0.9];
    let _kong_color: [f32; 4] = [0.85, 0.65, 0.20, 0.9];

    match yk {
        YakuKind::Tanyao => (
            "All tiles ranked 2\u{2013}8 \u{2014} no terminals (1/9) or honors.",
            meld_groups(&[
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
                (
                    "Pair",
                    MeldKind::Pair,
                    Suit::Manzu,
                    &[5, 5],
                    pair_color,
                ),
            ]),
        ),
        YakuKind::Toitoi => (
            "All melds are triplets or kongs \u{2014} no sequences allowed.",
            meld_groups(&[
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
        ),
        YakuKind::Honroutou => (
            "Every tile is a terminal (1 or 9) or an honor (wind/dragon).",
            meld_groups(&[
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
        ),
        YakuKind::Iipeikou => (
            "Two identical sequences in the same suit on a full 14-tile hand.",
            meld_groups(&[
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
                (
                    "Pair",
                    MeldKind::Pair,
                    Suit::Wind,
                    &[1, 1],
                    pair_color,
                ),
            ]),
        ),
        YakuKind::FullHand => (
            "Complete 14-tile hand: 4+4+4+4+2 (4 melds + 1 pair), not 2x7 seven pairs.",
            meld_groups(&[
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
        ),
        YakuKind::Chinitsu => (
            "All tiles from a single number suit, no honors.",
            meld_groups(&[
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
        ),
        YakuKind::SanshokuDoujun => (
            "Same numerical sequence in all three number suits.",
            meld_groups(&[
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
        ),
        YakuKind::Junchan => (
            "All tiles are 1, 9, or honors; every meld contains a terminal or honor.",
            meld_groups(&[
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
                (
                    "Pair",
                    MeldKind::Pair,
                    Suit::Souzu,
                    &[9, 9],
                    pair_color,
                ),
            ]),
        ),
        YakuKind::Ittsu => (
            "1\u{2013}9 straight in one suit: three sequences covering 1-2-3, 4-5-6, 7-8-9.",
            meld_groups(&[
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
                (
                    "Pair",
                    MeldKind::Pair,
                    Suit::Manzu,
                    &[5, 5],
                    pair_color,
                ),
            ]),
        ),
        YakuKind::Honitsu => (
            "One number suit plus honors only \u{2014} no other number suits.",
            meld_groups(&[
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
        ),
        YakuKind::Yakuhai => (
            "Triplet (or kong) of any dragon, or of the current round wind.",
            meld_groups(&[
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
        ),
        YakuKind::Chiitoitsu => (
            "Seven distinct pairs \u{2014} an alternate hand shape (no melds).",
            meld_groups(&[
                (
                    "Pair",
                    MeldKind::Pair,
                    Suit::Manzu,
                    &[1, 1],
                    pair_color,
                ),
                (
                    "Pair",
                    MeldKind::Pair,
                    Suit::Manzu,
                    &[3, 3],
                    pair_color,
                ),
                ("Pair", MeldKind::Pair, Suit::Souzu, &[5, 5], pair_color),
                ("Pair", MeldKind::Pair, Suit::Souzu, &[7, 7], pair_color),
                ("Pair", MeldKind::Pair, Suit::Pinzu, &[2, 2], pair_color),
                ("Pair", MeldKind::Pair, Suit::Pinzu, &[4, 4], pair_color),
                ("Pair", MeldKind::Pair, Suit::Wind, &[1, 1], pair_color),
            ]),
        ),
        YakuKind::KokushiMusou => (
            "One of each terminal and honor (13 types), plus one duplicate \u{2014} twelve singles and one pair.",
            meld_groups(&[
                (
                    "Pair",
                    MeldKind::Pair,
                    Suit::Manzu,
                    &[1, 1],
                    pair_color,
                ),
                (
                    "Single",
                    MeldKind::Single,
                    Suit::Manzu,
                    &[9],
                    single_color,
                ),
                (
                    "Single",
                    MeldKind::Single,
                    Suit::Souzu,
                    &[1],
                    single_color,
                ),
                (
                    "Single",
                    MeldKind::Single,
                    Suit::Souzu,
                    &[9],
                    single_color,
                ),
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
        ),
        YakuKind::ChickenHand => (
            "A valid hand that triggers no other yaku \u{2014} scores base chips \u{00d7} 1 mult.",
            meld_groups(&[
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
        ),
    }
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
                definition: None,
                note: None,
            }
        })
        .collect()
}

// ── Basic melds page (page 1) ─────────────────────────────────────────────

/// Two-line intro for the basic melds page.
fn push_basic_melds_intro(frame: &mut UiFrame, w: f32, h: f32, y: f32) -> f32 {
    let body_font = typography::size(typography::H36, h);
    let line_h = body_font * 1.22;
    let x = w * 0.1;
    let rw = w * 0.8;
    let lines = [
        "Melds are small tile groups.",
        "Most hands are built from these shapes.",
    ];
    let mut cursor = y;
    for line in lines {
        frame.text(TextLabel {
            rect: [x, cursor, rw, line_h],
            text: line.into(),
            color: color::PARCHMENT,
            align: TextAlign::Center,
            font_px: Some(body_font),
            ..Default::default()
        });
        cursor += line_h;
    }
    cursor + h * 0.014
}

/// Per-column copy rects for the four meld cards.
struct BasicMeldCardLayout {
    accent: [f32; 4],
    title_rect: [f32; 4],
    title: &'static str,
    def_rect: [f32; 4],
    definition: &'static str,
    note_rect: Option<[f32; 4]>,
    note: Option<&'static str>,
}

fn push_basic_meld_card_text(
    frame: &mut UiFrame,
    cards: &[BasicMeldCardLayout],
    h: f32,
    scale: f32,
) {
    let title_font = typography::size(typography::H28, h);
    let body_font = typography::size(typography::H36, h);
    let note_font = typography::size(typography::H42, h);
    let underline_h = (3.0 * scale).max(2.0);

    for card in cards {
        let [tx, ty, tw, th] = card.title_rect;
        let underline_y = ty + th - underline_h - 2.0 * scale;
        frame.quad(GpuInstance {
            rect: [tx, underline_y, tw, underline_h],
            color: card.accent,
            user: 0,
        });
        frame.text(TextLabel {
            rect: card.title_rect,
            text: card.title.into(),
            color: color::CHAMPAGNE,
            align: TextAlign::Center,
            font_px: Some(title_font),
            ..Default::default()
        });
        let def_w = card.def_rect[2];
        let def_wrapped = wrap_text(card.definition, def_w, body_font / 0.99);
        let def_line_h = body_font * 1.22;
        let def_h = def_line_h * def_wrapped.len().max(1) as f32;
        frame.text(TextLabel {
            rect: [card.def_rect[0], card.def_rect[1], def_w, def_h],
            text: def_wrapped.join("\n"),
            color: color::PARCHMENT,
            align: TextAlign::Center,
            font_px: Some(body_font),
            ..Default::default()
        });
        if let (Some(rect), Some(note)) = (card.note_rect, card.note) {
            let note_w = rect[2];
            let note_wrapped = wrap_text(note, note_w, note_font / 0.99);
            let note_line_h = note_font * 1.22;
            let note_h = note_line_h * note_wrapped.len().max(1) as f32;
            frame.text(TextLabel {
                rect: [rect[0], rect[1], note_w, note_h],
                text: note_wrapped.join("\n"),
                color: color::STONE,
                align: TextAlign::Center,
                font_px: Some(note_font),
                ..Default::default()
            });
        }
    }
}

/// Four-column card layout: title, definition, tiles, optional rule note.
fn layout_basic_meld_cards(
    groups: &[TileGroup],
    window_w: f32,
    window_h: f32,
    area_top_y: f32,
) -> (Vec<ShowcaseTilePlacement>, Vec<BasicMeldCardLayout>) {
    if groups.is_empty() {
        return (vec![], vec![]);
    }

    let margin = window_w * 0.04;
    let col_w = (window_w - margin * 2.0) / groups.len() as f32;
    let cards_bottom = window_h * 0.70;
    let title_font_h = typography::size(typography::H28, window_h) * 1.25;
    let def_font_h = typography::size(typography::H36, window_h) * 1.35;
    let note_font_h = typography::size(typography::H42, window_h) * 1.18;

    let mut placements = Vec::new();
    let mut cards = Vec::with_capacity(groups.len());

    for (i, group) in groups.iter().enumerate() {
        let col_x = margin + i as f32 * col_w;
        let pad = col_w * 0.07;
        let text_x = col_x + pad;
        let text_w = col_w - pad * 2.0;

        let mut y = area_top_y + window_h * 0.006;
        let title_rect = [text_x, y, text_w, title_font_h];
        y += title_font_h + window_h * 0.006;

        let definition = group.definition.unwrap_or("");
        let def_rect = [text_x, y, text_w, def_font_h];
        y += def_font_h + window_h * 0.008;

        let note_lines = group.note.map(|n| n.lines().count()).unwrap_or(0);
        let note_block_h = if note_lines > 0 {
            note_font_h * note_lines as f32 + window_h * 0.006
        } else {
            0.0
        };
        let tiles_bottom = cards_bottom - note_block_h;
        let tile_center_y = (y + tiles_bottom) * 0.5;

        let n_tiles = group.tiles.len();
        let tile_size = ((col_w * 0.88) / (n_tiles as f32 + 0.15))
            .min(window_h * 0.10)
            .max(28.0);
        let group_w = n_tiles as f32 * tile_size;
        let mut cursor_x = col_x + (col_w - group_w) * 0.5;

        for tile in &group.tiles {
            placements.push(ShowcaseTilePlacement {
                tile: *tile,
                center_pos: [cursor_x + tile_size * 0.5, tile_center_y, 0.0],
                rotation: [0.0, 0.0, std::f32::consts::PI],
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

        let note_rect = group.note.map(|_| {
            let note_y = tiles_bottom + window_h * 0.008;
            [text_x, note_y, text_w, note_block_h]
        });

        cards.push(BasicMeldCardLayout {
            accent: group.accent,
            title_rect,
            title: group.label,
            def_rect,
            definition,
            note_rect,
            note: group.note,
        });
    }

    (placements, cards)
}

// ── Tiles intro page (page 0) ─────────────────────────────────────────────

fn push_tiles_text_line(
    frame: &mut UiFrame,
    rect: [f32; 4],
    text: &str,
    font_px: f32,
    color: [f32; 4],
) -> f32 {
    let lh = font_px * 1.22;
    frame.text(TextLabel {
        rect: [rect[0], rect[1], rect[2], lh],
        text: text.into(),
        color,
        align: TextAlign::Center,
        font_px: Some(font_px),
        ..Default::default()
    });
    lh
}

/// Sectioned copy for the first guide page — short lines, two-column number suits.
fn push_tiles_page_text(frame: &mut UiFrame, w: f32, h: f32, y: f32) -> f32 {
    let body_font = typography::size(typography::H36, h);
    let head_font = typography::size(typography::H28, h);
    let full_x = w * 0.08;
    let full_w = w * 0.84;
    let col_w = w * 0.40;
    let left_x = w * 0.07;
    let right_x = w * 0.53;
    let mut cursor = y;

    cursor += push_tiles_text_line(
        frame,
        [full_x, cursor, full_w, 0.0],
        "Mahjuro\u{2019}s wall contains 5 suits. Most tiles have 4 copies.",
        body_font,
        color::PARCHMENT,
    );
    cursor += h * 0.008;

    cursor += push_tiles_text_line(
        frame,
        [full_x, cursor, full_w, 0.0],
        "Number Suits",
        head_font,
        color::CHAMPAGNE,
    );

    let number_left: &[&str] = &[
        "Manzu \u{2014} Characters",
        "Red number tiles, ranked 1\u{2013}9.",
        "Souzu \u{2014} Bamboo",
        "Green number tiles, ranked 1\u{2013}9.",
        "Pinzu \u{2014} Dots",
        "Blue number tiles, ranked 1\u{2013}9.",
    ];
    let number_right: &[&str] = &[
        "Only number suits can form sequences.",
        "Sequences must stay inside one suit.",
        "Ranks 1 and 9 are terminals.",
    ];

    let block_top = cursor;
    let mut left_y = block_top;
    for line in number_left {
        left_y += push_tiles_text_line(
            frame,
            [left_x, left_y, col_w, 0.0],
            line,
            body_font,
            color::PARCHMENT,
        );
    }
    let mut right_y = block_top;
    for line in number_right {
        right_y += push_tiles_text_line(
            frame,
            [right_x, right_y, col_w, 0.0],
            line,
            body_font,
            color::PARCHMENT,
        );
    }
    cursor = left_y.max(right_y) + h * 0.006;

    cursor += push_tiles_text_line(
        frame,
        [full_x, cursor, full_w, 0.0],
        "Honor Suits",
        head_font,
        color::CHAMPAGNE,
    );

    let honor_lines: &[&str] = &[
        "Winds \u{2014} East, South, West, North",
        "Dragons \u{2014} Red, Green, White",
        "Honors do not form sequences.",
        "Use honors as pairs or triplets.",
    ];
    for line in honor_lines {
        cursor += push_tiles_text_line(
            frame,
            [full_x, cursor, full_w, 0.0],
            line,
            body_font,
            color::PARCHMENT,
        );
    }

    cursor + h * 0.008
}

/// Three-row tile layout for page 0: number suits, sequence examples, honors.
fn layout_tiles_page_groups(
    groups: &[TileGroup],
    window_w: f32,
    window_h: f32,
    area_top_y: f32,
) -> (Vec<ShowcaseTilePlacement>, Vec<MeldLabel>) {
    let row_gap = window_h * 0.095;
    let tile_band_bottom = window_h * 0.765;
    let first_row_y = area_top_y + (tile_band_bottom - area_top_y) * 0.18;
    let mut placements = Vec::new();
    let mut labels = Vec::new();
    let mut y = first_row_y;

    for indices in [&[0, 1, 2][..], &[3, 4][..], &[5, 6][..]] {
        let (p, l) = layout_tile_groups_row(groups, indices, window_w, window_h, y);
        placements.extend(p);
        labels.extend(l);
        y += row_gap;
    }

    (placements, labels)
}

fn layout_tile_groups_row(
    groups: &[TileGroup],
    indices: &[usize],
    window_w: f32,
    window_h: f32,
    center_y: f32,
) -> (Vec<ShowcaseTilePlacement>, Vec<MeldLabel>) {
    let row_groups: Vec<&TileGroup> = indices.iter().map(|&i| &groups[i]).collect();
    layout_tile_groups(&row_groups, window_w, window_h, center_y)
}

// ── Tile layout ───────────────────────────────────────────────────────────

/// Lay out tile groups horizontally with gaps between groups. Returns
/// `ShowcaseTilePlacement`s and label annotations.
fn layout_tile_groups(
    groups: &[&TileGroup],
    window_w: f32,
    window_h: f32,
    center_y: f32,
) -> (Vec<ShowcaseTilePlacement>, Vec<MeldLabel>) {
    if groups.is_empty() {
        return (vec![], vec![]);
    }

    let total_tiles: usize = groups.iter().map(|g| g.tiles.len()).sum();
    let num_gaps = groups.len().saturating_sub(1);

    // Compute tile size to fill ~70% of window width, capped for readability.
    let max_tile = window_h * 0.12;
    let gap_equiv = num_gaps as f32 * 0.6; // gap = 0.6 tile widths
    let tile_size = ((window_w * 0.70) / (total_tiles as f32 + gap_equiv))
        .min(max_tile)
        .max(30.0);
    let gap = tile_size * 0.6;

    let total_w = total_tiles as f32 * tile_size + num_gaps as f32 * gap;
    let start_x = (window_w - total_w) * 0.5;

    let scale = (window_w.min(window_h)) / 600.0;
    let label_gap = (12.0 * scale).max(8.0);

    let mut placements = Vec::with_capacity(total_tiles);
    let mut labels = Vec::new();
    let mut cursor_x = start_x;

    for group in groups {
        let group_start_x = cursor_x;

        for tile in &group.tiles {
            let px = cursor_x + tile_size * 0.5;
            let py = center_y;
            placements.push(ShowcaseTilePlacement {
                tile: *tile,
                center_pos: [px, py, 0.0],
                rotation: [0.0, 0.0, std::f32::consts::PI],
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
        labels.push(MeldLabel {
            x: group_start_x,
            y: center_y + tile_size * 0.85 + label_gap,
            w: group_w,
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
    /// Chicken Hand is skipped: by definition its hand triggers *no* yaku,
    /// so the detector returns an empty list rather than `ChickenHand`.
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
