//! Scripted onboarding campaign scenes shown before the tutorial shop.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::core::tile::{Suit, Tile};
use crate::game::event_bus::GameEvent;
use crate::persistence::TilePreset;
use crate::render::doc_tile_camera::{DOC_TILE_ROTATION, doc_tile_camera};
use crate::render::draw_cmd::{
    CameraParams, DrawCmd, Object3d, Object3dKind, ShowcaseTilePlacement, UiFrame,
    camera_facing_euler_xyz_rad,
};
use crate::render::showcase_tile_layout::{
    ShowcaseTileLabelGaps, showcase_tile_group_label_anchor, showcase_tile_merge_projected_group,
};
use crate::render::table_transform;
use crate::render::theme::{ButtonState, ButtonVariant, color, metrics, typography};
use crate::render::vocabulary_colors::GlossaryMode;
use crate::render::wgpu_renderer::{GpuInstance, PointLight, TextAlign, TextLabel};
use crate::render::world_space::LayoutAnchorPx;
use crate::sfx_id::SfxId;
use crate::ui::controller_hints::{
    HintStyle, menu_footer_row, push_screen_footer_hint, screen_footer_reserve,
};
use crate::ui::focus_nav;
use crate::ui::placement::PlacementAnchor;
use crate::ui::styled_text;
use crate::ui::widget::{self, TextStyle};
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use super::melds_intro_copy;
use super::scoring_intro_copy;
use super::tiles_intro_copy;
use super::{BackgroundId, DrawCtx, SceneBehavior, SceneIntent, SceneTransition, UpdateCtx};

/// Gaps between intro + three left-panel sections on Part 1 (see guide tiles page).
const TUTORIAL_TILES_COPY_SECTION_GAPS: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TutorialNav {
    Back,
    Next,
    TryPlay,
    TryTrigger,
    TryDiscard,
}

/// Brief feedback after tapping a Part 2 demo prop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TryItFlash {
    Discard,
    Play,
    CashIn,
}

const TRY_IT_FLASH_SECS: f32 = 2.5;

impl TutorialNav {
    fn id(self) -> FocusId {
        FocusId(0x7000 + self as u32)
    }
}

/// Layout for the Discard / Play / Cash In demo strip (matches `draw_frame` geometry).
#[derive(Clone, Copy)]
struct TryItLayout {
    discard_rect: [f32; 4],
    play_rect: [f32; 4],
    trigger_rect: [f32; 4],
    heading_y: f32,
    /// Y position for the one-line demo result (chips × mult = total).
    demo_line_y: f32,
    /// Minimum Y where the bottom row (glossary / callout) may start.
    content_floor_y: f32,
}

/// Two-column grid for Part 2 — How to Score.
struct ScoringPageLayout {
    article_rect: [f32; 4],
    left_col_x: f32,
    left_col_w: f32,
    right_col_x: f32,
    right_col_w: f32,
    melds_heading_rect: [f32; 4],
    pair_row_top: f32,
    pair_tile_light_y: f32,
    try_it: TryItLayout,
    glossary_w: f32,
    glossary_block_h: f32,
    term_heights: Vec<f32>,
    callout_rect: Option<[f32; 4]>,
}

pub struct TutorialCampaignScene {
    page: usize,
    tree: TreeState,
    /// Transient demo-line feedback after tapping Discard / Play / Cash In.
    try_it_flash: Option<TryItFlash>,
    try_it_flash_until: Instant,
    /// Arrange-mode-tunable placements for the shop preview props and the
    /// try-it-demo bowl / mirror / trigger trio.
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
        tiles: &[(Suit::Dragon, 1), (Suit::Dragon, 2), (Suit::Dragon, 3)],
        debuffed_visual: false,
    },
    TileGroup {
        label: "Flowers",
        accent: Suit::Flower.keyword_color(),
        tiles: &[
            (Suit::Flower, 1),
            (Suit::Flower, 2),
            (Suit::Flower, 3),
            (Suit::Flower, 4),
        ],
        debuffed_visual: false,
    },
];

/// Part 2 — How to Score (0-based index into `PAGES`).
const TUTORIAL_PAGE_SCORING: usize = 1;

const SCORING_MELDS_HEADING: &str = melds_intro_copy::PAGE_TITLE;

const SCORING_DEMO_GROUPS: &[TileGroup] = &[
    TileGroup {
        label: "Pair",
        accent: color::CHAMPAGNE,
        tiles: &[(Suit::Pinzu, 5), (Suit::Pinzu, 5)],
        debuffed_visual: false,
    },
    TileGroup {
        label: "Sequence",
        accent: [0.35, 0.70, 0.85, 0.9],
        tiles: &[(Suit::Manzu, 4), (Suit::Manzu, 5), (Suit::Manzu, 6)],
        debuffed_visual: false,
    },
    TileGroup {
        label: "Triplet",
        accent: color::CHAMPAGNE,
        tiles: &[(Suit::Souzu, 7), (Suit::Souzu, 7), (Suit::Souzu, 7)],
        debuffed_visual: false,
    },
    TileGroup {
        label: "Kong",
        accent: [0.85, 0.65, 0.20, 0.9],
        tiles: &[
            (Suit::Wind, 1),
            (Suit::Wind, 1),
            (Suit::Wind, 1),
            (Suit::Wind, 1),
        ],
        debuffed_visual: false,
    },
];

/// Valid / invalid sequence examples — same captions as the guide melds page.
const SCORING_SEQUENCE_GROUPS: &[TileGroup] = &[
    TileGroup {
        label: "Valid sequence",
        accent: [0.35, 0.70, 0.85, 0.9],
        tiles: &[(Suit::Manzu, 3), (Suit::Manzu, 4), (Suit::Manzu, 5)],
        debuffed_visual: false,
    },
    TileGroup {
        label: "Invalid sequence",
        accent: color::STONE,
        tiles: &[(Suit::Manzu, 3), (Suit::Souzu, 4), (Suit::Pinzu, 5)],
        debuffed_visual: false,
    },
];

const PAGES: &[TutorialPage] = &[
    TutorialPage {
        title: "Part 1 — The Tiles",
        subtitle: "",
        glossary: &[],
        callout: Some("Next: melds and how to score."),
        try_it_demo: false,
        groups: PART1_TILE_GROUPS,
    },
    TutorialPage {
        title: "Part 2 — Melds & Scoring",
        subtitle: scoring_intro_copy::SUBTITLE,
        glossary: &[
            melds_intro_copy::PAGE_SUBTITLE,
            "Only number suits can form sequences.",
            "Sequences must stay in one suit.",
            "Honors cannot form sequences.",
            "Structure — played melds that will score when you cash in.",
            "Discard — remove unwanted tiles from your hand. Discards are a limited resource.",
            "Play — play melds to your structure. Plays are a limited resource.",
            scoring_intro_copy::FLOW_REMINDER,
            scoring_intro_copy::FINAL_EQUATION,
        ],
        callout: Some("Your actions are limited — make them count."),
        try_it_demo: true,
        groups: SCORING_DEMO_GROUPS,
    },
];

struct TutorialDemoLayoutPlan {
    terminals_size: f32,
    honor_size: f32,
    /// Gap after the number-suit row, before Winds / Dragons / Flowers.
    row_gap: f32,
    stack_top: f32,
}

/// Part 1 left-column copy: shared measure + draw so intro band and body stay aligned.
struct TutorialTilesCopyLayout {
    line_mul: f32,
    intro_h: f32,
    section_gap: f32,
    start_y: f32,
}

impl Default for TutorialCampaignScene {
    fn default() -> Self {
        Self::new()
    }
}

impl TutorialCampaignScene {
    pub fn new() -> Self {
        Self::with_page(0)
    }

    pub fn with_page(page: usize) -> Self {
        Self {
            page,
            tree: TreeState::new(),
            try_it_flash: None,
            try_it_flash_until: Instant::now(),
            positions: crate::ui::scene_layout::TutorialPositions::default(),
        }
    }

    fn reset_try_it_demo(&mut self) {
        self.try_it_flash = None;
    }

    fn show_try_it_flash(&mut self, kind: TryItFlash) {
        self.try_it_flash = Some(kind);
        self.try_it_flash_until = Instant::now() + Duration::from_secs_f32(TRY_IT_FLASH_SECS);
    }

    fn tick_try_it_flash(&mut self) {
        if self.try_it_flash.is_some() && Instant::now() >= self.try_it_flash_until {
            self.try_it_flash = None;
        }
    }

    fn page(&self) -> &'static TutorialPage {
        &PAGES[self.page.min(PAGES.len() - 1)]
    }

    fn try_it_demo_line(page_index: usize, flash: Option<TryItFlash>) -> Option<&'static str> {
        match (page_index, flash) {
            (TUTORIAL_PAGE_SCORING, Some(TryItFlash::CashIn)) => {
                Some("Demo: 4 **chips** × 3 **mult** = 12")
            }
            (TUTORIAL_PAGE_SCORING, Some(TryItFlash::Play)) => {
                Some("You **Play**ed a meld to your structure.")
            }
            (TUTORIAL_PAGE_SCORING, Some(TryItFlash::Discard)) => {
                Some("Discarded tiles are removed to the river.")
            }
            (TUTORIAL_PAGE_SCORING, None) => {
                Some("These are the **Discard**, **Play**, and **Cash In** buttons. Try them.")
            }
            _ => None,
        }
    }

    fn compute_try_it_layout(
        col_x: f32,
        col_w: f32,
        row_top_y: f32,
        h: f32,
        scale: f32,
        strip_drop: f32,
    ) -> TryItLayout {
        let gap = (16.0 * scale).max(10.0);
        let mut discard_w = (col_w * 0.38).max(78.0 * scale);
        let mut play_w = (col_w * 0.28).max(60.0 * scale);
        let mut tablet_w = (col_w * 0.28).max(58.0 * scale);
        let max_row_w = col_w * 0.98;
        let mut total_w = discard_w + play_w + tablet_w + gap * 2.0;
        if total_w > max_row_w {
            let shrink = max_row_w / total_w;
            discard_w *= shrink;
            play_w *= shrink;
            tablet_w *= shrink;
            total_w = max_row_w;
        }
        let prop_area_h = (h * 0.145).clamp(130.0 * scale, 175.0 * scale);
        let slot_h = prop_area_h;
        let heading_h = 22.0 * scale;
        let demo_line_h = Self::try_it_demo_callout_max_height(col_w, scale, h);
        let heading_y = row_top_y + 4.0 * scale;
        let strip_y = heading_y + heading_h + 4.0 * scale + strip_drop;
        let slot_y = strip_y;
        let left_x = col_x + (col_w - total_w) * 0.5;
        let demo_line_y = strip_y + prop_area_h + 6.0 * scale;
        let content_floor_y = demo_line_y + demo_line_h;
        TryItLayout {
            discard_rect: [left_x, slot_y, discard_w, slot_h],
            play_rect: [left_x + discard_w + gap, slot_y, play_w, slot_h],
            trigger_rect: [
                left_x + discard_w + gap + play_w + gap,
                slot_y,
                tablet_w,
                slot_h,
            ],
            heading_y,
            demo_line_y,
            content_floor_y,
        }
    }

    fn try_it_river_extents(slot_w: f32, slot_h: f32, scale: f32) -> [f32; 3] {
        let size = slot_w.max(slot_h * 0.88);
        let diam = (size * 0.68).clamp(56.0 * scale, 96.0 * scale);
        let river_len = (slot_w * 1.05).min(diam * 1.8);
        [river_len, diam, diam * 1.1]
    }

    fn try_it_mirror_diam(slot_w: f32, slot_h: f32, scale: f32) -> f32 {
        let size = slot_w.max(slot_h * 0.82);
        (size * 0.92).clamp(58.0 * scale, 98.0 * scale)
    }

    /// Wood tablet face size from slot width only — hit rects stay tall; mesh keeps gameplay proportions.
    fn try_it_tablet_extents(slot_w: f32, scale: f32) -> [f32; 3] {
        let face = (slot_w * 0.92).clamp(50.0 * scale, 84.0 * scale);
        let thickness = (face * 0.35).max(10.0);
        [face, thickness, face * 0.94]
    }

    fn scoring_page_subtitle_height(panel_w: f32, h: f32, scale: f32, subtitle: &str) -> f32 {
        let subtitle_w = panel_w - 60.0 * scale;
        styled_text::styled_line_block_height(
            subtitle,
            subtitle_w,
            typography::H36,
            h,
            GlossaryMode::Prose,
            color::PARCHMENT,
        )
        .min(128.0 * scale)
    }

    fn compute_scoring_page_layout(
        page: &TutorialPage,
        w: f32,
        h: f32,
        panel_x: f32,
        panel_y: f32,
        panel_w: f32,
        _panel_h: f32,
    ) -> ScoringPageLayout {
        let scale = metrics::scene_scale(w, h);
        let pad_x = 34.0 * scale;
        let gutter = (panel_w * 0.05).max(18.0 * scale);
        let inner_w = panel_w - pad_x * 2.0;
        let left_col_w = inner_w * 0.44;
        let right_col_w = inner_w - gutter - left_col_w;
        let left_col_x = panel_x + pad_x;
        let right_col_x = left_col_x + left_col_w + gutter;

        let article_y = panel_y + (18.0 + 40.0 + 4.0) * scale;
        let article_w = panel_w - 60.0 * scale;
        let article_x = panel_x + 30.0 * scale;
        let article_h = Self::scoring_page_subtitle_height(panel_w, h, scale, page.subtitle);
        let article_rect = [article_x, article_y, article_w, article_h];

        let section_row_top = article_y + article_h + 6.0 * scale;
        let melds_heading_h = 22.0 * scale;
        let melds_heading_y = section_row_top + 4.0 * scale;
        let pair_row_top = melds_heading_y + melds_heading_h + 4.0 * scale;
        let try_it = Self::compute_try_it_layout(
            right_col_x,
            right_col_w,
            section_row_top,
            h,
            scale,
            28.0 * scale,
        );

        let indices: Vec<usize> = (0..page.groups.len()).collect();
        let max_tile = h * 0.065;
        let tile_size = Self::tutorial_row_tile_size(page, &indices, left_col_w, max_tile);
        let pair_tile_light_y = pair_row_top + tile_size * 0.35;

        let glossary_w = left_col_w - 4.0 * scale;
        let term_font = typography::size(typography::H42, h);
        let (term_heights, terms_h) =
            Self::glossary_term_metrics(page.glossary, glossary_w, term_font, scale);
        let glossary_block_h = 28.0 * scale + 24.0 * scale + terms_h;

        let nav_top = Self::tutorial_nav_top(w, h, scale);
        let callout_rect = page.callout.map(|callout| {
            let callout_w = right_col_w - 8.0 * scale;
            let callout_x = right_col_x + 4.0 * scale;
            let callout_h =
                Self::callout_box_height(callout, callout_w, scale, h, None, typography::H36);
            let callout_y = nav_top - callout_h - 14.0 * scale;
            [callout_x, callout_y, callout_w, callout_h]
        });

        ScoringPageLayout {
            article_rect,
            left_col_x,
            left_col_w,
            right_col_x,
            right_col_w,
            melds_heading_rect: [
                left_col_x + 4.0 * scale,
                melds_heading_y,
                left_col_w - 8.0 * scale,
                melds_heading_h,
            ],
            pair_row_top,
            pair_tile_light_y,
            try_it,
            glossary_w,
            glossary_block_h,
            term_heights,
            callout_rect,
        }
    }

    /// Pair tile showcase for Part 2 (left column).
    fn layout_demo_page_tiles(
        page: &TutorialPage,
        w: f32,
        h: f32,
        col_x: f32,
        col_w: f32,
        area_top_y: f32,
        scale: f32,
    ) -> (Vec<ShowcaseTilePlacement>, Vec<TilesPageLabel>, f32) {
        let indices: Vec<usize> = (0..page.groups.len()).collect();
        let max_tile = h * 0.065;
        let tile_size = Self::tutorial_row_tile_size(page, &indices, col_w, max_tile);
        let center_y = area_top_y + tile_size * 0.5 + h * 0.006;
        let mut next_id = 30_000u32;
        Self::layout_tutorial_tile_row(
            &doc_tile_camera(h),
            page,
            &indices,
            col_x,
            w,
            col_w,
            h,
            center_y,
            scale,
            tile_size,
            &mut next_id,
        )
    }

    fn tutorial_text_style(tier: f32, color: [f32; 4], align: TextAlign) -> TextStyle {
        TextStyle {
            tier,
            color,
            padding: 0.0,
            align,
            glossary: GlossaryMode::Prose,
        }
    }

    fn push_tutorial_text_block(
        texts: &mut Vec<TextLabel>,
        rect: [f32; 4],
        text: &str,
        style: TextStyle,
        h: f32,
    ) {
        widget::push_text_block(texts, rect, text, style, h);
    }

    fn tutorial_text_block_height(text: &str, w: f32, tier: f32, h: f32, color: [f32; 4]) -> f32 {
        styled_text::styled_line_block_height(text, w, tier, h, GlossaryMode::Prose, color)
    }

    /// Inner text width inside a callout box (`draw_frame` uses 24px left + 16px right inset).
    fn callout_text_inner_w(callout_w: f32, scale: f32) -> f32 {
        (callout_w - 40.0 * scale).max(1.0)
    }

    /// Callout background height from wrapped copy — must match the text rect in `push_cta_callout`.
    fn callout_box_height(
        callout: &str,
        callout_w: f32,
        scale: f32,
        h: f32,
        min_h: Option<f32>,
        tier: f32,
    ) -> f32 {
        let inner_w = Self::callout_text_inner_w(callout_w, scale);
        let text_h = styled_text::styled_line_block_height(
            callout,
            inner_w,
            tier,
            h,
            GlossaryMode::Prose,
            color::CHAMPAGNE,
        );
        let box_h = text_h + 28.0 * scale;
        min_h.map(|m| box_h.max(m)).unwrap_or(box_h)
    }

    fn push_cta_callout(
        quads: &mut Vec<GpuInstance>,
        texts: &mut Vec<TextLabel>,
        rect: [f32; 4],
        text: &str,
        tier: f32,
        align: TextAlign,
        h: f32,
        scale: f32,
    ) {
        widget::push_panel_colored(quads, rect, color::WALNUT_SOFT, color::BRASS);
        Self::push_tutorial_text_block(
            texts,
            [
                rect[0] + 24.0 * scale,
                rect[1] + 14.0 * scale,
                rect[2] - 40.0 * scale,
                rect[3] - 28.0 * scale,
            ],
            text,
            Self::tutorial_text_style(tier, color::CHAMPAGNE, align),
            h,
        );
    }

    fn try_it_demo_callout_max_height(col_w: f32, scale: f32, h: f32) -> f32 {
        const LINES: &[&str] = &[
            "These are the **Discard**, **Play**, and **Cash In** buttons. Try them.",
            "Demo: 4 **chips** × 3 **mult** = 12",
            "You **Play**ed a meld to your structure.",
            "Discarded tiles are removed to the river.",
        ];
        let callout_w = col_w - 8.0 * scale;
        LINES
            .iter()
            .map(|line| Self::callout_box_height(line, callout_w, scale, h, None, typography::H42))
            .fold(0.0f32, f32::max)
    }

    fn colored_copy_block_height(text: &str, w: f32, tier: f32, h: f32) -> f32 {
        Self::tutorial_text_block_height(text, w, tier, h, color::PARCHMENT)
    }

    fn tutorial_tiles_copy_line_h(tier: f32, h: f32, line_mul: f32) -> f32 {
        typography::size(tier, h) * line_mul
    }

    fn tutorial_tiles_copy_block_height(
        text: &str,
        copy_w: f32,
        tier: f32,
        h: f32,
        line_mul: f32,
        default_color: [f32; 4],
    ) -> f32 {
        let font_px = Self::tutorial_tiles_copy_line_h(tier, h, line_mul);
        styled_text::styled_line_block_height_at_font_px(
            text,
            copy_w,
            font_px,
            GlossaryMode::Panel,
            default_color,
        )
    }

    fn tutorial_tiles_copy_natural_height(copy_w: f32, h: f32, line_mul: f32) -> f32 {
        let block = |text: &str, tier: f32, color: [f32; 4]| {
            Self::tutorial_tiles_copy_block_height(text, copy_w, tier, h, line_mul, color)
        };
        let mut natural_h = block(tiles_intro_copy::INTRO, typography::H32, color::PARCHMENT);
        natural_h += block(
            tiles_intro_copy::NUMBER_SUITS_HEADING,
            typography::H28,
            color::CHAMPAGNE,
        );
        for line in tiles_intro_copy::NUMBER_SUIT_LINES {
            natural_h += block(line, typography::H32, color::PARCHMENT);
        }
        natural_h += block(
            tiles_intro_copy::HONOR_SUITS_HEADING,
            typography::H28,
            color::CHAMPAGNE,
        );
        for line in tiles_intro_copy::HONOR_LINES {
            natural_h += block(line, typography::H32, color::PARCHMENT);
        }
        natural_h += block(
            tiles_intro_copy::FLOWERS_HEADING,
            typography::H28,
            color::CHAMPAGNE,
        );
        for line in tiles_intro_copy::FLOWER_LINES {
            natural_h += block(line, typography::H32, color::PARCHMENT);
        }
        natural_h
    }

    /// Largest `line_mul` in `[min, max]` whose wrapped copy fits `budget` px tall.
    /// Wrapping is non-linear in `line_mul`, so a single ratio estimate undershoots.
    fn tutorial_tiles_copy_line_mul_for_budget(
        copy_w: f32,
        h: f32,
        budget: f32,
        min: f32,
        max: f32,
    ) -> f32 {
        let mut lo = min;
        let mut hi = max;
        for _ in 0..20 {
            let mid = (lo + hi) * 0.5;
            let natural_h = Self::tutorial_tiles_copy_natural_height(copy_w, h, mid);
            if natural_h <= budget {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        lo
    }

    /// Left copy may extend below the tile-column `content_floor` — the page
    /// callout only occupies the right column, not this text stack.
    fn tutorial_nav_top(window_w: f32, h: f32, scale: f32) -> f32 {
        h - screen_footer_reserve(window_w, h) - (46.0 * scale).max(30.0) - 22.0 * scale
    }

    fn tutorial_tiles_copy_floor(nav_top: f32, scale: f32) -> f32 {
        nav_top - (24.0 * scale).max(18.0)
    }

    fn compute_tutorial_tiles_copy_layout(
        copy_w: f32,
        content_top: f32,
        copy_floor: f32,
        h: f32,
    ) -> TutorialTilesCopyLayout {
        const MIN_LINE_MUL: f32 = 0.55;
        const MAX_LINE_MUL: f32 = 3.0;
        let section_gap = h * 0.006;
        let copy_bottom_pad = h * 0.008;
        let available = (copy_floor - content_top - copy_bottom_pad).max(1.0);
        let gap_stack = section_gap * TUTORIAL_TILES_COPY_SECTION_GAPS as f32;
        let text_budget = (available - gap_stack).max(1.0);

        let line_mul = Self::tutorial_tiles_copy_line_mul_for_budget(
            copy_w,
            h,
            text_budget,
            MIN_LINE_MUL,
            MAX_LINE_MUL,
        );
        let start_y = content_top;
        let intro_h = Self::tutorial_tiles_copy_block_height(
            tiles_intro_copy::INTRO,
            copy_w,
            typography::H32,
            h,
            line_mul,
            color::PARCHMENT,
        );

        TutorialTilesCopyLayout {
            line_mul,
            intro_h,
            section_gap,
            start_y,
        }
    }

    fn tutorial_intro_band_height(layout: &TutorialTilesCopyLayout, h: f32) -> f32 {
        layout.intro_h + h * 0.014
    }

    fn push_tutorial_tiles_copy_line(
        texts: &mut Vec<TextLabel>,
        copy_x: f32,
        cursor: f32,
        copy_w: f32,
        text: &str,
        tier: f32,
        default_color: [f32; 4],
        h: f32,
        line_mul: f32,
    ) -> f32 {
        let line_h = Self::tutorial_tiles_copy_line_h(tier, h, line_mul);
        let block = styled_text::StyledTextBlock::measure_at_font_px(
            text,
            copy_w,
            line_h,
            GlossaryMode::Panel,
            default_color,
        );
        let block_h = block.block_height();
        block.push_at_font_px(
            texts,
            [copy_x, cursor, copy_w, block_h],
            styled_text::StyledBlockStyle {
                tier: typography::H36,
                color: default_color,
                padding: 0.0,
                align: TextAlign::Left,
                glossary: GlossaryMode::Panel,
                vertical_align: None,
            },
        );
        block_h
    }

    /// Left copy column for Part 1 — intro, number suits, honor suits.
    fn push_tutorial_tiles_copy_column(
        texts: &mut Vec<TextLabel>,
        copy_x: f32,
        layout: &TutorialTilesCopyLayout,
        copy_w: f32,
        h: f32,
    ) -> f32 {
        let line_mul = layout.line_mul;
        let section_gap = layout.section_gap;
        let mut cursor = layout.start_y;

        cursor += Self::push_tutorial_tiles_copy_line(
            texts,
            copy_x,
            cursor,
            copy_w,
            tiles_intro_copy::INTRO,
            typography::H32,
            color::PARCHMENT,
            h,
            line_mul,
        );
        cursor += section_gap;

        cursor += Self::push_tutorial_tiles_copy_line(
            texts,
            copy_x,
            cursor,
            copy_w,
            tiles_intro_copy::NUMBER_SUITS_HEADING,
            typography::H28,
            color::CHAMPAGNE,
            h,
            line_mul,
        );
        for line in tiles_intro_copy::NUMBER_SUIT_LINES {
            cursor += Self::push_tutorial_tiles_copy_line(
                texts,
                copy_x,
                cursor,
                copy_w,
                line,
                typography::H32,
                color::PARCHMENT,
                h,
                line_mul,
            );
        }
        cursor += section_gap;

        cursor += Self::push_tutorial_tiles_copy_line(
            texts,
            copy_x,
            cursor,
            copy_w,
            tiles_intro_copy::HONOR_SUITS_HEADING,
            typography::H28,
            color::CHAMPAGNE,
            h,
            line_mul,
        );
        for line in tiles_intro_copy::HONOR_LINES {
            cursor += Self::push_tutorial_tiles_copy_line(
                texts,
                copy_x,
                cursor,
                copy_w,
                line,
                typography::H32,
                color::PARCHMENT,
                h,
                line_mul,
            );
        }
        cursor += section_gap;

        cursor += Self::push_tutorial_tiles_copy_line(
            texts,
            copy_x,
            cursor,
            copy_w,
            tiles_intro_copy::FLOWERS_HEADING,
            typography::H28,
            color::CHAMPAGNE,
            h,
            line_mul,
        );
        for line in tiles_intro_copy::FLOWER_LINES {
            cursor += Self::push_tutorial_tiles_copy_line(
                texts,
                copy_x,
                cursor,
                copy_w,
                line,
                typography::H32,
                color::PARCHMENT,
                h,
                line_mul,
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
            let term_h = styled_text::styled_line_block_height_at_font_px(
                term,
                term_w,
                term_font,
                GlossaryMode::Prose,
                color::STONE,
            );
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
        let panel_h = h * 0.84;
        Self::compute_scoring_page_layout(page, w, h, panel_x, panel_y, panel_w, panel_h).try_it
    }

    fn flat_items(&self, w: f32, h: f32) -> Vec<FlatItem<TutorialNav>> {
        let scale = metrics::scene_scale(w, h);
        let btn_w = (170.0 * scale).max(120.0);
        let btn_h = (46.0 * scale).max(30.0);
        let gap = 14.0 * scale;
        let y = Self::tutorial_nav_top(w, h, scale);
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
                TutorialNav::TryDiscard.id(),
                t.discard_rect,
                TutorialNav::TryDiscard,
            ));
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
        let total_w = group.tiles.len() as f32 * tile_size
            + (group.tiles.len().saturating_sub(1) as f32) * inter_tile_gap;
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
            *next_id += 1;
            cursor_x += tile_size + inter_tile_gap;
        }
        (placements, start_x, start_x + total_w)
    }

    fn layout_sequence_comparison_cards(
        groups: &[TileGroup],
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
        const INVALID_HEADER: &str = "Invalid sequence";

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
                    group: &groups[0],
                    card_x: col_x + col_w * 0.04,
                    header: VALID_HEADER,
                    caption: "3-4-5 Manzu",
                    fill: color::alpha(valid_accent, 0.10),
                    border: valid_accent,
                    inter_gap: card_tile_size * 0.06,
                },
                SeqCardSpec {
                    group: &groups[1],
                    card_x: col_x + col_w * 0.04,
                    header: INVALID_HEADER,
                    caption: "3 Manzu / 4 Souzu / 5 Pinzu",
                    fill: color::alpha(color::STONE, 0.12),
                    border: invalid_accent,
                    inter_gap: card_tile_size * 0.12,
                },
            ]
        } else {
            vec![
                SeqCardSpec {
                    group: &groups[0],
                    card_x: valid_x,
                    header: VALID_HEADER,
                    caption: "3-4-5 Manzu",
                    fill: color::alpha(valid_accent, 0.10),
                    border: valid_accent,
                    inter_gap: card_tile_size * 0.06,
                },
                SeqCardSpec {
                    group: &groups[1],
                    card_x: invalid_x,
                    header: INVALID_HEADER,
                    caption: "3 Manzu / 4 Souzu / 5 Pinzu",
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
            Self::push_tutorial_text_block(
                texts,
                [spec.card_x, caption_y, card_w, spec_caption_h],
                spec.caption,
                Self::tutorial_text_style(caption_tier, color::PARCHMENT, TextAlign::Center),
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
        let mut max_tile = (available / 2.8).min(h * 0.054).max(20.0);

        for _ in 0..28 {
            let terminals_size = Self::tutorial_row_tile_size(page, &[0, 1, 2], col_w, max_tile);
            let honor_size = Self::tutorial_row_tile_size(page, &[3, 4, 5], col_w, max_tile);
            let terminals_block_h = Self::tutorial_labeled_row_height(terminals_size, scale, h);
            let honor_block_h = Self::tutorial_labeled_row_height(honor_size, scale, h);
            let blocks_total = terminals_block_h + row_gap + honor_block_h;
            if blocks_total <= available {
                return TutorialDemoLayoutPlan {
                    terminals_size,
                    honor_size,
                    row_gap,
                    stack_top: content_top,
                };
            }
            max_tile *= 0.88;
        }

        let terminals_size = Self::tutorial_row_tile_size(page, &[0, 1, 2], col_w, 20.0);
        let honor_size = Self::tutorial_row_tile_size(page, &[3, 4, 5], col_w, 20.0);
        TutorialDemoLayoutPlan {
            terminals_size,
            honor_size,
            row_gap,
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
    ) -> (Vec<ShowcaseTilePlacement>, Vec<TilesPageLabel>, f32, f32) {
        let plan =
            Self::plan_tutorial_demo_layout(page, col_w, h, scale, content_top, content_floor);

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

        let honor_center = cursor + plan.honor_size * 0.5;
        let (honor_placements, honor_labels, honor_bottom) = Self::layout_tutorial_tile_row(
            cam,
            page,
            &[3, 4, 5],
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
                    rotation: DOC_TILE_ROTATION,
                    scale: 1.0,
                    size_px: tile_size,
                    brightness: 1.08,
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
                *next_id += 1;
                cursor_x += tile_size;
            }

            let group_w = cursor_x - group_start_x;
            let bounds = showcase_tile_merge_projected_group(
                cam,
                window_w,
                window_h,
                TilePreset::Chinese,
                DOC_TILE_ROTATION,
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
            let label_rect = [
                label.x,
                label.y,
                label.w,
                styled_text::colored_row_line_step(label_font),
            ];
            styled_text::push_colored_line_clipped(
                texts,
                label_rect,
                None,
                label.text,
                color::PARCHMENT,
                label_font,
                TextAlign::Center,
                false,
                GlossaryMode::Prose,
            );
        }
    }
}

impl SceneBehavior for TutorialCampaignScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        self.tick_try_it_flash();
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
                    self.reset_try_it_demo();
                    ctx.bus.push(GameEvent::UiSound(SfxId::TileClick));
                }
                None
            }
            Some(TutorialNav::Next) => {
                ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                if self.page + 1 < PAGES.len() {
                    self.page += 1;
                    self.reset_try_it_demo();
                    ctx.bus.push(GameEvent::UiSound(SfxId::PackBuy));
                    None
                } else {
                    ctx.bus.push(GameEvent::UiSound(SfxId::RelicPickup));
                    Some(SceneIntent::GameplayLessonsFirstChamber)
                }
            }
            Some(TutorialNav::TryPlay) => {
                if !self.page().try_it_demo {
                    return None;
                }
                self.show_try_it_flash(TryItFlash::Play);
                ctx.bus.push(GameEvent::StructureCommitted);
                None
            }
            Some(TutorialNav::TryDiscard) => {
                if !self.page().try_it_demo {
                    return None;
                }
                self.show_try_it_flash(TryItFlash::Discard);
                ctx.bus.push(GameEvent::UiSound(SfxId::TileClick));
                None
            }
            Some(TutorialNav::TryTrigger) => {
                if !self.page().try_it_demo {
                    return None;
                }
                self.show_try_it_flash(TryItFlash::CashIn);
                ctx.bus.push(GameEvent::UiSound(SfxId::ScoreReveal));
                ctx.bus.push(GameEvent::UiSound(SfxId::ScoreFinal));
                None
            }
            None => None,
        }
    }

    fn draw_frame(&self, mut ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let scale = metrics::scene_scale(w, h);
        let page = self.page();

        let mut bg_quads = Vec::new();
        let mut fg_quads = Vec::new();
        let mut texts = Vec::new();
        let mut wood_tablet_placements: Vec<Object3d> = Vec::new();
        let mut bowl_placement: Option<Object3d> = None;
        let mut mirror_placement: Option<Object3d> = None;
        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);
        if ctx.effect_layers.starfield {
            frame.starfield();
        }
        if ctx.effect_layers.golden_dust {
            frame.golden_dust();
        }
        let cam = doc_tile_camera(h);
        frame.camera_override = Some(cam);
        frame.showcase_render_hints.layout_use_ray_plane_z = true;
        frame.showcase_render_hints.doc_tile_no_shadow = true;

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

        let nav_top = Self::tutorial_nav_top(w, h, scale);
        let content_top = panel_y + panel_h * 0.11;
        let pad_x = panel_w * 0.04;
        // Wider copy column — fewer wraps so body text can scale up to fill height.
        let copy_frac = if h < 820.0 { 0.46 } else { 0.40 };
        let copy_w = panel_w * copy_frac;
        let col_gutter = panel_w * 0.05;
        let copy_x = panel_x + pad_x;
        let tile_col_x = copy_x + copy_w + col_gutter;
        let tile_col_w = panel_x + panel_w - pad_x - tile_col_x;

        let tiles_callout_h = if self.page == TUTORIAL_PAGE_TILES {
            page.callout.map_or(0.0, |callout| {
                Self::callout_box_height(callout, tile_col_w, scale, h, None, typography::H36)
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

        let copy_floor = Self::tutorial_tiles_copy_floor(nav_top, scale);
        let tiles_copy_layout = if self.page == TUTORIAL_PAGE_TILES {
            Some(Self::compute_tutorial_tiles_copy_layout(
                copy_w,
                content_top,
                copy_floor,
                h,
            ))
        } else {
            None
        };
        let intro_band_h = tiles_copy_layout
            .as_ref()
            .map(|layout| Self::tutorial_intro_band_height(layout, h))
            .unwrap_or(0.0);
        let tile_area_top = content_top + intro_band_h;

        let scoring_layout = if self.page == TUTORIAL_PAGE_SCORING && page.try_it_demo {
            Some(Self::compute_scoring_page_layout(
                page, w, h, panel_x, panel_y, panel_w, panel_h,
            ))
        } else {
            None
        };

        let (showcase_tiles, tile_light_y, content_bottom) = if self.page == TUTORIAL_PAGE_TILES {
            let copy_layout = tiles_copy_layout.as_ref().expect("tiles page layout");
            Self::push_tutorial_tiles_copy_column(&mut texts, copy_x, copy_layout, copy_w, h);
            let gutter_x = copy_x + copy_w + col_gutter * 0.5;
            fg_quads.push(GpuInstance {
                rect: [
                    gutter_x,
                    content_top,
                    (1.0 * scale).max(1.0),
                    copy_floor - content_top,
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
            );
            Self::draw_tiles_page_group_labels(&mut texts, &mut fg_quads, &labels, scale, h);
            (placements, light_y, bottom)
        } else if let Some(ref scoring) = scoring_layout {
            let [article_x, article_y, article_w, article_h] = scoring.article_rect;
            Self::push_tutorial_text_block(
                &mut texts,
                [article_x, article_y, article_w, article_h],
                page.subtitle,
                Self::tutorial_text_style(typography::H36, color::PARCHMENT, TextAlign::Center),
                h,
            );
            let gutter_mid = scoring.left_col_x
                + scoring.left_col_w
                + (scoring.right_col_x - scoring.left_col_x - scoring.left_col_w) * 0.5;
            let [mh_x, mh_y, mh_w, mh_h] = scoring.melds_heading_rect;
            texts.push(TextLabel {
                rect: [mh_x, mh_y, mh_w, mh_h],
                text: SCORING_MELDS_HEADING.to_string(),
                color: color::GOLD,
                align: TextAlign::Center,
                font_px: Some(typography::size(typography::H36, h)),
                ..Default::default()
            });
            let (placements, labels, meld_bottom) = Self::layout_demo_page_tiles(
                page,
                w,
                h,
                scoring.left_col_x,
                scoring.left_col_w,
                scoring.pair_row_top,
                scale,
            );
            let stack_vertical = h < 760.0;
            let max_tile = h * 0.065;
            let mut seq_tile_id = 40_000u32;
            let mut showcase_tiles = placements;
            let (seq_placements, seq_bottom) = Self::layout_sequence_comparison_cards(
                SCORING_SEQUENCE_GROUPS,
                scoring.left_col_x,
                scoring.left_col_w,
                meld_bottom + 10.0 * scale,
                h,
                scale,
                max_tile,
                stack_vertical,
                &mut seq_tile_id,
                &mut fg_quads,
                &mut texts,
            );
            showcase_tiles.extend(seq_placements);
            let content_bottom = seq_bottom.max(meld_bottom);
            let gutter_bottom = scoring
                .callout_rect
                .map(|r| r[1] + r[3])
                .unwrap_or(scoring.try_it.content_floor_y)
                .max(content_bottom + 12.0 * scale + scoring.glossary_block_h);
            fg_quads.push(GpuInstance {
                rect: [
                    gutter_mid - (0.5 * scale).max(1.0),
                    scoring.pair_row_top,
                    (1.0 * scale).max(1.0),
                    gutter_bottom - scoring.pair_row_top + 6.0 * scale,
                ],
                color: color::alpha(color::BRASS, 0.35),
                user: 0,
            });
            Self::draw_tiles_page_group_labels(&mut texts, &mut fg_quads, &labels, scale, h);
            (showcase_tiles, scoring.pair_tile_light_y, content_bottom)
        } else {
            unreachable!("scoring page uses scoring_layout")
        };

        let try_it_layout = scoring_layout.as_ref().map(|s| s.try_it);
        let tiles_page_footer = content_floor;
        let scoring_glossary_metrics = scoring_layout
            .as_ref()
            .map(|s| (s.term_heights.clone(), s.term_heights.iter().sum::<f32>()));
        let glossary_y = if scoring_layout.is_some() {
            content_bottom + 12.0 * scale
        } else if let Some(ref t) = try_it_layout {
            t.content_floor_y + 10.0 * scale
        } else if self.page == TUTORIAL_PAGE_TILES {
            tiles_page_footer + 20.0 * scale
        } else {
            (content_bottom + 14.0 * scale).min(nav_top - 120.0 * scale)
        };

        if let (Some(ref layout), Some(scoring)) = (try_it_layout, scoring_layout.as_ref()) {
            let heading_h = 22.0 * scale;
            let try_it_lift = (28.0 * scale).max(20.0);
            let try_it_world_z_py_nudge = 18.0 * scale;
            let discard_focused = self.tree.focused() == Some(TutorialNav::TryDiscard.id());
            let play_focused = self.tree.focused() == Some(TutorialNav::TryPlay.id());
            let discard_center_x = layout.discard_rect[0] + layout.discard_rect[2] * 0.5;
            let discard_center_y = layout.discard_rect[1] + layout.discard_rect[3] * 0.5;
            let play_center_x = layout.play_rect[0] + layout.play_rect[2] * 0.5;
            let play_center_y = layout.play_rect[1] + layout.play_rect[3] * 0.5;
            let trigger_center_x = layout.trigger_rect[0] + layout.trigger_rect[2] * 0.5;
            let trigger_center_y = layout.trigger_rect[1] + layout.trigger_rect[3] * 0.5;
            let wobble_t = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs_f32())
                .unwrap_or(0.0);
            let bowl_extents =
                Self::try_it_river_extents(layout.discard_rect[2], layout.discard_rect[3], scale);
            let bp = &self.positions.try_it_bowl;
            let bowl_anchor_px = LayoutAnchorPx {
                px: discard_center_x,
                py: discard_center_y + try_it_world_z_py_nudge,
                lift_z: try_it_lift,
            }
            .to_draw_cmd_triple();
            let bowl_anchor = PlacementAnchor::new(
                [
                    bowl_anchor_px[0] + w * bp.nx,
                    bowl_anchor_px[1] + h * bp.ny,
                    bowl_anchor_px[2] + ctx.layout.mm(bp.lift_mm),
                ],
                table_transform::rot_fixed_axes_deg(90.0, 0.0, 0.0),
                bp,
                ctx.layout,
            );
            bowl_placement = Some(Object3d {
                pos: bowl_anchor.pos,
                extents: bowl_extents,
                rotation: bowl_anchor.object3d_rotation(),
                color: [1.0, 1.0, 1.0, 1.0],
                kind: Object3dKind::Bowl,
                hover_target: if discard_focused { 1.0 } else { 0.0 },
                anim_id: 1,
            });
            let mirror_diam =
                Self::try_it_mirror_diam(layout.play_rect[2], layout.play_rect[3], scale);
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
                rotation: crate::render::table_transform::euler_xyz_rad_from_deg(
                    36.0,
                    0.0,
                    (wobble_t * 2.4).sin() * 7.5,
                ),
                color: [1.0, 1.0, 1.0, 1.0],
                kind: Object3dKind::Mirror {
                    valid_play_glow: 0.0,
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
            let cam_euler = camera_facing_euler_xyz_rad(cam.eye, cam.target);
            let cam_m =
                table_transform::rot_euler_xyz_rad(cam_euler[0], cam_euler[1], cam_euler[2]);
            let tablet_extents = Self::try_it_tablet_extents(layout.trigger_rect[2], scale);
            wood_tablet_placements.push(Object3d {
                pos: [
                    trigger_pos[0] + w * tp.nx,
                    trigger_pos[1] + h * tp.ny,
                    trigger_pos[2] + ctx.layout.mm(tp.lift_mm),
                ],
                extents: tablet_extents,
                rotation: table_transform::compose_rotation_euler(cam_m, tp.rotation_deg()),
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
                    scoring.right_col_x + 4.0 * scale,
                    layout.heading_y,
                    scoring.right_col_w - 8.0 * scale,
                    heading_h,
                ],
                text: "Your implements of victory".to_string(),
                color: color::GOLD,
                align: TextAlign::Center,
                font_px: Some(typography::size(typography::H36, h)),
                ..Default::default()
            });

            if let Some(line) = Self::try_it_demo_line(self.page, self.try_it_flash) {
                let callout_w = scoring.right_col_w - 8.0 * scale;
                let callout_x = scoring.right_col_x + 4.0 * scale;
                let callout_h =
                    Self::callout_box_height(line, callout_w, scale, h, None, typography::H42);
                Self::push_cta_callout(
                    &mut fg_quads,
                    &mut texts,
                    [callout_x, layout.demo_line_y, callout_w, callout_h],
                    line,
                    typography::H42,
                    TextAlign::Center,
                    h,
                    scale,
                );
            }
        }

        if !page.glossary.is_empty() {
            let (term_w, term_heights) = if let Some((heights, _)) = scoring_glossary_metrics {
                if let Some(ref s) = scoring_layout {
                    (s.glossary_w, heights)
                } else {
                    (panel_w - 68.0 * scale, heights)
                }
            } else {
                let term_w = if page.try_it_demo {
                    panel_w * 0.42
                } else {
                    panel_w * 0.34
                };
                let term_font = typography::size(typography::H42, h);
                let (heights, _) =
                    Self::glossary_term_metrics(page.glossary, term_w, term_font, scale);
                (term_w, heights)
            };
            let glossary_x = scoring_layout
                .as_ref()
                .map(|s| s.left_col_x + 2.0 * scale)
                .unwrap_or(panel_x + 34.0 * scale);
            texts.push(TextLabel {
                rect: [glossary_x, glossary_y, term_w, 24.0 * scale],
                text: "Key Terms".to_string(),
                color: color::GOLD,
                align: TextAlign::Left,
                font_px: Some(typography::size(typography::H32, h)),
                ..Default::default()
            });
            let term_font = typography::size(typography::H42, h);
            let mut gy = glossary_y + 28.0 * scale;
            for (idx, term) in page.glossary.iter().enumerate() {
                let term_h = term_heights.get(idx).copied().unwrap_or(term_font * 1.25);
                Self::push_tutorial_text_block(
                    &mut texts,
                    [glossary_x + 2.0 * scale, gy, term_w, term_h],
                    term,
                    Self::tutorial_text_style(typography::H42, color::STONE, TextAlign::Left),
                    h,
                );
                gy += term_h + 6.0 * scale;
            }
        }

        if let Some(callout) = page.callout {
            let (callout_x, callout_y, callout_w, callout_h) = if let Some(rect) =
                scoring_layout.as_ref().and_then(|s| s.callout_rect)
            {
                (rect[0], rect[1], rect[2], rect[3])
            } else {
                let callout_x = if self.page == TUTORIAL_PAGE_TILES {
                    tile_col_x
                } else {
                    panel_x + panel_w * 0.47
                };
                let callout_y = if self.page == TUTORIAL_PAGE_TILES {
                    glossary_y
                } else {
                    glossary_y + 6.0 * scale
                };
                let callout_w = if self.page == TUTORIAL_PAGE_TILES {
                    tile_col_w
                } else {
                    panel_w * 0.45
                };
                let min_h = if self.page == TUTORIAL_PAGE_TILES {
                    None
                } else {
                    Some(112.0 * scale)
                };
                let callout_h =
                    Self::callout_box_height(callout, callout_w, scale, h, min_h, typography::H36);
                (callout_x, callout_y, callout_w, callout_h)
            };
            Self::push_cta_callout(
                &mut fg_quads,
                &mut texts,
                [callout_x, callout_y, callout_w, callout_h],
                callout,
                typography::H36,
                TextAlign::Left,
                h,
                scale,
            );
        }

        let items = self.flat_items(w, h);
        let mut buttons = Vec::new();
        for item in &items {
            if matches!(
                item.action,
                TutorialNav::TryDiscard | TutorialNav::TryPlay | TutorialNav::TryTrigger
            ) {
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
                TutorialNav::TryDiscard | TutorialNav::TryPlay | TutorialNav::TryTrigger => {
                    continue;
                }
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
            Some(id) if id == TutorialNav::TryDiscard.id() => ctx.proj.bowl_rect,
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
            frame
                .cmds
                .push(DrawCmd::ShowcaseTileBatch(showcase_tiles.into()));
        }
        if let Some(bowl) = bowl_placement {
            frame.object3d(bowl);
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
                (
                    tile_col_x + tile_col_w * 0.20,
                    tile_light_y - 10.0 * scale,
                    1.95,
                ),
                (
                    tile_col_x + tile_col_w * 0.50,
                    tile_light_y - 22.0 * scale,
                    2.15,
                ),
                (
                    tile_col_x + tile_col_w * 0.80,
                    tile_light_y - 10.0 * scale,
                    1.95,
                ),
            ]
        } else if let (Some(ref layout), Some(scoring)) = (try_it_layout, scoring_layout.as_ref()) {
            let prop_light_y = layout.discard_rect[1] + layout.discard_rect[3] * 0.35;
            &[
                (
                    scoring.left_col_x + scoring.left_col_w * 0.5,
                    tile_light_y - 8.0 * scale,
                    1.95,
                ),
                (
                    layout.discard_rect[0] + layout.discard_rect[2] * 0.5,
                    prop_light_y,
                    1.20,
                ),
                (
                    layout.play_rect[0] + layout.play_rect[2] * 0.5,
                    prop_light_y,
                    1.35,
                ),
                (
                    layout.trigger_rect[0] + layout.trigger_rect[2] * 0.5,
                    prop_light_y,
                    1.20,
                ),
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
        push_screen_footer_hint(
            &mut frame,
            &ctx,
            menu_footer_row(ctx.input_mode),
            HintStyle::standard(w, h),
        );
        frame.window_title = format!("Mahjuro — {}", page.title);
        ctx.stash_focus_nav_tree_flat(&self.tree, &items, |a| format!("{a:?}"));
        frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiles_page_copy_metrics(w: f32, h: f32) -> (f32, f32, f32, f32) {
        let scale = metrics::scene_scale(w, h);
        let panel_y = h * 0.07;
        let panel_w = w * 0.88;
        let panel_h = h * 0.84;
        let nav_top = TutorialCampaignScene::tutorial_nav_top(w, h, scale);
        let content_top = panel_y + panel_h * 0.11;
        let copy_frac = if h < 820.0 { 0.46 } else { 0.40 };
        let copy_w = panel_w * copy_frac;
        let copy_floor = TutorialCampaignScene::tutorial_tiles_copy_floor(nav_top, scale);
        (copy_w, content_top, copy_floor, nav_top)
    }

    #[test]
    fn tiles_copy_layout_fits_short_window() {
        let h = 720.0;
        let (copy_w, content_top, copy_floor, _) = tiles_page_copy_metrics(1280.0, h);

        let layout = TutorialCampaignScene::compute_tutorial_tiles_copy_layout(
            copy_w,
            content_top,
            copy_floor,
            h,
        );
        let natural_h =
            TutorialCampaignScene::tutorial_tiles_copy_natural_height(copy_w, h, layout.line_mul);
        let copy_bottom_pad = h * 0.008;
        let available = copy_floor - content_top - copy_bottom_pad;
        let total = natural_h + layout.section_gap * TUTORIAL_TILES_COPY_SECTION_GAPS as f32;
        let fill = total / available;
        assert!(
            total <= available + 1.0,
            "copy should fit: total={total} available={available} line_mul={}",
            layout.line_mul
        );
        assert!(
            fill > 0.90,
            "short window should use most of the column (fill={fill:.3}, line_mul={})",
            layout.line_mul
        );
    }

    #[test]
    fn tiles_copy_layout_fills_column() {
        let h = 1080.0;
        let (copy_w, content_top, copy_floor, _) = tiles_page_copy_metrics(1920.0, h);

        let layout = TutorialCampaignScene::compute_tutorial_tiles_copy_layout(
            copy_w,
            content_top,
            copy_floor,
            h,
        );
        let natural_h =
            TutorialCampaignScene::tutorial_tiles_copy_natural_height(copy_w, h, layout.line_mul);
        let copy_bottom_pad = h * 0.008;
        let available = copy_floor - content_top - copy_bottom_pad;
        let used = natural_h + layout.section_gap * 6.0;
        let fill = used / available;
        assert!(
            fill > 0.92,
            "copy should use most of the column (fill={fill:.3}, line_mul={})",
            layout.line_mul
        );
        assert_eq!(layout.start_y, content_top);
    }

    #[test]
    fn tiles_copy_floor_extends_past_tile_content_floor() {
        let h = 1080.0;
        let scale = metrics::scene_scale(1920.0, h);
        let (_, _, copy_floor, nav_top) = tiles_page_copy_metrics(1920.0, h);
        let callout_band = 28.0 * scale + 60.0;
        let tile_content_floor = nav_top - callout_band;
        assert!(
            copy_floor > tile_content_floor + 40.0,
            "left copy should not reserve the right-column callout band"
        );
    }

    #[test]
    fn scoring_page_callout_clears_nav_buttons() {
        let w = 1920.0;
        let h = 1080.0;
        let scale = metrics::scene_scale(w, h);
        let page = &PAGES[TUTORIAL_PAGE_SCORING];
        let panel_x = w * 0.06;
        let panel_y = h * 0.07;
        let panel_w = w * 0.88;
        let panel_h = h * 0.84;
        let nav_top = TutorialCampaignScene::tutorial_nav_top(w, h, scale);
        let layout = TutorialCampaignScene::compute_scoring_page_layout(
            page, w, h, panel_x, panel_y, panel_w, panel_h,
        );
        let [_, callout_y, _, callout_h] = layout.callout_rect.expect("callout rect");
        assert!(
            callout_y + callout_h <= nav_top - 14.0 * scale + 0.5,
            "callout bottom={} should sit above nav top={nav_top}",
            callout_y + callout_h,
        );
    }

    #[test]
    fn scoring_page_callout_fits_wrapped_text() {
        let w = 1920.0;
        let h = 1080.0;
        let scale = metrics::scene_scale(w, h);
        let page = &PAGES[TUTORIAL_PAGE_SCORING];
        let callout = page.callout.expect("scoring page callout");
        let panel_x = w * 0.06;
        let panel_y = h * 0.07;
        let panel_w = w * 0.88;
        let panel_h = h * 0.84;
        let layout = TutorialCampaignScene::compute_scoring_page_layout(
            page, w, h, panel_x, panel_y, panel_w, panel_h,
        );
        let [_, _, callout_w, callout_h] = layout.callout_rect.expect("callout rect");
        let inner_w = TutorialCampaignScene::callout_text_inner_w(callout_w, scale);
        let inner_h = (callout_h - 28.0 * scale).max(1.0);
        let font_px = typography::size(typography::H36, h);
        let block = styled_text::StyledTextBlock::measure_at_font_px(
            callout,
            inner_w,
            font_px,
            GlossaryMode::Prose,
            color::CHAMPAGNE,
        );
        assert!(
            inner_h + 0.01 >= block.block_height(),
            "callout box should fit all wrapped lines (inner_h={inner_h}, need={}, line_count={})",
            block.block_height(),
            block.line_count(),
        );
    }

    #[test]
    fn try_it_demo_line_flash_then_default() {
        let default = TutorialCampaignScene::try_it_demo_line(TUTORIAL_PAGE_SCORING, None)
            .expect("default line");
        assert!(default.contains("Try them"));
        let discard = TutorialCampaignScene::try_it_demo_line(
            TUTORIAL_PAGE_SCORING,
            Some(TryItFlash::Discard),
        )
        .expect("discard flash");
        assert!(discard.contains("river"));
        assert_ne!(discard, default);
    }
}
