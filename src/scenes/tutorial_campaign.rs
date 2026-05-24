//! Scripted onboarding campaign scenes shown before the tutorial shop.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::audio::SfxId;
use crate::core::tile::{Suit, Tile};
use crate::game::engine::GameEngine;
use crate::game::event_bus::GameEvent;
use crate::persistence::TilePreset;
use crate::render::draw_cmd::{
    CameraParams, DrawCmd, Object3d, Object3dKind, ShowcaseTilePlacement, UiFrame,
};
use crate::render::showcase_tile_layout::{
    ShowcaseTileLabelGaps, showcase_tile_group_label_anchor, showcase_tile_merge_projected_group,
};
use crate::render::theme::{ButtonState, ButtonVariant, color, metrics, typography};
use crate::render::wgpu_renderer::{GpuInstance, PointLight, TextAlign, TextLabel};
use crate::render::world_space::LayoutAnchorPx;
use crate::ui::colored_keywords;
use crate::ui::focus_nav;
use crate::ui::widget::{self, TextStyle};
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use super::tiles_intro_copy;
use super::{BackgroundId, DrawCtx, Scene, SceneBehavior, SceneTransition, UpdateCtx};

const TUTORIAL_TILE_ROTATION: [f32; 3] = [0.0, 0.0, std::f32::consts::PI];

fn tutorial_camera_params(h: f32) -> CameraParams {
    let cam_scale = h / 1600.0;
    CameraParams {
        eye: [0.0, -220.0 * cam_scale, 1960.0 * cam_scale],
        target: [0.0, -40.0 * cam_scale, 0.0],
        up: [0.0, 0.0, 1.0],
        fovy_deg: 45.0,
        clip_near: None,
        clip_far: None,
    }
}

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
    underline_y: f32,
}

/// Shared vertical rhythm for the valid/invalid sequence comparison cards.
/// One source of truth for layout *and* height budgeting so card backgrounds
/// do not reserve space the content path never uses.
struct SequenceCardSpacing {
    pad: f32,
    bottom_pad: f32,
    header_row_h: f32,
    header_to_tile_gap: f32,
    tile_to_caption: f32,
}

impl SequenceCardSpacing {
    fn new(scale: f32, h: f32) -> Self {
        let header_font = typography::size(typography::H42, h);
        Self {
            pad: 10.0 * scale,
            bottom_pad: (8.0 * scale).max(5.0),
            header_row_h: header_font * 1.35,
            // Showcase tiles project above their layout center — leave extra air under the title.
            header_to_tile_gap: (18.0 * scale).max(12.0),
            tile_to_caption: (4.0 * scale).max(2.0),
        }
    }

    fn card_height(&self, tile_size: f32, caption_h: f32) -> f32 {
        self.pad
            + self.header_row_h
            + self.header_to_tile_gap
            + tile_size
            + self.tile_to_caption
            + caption_h
            + self.bottom_pad
    }
}

/// Part 1 — The Tiles (0-based index into `PAGES`).
const TUTORIAL_PAGE_TILES: usize = 0;

const PART1_TILE_GROUPS: &[TileGroup] = &[
    TileGroup {
        label: "Manzu",
        accent: Suit::Manzu.keyword_color(),
        tiles: &[(Suit::Manzu, 1), (Suit::Manzu, 5), (Suit::Manzu, 9)],
        debuffed_visual: false,
    },
    TileGroup {
        label: "Souzu",
        accent: Suit::Souzu.keyword_color(),
        tiles: &[(Suit::Souzu, 1), (Suit::Souzu, 5), (Suit::Souzu, 9)],
        debuffed_visual: false,
    },
    TileGroup {
        label: "Pinzu",
        accent: Suit::Pinzu.keyword_color(),
        tiles: &[(Suit::Pinzu, 1), (Suit::Pinzu, 5), (Suit::Pinzu, 9)],
        debuffed_visual: false,
    },
    TileGroup {
        label: "3-4-5 Manzu",
        accent: [0.35, 0.70, 0.85, 0.9],
        tiles: &[(Suit::Manzu, 3), (Suit::Manzu, 4), (Suit::Manzu, 5)],
        debuffed_visual: false,
    },
    TileGroup {
        label: "Mixed suits",
        accent: color::STONE,
        tiles: &[(Suit::Manzu, 3), (Suit::Souzu, 4), (Suit::Pinzu, 5)],
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
        debuffed_visual: false,
    },
];

/// Part 2 — How to Score (0-based index into `PAGES`).
const TUTORIAL_PAGE_SCORING: usize = 1;

const SCORING_DEMO_GROUPS: &[TileGroup] = &[TileGroup {
    label: "Pair",
    accent: color::CHAMPAGNE,
    tiles: &[(Suit::Pinzu, 5), (Suit::Pinzu, 5)],
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

struct TutorialDemoLayoutPlan {
    terminals_size: f32,
    honor_size: f32,
    /// Gap after the number-suit row, before the sequence comparison cards.
    row_gap: f32,
    /// Extra breathing room before the honor-suit row — sequence cards read as a
    /// distinct block and should not crowd the Winds / Dragons row below.
    sequence_to_honors_gap: f32,
    stack_top: f32,
}

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
        let indices: Vec<usize> = (0..page.groups.len()).collect();
        let max_tile = h * 0.075;
        let tile_size = Self::tutorial_row_tile_size(page, &indices, w, max_tile);
        let center_y = area_top_y + tile_size * 0.5 + h * 0.006;
        let mut next_id = 30_000u32;
        Self::layout_tutorial_tile_row(
            &tutorial_camera_params(h),
            page,
            &indices,
            0.0,
            w,
            w,
            h,
            center_y,
            scale,
            tile_size,
            &mut next_id,
        )
    }

    fn colored_copy_block_height(text: &str, w: f32, tier: f32, h: f32) -> f32 {
        let font = typography::size(tier, h);
        colored_keywords::colored_line_block_height(text, w, font, color::PARCHMENT)
    }

    fn push_colored_copy_block(
        texts: &mut Vec<TextLabel>,
        rect: [f32; 4],
        text: &str,
        tier: f32,
        default_color: [f32; 4],
        h: f32,
    ) -> f32 {
        let block_h = Self::colored_copy_block_height(text, rect[2], tier, h);
        colored_keywords::push_colored_text_block(
            texts,
            [rect[0], rect[1], rect[2], block_h],
            text,
            TextStyle {
                tier,
                color: default_color,
                padding: 0.0,
                align: TextAlign::Left,
                ..Default::default()
            },
            h,
        );
        block_h
    }

    fn tutorial_intro_band_height(copy_w: f32, h: f32) -> f32 {
        let intro_h =
            Self::colored_copy_block_height(tiles_intro_copy::INTRO, copy_w, typography::H36, h);
        intro_h + h * 0.014
    }

    /// Left copy column for Part 1 — intro, number suits, honor suits.
    fn push_tutorial_tiles_copy_column(
        texts: &mut Vec<TextLabel>,
        copy_x: f32,
        content_top: f32,
        content_floor: f32,
        copy_w: f32,
        h: f32,
    ) -> f32 {
        let body_tier = typography::H36;
        let head_tier = typography::H28;
        let block_h =
            |text: &str, tier: f32| Self::colored_copy_block_height(text, copy_w, tier, h);

        let mut natural_h = block_h(tiles_intro_copy::INTRO, body_tier);
        natural_h += block_h(tiles_intro_copy::NUMBER_SUITS_HEADING, head_tier);
        for line in tiles_intro_copy::NUMBER_SUIT_LINES {
            natural_h += block_h(line, body_tier);
        }
        natural_h += block_h(tiles_intro_copy::HONOR_SUITS_HEADING, head_tier);
        for line in tiles_intro_copy::HONOR_LINES {
            natural_h += block_h(line, body_tier);
        }
        natural_h += block_h(tiles_intro_copy::RANK_TERMS_HEADING, head_tier);
        for line in tiles_intro_copy::RANK_TERM_LINES {
            natural_h += block_h(line, body_tier);
        }
        natural_h += block_h(tiles_intro_copy::SEQUENCE_RULES_HEADING, head_tier);
        for line in tiles_intro_copy::SEQUENCE_RULE_LINES {
            natural_h += block_h(line, body_tier);
        }

        let section_gaps = 6;
        let min_section_gap = h * 0.008;
        let copy_bottom_pad = h * 0.012;
        let available = (content_floor - content_top - copy_bottom_pad).max(natural_h);
        let extra = (available - natural_h).max(0.0);
        let section_gap = (extra / section_gaps as f32).max(min_section_gap);
        let mut cursor = if natural_h + section_gap * section_gaps as f32 <= available {
            content_top + (available - natural_h - section_gap * section_gaps as f32) * 0.5
        } else {
            content_top
        };

        cursor += Self::push_colored_copy_block(
            texts,
            [copy_x, cursor, copy_w, 0.0],
            tiles_intro_copy::INTRO,
            body_tier,
            color::PARCHMENT,
            h,
        );
        cursor += section_gap;

        cursor += Self::push_colored_copy_block(
            texts,
            [copy_x, cursor, copy_w, 0.0],
            tiles_intro_copy::NUMBER_SUITS_HEADING,
            head_tier,
            color::CHAMPAGNE,
            h,
        );

        for line in tiles_intro_copy::NUMBER_SUIT_LINES {
            cursor += Self::push_colored_copy_block(
                texts,
                [copy_x, cursor, copy_w, 0.0],
                line,
                body_tier,
                color::PARCHMENT,
                h,
            );
        }
        cursor += section_gap;

        cursor += Self::push_colored_copy_block(
            texts,
            [copy_x, cursor, copy_w, 0.0],
            tiles_intro_copy::HONOR_SUITS_HEADING,
            head_tier,
            color::CHAMPAGNE,
            h,
        );

        for line in tiles_intro_copy::HONOR_LINES {
            cursor += Self::push_colored_copy_block(
                texts,
                [copy_x, cursor, copy_w, 0.0],
                line,
                body_tier,
                color::PARCHMENT,
                h,
            );
        }
        cursor += section_gap;

        cursor += Self::push_colored_copy_block(
            texts,
            [copy_x, cursor, copy_w, 0.0],
            tiles_intro_copy::RANK_TERMS_HEADING,
            head_tier,
            color::CHAMPAGNE,
            h,
        );
        for line in tiles_intro_copy::RANK_TERM_LINES {
            cursor += Self::push_colored_copy_block(
                texts,
                [copy_x, cursor, copy_w, 0.0],
                line,
                body_tier,
                color::PARCHMENT,
                h,
            );
        }
        cursor += section_gap;

        cursor += Self::push_colored_copy_block(
            texts,
            [copy_x, cursor, copy_w, 0.0],
            tiles_intro_copy::SEQUENCE_RULES_HEADING,
            head_tier,
            color::CHAMPAGNE,
            h,
        );
        for line in tiles_intro_copy::SEQUENCE_RULE_LINES {
            cursor += Self::push_colored_copy_block(
                texts,
                [copy_x, cursor, copy_w, 0.0],
                line,
                body_tier,
                color::PARCHMENT,
                h,
            );
        }

        cursor
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

    fn tutorial_row_tile_size(
        page: &TutorialPage,
        indices: &[usize],
        column_w: f32,
        max_tile: f32,
    ) -> f32 {
        let groups: Vec<&TileGroup> = indices.iter().map(|&i| &page.groups[i]).collect();
        let total_tiles: usize = groups.iter().map(|g| g.tiles.len()).sum();
        let num_gaps = groups.len().saturating_sub(1);
        let gap_equiv = num_gaps as f32 * 0.6;
        ((column_w * 0.92) / (total_tiles as f32 + gap_equiv))
            .min(max_tile)
            .max(22.0)
    }

    fn layout_tile_group_at(
        group: &TileGroup,
        start_x: f32,
        center_y: f32,
        tile_size: f32,
        inter_tile_gap: f32,
        next_id: &mut u32,
    ) -> (Vec<ShowcaseTilePlacement>, f32, f32) {
        let total_w =
            group.tiles.len() as f32 * tile_size + (group.tiles.len().saturating_sub(1) as f32) * inter_tile_gap;
        let mut placements = Vec::new();
        let mut cursor_x = start_x;
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
            cursor_x += tile_size + inter_tile_gap;
        }
        (placements, start_x, start_x + total_w)
    }

    fn layout_sequence_comparison_cards(
        page: &TutorialPage,
        col_x: f32,
        col_w: f32,
        card_top_y: f32,
        h: f32,
        scale: f32,
        max_tile: f32,
        stack_vertical: bool,
        next_id: &mut u32,
        fg_quads: &mut Vec<GpuInstance>,
        texts: &mut Vec<TextLabel>,
    ) -> (Vec<ShowcaseTilePlacement>, f32) {
        let header_font = typography::size(typography::H42, h);
        let caption_tier = typography::H36;
        let spacing = SequenceCardSpacing::new(scale, h);
        let (card_w, card_tile_size) =
            Self::sequence_card_tile_metrics(col_w, h, scale, max_tile, stack_vertical);
        let card_gap = if stack_vertical { 0.0 } else { col_w * 0.065 };
        let valid_accent = [0.35, 0.70, 0.85, 0.9];
        let invalid_accent = [0.65, 0.35, 0.35, 0.9];
        const VALID_HEADER: &str = "Valid sequence";
        const INVALID_HEADER: &str = "Invalid mix";

        struct SeqCardSpec<'a> {
            group: &'a TileGroup,
            card_x: f32,
            header: &'static str,
            caption: &'static str,
            fill: [f32; 4],
            border: [f32; 4],
            inter_gap: f32,
        }

        let valid_x = col_x;
        let invalid_x = col_x + card_w + card_gap;
        let specs = if stack_vertical {
            vec![
                SeqCardSpec {
                    group: &page.groups[3],
                    card_x: col_x + col_w * 0.04,
                    header: VALID_HEADER,
                    caption: "3-4-5 Manzu",
                    fill: color::alpha(valid_accent, 0.10),
                    border: valid_accent,
                    inter_gap: card_tile_size * 0.06,
                },
                SeqCardSpec {
                    group: &page.groups[4],
                    card_x: col_x + col_w * 0.04,
                    header: INVALID_HEADER,
                    caption: "3 Manzu - 4 Souzu - 5 Pinzu",
                    fill: color::alpha(color::STONE, 0.12),
                    border: invalid_accent,
                    inter_gap: card_tile_size * 0.12,
                },
            ]
        } else {
            vec![
                SeqCardSpec {
                    group: &page.groups[3],
                    card_x: valid_x,
                    header: VALID_HEADER,
                    caption: "3-4-5 Manzu",
                    fill: color::alpha(valid_accent, 0.10),
                    border: valid_accent,
                    inter_gap: card_tile_size * 0.06,
                },
                SeqCardSpec {
                    group: &page.groups[4],
                    card_x: invalid_x,
                    header: INVALID_HEADER,
                    caption: "3 Manzu - 4 Souzu - 5 Pinzu",
                    fill: color::alpha(color::STONE, 0.12),
                    border: invalid_accent,
                    inter_gap: card_tile_size * 0.12,
                },
            ]
        };

        let mut placements = Vec::new();
        let mut row_bottom = card_top_y;
        let mut stack_y = card_top_y;

        for spec in &specs {
            let spec_caption_h =
                Self::colored_copy_block_height(spec.caption, card_w, caption_tier, h);
            let card_h = spacing.card_height(card_tile_size, spec_caption_h);
            let card_y = if stack_vertical {
                let y = stack_y;
                stack_y = y + card_h + h * 0.018;
                y
            } else {
                card_top_y
            };
            fg_quads.push(GpuInstance {
                rect: [spec.card_x, card_y, card_w, card_h],
                color: spec.fill,
                user: 0,
            });
            fg_quads.push(GpuInstance {
                rect: [
                    spec.card_x,
                    card_y + card_h - (2.0 * scale).max(1.0),
                    card_w,
                    (2.0 * scale).max(1.0),
                ],
                color: spec.border,
                user: 0,
            });
            texts.push(TextLabel {
                rect: [
                    spec.card_x,
                    card_y + spacing.pad,
                    card_w,
                    spacing.header_row_h,
                ],
                text: spec.header.into(),
                color: spec.border,
                align: TextAlign::Center,
                font_px: Some(header_font),
                ..Default::default()
            });
            let tile_center_y = card_y
                + spacing.pad
                + spacing.header_row_h
                + spacing.header_to_tile_gap
                + card_tile_size * 0.5;
            let row_w = spec.group.tiles.len() as f32 * card_tile_size
                + (spec.group.tiles.len().saturating_sub(1) as f32) * spec.inter_gap;
            let tile_start_x = spec.card_x + (card_w - row_w) * 0.5;
            let (group_placements, _, _) = Self::layout_tile_group_at(
                spec.group,
                tile_start_x,
                tile_center_y,
                card_tile_size,
                spec.inter_gap,
                next_id,
            );
            placements.extend(group_placements);
            let caption_y = tile_center_y + card_tile_size * 0.5 + spacing.tile_to_caption;
            colored_keywords::push_colored_text_block(
                texts,
                [spec.card_x, caption_y, card_w, spec_caption_h],
                spec.caption,
                TextStyle {
                    tier: caption_tier,
                    color: color::PARCHMENT,
                    padding: 0.0,
                    align: TextAlign::Center,
                    ..Default::default()
                },
                h,
            );
            row_bottom = row_bottom.max(card_y + card_h);
        }

        if stack_vertical {
            row_bottom += h * 0.012;
        }

        (placements, row_bottom)
    }

    fn tutorial_labeled_row_height(tile_size: f32, scale: f32, h: f32) -> f32 {
        let underline_gap = (8.0 * scale).max(5.0);
        let underline_h = (3.0 * scale).max(2.0);
        let label_gap = (5.0 * scale).max(3.0);
        let label_line_h = typography::size(typography::H42, h) * 1.22;
        tile_size + underline_gap + underline_h + label_gap + label_line_h
    }

    fn sequence_card_tile_metrics(
        col_w: f32,
        h: f32,
        scale: f32,
        max_tile: f32,
        stack_vertical: bool,
    ) -> (f32, f32) {
        let spacing = SequenceCardSpacing::new(scale, h);
        let card_gap = if stack_vertical { 0.0 } else { col_w * 0.065 };
        let card_w = if stack_vertical {
            col_w * 0.92
        } else {
            (col_w - card_gap) * 0.5
        };
        let card_tile_size = ((card_w - spacing.pad * 2.0) / 3.0)
            .min(max_tile * 1.05)
            .max(22.0);
        (card_w, card_tile_size)
    }

    fn sequence_cards_block_height(
        col_w: f32,
        h: f32,
        scale: f32,
        max_tile: f32,
        stack_vertical: bool,
    ) -> f32 {
        let spacing = SequenceCardSpacing::new(scale, h);
        let (card_w, card_tile_size) =
            Self::sequence_card_tile_metrics(col_w, h, scale, max_tile, stack_vertical);
        let caption_block_h = ["3-4-5 Manzu", "3 Manzu / 4 Souzu / 5 Pinzu"]
            .iter()
            .map(|caption| Self::colored_copy_block_height(caption, card_w, typography::H36, h))
            .fold(0.0_f32, f32::max);
        let card_h = spacing.card_height(card_tile_size, caption_block_h);
        if stack_vertical {
            card_h * 2.0 + h * 0.018
        } else {
            card_h
        }
    }

    fn plan_tutorial_demo_layout(
        page: &TutorialPage,
        col_w: f32,
        h: f32,
        scale: f32,
        content_top: f32,
        content_floor: f32,
    ) -> TutorialDemoLayoutPlan {
        let available = (content_floor - content_top).max(h * 0.30);
        let row_gap = (14.0 * scale).max(h * 0.016);
        let sequence_to_honors_gap = (26.0 * scale).max(h * 0.030);
        let stack_vertical = h < 760.0;
        let mut max_tile = (available / 4.4).min(h * 0.054).max(20.0);

        for _ in 0..28 {
            let terminals_size = Self::tutorial_row_tile_size(page, &[0, 1, 2], col_w, max_tile);
            let honor_size = Self::tutorial_row_tile_size(page, &[5, 6], col_w, max_tile);
            let terminals_block_h = Self::tutorial_labeled_row_height(terminals_size, scale, h);
            let seq_block_h =
                Self::sequence_cards_block_height(col_w, h, scale, max_tile, stack_vertical);
            let honor_block_h = Self::tutorial_labeled_row_height(honor_size, scale, h);
            let blocks_total = terminals_block_h + seq_block_h + honor_block_h;
            if blocks_total + row_gap + sequence_to_honors_gap <= available {
                return TutorialDemoLayoutPlan {
                    terminals_size,
                    honor_size,
                    row_gap,
                    sequence_to_honors_gap,
                    stack_top: content_top,
                };
            }
            max_tile *= 0.88;
        }

        let terminals_size = Self::tutorial_row_tile_size(page, &[0, 1, 2], col_w, 20.0);
        let honor_size = Self::tutorial_row_tile_size(page, &[5, 6], col_w, 20.0);
        TutorialDemoLayoutPlan {
            terminals_size,
            honor_size,
            row_gap,
            sequence_to_honors_gap,
            stack_top: content_top,
        }
    }

    /// Part 1 — two-column layout: copy left, tile demos right.
    fn layout_tutorial_tiles_demo_column(
        cam: &CameraParams,
        page: &TutorialPage,
        window_w: f32,
        h: f32,
        col_x: f32,
        col_w: f32,
        content_top: f32,
        content_floor: f32,
        scale: f32,
        fg_quads: &mut Vec<GpuInstance>,
        texts: &mut Vec<TextLabel>,
    ) -> (Vec<ShowcaseTilePlacement>, Vec<TilesPageLabel>, f32, f32) {
        let stack_vertical = h < 760.0;
        let plan = Self::plan_tutorial_demo_layout(page, col_w, h, scale, content_top, content_floor);
        let max_tile = plan.terminals_size.max(plan.honor_size);

        let mut placements = Vec::new();
        let mut labels = Vec::new();
        let mut next_id = 30_000u32;
        let mut cursor = plan.stack_top;

        let terminals_center = cursor + plan.terminals_size * 0.5;
        let (row_placements, row_labels, terminals_bottom) = Self::layout_tutorial_tile_row(
            cam,
            page,
            &[0, 1, 2],
            col_x,
            window_w,
            col_w,
            h,
            terminals_center,
            scale,
            plan.terminals_size,
            &mut next_id,
        );
        placements.extend(row_placements);
        labels.extend(row_labels);
        cursor = terminals_bottom + plan.row_gap;

        let (seq_placements, seq_bottom) = Self::layout_sequence_comparison_cards(
            page,
            col_x,
            col_w,
            cursor,
            h,
            scale,
            max_tile,
            stack_vertical,
            &mut next_id,
            fg_quads,
            texts,
        );
        placements.extend(seq_placements);
        cursor = seq_bottom + plan.sequence_to_honors_gap;

        let honor_center = cursor + plan.honor_size * 0.5;
        let (honor_placements, honor_labels, honor_bottom) = Self::layout_tutorial_tile_row(
            cam,
            page,
            &[5, 6],
            col_x,
            window_w,
            col_w,
            h,
            honor_center,
            scale,
            plan.honor_size,
            &mut next_id,
        );
        placements.extend(honor_placements);
        labels.extend(honor_labels);

        let content_bottom = honor_bottom.min(content_floor);
        let tile_light_y = content_top + (content_bottom - content_top) * 0.35;
        (placements, labels, content_bottom, tile_light_y)
    }

    fn layout_tutorial_tile_row(
        cam: &CameraParams,
        page: &TutorialPage,
        indices: &[usize],
        col_x: f32,
        window_w: f32,
        col_w: f32,
        window_h: f32,
        center_y: f32,
        scale: f32,
        tile_size: f32,
        next_id: &mut u32,
    ) -> (Vec<ShowcaseTilePlacement>, Vec<TilesPageLabel>, f32) {
        let groups: Vec<&TileGroup> = indices.iter().map(|&i| &page.groups[i]).collect();
        let total_tiles: usize = groups.iter().map(|g| g.tiles.len()).sum();
        let num_gaps = groups.len().saturating_sub(1);

        let gap = tile_size * 0.6;

        let total_w = total_tiles as f32 * tile_size + num_gaps as f32 * gap;
        let start_x = col_x + (col_w - total_w) * 0.5;
        let label_gaps = ShowcaseTileLabelGaps {
            underline_gap: (8.0 * scale).max(5.0),
            underline_h: (3.0 * scale).max(2.0),
            label_text_gap: (5.0 * scale).max(3.0),
        };
        let label_line_h = typography::size(typography::H42, window_h) * 1.22;

        let mut placements = Vec::new();
        let mut labels = Vec::new();
        let mut cursor_x = start_x;
        let mut row_bottom = center_y;

        for group in groups {
            let group_start_x = cursor_x;
            let mut centers_xy = Vec::with_capacity(group.tiles.len());
            for &(suit, rank) in group.tiles {
                let px = cursor_x + tile_size * 0.5;
                centers_xy.push([px, center_y]);
                let mut tile = Tile::new(suit, rank, *next_id);
                tile.debuffed_visual = group.debuffed_visual;
                placements.push(ShowcaseTilePlacement {
                    tile,
                    center_pos: [px, center_y, 0.0],
                    rotation: TUTORIAL_TILE_ROTATION,
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
            let bounds = showcase_tile_merge_projected_group(
                cam,
                window_w,
                window_h,
                TilePreset::Chinese,
                TUTORIAL_TILE_ROTATION,
                1.0,
                tile_size,
                0.0,
                &centers_xy,
            );
            let anchor = showcase_tile_group_label_anchor(bounds, label_gaps);
            labels.push(TilesPageLabel {
                x: group_start_x,
                y: anchor.label_y,
                w: group_w,
                text: group.label,
                accent: group.accent,
                underline_y: anchor.underline_y,
            });
            row_bottom = anchor.label_y + label_line_h;
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
            fg_quads.push(GpuInstance {
                rect: [label.x, label.underline_y, label.w, underline_h],
                color: label.accent,
                user: 0,
            });
            texts.push(TextLabel {
                rect: [
                    label.x,
                    label.y,
                    label.w,
                    colored_keywords::colored_row_line_step(label_font),
                ],
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
        let cam = tutorial_camera_params(h);
        frame.camera_override = Some(cam);
        frame.showcase_render_hints.layout_use_ray_plane_z = true;

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

        let title_font = if self.page == TUTORIAL_PAGE_TILES {
            typography::size(typography::H24, h)
        } else {
            typography::size(typography::H36, h)
        };
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
            font_px: Some(title_font),
            ..Default::default()
        });

        let nav_top = h - (46.0 * scale).max(30.0) - 22.0 * scale;
        let content_top = panel_y + panel_h * 0.11;
        let pad_x = panel_w * 0.04;
        let copy_w = panel_w * 0.36;
        let col_gutter = panel_w * 0.05;
        let copy_x = panel_x + pad_x;
        let tile_col_x = copy_x + copy_w + col_gutter;
        let tile_col_w = panel_x + panel_w - pad_x - tile_col_x;
        let intro_band_h = Self::tutorial_intro_band_height(copy_w, h);
        let tile_area_top = content_top + intro_band_h;

        let tiles_callout_h = if self.page == TUTORIAL_PAGE_TILES {
            page.callout.map_or(0.0, |callout| {
                let callout_w = tile_col_w;
                let callout_font = typography::size(typography::H36, h);
                let callout_lines_n = colored_keywords::colored_wrapped_line_count(
                    callout,
                    callout_w - 32.0 * scale,
                    callout_font,
                    color::CHAMPAGNE,
                );
                callout_lines_n as f32 * callout_font * 1.3 + 28.0 * scale
            })
        } else {
            0.0
        };
        let callout_band = tiles_callout_h + 28.0 * scale;
        let content_floor = if self.page == TUTORIAL_PAGE_TILES {
            nav_top - callout_band
        } else {
            nav_top - panel_h * 0.13
        };

        let (showcase_tiles, tile_light_y, content_bottom) = if self.page == TUTORIAL_PAGE_TILES {
            Self::push_tutorial_tiles_copy_column(
                &mut texts,
                copy_x,
                content_top,
                content_floor,
                copy_w,
                h,
            );
            let gutter_x = copy_x + copy_w + col_gutter * 0.5;
            fg_quads.push(GpuInstance {
                rect: [
                    gutter_x,
                    content_top,
                    (1.0 * scale).max(1.0),
                    content_floor - content_top,
                ],
                color: color::alpha(color::BRASS, 0.35),
                user: 0,
            });
            let (placements, labels, bottom, light_y) = Self::layout_tutorial_tiles_demo_column(
                &cam,
                page,
                w,
                h,
                tile_col_x,
                tile_col_w,
                tile_area_top,
                content_floor,
                scale,
                &mut fg_quads,
                &mut texts,
            );
            Self::draw_tiles_page_group_labels(&mut texts, &mut fg_quads, &labels, scale, h);
            (placements, light_y, bottom)
        } else {
            let subtitle_y = panel_y + 70.0 * scale;
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
            let tile_area_y = subtitle_end + h * 0.012;
            let (placements, labels, bottom) =
                Self::layout_demo_page_tiles(page, w, h, tile_area_y, scale);
            Self::draw_tiles_page_group_labels(&mut texts, &mut fg_quads, &labels, scale, h);
            (placements, tile_area_y, bottom)
        };

        let try_it_layout = page.try_it_demo.then(|| {
            Self::compute_try_it_layout(panel_x, panel_w, content_bottom, scale)
        });
        let tiles_page_footer = content_floor;
        let glossary_y = if let Some(ref t) = try_it_layout {
            (t.content_floor_y + 14.0 * scale).min(nav_top - 180.0 * scale)
        } else if self.page == TUTORIAL_PAGE_TILES {
            tiles_page_footer + 20.0 * scale
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
                    glossary_tint: true,
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
                tile_col_x
            } else {
                panel_x + panel_w * 0.47
            };
            let callout_y = if self.page == TUTORIAL_PAGE_TILES {
                glossary_y
            } else {
                glossary_y + 6.0 * scale
            };
            let callout_w = if page.try_it_demo {
                panel_w * 0.36
            } else if self.page == TUTORIAL_PAGE_TILES {
                tile_col_w
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
            let callout_h = if self.page == TUTORIAL_PAGE_TILES {
                callout_lines_n as f32 * callout_font * 1.3 + 28.0 * scale
            } else {
                (callout_lines_n as f32 * callout_font * 1.3 + 36.0 * scale).max(112.0 * scale)
            };
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
                    callout_x + 24.0 * scale,
                    callout_y + 14.0 * scale,
                    callout_w - 40.0 * scale,
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
            let focused = self.tree.focused() == Some(item.id);
            let (label, variant, state) = match item.action {
                TutorialNav::Next => {
                    let label = if self.page + 1 == PAGES.len() {
                        "Start Lesson"
                    } else {
                        "Next"
                    };
                    (
                        label,
                        ButtonVariant::Primary,
                        if focused {
                            ButtonState::Hover
                        } else {
                            ButtonState::Rest
                        },
                    )
                }
                TutorialNav::Back => (
                    "Back",
                    ButtonVariant::Default,
                    if focused {
                        ButtonState::Hover
                    } else {
                        ButtonState::Rest
                    },
                ),
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
            if focused {
                focus_nav::push_focus_ring(item.rect, scale, w, h, &mut fg_quads);
            }
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
        let light_rows: &[(f32, f32, f32)] = if self.page == TUTORIAL_PAGE_TILES {
            &[
                (tile_col_x + tile_col_w * 0.20, tile_light_y - 10.0 * scale, 1.95),
                (tile_col_x + tile_col_w * 0.50, tile_light_y - 22.0 * scale, 2.15),
                (tile_col_x + tile_col_w * 0.80, tile_light_y - 10.0 * scale, 1.95),
            ]
        } else {
            &[
                (panel_x + panel_w * 0.24, tile_light_y - 12.0 * scale, 1.95),
                (panel_x + panel_w * 0.50, tile_light_y - 24.0 * scale, 2.15),
                (panel_x + panel_w * 0.76, tile_light_y - 12.0 * scale, 1.95),
                (panel_x + panel_w * 0.34, content_bottom + 8.0 * scale, 1.10),
                (panel_x + panel_w * 0.66, content_bottom + 8.0 * scale, 1.10),
            ]
        };
        for &(lx, ly, intensity) in light_rows {
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
