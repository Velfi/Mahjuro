//! Meld Guide — visual onboarding scene teaching melds, tile categories, and yaku.
//!
//! Replaces the old text-only glossary overlay with a paginated 3D-tile diagram
//! that shows concrete tile examples for each meld type, tile category, flower
//! wildcards, and every yaku hand pattern.
//!
//! Accessible from the start screen ("Meld Guide" button) or mid-game via the
//! pause menu. When entered mid-game, the previous scene is suspended by `App`
//! and restored when the player presses Back.

use crate::core::hand::SetKind;
use crate::core::tile::{Suit, Tile};
use crate::core::yaku::YakuKind;
use crate::render::draw_cmd::{CameraParams, DrawCmd, ShowcaseTilePlacement, UiFrame};
use crate::render::theme::{color, typography};
use crate::render::wgpu_renderer::{GpuInstance, PointLight, TextAlign, TextLabel};
use crate::ui::input::UiAction;

use super::start_screen::StartScreenScene;
use super::{BackgroundId, ButtonDef, DrawCtx, Scene, SceneBehavior, SceneTransition, UpdateCtx};

// ── Page indices ──────────────────────────────────────────────────────────

const PAGE_PAIR: usize = 0;
const PAGE_SEQUENCE: usize = 1;
const PAGE_TRIPLET: usize = 2;
const PAGE_KONG: usize = 3;
const PAGE_CATEGORIES: usize = 4;
const PAGE_FLOWERS: usize = 5;
const YAKU_PAGE_START: usize = 6;

fn total_pages() -> usize {
    YAKU_PAGE_START + YakuKind::all().len()
}

// ── Button click IDs ──────────────────────────────────────────────────────

const CLICK_PREV: u32 = 0xD001;
const CLICK_NEXT: u32 = 0xD002;
const CLICK_BACK: u32 = 0xD003;

// ── Scene ─────────────────────────────────────────────────────────────────

pub struct MeldGuideScene {
    page: usize,
    /// `true` when entered from gameplay/shop (i.e. there is a suspended scene
    /// to return to). Affects the "Back" button label.
    #[allow(dead_code)]
    has_suspended: bool,
}

impl MeldGuideScene {
    pub fn new(has_suspended: bool) -> Self {
        Self {
            page: 0,
            has_suspended,
        }
    }

    /// Transition to return to the start screen. When entered as an overlay
    /// (from in-game pause menu or shop), pops the overlay instead.
    fn go_back(&self, overlay_request: &mut Option<super::OverlayRequest>) -> SceneTransition {
        if self.has_suspended {
            *overlay_request = Some(super::OverlayRequest::Pop);
            None
        } else {
            Some(Scene::StartScreen(StartScreenScene::new()))
        }
    }
}

impl SceneBehavior for MeldGuideScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        let pages = total_pages();

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
        let ui_scale = ctx.ui_scale;
        let scale = (w.min(h)) / 600.0 * ui_scale;

        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);

        // ── Camera ────────────────────────────────────────────────
        let cam_scale = h / 1600.0;
        frame.camera_override = Some(CameraParams {
            eye: [0.0, -200.0 * cam_scale, 2040.0 * cam_scale],
            target: [0.0, -50.0 * cam_scale, 0.0],
            up: [0.0, 0.0, 1.0],
            fovy_deg: 45.0,
        });

        // ── Lights ────────────────────────────────────────────────
        let light_y = h * 0.20;
        for &(lx, ly) in &[
            (w * 0.25, h * 0.30),
            (w * 0.75, h * 0.30),
            (w * 0.50, h * 0.50),
            (w * 0.25, h * 0.70),
            (w * 0.75, h * 0.70),
        ] {
            frame.point_lights.push(PointLight {
                pos: [lx, ly, light_y],
                radius: h * 1.2,
                color: [1.0, 0.97, 0.90],
                intensity: 2.2,
            });
        }

        // ── Page content ──────────────────────────────────────────
        let (title, description, groups) = page_content(self.page);

        // Title
        let title_font = typography::size(typography::TITLE, h, ui_scale).max(24.0);
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

        // Description
        let desc_font = typography::size(typography::BODY, h, ui_scale).max(15.0);
        let desc_h = desc_font * 3.5;
        let desc_y = title_y + title_h + h * 0.01;
        frame.text(TextLabel {
            rect: [w * 0.1, desc_y, w * 0.8, desc_h],
            text: description.into(),
            color: color::PARCHMENT,
            align: TextAlign::Center,
            font_px: Some(desc_font),
            ..Default::default()
        });

        // ── Tile placements ───────────────────────────────────────
        let tile_area_y = desc_y + desc_h + h * 0.02;
        let (placements, labels) = layout_tile_groups(&groups, w, h, tile_area_y, ui_scale);

        if !placements.is_empty() {
            frame.cmds.push(DrawCmd::ShowcaseTileBatch(placements));
        }

        // Meld group labels below each group
        let label_font = typography::size(typography::CAPTION, h, ui_scale).max(13.0);
        let label_h = label_font * 1.4;
        for ml in &labels {
            // Colored underline
            let underline_h = (3.0 * scale).max(2.0);
            let underline_y = ml.y - underline_h - 2.0 * scale;
            frame.quad(GpuInstance {
                rect: [ml.x, underline_y, ml.w, underline_h],
                color: ml.color,
            });
            // Label text
            frame.text(TextLabel {
                rect: [ml.x, ml.y, ml.w, label_h],
                text: ml.text.clone(),
                color: color::PARCHMENT,
                align: TextAlign::Center,
                font_px: Some(label_font),
                ..Default::default()
            });
        }

        // ── Scoring info (yaku pages) ─────────────────────────────
        if self.page >= YAKU_PAGE_START {
            let yaku_idx = self.page - YAKU_PAGE_START;
            if let Some(&yk) = YakuKind::all().get(yaku_idx) {
                let score_font = typography::size(typography::BODY, h, ui_scale).max(16.0);
                let score_h = score_font * 1.5;
                let score_y = h * 0.78;
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
        let pages = total_pages();
        let page_font = typography::size(typography::CAPTION, h, ui_scale).max(13.0);
        let page_h = page_font * 1.4;
        let page_y = h * 0.84;
        frame.text(TextLabel {
            rect: [0.0, page_y, w, page_h],
            text: format!("{} / {}", self.page + 1, pages),
            color: color::SLATE,
            align: TextAlign::Center,
            font_px: Some(page_font),
            ..Default::default()
        });

        // ── Navigation buttons ────────────────────────────────────
        let btn_font = typography::size(typography::BODY, h, ui_scale).max(16.0);
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
            color::OBSIDIAN
        } else {
            [0.15, 0.15, 0.18, 0.5]
        };
        frame.quad(GpuInstance {
            rect: [prev_x, btn_y, btn_w, btn_h],
            color: prev_color,
        });
        frame.text(TextLabel {
            rect: [prev_x, btn_y, btn_w, btn_h],
            text: "< Prev".into(),
            color: if prev_enabled {
                color::CHAMPAGNE
            } else {
                color::SLATE
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
            color: color::OBSIDIAN,
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
            color::OBSIDIAN
        } else {
            [0.15, 0.15, 0.18, 0.5]
        };
        frame.quad(GpuInstance {
            rect: [next_x, btn_y, btn_w, btn_h],
            color: next_color,
        });
        frame.text(TextLabel {
            rect: [next_x, btn_y, btn_w, btn_h],
            text: "Next >".into(),
            color: if next_enabled {
                color::CHAMPAGNE
            } else {
                color::SLATE
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

        frame.window_title = "Mahjuro \u{2014} Meld Guide".into();
        frame
    }
}

// ── Page content ──────────────────────────────────────────────────────────

/// A labelled group of tiles forming one meld (or tile-category cluster).
struct TileGroup {
    label: &'static str,
    tiles: Vec<Tile>,
    /// Accent color for the underline bar.
    accent: [f32; 4],
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

/// Returns `(title, description, groups)` for the given page index.
fn page_content(page: usize) -> (&'static str, &'static str, Vec<TileGroup>) {
    match page {
        PAGE_PAIR => (
            "Pair",
            "Two identical tiles. Every complete hand needs exactly one pair.",
            vec![TileGroup {
                label: "Pair",
                tiles: vec![t(Suit::Bamboos, 5, 0), t(Suit::Bamboos, 5, 1)],
                accent: color::CHAMPAGNE,
            }],
        ),
        PAGE_SEQUENCE => (
            "Sequence",
            "Three consecutive tiles of the same number suit (e.g. 4-5-6). Honors and flowers cannot form sequences.",
            vec![TileGroup {
                label: "Sequence",
                tiles: vec![
                    t(Suit::Characters, 4, 0),
                    t(Suit::Characters, 5, 1),
                    t(Suit::Characters, 6, 2),
                ],
                accent: [0.35, 0.70, 0.85, 0.9],
            }],
        ),
        PAGE_TRIPLET => (
            "Triplet",
            "Three identical tiles. Scores 50 chips \u{2014} almost double a sequence's 28.",
            vec![TileGroup {
                label: "Triplet",
                tiles: vec![
                    t(Suit::Circles, 7, 0),
                    t(Suit::Circles, 7, 1),
                    t(Suit::Circles, 7, 2),
                ],
                accent: color::GOLD,
            }],
        ),
        PAGE_KONG => (
            "Kong",
            "Four identical tiles. Counts as a triplet for yaku detection but scores 80 chips.",
            vec![TileGroup {
                label: "Kong",
                tiles: vec![
                    t(Suit::Wind, 1, 0),
                    t(Suit::Wind, 1, 1),
                    t(Suit::Wind, 1, 2),
                    t(Suit::Wind, 1, 3),
                ],
                accent: [0.85, 0.65, 0.20, 0.9],
            }],
        ),
        PAGE_CATEGORIES => (
            "Tile Categories",
            "Number suits have ranks 1\u{2013}9. Rank 1 and 9 are 'terminals.' Winds and Dragons are 'honors.' Many yaku care about these categories.",
            vec![
                TileGroup {
                    label: "Number Suits",
                    tiles: vec![
                        t(Suit::Characters, 3, 0),
                        t(Suit::Bamboos, 6, 1),
                        t(Suit::Circles, 8, 2),
                    ],
                    accent: [0.35, 0.70, 0.85, 0.9],
                },
                TileGroup {
                    label: "Terminals (1 & 9)",
                    tiles: vec![t(Suit::Characters, 1, 3), t(Suit::Bamboos, 9, 4)],
                    accent: [0.85, 0.45, 0.35, 0.9],
                },
                TileGroup {
                    label: "Honors",
                    tiles: vec![
                        t(Suit::Wind, 1, 5),
                        t(Suit::Wind, 3, 6),
                        t(Suit::Dragon, 1, 7),
                    ],
                    accent: [0.70, 0.55, 0.85, 0.9],
                },
            ],
        ),
        PAGE_FLOWERS => (
            "Flowers (Wildcards)",
            "Flower tiles are wildcards. They can form their own melds (any 2 = pair, any 3 = triplet) or substitute for one missing tile in a sequence or triplet.",
            vec![
                TileGroup {
                    label: "Flower Pair",
                    tiles: vec![t(Suit::Flower, 1, 0), t(Suit::Flower, 2, 1)],
                    accent: [0.85, 0.55, 0.70, 0.9],
                },
                TileGroup {
                    label: "Flower as Wildcard",
                    tiles: vec![
                        t(Suit::Characters, 4, 2),
                        t(Suit::Flower, 3, 3),
                        t(Suit::Characters, 6, 4),
                    ],
                    accent: [0.85, 0.55, 0.70, 0.9],
                },
            ],
        ),
        _ => {
            // Yaku pages
            let yaku_idx = page - YAKU_PAGE_START;
            let all = YakuKind::all();
            if let Some(&yk) = all.get(yaku_idx) {
                let (desc, groups) = yaku_page(yk);
                (yk.name(), desc, groups)
            } else {
                ("", "", vec![])
            }
        }
    }
}

/// Build tile groups for a yaku example hand.
fn yaku_page(yk: YakuKind) -> (&'static str, Vec<TileGroup>) {
    let seq_color: [f32; 4] = [0.35, 0.70, 0.85, 0.9];
    let trip_color: [f32; 4] = color::GOLD;
    let pair_color: [f32; 4] = color::CHAMPAGNE;
    let _kong_color: [f32; 4] = [0.85, 0.65, 0.20, 0.9];

    match yk {
        YakuKind::Tanyao => (
            "All tiles ranked 2\u{2013}8 \u{2014} no terminals (1/9) or honors.",
            meld_groups(&[
                (
                    "Seq",
                    SetKind::Sequence,
                    Suit::Characters,
                    &[2, 3, 4],
                    seq_color,
                ),
                (
                    "Seq",
                    SetKind::Sequence,
                    Suit::Bamboos,
                    &[5, 6, 7],
                    seq_color,
                ),
                (
                    "Trip",
                    SetKind::Triplet,
                    Suit::Circles,
                    &[8, 8, 8],
                    trip_color,
                ),
                ("Pair", SetKind::Pair, Suit::Characters, &[5, 5], pair_color),
            ]),
        ),
        YakuKind::Toitoi => (
            "All melds are triplets or kongs \u{2014} no sequences allowed.",
            meld_groups(&[
                (
                    "Trip",
                    SetKind::Triplet,
                    Suit::Characters,
                    &[1, 1, 1],
                    trip_color,
                ),
                (
                    "Trip",
                    SetKind::Triplet,
                    Suit::Bamboos,
                    &[5, 5, 5],
                    trip_color,
                ),
                (
                    "Trip",
                    SetKind::Triplet,
                    Suit::Circles,
                    &[9, 9, 9],
                    trip_color,
                ),
                ("Pair", SetKind::Pair, Suit::Wind, &[1, 1], pair_color),
            ]),
        ),
        YakuKind::Honroutou => (
            "Every tile is a terminal (1 or 9) or an honor (wind/dragon).",
            meld_groups(&[
                (
                    "Trip",
                    SetKind::Triplet,
                    Suit::Characters,
                    &[1, 1, 1],
                    trip_color,
                ),
                (
                    "Trip",
                    SetKind::Triplet,
                    Suit::Bamboos,
                    &[9, 9, 9],
                    trip_color,
                ),
                ("Trip", SetKind::Triplet, Suit::Wind, &[1, 1, 1], trip_color),
                ("Pair", SetKind::Pair, Suit::Circles, &[1, 1], pair_color),
            ]),
        ),
        YakuKind::Iipeikou => (
            "Two identical sequences in the same suit.",
            meld_groups(&[
                (
                    "Seq",
                    SetKind::Sequence,
                    Suit::Bamboos,
                    &[1, 2, 3],
                    seq_color,
                ),
                (
                    "Seq",
                    SetKind::Sequence,
                    Suit::Bamboos,
                    &[1, 2, 3],
                    seq_color,
                ),
                (
                    "Trip",
                    SetKind::Triplet,
                    Suit::Circles,
                    &[4, 4, 4],
                    trip_color,
                ),
                ("Pair", SetKind::Pair, Suit::Characters, &[9, 9], pair_color),
            ]),
        ),
        YakuKind::FullHand => (
            "Complete 14-tile hand: 4+4+4+4+2 (4 melds + 1 pair), not 2x7 seven pairs.",
            meld_groups(&[
                (
                    "Seq",
                    SetKind::Sequence,
                    Suit::Characters,
                    &[1, 2, 3],
                    seq_color,
                ),
                (
                    "Seq",
                    SetKind::Sequence,
                    Suit::Bamboos,
                    &[4, 5, 6],
                    seq_color,
                ),
                (
                    "Trip",
                    SetKind::Triplet,
                    Suit::Circles,
                    &[7, 7, 7],
                    trip_color,
                ),
                (
                    "Seq",
                    SetKind::Sequence,
                    Suit::Characters,
                    &[7, 8, 9],
                    seq_color,
                ),
                ("Pair", SetKind::Pair, Suit::Dragon, &[1, 1], pair_color),
            ]),
        ),
        YakuKind::Chinitsu => (
            "All tiles from a single number suit, no honors.",
            meld_groups(&[
                (
                    "Seq",
                    SetKind::Sequence,
                    Suit::Bamboos,
                    &[1, 2, 3],
                    seq_color,
                ),
                (
                    "Seq",
                    SetKind::Sequence,
                    Suit::Bamboos,
                    &[4, 5, 6],
                    seq_color,
                ),
                (
                    "Trip",
                    SetKind::Triplet,
                    Suit::Bamboos,
                    &[7, 7, 7],
                    trip_color,
                ),
                ("Pair", SetKind::Pair, Suit::Bamboos, &[9, 9], pair_color),
            ]),
        ),
        YakuKind::SanshokuDoujun => (
            "Same numerical sequence in all three number suits.",
            meld_groups(&[
                (
                    "Seq",
                    SetKind::Sequence,
                    Suit::Characters,
                    &[4, 5, 6],
                    seq_color,
                ),
                (
                    "Seq",
                    SetKind::Sequence,
                    Suit::Bamboos,
                    &[4, 5, 6],
                    seq_color,
                ),
                (
                    "Seq",
                    SetKind::Sequence,
                    Suit::Circles,
                    &[4, 5, 6],
                    seq_color,
                ),
                ("Pair", SetKind::Pair, Suit::Wind, &[1, 1], pair_color),
            ]),
        ),
        YakuKind::Junchan => (
            "Every meld contains a terminal (rank 1 or 9). Pair is also terminal.",
            meld_groups(&[
                (
                    "Seq",
                    SetKind::Sequence,
                    Suit::Characters,
                    &[1, 2, 3],
                    seq_color,
                ),
                (
                    "Seq",
                    SetKind::Sequence,
                    Suit::Bamboos,
                    &[7, 8, 9],
                    seq_color,
                ),
                (
                    "Trip",
                    SetKind::Triplet,
                    Suit::Circles,
                    &[1, 1, 1],
                    trip_color,
                ),
                ("Pair", SetKind::Pair, Suit::Characters, &[9, 9], pair_color),
            ]),
        ),
        YakuKind::Ittsu => (
            "1\u{2013}9 straight in one suit: three sequences covering 1-2-3, 4-5-6, 7-8-9.",
            meld_groups(&[
                (
                    "Seq",
                    SetKind::Sequence,
                    Suit::Bamboos,
                    &[1, 2, 3],
                    seq_color,
                ),
                (
                    "Seq",
                    SetKind::Sequence,
                    Suit::Bamboos,
                    &[4, 5, 6],
                    seq_color,
                ),
                (
                    "Seq",
                    SetKind::Sequence,
                    Suit::Bamboos,
                    &[7, 8, 9],
                    seq_color,
                ),
                ("Pair", SetKind::Pair, Suit::Characters, &[5, 5], pair_color),
            ]),
        ),
        YakuKind::Honitsu => (
            "One number suit plus honors only \u{2014} no other number suits.",
            meld_groups(&[
                (
                    "Seq",
                    SetKind::Sequence,
                    Suit::Bamboos,
                    &[2, 3, 4],
                    seq_color,
                ),
                (
                    "Seq",
                    SetKind::Sequence,
                    Suit::Bamboos,
                    &[6, 7, 8],
                    seq_color,
                ),
                ("Trip", SetKind::Triplet, Suit::Wind, &[1, 1, 1], trip_color),
                ("Pair", SetKind::Pair, Suit::Bamboos, &[9, 9], pair_color),
            ]),
        ),
        YakuKind::Yakuhai => (
            "Triplet (or kong) of any dragon, or of the current round wind.",
            meld_groups(&[
                (
                    "Trip",
                    SetKind::Triplet,
                    Suit::Dragon,
                    &[1, 1, 1],
                    trip_color,
                ),
                (
                    "Seq",
                    SetKind::Sequence,
                    Suit::Characters,
                    &[2, 3, 4],
                    seq_color,
                ),
                ("Pair", SetKind::Pair, Suit::Bamboos, &[5, 5], pair_color),
            ]),
        ),
        YakuKind::Chiitoitsu => (
            "Seven distinct pairs \u{2014} an alternate hand shape (no melds).",
            meld_groups(&[
                ("Pair", SetKind::Pair, Suit::Characters, &[1, 1], pair_color),
                ("Pair", SetKind::Pair, Suit::Characters, &[3, 3], pair_color),
                ("Pair", SetKind::Pair, Suit::Bamboos, &[5, 5], pair_color),
                ("Pair", SetKind::Pair, Suit::Bamboos, &[7, 7], pair_color),
                ("Pair", SetKind::Pair, Suit::Circles, &[2, 2], pair_color),
                ("Pair", SetKind::Pair, Suit::Circles, &[4, 4], pair_color),
                ("Pair", SetKind::Pair, Suit::Wind, &[1, 1], pair_color),
            ]),
        ),
        YakuKind::ChickenHand => (
            "A valid hand that triggers no other yaku \u{2014} scores base chips \u{00d7} 1 mult.",
            meld_groups(&[
                (
                    "Seq",
                    SetKind::Sequence,
                    Suit::Characters,
                    &[1, 2, 3],
                    seq_color,
                ),
                (
                    "Seq",
                    SetKind::Sequence,
                    Suit::Bamboos,
                    &[4, 5, 6],
                    seq_color,
                ),
                (
                    "Trip",
                    SetKind::Triplet,
                    Suit::Circles,
                    &[3, 3, 3],
                    trip_color,
                ),
                ("Pair", SetKind::Pair, Suit::Wind, &[2, 2], pair_color),
            ]),
        ),
    }
}

/// Build `TileGroup`s from a compact descriptor list. Assigns sequential tile
/// ids across all groups so the renderer treats each tile as unique.
fn meld_groups(specs: &[(&'static str, SetKind, Suit, &[u8], [f32; 4])]) -> Vec<TileGroup> {
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
            }
        })
        .collect()
}

// ── Tile layout ───────────────────────────────────────────────────────────

/// Lay out tile groups horizontally with gaps between groups. Returns
/// `ShowcaseTilePlacement`s and label annotations.
fn layout_tile_groups(
    groups: &[TileGroup],
    window_w: f32,
    window_h: f32,
    area_top_y: f32,
    ui_scale: f32,
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
    let center_y = area_top_y + (window_h * 0.75 - area_top_y) * 0.45;

    let scale = (window_w.min(window_h)) / 600.0 * ui_scale;
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

/// Short hand-shape description for each yaku. Moved here from the old
/// glossary module; still used by the shop yaku list and journal overlay.
pub(crate) fn yaku_shape_text(yk: YakuKind) -> &'static str {
    match yk {
        YakuKind::Tanyao => {
            "All tiles 2\u{2013}8, no honors/terminals (e.g. \u{1f3b4}234 \u{1f38b}567 \u{1f534}88)"
        }
        YakuKind::Toitoi => {
            "All triplets/kongs, no sequences (e.g. \u{1f3b4}222 \u{1f38b}555 \u{1f534}999)"
        }
        YakuKind::FullHand => "Complete 14-tile hand: 4+4+4+4+2 (4 melds + 1 pair), not 2x7",
        YakuKind::Yakuhai => {
            "Triplet of any dragon or round wind (e.g. \u{1f409}\u{1f409}\u{1f409})"
        }
        YakuKind::Iipeikou => {
            "Two identical sequences in one suit (e.g. \u{1f38b}123 \u{1f38b}123)"
        }
        YakuKind::SanshokuDoujun => {
            "Same sequence in all 3 suits (e.g. \u{1f3b4}456 \u{1f38b}456 \u{1f534}456)"
        }
        YakuKind::Ittsu => {
            "1\u{2013}9 straight in one suit (e.g. \u{1f38b}123 \u{1f38b}456 \u{1f38b}789)"
        }
        YakuKind::Honitsu => {
            "One number suit + honors only (e.g. \u{1f38b}234 \u{1f38b}678 \u{1f32c}\u{1f32c}\u{1f32c})"
        }
        YakuKind::Chinitsu => {
            "All one number suit, no honors (e.g. \u{1f38b}123 \u{1f38b}456 \u{1f38b}789 \u{1f38b}11)"
        }
        YakuKind::Junchan => {
            "Every meld has a 1 or 9 (e.g. \u{1f38b}123 \u{1f3b4}789 \u{1f534}111 \u{1f38b}99)"
        }
        YakuKind::Honroutou => {
            "Only 1s, 9s, and honors (e.g. \u{1f38b}111 \u{1f3b4}999 \u{1f32c}\u{1f32c}\u{1f32c})"
        }
        YakuKind::Chiitoitsu => {
            "Seven distinct pairs (e.g. \u{1f3b4}11 \u{1f3b4}33 \u{1f38b}55 \u{1f38b}77 \u{1f534}22 \u{1f534}44 \u{1f32c}\u{1f32c})"
        }
        YakuKind::ChickenHand => {
            "Valid hand with no yaku \u{2014} scores base chips \u{00d7} 1 mult"
        }
    }
}
