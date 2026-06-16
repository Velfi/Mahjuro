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
    HintStyle, guide_footer_row, menu_footer_row, push_screen_footer_hint, screen_footer_top,
};
use crate::ui::focus_nav;
use crate::ui::input::{InputMode, UiAction};
use crate::ui::placement::PlacementAnchor;
use crate::ui::styled_text;
use crate::ui::widget::{self, TextStyle};
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use super::guide::{GuideLayout, GuideNavHeader, guide_nav_header, push_guide_chrome};
use super::header_chrome::HeaderChromeMetrics;
use super::tutorial_intro_copy::{campaign, melds, scoring, tiles, try_it};
use super::{BackgroundId, DrawCtx, SceneBehavior, SceneIntent, SceneTransition, UpdateCtx};

/// Gaps between intro + two left-panel sections on tiles intro page.
const TUTORIAL_TILES_INTRO_COPY_SECTION_GAPS: usize = 0;

/// Gaps between honor + flower sections on the honors page.
const TUTORIAL_TILES_HONORS_COPY_SECTION_GAPS: usize = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TutorialPageKind {
    TilesIntro,
    TilesHonors,
    Melds,
    ScoringTerms,
    TryIt,
}

/// Large bottom navigation bar — Back / Previous / Next(or Start) + page counter.
struct TutorialBottomNav {
    back: [f32; 4],
    prev: [f32; 4],
    next: [f32; 4],
    counter: [f32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TutorialNav {
    Prev,
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
    kind: TutorialPageKind,
    title: &'static str,
    subtitle: &'static str,
    glossary: &'static [&'static str],
    callout: Option<&'static str>,
    /// Interactive Play → Trigger demo strip with fake totals.
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


#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TilesCopySections {
    Intro,
    HonorsAndFlowers,
}

macro_rules! suit_ranks_tiles {
    ($suit:expr) => {
        &[
            ($suit, 1),
            ($suit, 2),
            ($suit, 3),
            ($suit, 4),
            ($suit, 5),
            ($suit, 6),
            ($suit, 7),
            ($suit, 8),
            ($suit, 9),
        ]
    };
}

const TILES_NUMBER_GROUPS: &[TileGroup] = &[
    TileGroup {
        label: "Manzu",
        accent: Suit::Manzu.keyword_color(),
        tiles: suit_ranks_tiles!(Suit::Manzu),
        debuffed_visual: false,
    },
    TileGroup {
        label: "Souzu",
        accent: Suit::Souzu.keyword_color(),
        tiles: suit_ranks_tiles!(Suit::Souzu),
        debuffed_visual: false,
    },
    TileGroup {
        label: "Pinzu",
        accent: Suit::Pinzu.keyword_color(),
        tiles: suit_ranks_tiles!(Suit::Pinzu),
        debuffed_visual: false,
    },
];

const TILES_HONOR_GROUPS: &[TileGroup] = &[
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

const PAGES: &[TutorialPage] = &[
    TutorialPage {
        kind: TutorialPageKind::TilesIntro,
        title: campaign::PAGE_TILES_INTRO_TITLE,
        subtitle: "",
        glossary: &[],
        callout: Some(campaign::PAGE_NAV_CALLOUT),
        try_it_demo: false,
        groups: TILES_NUMBER_GROUPS,
    },
    TutorialPage {
        kind: TutorialPageKind::TilesHonors,
        title: campaign::PAGE_TILES_HONORS_TITLE,
        subtitle: "",
        glossary: &[],
        callout: Some(campaign::PAGE_NAV_CALLOUT),
        try_it_demo: false,
        groups: TILES_HONOR_GROUPS,
    },
    TutorialPage {
        kind: TutorialPageKind::Melds,
        title: campaign::PAGE_MELDS_TITLE,
        subtitle: melds::PAGE_SUBTITLE,
        glossary: &[],
        callout: None,
        try_it_demo: false,
        groups: SCORING_DEMO_GROUPS,
    },
    TutorialPage {
        kind: TutorialPageKind::ScoringTerms,
        title: campaign::PAGE_SCORING_TITLE,
        subtitle: scoring::SUBTITLE,
        glossary: campaign::SCORING_GLOSSARY,
        callout: None,
        try_it_demo: false,
        groups: &[],
    },
    TutorialPage {
        kind: TutorialPageKind::TryIt,
        title: campaign::PAGE_TRY_IT_TITLE,
        subtitle: try_it::PROMPT,
        glossary: &[],
        callout: Some(campaign::PAGE_START_CALLOUT),
        try_it_demo: true,
        groups: &[],
    },
];

const SCORING_MELDS_HEADING: &str = melds::PAGE_TITLE;

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
        label: melds::VALID_SEQUENCE,
        accent: [0.35, 0.70, 0.85, 0.9],
        tiles: &[(Suit::Manzu, 3), (Suit::Manzu, 4), (Suit::Manzu, 5)],
        debuffed_visual: false,
    },
    TileGroup {
        label: melds::INVALID_SEQUENCE,
        accent: color::STONE,
        tiles: &[(Suit::Manzu, 3), (Suit::Souzu, 4), (Suit::Pinzu, 5)],
        debuffed_visual: false,
    },
];

fn tutorial_page_subtitle(page: &TutorialPage) -> Option<&str> {
    if page.subtitle.is_empty() {
        None
    } else {
        Some(page.subtitle)
    }
}

fn tutorial_bottom_nav(w: f32, h: f32) -> (TutorialBottomNav, f32) {
    let scale = metrics::scene_scale(w, h);
    let chrome = HeaderChromeMetrics::from_window(w, h);
    let footer_top = screen_footer_top(h, HintStyle::standard(w, h));
    let bar_h = (h * 0.078).clamp(56.0, 96.0);
    let bar_y = footer_top - bar_h - (12.0 * scale);
    let btn_h = bar_h - (10.0 * scale).max(6.0);
    let btn_y = bar_y + (bar_h - btn_h) * 0.5;
    let gap = (14.0 * scale).max(10.0);
    let back_w = (140.0 * (chrome.margin / 48.0)).clamp(112.0, 180.0);
    let nav_w = (168.0 * (chrome.margin / 48.0)).clamp(128.0, 220.0);
    let counter_w = (120.0 * (chrome.margin / 48.0)).clamp(96.0, 150.0);

    let back = [chrome.margin, btn_y, back_w, btn_h];
    let right_edge = w - chrome.margin;
    let next = [right_edge - nav_w, btn_y, nav_w, btn_h];
    let prev = [next[0] - gap - nav_w, btn_y, nav_w, btn_h];
    let counter = [
        (prev[0] + next[0] + next[2]) * 0.5 - counter_w * 0.5,
        btn_y,
        counter_w,
        btn_h,
    ];

    (
        TutorialBottomNav {
            back,
            prev,
            next,
            counter,
        },
        bar_y - (10.0 * scale).max(8.0),
    )
}

fn tutorial_content_band(
    w: f32,
    h: f32,
    subtitle: Option<&str>,
) -> (GuideLayout, GuideNavHeader, f32, f32) {
    let mut layout = GuideLayout::new(w, h);
    let nav_header = guide_nav_header(w, h, layout.header_chrome().back, subtitle);
    let jr = (w.min(h) / 720.0).clamp(1.0, 1.38);
    let content_top = nav_header.content_top + 1.0 + (18.0 * jr).max(14.0);
    let (_, content_bottom) = tutorial_bottom_nav(w, h);
    layout.content_bottom = content_bottom.min(layout.content_bottom);
    (layout, nav_header, content_top, content_bottom)
}

fn push_tutorial_header(
    frame: &mut UiFrame,
    w: f32,
    page_title: &str,
    nav_header: &GuideNavHeader,
    subtitle: Option<&str>,
) {
    frame.text(TextLabel {
        rect: [
            nav_header.copy_x,
            nav_header.title_y,
            w * 0.72,
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

fn push_tutorial_bottom_nav(
    frame: &mut UiFrame,
    tree: &TreeState,
    nav: &TutorialBottomNav,
    page: usize,
    pages: usize,
    scale: f32,
    w: f32,
    h: f32,
) {
    let prev_enabled = page > 0;
    let on_last = page + 1 >= pages;
    let items = [
        FlatItem::new(TutorialNav::Back.id(), nav.back, TutorialNav::Back),
        FlatItem::new(TutorialNav::Prev.id(), nav.prev, TutorialNav::Prev),
        FlatItem::new(TutorialNav::Next.id(), nav.next, TutorialNav::Next),
    ];
    let mut nav_quads = Vec::new();
    let mut nav_texts = Vec::new();
    let mut junk_buttons = Vec::new();
    for item in &items {
        let focused = tree.focused() == Some(item.id);
        let (label, variant, state) = match item.action {
            TutorialNav::Prev => {
                let state = if !prev_enabled {
                    ButtonState::Disabled
                } else if focused {
                    ButtonState::Hover
                } else {
                    ButtonState::Rest
                };
                ("Previous", ButtonVariant::Default, state)
            }
            TutorialNav::Back => {
                let state = if !prev_enabled {
                    ButtonState::Disabled
                } else if focused {
                    ButtonState::Hover
                } else {
                    ButtonState::Rest
                };
                ("Back", ButtonVariant::Default, state)
            }
            TutorialNav::Next => {
                let state = if focused {
                    ButtonState::Hover
                } else {
                    ButtonState::Rest
                };
                let label = if on_last { "Start" } else { "Next" };
                (label, ButtonVariant::Primary, state)
            }
            TutorialNav::TryDiscard | TutorialNav::TryPlay | TutorialNav::TryTrigger => continue,
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
    let counter_rect = [
        nav.counter[0],
        nav.counter[1] + (nav.counter[3] - counter_line_h) * 0.5,
        nav.counter[2],
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
}

/// Part 1 tile showcase — side labels, tiles left-aligned in each row.
struct TutorialTilesGridPlan {
    tile_size: f32,
    tile_gap: f32,
    row_gap: f32,
    pad: f32,
    label_col_w: f32,
}

struct TutorialTilesSideLabel {
    rect: [f32; 4],
    text: &'static str,
    accent: [f32; 4],
}

/// Part 1 left-column copy: shared measure + draw so intro band and body stay aligned.
struct TutorialTilesCopyLayout {
    font_cap_px: f32,
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

    fn try_it_demo_line(kind: TutorialPageKind, flash: Option<TryItFlash>) -> Option<&'static str> {
        match (kind, flash) {
            (TutorialPageKind::TryIt, Some(TryItFlash::CashIn)) => Some(try_it::CASH_IN),
            (TutorialPageKind::TryIt, Some(TryItFlash::Play)) => Some(try_it::PLAY),
            (TutorialPageKind::TryIt, Some(TryItFlash::Discard)) => Some(try_it::DISCARD),
            (TutorialPageKind::TryIt, None) => Some(try_it::PROMPT),
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

    /// Pair tile showcase for melds page.
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
        let callout_w = col_w - 8.0 * scale;
        try_it::FLASH_LINES
            .iter()
            .map(|line| Self::callout_box_height(line, callout_w, scale, h, None, typography::H42))
            .fold(0.0f32, f32::max)
    }

    fn colored_copy_block_height(text: &str, w: f32, tier: f32, h: f32) -> f32 {
        Self::tutorial_text_block_height(text, w, tier, h, color::PARCHMENT)
    }

    /// `font_cap_px` is the scaled body (`H32`) size; other tiers scale proportionally
    /// so the column can grow past the nominal ladder step when filling height.
    fn tutorial_tiles_copy_font_px(tier: f32, h: f32, font_cap_px: f32) -> f32 {
        let body_nominal = typography::size(typography::H32, h);
        let scale = font_cap_px / body_nominal;
        (typography::size(tier, h) * scale).max(typography::readable_floor_px(h))
    }

    fn tutorial_tiles_copy_line_h(tier: f32, h: f32, font_cap_px: f32) -> f32 {
        Self::tutorial_tiles_copy_font_px(tier, h, font_cap_px)
    }

    fn tutorial_tiles_copy_block_height(
        text: &str,
        copy_w: f32,
        tier: f32,
        h: f32,
        font_cap_px: f32,
        default_color: [f32; 4],
    ) -> f32 {
        let font_px = Self::tutorial_tiles_copy_line_h(tier, h, font_cap_px);
        styled_text::styled_line_block_height_at_font_px(
            text,
            copy_w,
            font_px,
            GlossaryMode::Panel,
            default_color,
        )
    }

    fn tutorial_tiles_copy_natural_height(
        copy_w: f32,
        h: f32,
        font_cap_px: f32,
        sections: TilesCopySections,
    ) -> f32 {
        let block = |text: &str, tier: f32, color: [f32; 4]| {
            Self::tutorial_tiles_copy_block_height(text, copy_w, tier, h, font_cap_px, color)
        };
        match sections {
            TilesCopySections::Intro => block(tiles::INTRO, typography::H32, color::PARCHMENT),
            TilesCopySections::HonorsAndFlowers => {
                let mut natural_h = block(
                    tiles::HONOR_SUITS_HEADING,
                    typography::H28,
                    color::CHAMPAGNE,
                );
                for line in tiles::HONOR_LINES {
                    natural_h += block(line, typography::H32, color::PARCHMENT);
                }
                natural_h += block(tiles::FLOWERS_HEADING, typography::H28, color::CHAMPAGNE);
                for line in tiles::FLOWER_LINES {
                    natural_h += block(line, typography::H32, color::PARCHMENT);
                }
                natural_h
            }
        }
    }

    fn tutorial_tiles_copy_section_gaps(sections: TilesCopySections) -> usize {
        match sections {
            TilesCopySections::Intro => TUTORIAL_TILES_INTRO_COPY_SECTION_GAPS,
            TilesCopySections::HonorsAndFlowers => TUTORIAL_TILES_HONORS_COPY_SECTION_GAPS,
        }
    }

    /// Largest font cap in `[min_cap, max_cap]` whose wrapped copy fits `budget` px tall.
    fn tutorial_tiles_copy_font_cap_for_budget(
        copy_w: f32,
        h: f32,
        budget: f32,
        min_cap: f32,
        max_cap: f32,
        sections: TilesCopySections,
    ) -> f32 {
        let mut lo = min_cap;
        let mut hi = max_cap;
        for _ in 0..20 {
            let mid = (lo + hi) * 0.5;
            let natural_h = Self::tutorial_tiles_copy_natural_height(copy_w, h, mid, sections);
            if natural_h <= budget {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        lo
    }

    fn tutorial_tiles_copy_floor(content_bottom: f32, scale: f32) -> f32 {
        content_bottom - (24.0 * scale).max(18.0)
    }

    fn compute_tutorial_tiles_copy_layout(
        copy_w: f32,
        content_top: f32,
        copy_floor: f32,
        h: f32,
        sections: TilesCopySections,
    ) -> TutorialTilesCopyLayout {
        let body_nominal = typography::size(typography::H32, h);
        let min_cap = body_nominal * 0.55;
        let max_cap = body_nominal * 1.25;
        let section_gap = h * 0.006;
        let copy_bottom_pad = h * 0.008;
        let available = (copy_floor - content_top - copy_bottom_pad).max(1.0);
        let gap_stack =
            section_gap * Self::tutorial_tiles_copy_section_gaps(sections) as f32;
        let text_budget = (available - gap_stack).max(1.0);

        let font_cap_px = Self::tutorial_tiles_copy_font_cap_for_budget(
            copy_w,
            h,
            text_budget,
            min_cap,
            max_cap,
            sections,
        );
        let start_y = content_top;

        TutorialTilesCopyLayout {
            font_cap_px,
            section_gap,
            start_y,
        }
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
        font_cap_px: f32,
    ) -> f32 {
        let line_h = Self::tutorial_tiles_copy_line_h(tier, h, font_cap_px);
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
                clip_rect: None,
            },
        );
        block_h
    }

    /// Left copy column for tiles pages.
    fn push_tutorial_tiles_copy_column(
        texts: &mut Vec<TextLabel>,
        copy_x: f32,
        layout: &TutorialTilesCopyLayout,
        copy_w: f32,
        h: f32,
        sections: TilesCopySections,
    ) -> f32 {
        let font_cap_px = layout.font_cap_px;
        let section_gap = layout.section_gap;
        let mut cursor = layout.start_y;

        match sections {
            TilesCopySections::Intro => {
                cursor += Self::push_tutorial_tiles_copy_line(
                    texts,
                    copy_x,
                    cursor,
                    copy_w,
                    tiles::INTRO,
                    typography::H32,
                    color::PARCHMENT,
                    h,
                    font_cap_px,
                );
            }
            TilesCopySections::HonorsAndFlowers => {
                cursor += Self::push_tutorial_tiles_copy_line(
                    texts,
                    copy_x,
                    cursor,
                    copy_w,
                    tiles::HONOR_SUITS_HEADING,
                    typography::H28,
                    color::CHAMPAGNE,
                    h,
                    font_cap_px,
                );
                for line in tiles::HONOR_LINES {
                    cursor += Self::push_tutorial_tiles_copy_line(
                        texts,
                        copy_x,
                        cursor,
                        copy_w,
                        line,
                        typography::H32,
                        color::PARCHMENT,
                        h,
                        font_cap_px,
                    );
                }
                cursor += section_gap;
                cursor += Self::push_tutorial_tiles_copy_line(
                    texts,
                    copy_x,
                    cursor,
                    copy_w,
                    tiles::FLOWERS_HEADING,
                    typography::H28,
                    color::CHAMPAGNE,
                    h,
                    font_cap_px,
                );
                for line in tiles::FLOWER_LINES {
                    cursor += Self::push_tutorial_tiles_copy_line(
                        texts,
                        copy_x,
                        cursor,
                        copy_w,
                        line,
                        typography::H32,
                        color::PARCHMENT,
                        h,
                        font_cap_px,
                    );
                }
            }
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

    fn try_it_page_layout(page: &TutorialPage, w: f32, h: f32) -> TryItLayout {
        let (layout, _, content_top, _content_bottom) =
            tutorial_content_band(w, h, tutorial_page_subtitle(page));
        let scale = metrics::scene_scale(w, h);
        let col_x = layout.content_x + layout.content_w * 0.12;
        let col_w = layout.content_w * 0.76;
        Self::compute_try_it_layout(col_x, col_w, content_top, h, scale, 0.0)
    }

    fn scoring_page_try_it_layout(page: &TutorialPage, w: f32, h: f32) -> TryItLayout {
        Self::try_it_page_layout(page, w, h)
    }

    fn flat_items(&self, w: f32, h: f32) -> Vec<FlatItem<TutorialNav>> {
        let page = self.page();
        let (bottom_nav, _) = tutorial_bottom_nav(w, h);
        let mut items = vec![
            FlatItem::new(TutorialNav::Back.id(), bottom_nav.back, TutorialNav::Back),
            FlatItem::new(TutorialNav::Prev.id(), bottom_nav.prev, TutorialNav::Prev),
            FlatItem::new(TutorialNav::Next.id(), bottom_nav.next, TutorialNav::Next),
        ];

        if page.try_it_demo {
            let t = Self::scoring_page_try_it_layout(page, w, h);
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
        const VALID_HEADER: &str = melds::VALID_SEQUENCE;
        const INVALID_HEADER: &str = melds::INVALID_SEQUENCE;

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

    fn tutorial_tiles_row_size(tile_count: usize, tile_span_w: f32, min_gap: f32) -> f32 {
        let n = tile_count.max(1);
        ((tile_span_w - min_gap * (n.saturating_sub(1)) as f32) / n as f32).max(1.0)
    }

    fn layout_tile_row_left_aligned(
        tiles: &[(Suit, u8)],
        debuffed_visual: bool,
        start_x: f32,
        center_y: f32,
        tile_size: f32,
        tile_gap: f32,
        next_id: &mut u32,
    ) -> Vec<ShowcaseTilePlacement> {
        let mut placements = Vec::with_capacity(tiles.len());
        let mut cursor_x = start_x;
        for &(suit, rank) in tiles {
            let px = cursor_x + tile_size * 0.5;
            let mut tile = Tile::new(suit, rank, *next_id);
            tile.debuffed_visual = debuffed_visual;
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
            cursor_x += tile_size + tile_gap;
        }
        placements
    }

    fn plan_tutorial_tiles_grid_layout(
        groups: &[TileGroup],
        col_w: f32,
        scale: f32,
        content_top: f32,
        content_floor: f32,
    ) -> TutorialTilesGridPlan {
        let available_h = (content_floor - content_top).max(1.0);
        let row_gap = (5.0 * scale).max(3.0);
        let pad = (3.0 * scale).max(2.0);
        let label_col_w = (col_w * 0.18).clamp(100.0, 132.0);
        let tile_span_w = (col_w - label_col_w).max(1.0);
        let min_gap = (2.0 * scale).max(1.5);
        let max_tiles = groups
            .iter()
            .map(|group| group.tiles.len())
            .max()
            .unwrap_or(1);
        let mut tile_size = Self::tutorial_tiles_row_size(max_tiles, tile_span_w, min_gap);
        let row_count = groups.len().max(1);
        let natural_h = (pad * 2.0 + tile_size) * row_count as f32
            + row_gap * row_count.saturating_sub(1) as f32;
        if natural_h > available_h * 0.96 {
            tile_size *= (available_h * 0.96 / natural_h).clamp(0.5, 1.0);
        }

        TutorialTilesGridPlan {
            tile_size,
            tile_gap: min_gap,
            row_gap,
            pad,
            label_col_w,
        }
    }

    /// Part 1 — two-column layout: copy left, tile demos right (labels beside rows).
    fn layout_tutorial_tiles_demo_column(
        page: &TutorialPage,
        col_x: f32,
        col_w: f32,
        h: f32,
        scale: f32,
        content_top: f32,
        content_floor: f32,
    ) -> (
        Vec<ShowcaseTilePlacement>,
        Vec<TutorialTilesSideLabel>,
        f32,
        f32,
    ) {
        let plan = Self::plan_tutorial_tiles_grid_layout(
            page.groups,
            col_w,
            scale,
            content_top,
            content_floor,
        );

        let title_font = typography::size(typography::H24, h);
        let tile_start_x = col_x + plan.label_col_w;
        let mut placements = Vec::new();
        let mut labels = Vec::new();
        let mut next_id = 30_000u32;
        let mut cursor_y = content_top;

        let tile_size = plan.tile_size;
        let block_h = plan.pad * 2.0 + tile_size;

        for group in page.groups {
            let center_y = cursor_y + plan.pad + tile_size * 0.5;
            labels.push(TutorialTilesSideLabel {
                rect: [
                    col_x + plan.pad,
                    cursor_y + plan.pad + (block_h - plan.pad * 2.0 - title_font) * 0.5,
                    plan.label_col_w - plan.pad * 2.0,
                    title_font * 1.15,
                ],
                text: group.label,
                accent: group.accent,
            });
            placements.extend(Self::layout_tile_row_left_aligned(
                group.tiles,
                group.debuffed_visual,
                tile_start_x,
                center_y,
                tile_size,
                plan.tile_gap,
                &mut next_id,
            ));
            cursor_y += block_h + plan.row_gap;
        }

        let content_bottom = (cursor_y - plan.row_gap).min(content_floor);
        let tile_light_y = content_top + (content_bottom - content_top) * 0.35;
        (placements, labels, content_bottom, tile_light_y)
    }

    fn draw_tutorial_tiles_side_labels(
        texts: &mut Vec<TextLabel>,
        labels: &[TutorialTilesSideLabel],
        h: f32,
    ) {
        let title_font = typography::size(typography::H24, h);
        for label in labels {
            texts.push(TextLabel {
                rect: label.rect,
                text: label.text.into(),
                color: label.accent,
                align: TextAlign::Left,
                font_px: Some(title_font),
                bold: true,
                ..Default::default()
            });
        }
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
        let label_line_h = typography::size(typography::H42, window_h)
            * crate::ui::widget::PLAIN_TEXT_LINE_STEP_MUL;

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
        for a in ctx.actions {
            match a {
                UiAction::TabPrev | UiAction::PagePrev => {
                    if self.page > 0 {
                        ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
                        self.page -= 1;
                        self.reset_try_it_demo();
                        ctx.bus.push(GameEvent::UiSound(SfxId::TileClick));
                    } else {
                        ctx.bus.push(GameEvent::UiSound(SfxId::InvalidAction));
                    }
                }
                UiAction::TabNext | UiAction::PageNext => {
                    ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                    if self.page + 1 < PAGES.len() {
                        self.page += 1;
                        self.reset_try_it_demo();
                        ctx.bus.push(GameEvent::UiSound(SfxId::PackBuy));
                    } else {
                        ctx.bus.push(GameEvent::UiSound(SfxId::RelicPickup));
                        return Some(SceneIntent::GameplayLessonsFirstChamber);
                    }
                }
                _ => {}
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
            Some(TutorialNav::Back) | Some(TutorialNav::Prev) => {
                if self.page > 0 {
                    ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
                    self.page -= 1;
                    self.reset_try_it_demo();
                    ctx.bus.push(GameEvent::UiSound(SfxId::TileClick));
                } else {
                    ctx.bus.push(GameEvent::UiSound(SfxId::InvalidAction));
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
        let subtitle = tutorial_page_subtitle(page);

        let mut fg_quads = Vec::new();
        let mut texts = Vec::new();
        let mut wood_tablet_placements: Vec<Object3d> = Vec::new();
        let mut bowl_placement: Option<Object3d> = None;
        let mut mirror_placement: Option<Object3d> = None;
        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);
        let cam = doc_tile_camera(h);
        frame.camera_override = Some(cam);
        frame.showcase_render_hints.layout_use_ray_plane_z = true;
        frame.showcase_render_hints.doc_tile_no_shadow = true;

        frame.scene_lighting.push_smooth(PointLight {
            pos: [w * 0.5, h * 0.38, h * 1.35],
            radius: h * 2.9,
            color: color::rgb(color::PARCHMENT),
            intensity: 1.15,
        });

        let (layout, nav_header, _, content_bottom) = tutorial_content_band(w, h, subtitle);
        let content_top = push_guide_chrome(&mut frame, &layout, nav_header.content_top);
        push_tutorial_header(
            &mut frame,
            w,
            page.title,
            &nav_header,
            subtitle,
        );
        let content_floor = content_bottom;
        let (bottom_nav, _) = tutorial_bottom_nav(w, h);

        let is_tiles_page = matches!(
            page.kind,
            TutorialPageKind::TilesIntro | TutorialPageKind::TilesHonors
        );
        let tiles_copy_sections = match page.kind {
            TutorialPageKind::TilesIntro => TilesCopySections::Intro,
            TutorialPageKind::TilesHonors => TilesCopySections::HonorsAndFlowers,
            _ => TilesCopySections::Intro,
        };

        let left_w = if is_tiles_page {
            layout.content_w * 0.38
        } else {
            layout.content_w
        };
        let col_gutter = layout.content_w * 0.02;
        let right_w = layout.content_w - left_w - col_gutter;
        let copy_x = layout.content_x;
        let copy_w = left_w;
        let tile_col_x = copy_x + left_w + col_gutter;
        let tile_col_w = right_w;

        let tiles_callout_h = if is_tiles_page {
            page.callout.map_or(0.0, |callout| {
                Self::callout_box_height(callout, tile_col_w, scale, h, None, typography::H36)
            })
        } else {
            0.0
        };
        let callout_band = tiles_callout_h + 28.0 * scale;
        let tile_content_floor = if is_tiles_page {
            content_floor - callout_band
        } else {
            content_floor
        };

        let copy_floor = Self::tutorial_tiles_copy_floor(content_floor, scale);
        let tiles_copy_layout = if is_tiles_page {
            Some(Self::compute_tutorial_tiles_copy_layout(
                copy_w,
                content_top,
                copy_floor,
                h,
                tiles_copy_sections,
            ))
        } else {
            None
        };
        let tile_area_top = content_top;

        let try_it_layout = if page.try_it_demo {
            Some(Self::try_it_page_layout(page, w, h))
        } else {
            None
        };

        let (showcase_tiles, tile_light_y, content_bottom) = match page.kind {
            TutorialPageKind::TilesIntro | TutorialPageKind::TilesHonors => {
                let copy_layout = tiles_copy_layout.as_ref().expect("tiles page layout");
                Self::push_tutorial_tiles_copy_column(
                    &mut texts,
                    copy_x,
                    copy_layout,
                    copy_w,
                    h,
                    tiles_copy_sections,
                );
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
                let (placements, labels, bottom, light_y) =
                    Self::layout_tutorial_tiles_demo_column(
                        page,
                        tile_col_x,
                        tile_col_w,
                        h,
                        scale,
                        tile_area_top,
                        tile_content_floor,
                    );
                Self::draw_tutorial_tiles_side_labels(&mut texts, &labels, h);
                (placements, light_y, bottom)
            }
            TutorialPageKind::Melds => {
                let col_x = layout.content_x;
                let col_w = layout.content_w;
                let melds_heading_h = 22.0 * scale;
                let melds_heading_y = content_top + 4.0 * scale;
                texts.push(TextLabel {
                    rect: [
                        col_x + 4.0 * scale,
                        melds_heading_y,
                        col_w - 8.0 * scale,
                        melds_heading_h,
                    ],
                    text: SCORING_MELDS_HEADING.to_string(),
                    color: color::GOLD,
                    align: TextAlign::Center,
                    font_px: Some(typography::size(typography::H36, h)),
                    ..Default::default()
                });
                let pair_row_top = melds_heading_y + melds_heading_h + 4.0 * scale;
                let (placements, labels, meld_bottom) = Self::layout_demo_page_tiles(
                    page,
                    w,
                    h,
                    col_x,
                    col_w,
                    pair_row_top,
                    scale,
                );
                let stack_vertical = h < 760.0;
                let max_tile = h * 0.065;
                let mut seq_tile_id = 40_000u32;
                let mut showcase_tiles = placements;
                let (seq_placements, seq_bottom) = Self::layout_sequence_comparison_cards(
                    SCORING_SEQUENCE_GROUPS,
                    col_x,
                    col_w,
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
                let pair_tile_light_y = pair_row_top + h * 0.065 * 0.35;
                Self::draw_tiles_page_group_labels(&mut texts, &mut fg_quads, &labels, scale, h);
                (
                    showcase_tiles,
                    pair_tile_light_y,
                    seq_bottom.max(meld_bottom),
                )
            }
            TutorialPageKind::ScoringTerms | TutorialPageKind::TryIt => {
                (Vec::new(), content_top, content_top)
            }
        };

        let tiles_page_footer = tile_content_floor;
        let glossary_y = if is_tiles_page {
            tiles_page_footer + 20.0 * scale
        } else if page.kind == TutorialPageKind::ScoringTerms {
            content_top + 8.0 * scale
        } else if let Some(ref t) = try_it_layout {
            t.content_floor_y + 10.0 * scale
        } else {
            (content_bottom + 14.0 * scale).min(content_floor - 120.0 * scale)
        };

        if let Some(ref layout) = try_it_layout {
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
            let trigger_focused = self.tree.focused() == Some(TutorialNav::TryTrigger.id());
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
                hover_target: if trigger_focused { 1.0 } else { 0.0 },
                anim_id: 0,
            });
            let strip_w = layout.discard_rect[2]
                + layout.play_rect[2]
                + layout.trigger_rect[2]
                + (layout.play_rect[0] - layout.discard_rect[0] - layout.discard_rect[2]).max(0.0)
                + (layout.trigger_rect[0] - layout.play_rect[0] - layout.play_rect[2]).max(0.0);
            let strip_x = layout.discard_rect[0];
            texts.push(TextLabel {
                rect: [strip_x, layout.heading_y, strip_w, heading_h],
                text: "Your implements of victory".to_string(),
                color: color::GOLD,
                align: TextAlign::Center,
                font_px: Some(typography::size(typography::H36, h)),
                ..Default::default()
            });

            if let Some(line) = Self::try_it_demo_line(page.kind, self.try_it_flash) {
                let callout_w = strip_w;
                let callout_x = strip_x;
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
            let term_w = layout.content_w - 68.0 * scale;
            let term_font = typography::size(typography::H42, h);
            let (term_heights, _) =
                Self::glossary_term_metrics(page.glossary, term_w, term_font, scale);
            let glossary_x = layout.content_x + 34.0 * scale;
            texts.push(TextLabel {
                rect: [glossary_x, glossary_y, term_w, 24.0 * scale],
                text: "Key Ideas".to_string(),
                color: color::GOLD,
                align: TextAlign::Left,
                font_px: Some(typography::size(typography::H32, h)),
                ..Default::default()
            });
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
            let (callout_x, callout_y, callout_w, callout_h) = if page.kind == TutorialPageKind::TryIt {
                let callout_w = layout.content_w * 0.76;
                let callout_x = layout.content_x + layout.content_w * 0.12;
                let callout_y = bottom_nav.back[1] - (120.0 * scale).max(90.0);
                let callout_h =
                    Self::callout_box_height(callout, callout_w, scale, h, None, typography::H36);
                (callout_x, callout_y, callout_w, callout_h)
            } else {
                let callout_x = tile_col_x;
                let callout_y = glossary_y;
                let callout_w = tile_col_w;
                let callout_h =
                    Self::callout_box_height(callout, callout_w, scale, h, None, typography::H36);
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

        let mut focus_ring_quads = Vec::new();
        if let Some(rect) = match self.tree.focused() {
            Some(id) if id == TutorialNav::TryDiscard.id() => ctx.proj.bowl_rect,
            Some(id) if id == TutorialNav::TryPlay.id() => ctx.proj.mirror_rect,
            Some(id) if id == TutorialNav::TryTrigger.id() => {
                ctx.proj.wood_tablet_rects.first().copied()
            }
            _ => None,
        } {
            focus_nav::push_focus_ring(rect, scale, w, h, &mut focus_ring_quads);
        }

        // Panel fills and gutters before doc tiles (same order as guide melds page).
        frame.quads(fg_quads);
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
        let light_y = h * 0.18;
        let light_rows: Vec<(f32, f32, f32)> = if is_tiles_page {
            vec![
                (
                    tile_col_x + tile_col_w * 0.20,
                    tile_light_y - 10.0 * scale,
                    1.10,
                ),
                (
                    tile_col_x + tile_col_w * 0.50,
                    tile_light_y - 22.0 * scale,
                    1.20,
                ),
                (
                    tile_col_x + tile_col_w * 0.80,
                    tile_light_y - 10.0 * scale,
                    1.10,
                ),
            ]
        } else if page.kind == TutorialPageKind::Melds {
            vec![(
                layout.content_x + layout.content_w * 0.5,
                tile_light_y - 8.0 * scale,
                1.95,
            )]
        } else if let Some(ref try_it) = try_it_layout {
            let prop_light_y = try_it.discard_rect[1] + try_it.discard_rect[3] * 0.35;
            vec![
                (
                    try_it.discard_rect[0] + try_it.discard_rect[2] * 0.5,
                    prop_light_y,
                    1.20,
                ),
                (
                    try_it.play_rect[0] + try_it.play_rect[2] * 0.5,
                    prop_light_y,
                    1.35,
                ),
                (
                    try_it.trigger_rect[0] + try_it.trigger_rect[2] * 0.5,
                    prop_light_y,
                    1.20,
                ),
            ]
        } else {
            Vec::new()
        };
        for &(lx, ly, intensity) in &light_rows {
            frame.scene_lighting.push_smooth(PointLight {
                pos: [lx, ly, light_y],
                radius: h * 0.95,
                color: color::rgb(color::PARCHMENT),
                intensity,
            });
        }
        frame.quads(focus_ring_quads);
        frame.texts(texts);
        push_tutorial_bottom_nav(
            &mut frame,
            &self.tree,
            &bottom_nav,
            self.page,
            PAGES.len(),
            scale,
            w,
            h,
        );
        self.tree.register_flat_buttons(&items, &mut frame.buttons);
        push_screen_footer_hint(
            &mut frame,
            &ctx,
            match ctx.input_mode {
                InputMode::Cursor => menu_footer_row(ctx.input_mode),
                InputMode::Keyboard | InputMode::Controller => guide_footer_row(ctx.input_mode),
            },
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

    fn tiles_page_copy_metrics(w: f32, h: f32, sections: TilesCopySections) -> (f32, f32, f32, f32) {
        let scale = metrics::scene_scale(w, h);
        let (layout, nav_header, content_top, content_bottom) = tutorial_content_band(w, h, None);
        let _ = nav_header;
        let left_w = layout.content_w * 0.38;
        let copy_w = left_w;
        let copy_floor = TutorialCampaignScene::tutorial_tiles_copy_floor(content_bottom, scale);
        let _ = sections;
        (copy_w, content_top, copy_floor, content_bottom)
    }

    #[test]
    fn tutorial_has_five_pages() {
        assert_eq!(PAGES.len(), 5);
    }

    #[test]
    fn bottom_nav_buttons_are_large() {
        let (nav, _) = tutorial_bottom_nav(1920.0, 1080.0);
        assert!(nav.prev[2] >= 128.0, "prev button should be wide");
        assert!(nav.next[3] >= 56.0, "next button should be tall");
        assert!(nav.prev[1] + nav.prev[3] < nav.back[1] + nav.back[3] + 1.0);
    }

    #[test]
    fn tiles_copy_layout_fits_short_window() {
        let h = 720.0;
        let (copy_w, content_top, copy_floor, _) =
            tiles_page_copy_metrics(1280.0, h, TilesCopySections::Intro);

        let layout = TutorialCampaignScene::compute_tutorial_tiles_copy_layout(
            copy_w,
            content_top,
            copy_floor,
            h,
            TilesCopySections::Intro,
        );
        let natural_h = TutorialCampaignScene::tutorial_tiles_copy_natural_height(
            copy_w,
            h,
            layout.font_cap_px,
            TilesCopySections::Intro,
        );
        let copy_bottom_pad = h * 0.008;
        let available = copy_floor - content_top - copy_bottom_pad;
        let total =
            natural_h + layout.section_gap * TUTORIAL_TILES_INTRO_COPY_SECTION_GAPS as f32;
        assert!(
            total <= available + 1.0,
            "copy should fit: total={total} available={available} font_cap_px={}",
            layout.font_cap_px
        );
    }

    #[test]
    fn tiles_copy_layout_fills_column() {
        let h = 1080.0;
        let (copy_w, content_top, copy_floor, _) =
            tiles_page_copy_metrics(1920.0, h, TilesCopySections::Intro);

        let layout = TutorialCampaignScene::compute_tutorial_tiles_copy_layout(
            copy_w,
            content_top,
            copy_floor,
            h,
            TilesCopySections::Intro,
        );
        let natural_h = TutorialCampaignScene::tutorial_tiles_copy_natural_height(
            copy_w,
            h,
            layout.font_cap_px,
            TilesCopySections::Intro,
        );
        let copy_bottom_pad = h * 0.008;
        let available = copy_floor - content_top - copy_bottom_pad;
        let used = natural_h + layout.section_gap * TUTORIAL_TILES_INTRO_COPY_SECTION_GAPS as f32;
        assert!(
            used <= available + 1.0,
            "intro copy should fit the column (used={used}, available={available})"
        );
        assert_eq!(layout.start_y, content_top);
    }

    #[test]
    fn tiles_copy_floor_extends_past_tile_content_floor() {
        let h = 1080.0;
        let scale = metrics::scene_scale(1920.0, h);
        let (_, _, copy_floor, content_bottom) =
            tiles_page_copy_metrics(1920.0, h, TilesCopySections::Intro);
        let callout_band = 28.0 * scale + 60.0;
        let tile_content_floor = content_bottom - callout_band;
        assert!(
            copy_floor > tile_content_floor + 40.0,
            "left copy should not reserve the right-column callout band"
        );
    }

    #[test]
    fn try_it_page_callout_fits_above_bottom_nav() {
        let w = 1920.0;
        let h = 1080.0;
        let scale = metrics::scene_scale(w, h);
        let page = PAGES.last().expect("try-it page");
        let callout = page.callout.expect("try-it page callout");
        let (guide_layout, _, _, content_bottom) =
            tutorial_content_band(w, h, tutorial_page_subtitle(page));
        let (bottom_nav, _) = tutorial_bottom_nav(w, h);
        let callout_w = guide_layout.content_w * 0.76;
        let callout_y = bottom_nav.back[1] - (120.0 * scale).max(90.0);
        let callout_h =
            TutorialCampaignScene::callout_box_height(callout, callout_w, scale, h, None, typography::H36);
        assert!(
            callout_y + callout_h <= content_bottom + 0.5,
            "callout should sit above bottom nav"
        );
    }

    #[test]
    fn try_it_page_callout_fits_wrapped_text() {
        let w = 1920.0;
        let h = 1080.0;
        let scale = metrics::scene_scale(w, h);
        let page = PAGES.last().expect("try-it page");
        let callout = page.callout.expect("try-it page callout");
        let (guide_layout, _, _, _) = tutorial_content_band(w, h, tutorial_page_subtitle(page));
        let callout_w = guide_layout.content_w * 0.76;
        let callout_h =
            TutorialCampaignScene::callout_box_height(callout, callout_w, scale, h, None, typography::H36);
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
        let default =
            TutorialCampaignScene::try_it_demo_line(TutorialPageKind::TryIt, None).expect("default line");
        assert!(default.contains("Try them"));
        let discard = TutorialCampaignScene::try_it_demo_line(
            TutorialPageKind::TryIt,
            Some(TryItFlash::Discard),
        )
        .expect("discard flash");
        assert!(discard.contains("river"));
        assert_ne!(discard, default);
    }
}
