//! Scripted onboarding campaign scenes shown before the tutorial shop.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::audio::SfxId;
use crate::core::tile::{Suit, Tile};
use crate::game::engine::GameEngine;
use crate::game::event_bus::GameEvent;
use crate::render::draw_cmd::{
    CameraParams, DrawCmd, Object3d, Object3dKind, ShowcaseTilePlacement, UiFrame,
};
use crate::render::theme::{ButtonState, ButtonVariant, color, metrics, typography};
use crate::render::wgpu_renderer::{GpuInstance, PointLight, TextAlign, TextLabel};
use crate::render::world_space::LayoutAnchorPx;
use crate::ui::colored_keywords;
use crate::ui::focus_nav;
use crate::ui::widget::{self, TextStyle};
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use super::{BackgroundId, DrawCtx, Scene, SceneBehavior, SceneTransition, UpdateCtx};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TutorialNav {
    Back,
    Next,
    TryPlay,
    TryTrigger,
}

impl TutorialNav {
    fn id(self) -> FocusId {
        FocusId(0x7000 + self as u32)
    }
}

/// Layout for the Play / Trigger demo strip (matches `draw_frame` geometry).
struct TryItLayout {
    play_rect: [f32; 4],
    trigger_rect: [f32; 4],
    /// Y position for the one-line demo result (chips × mult = total).
    demo_line_y: f32,
    /// Minimum Y where glossary / callout may start (below demo line).
    content_floor_y: f32,
}

pub struct TutorialCampaignScene {
    page: usize,
    tree: TreeState,
    /// Demo rhythm: 0 = idle, 1 = banked (after Play), 2 = scored (after Trigger).
    try_it_phase: u8,
    /// Arrange-mode-tunable placements for the shop preview props and the
    /// try-it-demo Mirror/Trigger pair.
    pub positions: crate::ui::scene_layout::TutorialPositions,
}

struct TileGroup {
    label: &'static str,
    accent: [f32; 4],
    tiles: &'static [(Suit, u8)],
    rows: &'static [&'static [(Suit, u8)]],
    debuffed_visual: bool,
}

struct TutorialPage {
    title: &'static str,
    subtitle: &'static str,
    glossary: &'static [&'static str],
    callout: Option<&'static str>,
    /// Interactive Play → Trigger demo strip with fake totals (structure + boss pages).
    try_it_demo: bool,
    groups: &'static [TileGroup],
}

struct TilesPageLabel {
    x: f32,
    y: f32,
    w: f32,
    text: &'static str,
    accent: [f32; 4],
}

/// Part 1 — The Tiles (0-based index into `PAGES`).
const TUTORIAL_PAGE_TILES: usize = 0;

const PART1_TILE_GROUPS: &[TileGroup] = &[
    TileGroup {
        label: "Manzu",
        accent: Suit::Manzu.keyword_color(),
        tiles: &[(Suit::Manzu, 1), (Suit::Manzu, 5), (Suit::Manzu, 9)],
        rows: &[],
        debuffed_visual: false,
    },
    TileGroup {
        label: "Souzu",
        accent: Suit::Souzu.keyword_color(),
        tiles: &[(Suit::Souzu, 1), (Suit::Souzu, 5), (Suit::Souzu, 9)],
        rows: &[],
        debuffed_visual: false,
    },
    TileGroup {
        label: "Pinzu",
        accent: Suit::Pinzu.keyword_color(),
        tiles: &[(Suit::Pinzu, 1), (Suit::Pinzu, 5), (Suit::Pinzu, 9)],
        rows: &[],
        debuffed_visual: false,
    },
    TileGroup {
        label: "3-4-5 Manzu = valid",
        accent: [0.35, 0.70, 0.85, 0.9],
        tiles: &[(Suit::Manzu, 3), (Suit::Manzu, 4), (Suit::Manzu, 5)],
        rows: &[],
        debuffed_visual: false,
    },
    TileGroup {
        label: "3 Manzu / 4 Souzu / 5 Pinzu = invalid",
        accent: color::STONE,
        tiles: &[(Suit::Manzu, 3), (Suit::Souzu, 4), (Suit::Pinzu, 5)],
        rows: &[],
        debuffed_visual: false,
    },
    TileGroup {
        label: "Winds",
        accent: Suit::Wind.keyword_color(),
        tiles: &[
            (Suit::Wind, 1),
            (Suit::Wind, 2),
            (Suit::Wind, 3),
            (Suit::Wind, 4),
        ],
        rows: &[],
        debuffed_visual: false,
    },
    TileGroup {
        label: "Dragons",
        accent: Suit::Dragon.keyword_color(),
        tiles: &[
            (Suit::Dragon, 1),
            (Suit::Dragon, 2),
            (Suit::Dragon, 3),
        ],
        rows: &[],
        debuffed_visual: false,
    },
];

/// Part 2 — How to Score (0-based index into `PAGES`).
const TUTORIAL_PAGE_SCORING: usize = 1;

const SCORING_DEMO_GROUPS: &[TileGroup] = &[TileGroup {
    label: "Pair",
    accent: color::CHAMPAGNE,
    tiles: &[(Suit::Pinzu, 5), (Suit::Pinzu, 5)],
    rows: &[],
    debuffed_visual: false,
}];

const PAGES: &[TutorialPage] = &[
    TutorialPage {
        title: "Part 1 — The Tiles",
        subtitle: "",
        glossary: &[],
        callout: Some("Next: basic melds and how to score."),
        try_it_demo: false,
        groups: PART1_TILE_GROUPS,
    },
    TutorialPage {
        title: "Part 2 — How to Score",
        subtitle: "Select tiles, press Play to bank them into your structure, then Cash In to score. Your round score is chips × mult.",
        glossary: &[
            "Structure = banked melds until you cash in",
            "Play = bank selected tiles into your structure",
            "Cash In = score your structure",
            "Bank = lock tiles into structure (Play)",
            "Chips = base points",
            "Mult = multiplier on chips",
        ],
        callout: Some("Try the demo below, then you'll play a short guided blind."),
        try_it_demo: true,
        groups: SCORING_DEMO_GROUPS,
    },
];

impl TutorialCampaignScene {
    pub fn new() -> Self {
        Self {
            page: 0,
            tree: TreeState::new(),
            try_it_phase: 0,
            positions: crate::ui::scene_layout::TutorialPositions::default(),
        }
    }

    fn page(&self) -> &'static TutorialPage {
        &PAGES[self.page.min(PAGES.len() - 1)]
    }

    fn try_it_demo_line(page_index: usize, phase: u8) -> Option<&'static str> {
        match (page_index, phase) {
            (TUTORIAL_PAGE_SCORING, 0) => Some("Tap Play (bank), then Cash In."),
            (TUTORIAL_PAGE_SCORING, 1) => Some("Banked — structure is locked in."),
            (TUTORIAL_PAGE_SCORING, 2) => Some("Demo: 4 chips × 3 mult = 12"),
            _ => None,
        }
    }

    fn compute_try_it_layout(
        panel_x: f32,
        panel_w: f32,
        content_bottom_y: f32,
        scale: f32,
    ) -> TryItLayout {
        let btn_w = (150.0 * scale).max(100.0);
        let btn_h = (40.0 * scale).max(28.0);
        let gap = 12.0 * scale;
        let heading_h = 22.0 * scale;
        let strip_y = content_bottom_y + 14.0 * scale + heading_h;
        let center = panel_x + panel_w * 0.5;
        let play_x = center - btn_w - gap * 0.5;
        let trigger_x = center + gap * 0.5;
        let demo_line_y = strip_y + btn_h + 10.0 * scale;
        let content_floor_y = demo_line_y + 24.0 * scale;
        TryItLayout {
            play_rect: [play_x, strip_y, btn_w, btn_h],
            trigger_rect: [trigger_x, strip_y, btn_w, btn_h],
            demo_line_y,
            content_floor_y,
        }
    }

    fn scoring_page_subtitle_end_y(
        subtitle_y: f32,
        panel_w: f32,
        h: f32,
        scale: f32,
        subtitle: &str,
    ) -> f32 {
        let subtitle_w = panel_w - 60.0 * scale;
        let subtitle_font = typography::size(typography::H36, h);
        let subtitle_lines_n = colored_keywords::colored_wrapped_line_count(
            subtitle,
            subtitle_w,
            subtitle_font,
            color::PARCHMENT,
        );
        let subtitle_h = (subtitle_lines_n as f32 * subtitle_font * 1.35)
            .max(70.0 * scale)
            .min(128.0 * scale);
        subtitle_y + subtitle_h
    }

    /// Single-row tile showcase for Part 2 (pair demo).
    fn layout_demo_page_tiles(
        page: &TutorialPage,
        w: f32,
        h: f32,
        area_top_y: f32,
        scale: f32,
    ) -> (Vec<ShowcaseTilePlacement>, Vec<TilesPageLabel>, f32) {
        let tile_band_bottom = h * 0.52;
        let tile_center_y = area_top_y + (tile_band_bottom - area_top_y) * 0.35;
        let indices: Vec<usize> = (0..page.groups.len()).collect();
        let mut next_id = 30_000u32;
        Self::layout_tutorial_tile_row(page, &indices, w, h, tile_center_y, scale, &mut next_id)
    }

    fn push_tiles_text_line(
        texts: &mut Vec<TextLabel>,
        rect: [f32; 4],
        text: &str,
        font_px: f32,
        color: [f32; 4],
    ) -> f32 {
        let lh = font_px * 1.22;
        texts.push(TextLabel {
            rect: [rect[0], rect[1], rect[2], lh],
            text: text.into(),
            color,
            align: TextAlign::Center,
            font_px: Some(font_px),
            ..Default::default()
        });
        lh
    }

    /// Sectioned copy for Part 1 — mirrors the in-game Guide tiles page.
    fn push_tutorial_tiles_page_copy(
        texts: &mut Vec<TextLabel>,
        panel_x: f32,
        start_y: f32,
        panel_w: f32,
        h: f32,
    ) -> f32 {
        let body_font = typography::size(typography::H36, h);
        let head_font = typography::size(typography::H28, h);
        let full_x = panel_x + 30.0;
        let full_w = panel_w - 60.0;
        let col_w = panel_w * 0.40;
        let left_x = panel_x + panel_w * 0.07;
        let right_x = panel_x + panel_w * 0.53;
        let mut cursor = start_y;

        cursor += Self::push_tiles_text_line(
            texts,
            [full_x, cursor, full_w, 0.0],
            "Mahjuro\u{2019}s wall contains 5 suits. Most tiles have 4 copies.",
            body_font,
            color::PARCHMENT,
        );
        cursor += h * 0.006;

        cursor += Self::push_tiles_text_line(
            texts,
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
            left_y += Self::push_tiles_text_line(
                texts,
                [left_x, left_y, col_w, 0.0],
                line,
                body_font,
                color::PARCHMENT,
            );
        }
        let mut right_y = block_top;
        for line in number_right {
            right_y += Self::push_tiles_text_line(
                texts,
                [right_x, right_y, col_w, 0.0],
                line,
                body_font,
                color::PARCHMENT,
            );
        }
        cursor = left_y.max(right_y) + h * 0.005;

        cursor += Self::push_tiles_text_line(
            texts,
            [full_x, cursor, full_w, 0.0],
            "Honor Suits",
            head_font,
            color::CHAMPAGNE,
        );

        for line in &[
            "Winds \u{2014} East, South, West, North",
            "Dragons \u{2014} Red, Green, White",
            "Honors do not form sequences.",
            "Use honors as pairs or triplets.",
        ] {
            cursor += Self::push_tiles_text_line(
                texts,
                [full_x, cursor, full_w, 0.0],
                line,
                body_font,
                color::PARCHMENT,
            );
        }

        cursor + h * 0.006
    }

    fn glossary_term_metrics(
        glossary: &[&'static str],
        term_w: f32,
        term_font: f32,
        scale: f32,
    ) -> (Vec<f32>, f32) {
        let mut heights = Vec::with_capacity(glossary.len());
        let mut total_h = 0.0;
        for term in glossary {
            let lines_n =
                colored_keywords::colored_wrapped_line_count(term, term_w, term_font, color::STONE);
            let term_h = lines_n as f32 * term_font * 1.25;
            heights.push(term_h);
            total_h += term_h;
        }
        if !glossary.is_empty() {
            total_h += (glossary.len().saturating_sub(1) as f32) * 6.0 * scale;
        }
        (heights, total_h)
    }

    fn scoring_page_try_it_layout(
        page: &TutorialPage,
        w: f32,
        h: f32,
        panel_x: f32,
        panel_y: f32,
        panel_w: f32,
    ) -> TryItLayout {
        let scale = metrics::scene_scale(w, h);
        let subtitle_y = panel_y + 70.0 * scale;
        let subtitle_end_y =
            Self::scoring_page_subtitle_end_y(subtitle_y, panel_w, h, scale, page.subtitle);
        let tile_area_y = subtitle_end_y + h * 0.012;
        let (_, _, tile_bottom) = Self::layout_demo_page_tiles(page, w, h, tile_area_y, scale);
        Self::compute_try_it_layout(panel_x, panel_w, tile_bottom, scale)
    }

    fn flat_items(&self, w: f32, h: f32) -> Vec<FlatItem<TutorialNav>> {
        let scale = metrics::scene_scale(w, h);
        let btn_w = (170.0 * scale).max(120.0);
        let btn_h = (46.0 * scale).max(30.0);
        let gap = 14.0 * scale;
        let y = h - btn_h - 22.0 * scale;
        let next_x = w * 0.5 + gap * 0.5;
        let back_x = next_x - btn_w - gap;

        let page = self.page();
        let panel_x = w * 0.06;
        let panel_w = w * 0.88;
        let panel_y = h * 0.07;

        let mut items = Vec::new();
        if self.page > 0 {
            items.push(FlatItem::new(
                TutorialNav::Back.id(),
                [back_x, y, btn_w, btn_h],
                TutorialNav::Back,
            ));
        }
        items.push(FlatItem::new(
            TutorialNav::Next.id(),
            [next_x, y, btn_w, btn_h],
            TutorialNav::Next,
        ));

        if page.try_it_demo {
            let t = Self::scoring_page_try_it_layout(page, w, h, panel_x, panel_y, panel_w);
            items.push(FlatItem::new(
                TutorialNav::TryPlay.id(),
                t.play_rect,
                TutorialNav::TryPlay,
            ));
            items.push(FlatItem::new(
                TutorialNav::TryTrigger.id(),
                t.trigger_rect,
                TutorialNav::TryTrigger,
            ));
        }

        items
    }

    /// Three-row tile layout for Part 1 — flows downward from the copy block (matches Guide).
    fn layout_tutorial_tiles_page(
        page: &TutorialPage,
        w: f32,
        h: f32,
        area_top_y: f32,
        scale: f32,
    ) -> (Vec<ShowcaseTilePlacement>, Vec<TilesPageLabel>, f32) {
        let row_gap = h * 0.085;
        let tile_band_bottom = h * 0.72;
        let mut row_y = area_top_y + (tile_band_bottom - area_top_y) * 0.12;
        let row_sets: &[&[usize]] = &[&[0, 1, 2], &[3, 4], &[5, 6]];
        let mut placements = Vec::new();
        let mut labels = Vec::new();
        let mut next_id = 30_000u32;
        let mut content_bottom = row_y;

        for indices in row_sets {
            let (row_placements, row_labels, row_bottom) = Self::layout_tutorial_tile_row(
                page,
                indices,
                w,
                h,
                row_y,
                scale,
                &mut next_id,
            );
            placements.extend(row_placements);
            labels.extend(row_labels);
            content_bottom = row_bottom;
            row_y += row_gap;
        }

        (placements, labels, content_bottom)
    }

    fn layout_tutorial_tile_row(
        page: &TutorialPage,
        indices: &[usize],
        window_w: f32,
        window_h: f32,
        center_y: f32,
        scale: f32,
        next_id: &mut u32,
    ) -> (Vec<ShowcaseTilePlacement>, Vec<TilesPageLabel>, f32) {
        let groups: Vec<&TileGroup> = indices.iter().map(|&i| &page.groups[i]).collect();
        let total_tiles: usize = groups.iter().map(|g| g.tiles.len()).sum();
        let num_gaps = groups.len().saturating_sub(1);

        let max_tile = window_h * 0.09;
        let gap_equiv = num_gaps as f32 * 0.6;
        let tile_size = ((window_w * 0.70) / (total_tiles as f32 + gap_equiv))
            .min(max_tile)
            .max(24.0);
        let gap = tile_size * 0.6;

        let total_w = total_tiles as f32 * tile_size + num_gaps as f32 * gap;
        let start_x = (window_w - total_w) * 0.5;
        let label_gap = (10.0 * scale).max(6.0);
        let label_line_h = typography::size(typography::H42, window_h) * 1.22;

        let mut placements = Vec::new();
        let mut labels = Vec::new();
        let mut cursor_x = start_x;
        let mut row_bottom = center_y + tile_size * 0.5;

        for group in groups {
            let group_start_x = cursor_x;
            for &(suit, rank) in group.tiles {
                let px = cursor_x + tile_size * 0.5;
                let mut tile = Tile::new(suit, rank, *next_id);
                tile.debuffed_visual = group.debuffed_visual;
                placements.push(ShowcaseTilePlacement {
                    tile,
                    center_pos: [px, center_y, 0.0],
                    rotation: [0.0, 0.0, std::f32::consts::PI],
                    scale: 1.0,
                    size_px: tile_size,
                    brightness: 1.08,
                    selected: false,
                    hovered: false,
                    outline: false,
                    glow: false,
                    glow_color: None,
                    pick_id: None,
                    overlay_rect_group: None,
                });
                *next_id += 1;
                cursor_x += tile_size;
            }

            let group_w = cursor_x - group_start_x;
            let label_y = center_y + tile_size * 0.85 + label_gap;
            labels.push(TilesPageLabel {
                x: group_start_x,
                y: label_y,
                w: group_w,
                text: group.label,
                accent: group.accent,
            });
            row_bottom = label_y + label_line_h;
            cursor_x += gap;
        }

        (placements, labels, row_bottom)
    }

    fn draw_tiles_page_group_labels(
        texts: &mut Vec<TextLabel>,
        fg_quads: &mut Vec<GpuInstance>,
        labels: &[TilesPageLabel],
        scale: f32,
        h: f32,
    ) {
        let label_font = typography::size(typography::H42, h);
        let underline_h = (3.0 * scale).max(2.0);

        for label in labels {
            let underline_y = label.y - underline_h - 2.0 * scale;
            fg_quads.push(GpuInstance {
                rect: [label.x, underline_y, label.w, underline_h],
                color: label.accent,
                user: 0,
            });
            texts.push(TextLabel {
                rect: [label.x, label.y, label.w, label_font * 1.4],
                text: label.text.to_string(),
                color: color::PARCHMENT,
                align: TextAlign::Center,
                font_px: Some(label_font),
                ..Default::default()
            });
        }
    }
}

impl SceneBehavior for TutorialCampaignScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
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
            Some(TutorialNav::Back) => {
                ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
                if self.page > 0 {
                    self.page -= 1;
                    self.try_it_phase = 0;
                    ctx.bus.push(GameEvent::UiSound(SfxId::TileClick));
                }
                None
            }
            Some(TutorialNav::Next) => {
                ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                if self.page + 1 < PAGES.len() {
                    self.page += 1;
                    self.try_it_phase = 0;
                    ctx.bus.push(GameEvent::UiSound(SfxId::PackBuy));
                    None
                } else {
                    ctx.bus.push(GameEvent::UiSound(SfxId::RelicPickup));
                    GameEngine::begin_onboarding_lessons(ctx.run);
                    Some(Scene::Gameplay(Box::new(
                        super::gameplay::GameplayScene::with_pending_blind(
                            crate::core::rules::BlindKind::Small,
                        ),
                    )))
                }
            }
            Some(TutorialNav::TryPlay) => {
                if !self.page().try_it_demo {
                    return None;
                }
                match self.try_it_phase {
                    0 => {
                        self.try_it_phase = 1;
                        ctx.bus.push(GameEvent::StructureCommitted);
                    }
                    2 => {
                        self.try_it_phase = 0;
                    }
                    _ => {
                        ctx.bus.push(GameEvent::UiSound(SfxId::InvalidAction));
                    }
                }
                None
            }
            Some(TutorialNav::TryTrigger) => {
                if !self.page().try_it_demo {
                    return None;
                }
                if self.try_it_phase == 1 {
                    self.try_it_phase = 2;
                    ctx.bus.push(GameEvent::UiSound(SfxId::ScoreReveal));
                    ctx.bus.push(GameEvent::UiSound(SfxId::ScoreFinal));
                } else {
                    ctx.bus.push(GameEvent::UiSound(SfxId::InvalidAction));
                }
                None
            }
            None => None,
        }
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let scale = metrics::scene_scale(w, h);
        let page = self.page();

        let mut bg_quads = Vec::new();
        let mut fg_quads = Vec::new();
        let mut texts = Vec::new();
        let mut wood_tablet_placements: Vec<Object3d> = Vec::new();
        let mut mirror_placement: Option<Object3d> = None;
        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);
        if ctx.effect_layers.starfield {
            frame.starfield();
        }
        if ctx.effect_layers.golden_dust {
            frame.golden_dust();
        }
        let cam_scale = h / 1600.0;
        frame.camera_override = Some(CameraParams {
            eye: [0.0, -220.0 * cam_scale, 1960.0 * cam_scale],
            target: [0.0, -40.0 * cam_scale, 0.0],
            up: [0.0, 0.0, 1.0],
            fovy_deg: 45.0,
            clip_near: None,
            clip_far: None,
        });

        let panel_x = w * 0.06;
        let panel_y = h * 0.07;
        let panel_w = w * 0.88;
        let panel_h = h * 0.84;
        bg_quads.push(GpuInstance {
            rect: [panel_x, panel_y, panel_w, panel_h],
            color: color::WALNUT_DEEP,
            user: 0,
        });
        bg_quads.push(GpuInstance {
            rect: [panel_x, panel_y, panel_w, (2.0 * scale).max(1.0)],
            color: color::BRASS,
            user: 0,
        });
        bg_quads.push(GpuInstance {
            rect: [panel_x, panel_y, (2.0 * scale).max(1.0), panel_h],
            color: color::BRASS,
            user: 0,
        });
        bg_quads.push(GpuInstance {
            rect: [
                panel_x + panel_w - (2.0 * scale).max(1.0),
                panel_y,
                (2.0 * scale).max(1.0),
                panel_h,
            ],
            color: color::BRASS,
            user: 0,
        });
        bg_quads.push(GpuInstance {
            rect: [
                panel_x,
                panel_y + panel_h - (2.0 * scale).max(1.0),
                panel_w,
                (2.0 * scale).max(1.0),
            ],
            color: color::BRASS,
            user: 0,
        });

        texts.push(TextLabel {
            rect: [
                panel_x + 24.0 * scale,
                panel_y + 18.0 * scale,
                panel_w - 48.0 * scale,
                40.0 * scale,
            ],
            text: page.title.to_string(),
            color: color::CHAMPAGNE,
            align: TextAlign::Center,
            font_px: Some(typography::size(typography::H36, h)),
            ..Default::default()
        });

        let subtitle_y = panel_y + 70.0 * scale;
        let copy_end = if self.page == TUTORIAL_PAGE_TILES {
            Self::push_tutorial_tiles_page_copy(&mut texts, panel_x, subtitle_y, panel_w, h)
        } else {
            let subtitle_x = panel_x + 30.0 * scale;
            let subtitle_w = panel_w - 60.0 * scale;
            let subtitle_end = Self::scoring_page_subtitle_end_y(
                subtitle_y,
                panel_w,
                h,
                scale,
                page.subtitle,
            );
            colored_keywords::push_colored_text_block(
                &mut texts,
                [subtitle_x, subtitle_y, subtitle_w, subtitle_end - subtitle_y],
                page.subtitle,
                TextStyle {
                    tier: typography::H36,
                    color: color::PARCHMENT,
                    padding: 0.0,
                    align: TextAlign::Center,
                    ..Default::default()
                },
                h,
            );
            subtitle_end
        };

        let (showcase_tiles, tile_light_y, content_bottom) = if self.page == TUTORIAL_PAGE_TILES {
            let tile_area_y = copy_end + h * 0.012;
            let (placements, labels, bottom) =
                Self::layout_tutorial_tiles_page(page, w, h, tile_area_y, scale);
            Self::draw_tiles_page_group_labels(&mut texts, &mut fg_quads, &labels, scale, h);
            (placements, tile_area_y, bottom)
        } else {
            let tile_area_y = copy_end + h * 0.012;
            let (placements, labels, bottom) =
                Self::layout_demo_page_tiles(page, w, h, tile_area_y, scale);
            Self::draw_tiles_page_group_labels(&mut texts, &mut fg_quads, &labels, scale, h);
            (placements, tile_area_y, bottom)
        };

        let try_it_layout = page.try_it_demo.then(|| {
            Self::compute_try_it_layout(panel_x, panel_w, content_bottom, scale)
        });
        let nav_top = h - (46.0 * scale).max(30.0) - 22.0 * scale;
        let glossary_y = if let Some(ref t) = try_it_layout {
            (t.content_floor_y + 14.0 * scale).min(nav_top - 180.0 * scale)
        } else if self.page == TUTORIAL_PAGE_TILES {
            (content_bottom + 10.0 * scale).min(nav_top - 120.0 * scale)
        } else {
            (content_bottom + 14.0 * scale).min(nav_top - 120.0 * scale)
        };

        if let Some(ref layout) = try_it_layout {
            let heading_y = layout.play_rect[1] - 22.0 * scale;
            let try_it_lift = (28.0 * scale).max(20.0);
            let try_it_world_z_py_nudge = 18.0 * scale;
            let play_focused = self.tree.focused() == Some(TutorialNav::TryPlay.id());
            let play_center_x = layout.play_rect[0] + layout.play_rect[2] * 0.5;
            let play_center_y = layout.play_rect[1] + layout.play_rect[3] * 0.5;
            let trigger_center_x = layout.trigger_rect[0] + layout.trigger_rect[2] * 0.5;
            let trigger_center_y = layout.trigger_rect[1] + layout.trigger_rect[3] * 0.5;
            let wobble_t = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs_f32())
                .unwrap_or(0.0);
            let mirror_diam = layout.play_rect[2]
                .max(layout.play_rect[3] * 1.8)
                .max(72.0 * scale);
            let mp = &self.positions.try_it_mirror;
            let mirror_pos = LayoutAnchorPx {
                px: play_center_x,
                py: play_center_y + try_it_world_z_py_nudge,
                lift_z: try_it_lift,
            }
            .to_draw_cmd_triple();
            mirror_placement = Some(Object3d {
                pos: [
                    mirror_pos[0] + w * mp.nx,
                    mirror_pos[1] + h * mp.ny,
                    mirror_pos[2] + ctx.layout.mm(mp.lift_mm),
                ],
                extents: [mirror_diam, mirror_diam, mirror_diam],
                rotation: [0.0, 0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
                kind: Object3dKind::Mirror {
                    rotation_x_deg: 36.0,
                    rotation_z_deg: (wobble_t * 2.4).sin() * 7.5,
                },
                hover_target: if play_focused { 1.0 } else { 0.0 },
                anim_id: 2,
            });
            let tp = &self.positions.try_it_trigger;
            let trigger_pos = LayoutAnchorPx {
                px: trigger_center_x,
                py: trigger_center_y + try_it_world_z_py_nudge,
                lift_z: try_it_lift,
            }
            .to_draw_cmd_triple();
            wood_tablet_placements.push(Object3d {
                pos: [
                    trigger_pos[0] + w * tp.nx,
                    trigger_pos[1] + h * tp.ny,
                    trigger_pos[2] + ctx.layout.mm(tp.lift_mm),
                ],
                extents: [
                    layout.trigger_rect[2],
                    (layout.trigger_rect[3] * 0.35).max(8.0),
                    layout.trigger_rect[3],
                ],
                rotation: [0.0, 0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
                kind: Object3dKind::WoodTablet {
                    label: std::borrow::Cow::Borrowed("Cash In"),
                    pick_id: None,
                },
                hover_target: 0.0,
                anim_id: 0,
            });
            texts.push(TextLabel {
                rect: [
                    panel_x + 24.0 * scale,
                    heading_y,
                    panel_w - 48.0 * scale,
                    20.0 * scale,
                ],
                text: "Try it (demo)".to_string(),
                color: color::GOLD,
                align: TextAlign::Center,
                font_px: Some(typography::size(typography::H36, h)),
                ..Default::default()
            });
            let note_w = (150.0 * scale).max(120.0);
            let note_h = (54.0 * scale).max(40.0);
            let note_x = (layout.play_rect[0] - note_w - 22.0 * scale).max(panel_x + 28.0 * scale);
            let note_y = layout.play_rect[1] - 10.0 * scale;
            fg_quads.push(GpuInstance {
                rect: [note_x, note_y, note_w, note_h],
                color: color::alpha(color::CHAMPAGNE, 0.16),
                user: 0,
            });
            fg_quads.push(GpuInstance {
                rect: [note_x, note_y, 3.0 * scale, note_h],
                color: color::GOLD,
                user: 0,
            });
            widget::push_text_block(
                &mut texts,
                [
                    note_x + 10.0 * scale,
                    note_y + 8.0 * scale,
                    note_w - 18.0 * scale,
                    note_h - 12.0 * scale,
                ],
                "Note: the bronze mirror is the **Play** button — tap it to play your melds into the structure.",
                TextStyle {
                    tier: typography::H42,
                    color: color::CHAMPAGNE,
                    padding: 0.0,
                    align: TextAlign::Left,
                    ..Default::default()
                },
                h,
            );
            if let Some(line) = Self::try_it_demo_line(self.page, self.try_it_phase) {
                colored_keywords::push_colored_text_block(
                    &mut texts,
                    [
                        panel_x + 24.0 * scale,
                        layout.demo_line_y,
                        panel_w - 48.0 * scale,
                        22.0 * scale,
                    ],
                    line,
                    TextStyle {
                        tier: typography::H42,
                        color: color::CHAMPAGNE,
                        padding: 0.0,
                        align: TextAlign::Center,
                        ..Default::default()
                    },
                    h,
                );
            }
        }

        if !page.glossary.is_empty() {
            texts.push(TextLabel {
                rect: [
                    panel_x + 34.0 * scale,
                    glossary_y,
                    if page.try_it_demo {
                        panel_w * 0.42
                    } else {
                        panel_w * 0.34
                    },
                    24.0 * scale,
                ],
                text: "Key Terms".to_string(),
                color: color::GOLD,
                align: TextAlign::Left,
                font_px: Some(typography::size(typography::H32, h)),
                ..Default::default()
            });
            let term_w = if page.try_it_demo {
                panel_w * 0.42
            } else {
                panel_w * 0.34
            };
            let term_font = typography::size(typography::H42, h);
            let (term_heights, _glossary_total_h) =
                Self::glossary_term_metrics(page.glossary, term_w, term_font, scale);
            let mut gy = glossary_y + 28.0 * scale;
            for (idx, term) in page.glossary.iter().enumerate() {
                let term_h = term_heights.get(idx).copied().unwrap_or(term_font * 1.25);
                colored_keywords::push_colored_text_block(
                    &mut texts,
                    [panel_x + 36.0 * scale, gy, term_w, term_h],
                    term,
                    TextStyle {
                        tier: typography::H42,
                        color: color::STONE,
                        padding: 0.0,
                        align: TextAlign::Left,
                        ..Default::default()
                    },
                    h,
                );
                gy += term_h + 6.0 * scale;
            }
        }

        if let Some(callout) = page.callout {
            let callout_x = if page.try_it_demo {
                panel_x + panel_w * 0.54
            } else if self.page == TUTORIAL_PAGE_TILES {
                panel_x + panel_w * 0.12
            } else {
                panel_x + panel_w * 0.47
            };
            let callout_y = glossary_y + 6.0 * scale;
            let callout_w = if page.try_it_demo {
                panel_w * 0.36
            } else if self.page == TUTORIAL_PAGE_TILES {
                panel_w * 0.76
            } else {
                panel_w * 0.45
            };
            let callout_font = typography::size(typography::H36, h);
            let callout_lines_n = colored_keywords::colored_wrapped_line_count(
                callout,
                callout_w - 32.0 * scale,
                callout_font,
                color::CHAMPAGNE,
            );
            let callout_h =
                (callout_lines_n as f32 * callout_font * 1.3 + 36.0 * scale).max(112.0 * scale);
            fg_quads.push(GpuInstance {
                rect: [callout_x, callout_y, callout_w, callout_h],
                color: color::alpha(color::WALNUT_INK, 0.85),
                user: 0,
            });
            fg_quads.push(GpuInstance {
                rect: [callout_x, callout_y, 4.0 * scale, callout_h],
                color: color::GOLD,
                user: 0,
            });
            colored_keywords::push_colored_text_block(
                &mut texts,
                [
                    callout_x + 18.0 * scale,
                    callout_y + 14.0 * scale,
                    callout_w - 32.0 * scale,
                    callout_h - 28.0 * scale,
                ],
                callout,
                TextStyle {
                    tier: typography::H36,
                    color: color::CHAMPAGNE,
                    padding: 0.0,
                    align: TextAlign::Left,
                    ..Default::default()
                },
                h,
            );
        }

        let items = self.flat_items(w, h);
        let mut buttons = Vec::new();
        for item in &items {
            if matches!(item.action, TutorialNav::TryPlay | TutorialNav::TryTrigger) {
                continue;
            }
            let (label, variant, state) = match item.action {
                TutorialNav::Next => {
                    let label = if self.page + 1 == PAGES.len() {
                        "Start Lesson"
                    } else {
                        "Next"
                    };
                    (label, ButtonVariant::Primary, ButtonState::Rest)
                }
                TutorialNav::Back => ("Back", ButtonVariant::Default, ButtonState::Rest),
                TutorialNav::TryPlay | TutorialNav::TryTrigger => continue,
            };
            widget::push_button(
                &mut fg_quads,
                &mut texts,
                &mut buttons,
                widget::ButtonSpec {
                    rect: item.rect,
                    label,
                    variant,
                    state,
                    action: crate::ui::input::UiAction::Confirm,
                },
            );
        }
        buttons.clear();
        self.tree.register_flat_buttons(&items, &mut buttons);

        if let Some(rect) = match self.tree.focused() {
            Some(id) if id == TutorialNav::TryPlay.id() => ctx.proj.mirror_rect,
            Some(id) if id == TutorialNav::TryTrigger.id() => {
                ctx.proj.wood_tablet_rects.first().copied()
            }
            _ => None,
        } {
            focus_nav::push_focus_ring(rect, scale, w, h, &mut fg_quads);
        }

        frame.quads(bg_quads);
        if !showcase_tiles.is_empty() {
            frame.cmds.push(DrawCmd::ShowcaseTileBatch(showcase_tiles));
        }
        if let Some(mirror) = mirror_placement {
            frame.object3d(mirror);
        }
        if !wood_tablet_placements.is_empty() {
            frame.object3d_batch(wood_tablet_placements);
        }
        // Broad, forgiving lighting for educational showcase objects.
        let light_y = h * 0.18;
        for &(lx, ly, intensity) in &[
            (panel_x + panel_w * 0.24, tile_light_y - 12.0 * scale, 1.95),
            (panel_x + panel_w * 0.50, tile_light_y - 24.0 * scale, 2.15),
            (panel_x + panel_w * 0.76, tile_light_y - 12.0 * scale, 1.95),
            (panel_x + panel_w * 0.34, content_bottom + 8.0 * scale, 1.10),
            (panel_x + panel_w * 0.66, content_bottom + 8.0 * scale, 1.10),
        ] {
            frame.scene_lighting.push_smooth(PointLight {
                pos: [lx, ly, light_y],
                radius: h * 0.95,
                color: color::rgb(color::PARCHMENT),
                intensity,
            });
        }
        frame.quads(fg_quads);
        frame.texts(texts);
        frame.buttons = buttons;
        frame.window_title = format!("Mahjuro — {}", page.title);
        frame
    }
}
