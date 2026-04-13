//! Scripted onboarding campaign scenes shown before the tutorial shop.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::audio::SfxId;
use crate::core::relic::RelicId;
use crate::core::talisman::TalismanKind;
use crate::core::tile::{Suit, Tile};
use crate::core::tile_pack::TilePackKind;
use crate::core::zodiac::ZodiacKind;
use crate::game::event_bus::GameEvent;
use crate::game::onboarding::OnboardingPhase;
use crate::render::draw_cmd::{
    CameraParams, DrawCmd, MirrorPlacement, PackPlacement, ShowcaseTilePlacement,
    TalismanPlacement, UiFrame, WoodTabletPlacement, ZodiacRibbonPlacement,
};
use crate::render::table_space::TableAnchorPx;
use crate::render::theme::{ButtonState, ButtonVariant, color, metrics, typography};
use crate::render::wgpu_renderer::{GpuInstance, PointLight, RelicIcon, TextAlign, TextLabel};
use crate::ui::focus_nav;
use crate::ui::widget::{self, TextStyle};
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use super::shop::ShopScene;
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
}

struct TileGroup {
    label: &'static str,
    accent: [f32; 4],
    tiles: &'static [(Suit, u8)],
    rows: &'static [&'static [(Suit, u8)]],
    layout: TileGroupLayout,
    debuffed_visual: bool,
}

#[derive(Clone, Copy)]
enum TileGroupLayout {
    Flat,
    FullHand,
    Pairs,
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

/// Three numbered suits (2–9 tiles each in the full wall).
const PART1_SUITS_GROUPS: &[TileGroup] = &[
    TileGroup {
        label: "Bamboos",
        accent: color::TWILIGHT,
        tiles: &[(Suit::Bamboos, 2), (Suit::Bamboos, 5), (Suit::Bamboos, 8)],
        rows: &[],
        layout: TileGroupLayout::Flat,
        debuffed_visual: false,
    },
    TileGroup {
        label: "Circles",
        accent: color::CHAMPAGNE,
        tiles: &[(Suit::Circles, 3), (Suit::Circles, 5), (Suit::Circles, 7)],
        rows: &[],
        layout: TileGroupLayout::Flat,
        debuffed_visual: false,
    },
    TileGroup {
        label: "Characters",
        accent: color::MIST,
        tiles: &[
            (Suit::Characters, 1),
            (Suit::Characters, 5),
            (Suit::Characters, 9),
        ],
        rows: &[],
        layout: TileGroupLayout::Flat,
        debuffed_visual: false,
    },
];

/// Ranks 2–8 (simples) and 1 / 9 (terminals) within the three suits.
const PART1_NUMBER_GROUPS: &[TileGroup] = &[
    TileGroup {
        label: "Simples",
        accent: color::TWILIGHT,
        tiles: &[(Suit::Bamboos, 3), (Suit::Bamboos, 5), (Suit::Bamboos, 7)],
        rows: &[],
        layout: TileGroupLayout::Flat,
        debuffed_visual: false,
    },
    TileGroup {
        label: "Terminals",
        accent: color::GOLD,
        tiles: &[
            (Suit::Circles, 1),
            (Suit::Circles, 9),
            (Suit::Characters, 9),
        ],
        rows: &[],
        layout: TileGroupLayout::Flat,
        debuffed_visual: false,
    },
];

/// Winds, dragons, honors umbrella, and flower bonus tiles.
const PART1_HONOR_GROUPS: &[TileGroup] = &[
    TileGroup {
        label: "Winds",
        accent: color::MIST,
        tiles: &[
            (Suit::Wind, 1),
            (Suit::Wind, 2),
            (Suit::Wind, 3),
            (Suit::Wind, 4),
        ],
        rows: &[],
        layout: TileGroupLayout::Flat,
        debuffed_visual: false,
    },
    TileGroup {
        label: "Dragons",
        accent: color::RUBY,
        tiles: &[(Suit::Dragon, 1), (Suit::Dragon, 2), (Suit::Dragon, 3)],
        rows: &[],
        layout: TileGroupLayout::Flat,
        debuffed_visual: false,
    },
    TileGroup {
        label: "Honors",
        accent: color::CHAMPAGNE,
        tiles: &[
            (Suit::Wind, 1),
            (Suit::Dragon, 1),
            (Suit::Wind, 4),
            (Suit::Dragon, 3),
        ],
        rows: &[],
        layout: TileGroupLayout::Flat,
        debuffed_visual: false,
    },
    TileGroup {
        label: "Flowers",
        accent: color::GOLD,
        tiles: &[
            (Suit::Flower, 1),
            (Suit::Flower, 2),
            (Suit::Flower, 3),
            (Suit::Flower, 4),
        ],
        rows: &[],
        layout: TileGroupLayout::Flat,
        debuffed_visual: false,
    },
];

const FULL_HAND_ROW_TOP: &[(Suit, u8)] = &[
    (Suit::Characters, 1),
    (Suit::Characters, 2),
    (Suit::Characters, 3),
    (Suit::Bamboos, 4),
    (Suit::Bamboos, 5),
    (Suit::Bamboos, 6),
];

const FULL_HAND_ROW_BOTTOM: &[(Suit, u8)] = &[
    (Suit::Circles, 7),
    (Suit::Circles, 7),
    (Suit::Circles, 7),
    (Suit::Characters, 7),
    (Suit::Characters, 8),
    (Suit::Characters, 9),
    (Suit::Dragon, 1),
    (Suit::Dragon, 1),
];

const FULL_HAND_ROWS: &[&[(Suit, u8)]] = &[FULL_HAND_ROW_TOP, FULL_HAND_ROW_BOTTOM];

const CHIITOITSU_ROW_TOP: &[(Suit, u8)] = &[
    (Suit::Bamboos, 1),
    (Suit::Bamboos, 1),
    (Suit::Circles, 2),
    (Suit::Circles, 2),
    (Suit::Characters, 3),
    (Suit::Characters, 3),
    (Suit::Bamboos, 5),
    (Suit::Bamboos, 5),
];

const CHIITOITSU_ROW_BOTTOM: &[(Suit, u8)] = &[
    (Suit::Circles, 7),
    (Suit::Circles, 7),
    (Suit::Characters, 8),
    (Suit::Characters, 8),
    (Suit::Dragon, 1),
    (Suit::Dragon, 1),
];

const CHIITOITSU_ROWS: &[&[(Suit, u8)]] = &[CHIITOITSU_ROW_TOP, CHIITOITSU_ROW_BOTTOM];

const BOSS_SAFE_FULL_HAND_ROW_TOP: &[(Suit, u8)] = &[
    (Suit::Bamboos, 2),
    (Suit::Bamboos, 3),
    (Suit::Bamboos, 4),
    (Suit::Circles, 4),
    (Suit::Circles, 5),
    (Suit::Circles, 6),
];

const BOSS_SAFE_FULL_HAND_ROW_BOTTOM: &[(Suit, u8)] = &[
    (Suit::Characters, 7),
    (Suit::Characters, 7),
    (Suit::Characters, 7),
    (Suit::Bamboos, 6),
    (Suit::Bamboos, 7),
    (Suit::Bamboos, 8),
    (Suit::Circles, 9),
    (Suit::Circles, 9),
];

const BOSS_SAFE_FULL_HAND_ROWS: &[&[(Suit, u8)]] =
    &[BOSS_SAFE_FULL_HAND_ROW_TOP, BOSS_SAFE_FULL_HAND_ROW_BOTTOM];

const BOSS_SAFE_CHIITOITSU_ROW_TOP: &[(Suit, u8)] = &[
    (Suit::Bamboos, 1),
    (Suit::Bamboos, 1),
    (Suit::Circles, 2),
    (Suit::Circles, 2),
    (Suit::Characters, 3),
    (Suit::Characters, 3),
    (Suit::Bamboos, 5),
    (Suit::Bamboos, 5),
];

const BOSS_SAFE_CHIITOITSU_ROW_BOTTOM: &[(Suit, u8)] = &[
    (Suit::Circles, 7),
    (Suit::Circles, 7),
    (Suit::Characters, 8),
    (Suit::Characters, 8),
    (Suit::Bamboos, 9),
    (Suit::Bamboos, 9),
];

const BOSS_SAFE_CHIITOITSU_ROWS: &[&[(Suit, u8)]] = &[
    BOSS_SAFE_CHIITOITSU_ROW_TOP,
    BOSS_SAFE_CHIITOITSU_ROW_BOTTOM,
];

const PART1_GROUPS: &[TileGroup] = &[
    TileGroup {
        label: "Full Hand",
        accent: color::CHAMPAGNE,
        tiles: &[],
        rows: FULL_HAND_ROWS,
        layout: TileGroupLayout::FullHand,
        debuffed_visual: false,
    },
    TileGroup {
        label: "Chiitoitsu",
        accent: color::MIST,
        tiles: &[],
        rows: CHIITOITSU_ROWS,
        layout: TileGroupLayout::Pairs,
        debuffed_visual: false,
    },
];

const PART2_GROUPS: &[TileGroup] = &[
    TileGroup {
        label: "Full Hand",
        accent: color::CHAMPAGNE,
        tiles: &[],
        rows: FULL_HAND_ROWS,
        layout: TileGroupLayout::FullHand,
        debuffed_visual: false,
    },
    TileGroup {
        label: "Chiitoitsu",
        accent: color::MIST,
        tiles: &[],
        rows: CHIITOITSU_ROWS,
        layout: TileGroupLayout::Pairs,
        debuffed_visual: false,
    },
];

const PART3_GROUPS: &[TileGroup] = &[
    TileGroup {
        label: "Relic",
        accent: color::GOLD,
        tiles: &[(Suit::Bamboos, 4), (Suit::Bamboos, 4), (Suit::Bamboos, 4)],
        rows: &[],
        layout: TileGroupLayout::Flat,
        debuffed_visual: false,
    },
    TileGroup {
        label: "Boss Debuff",
        accent: color::RUBY,
        tiles: &[
            (Suit::Wind, 1),
            (Suit::Wind, 1),
            (Suit::Wind, 1),
            (Suit::Dragon, 2),
            (Suit::Dragon, 2),
        ],
        rows: &[],
        layout: TileGroupLayout::Flat,
        debuffed_visual: true,
    },
];

const PART4_GROUPS: &[TileGroup] = &[
    TileGroup {
        label: "Relic",
        accent: color::GOLD,
        tiles: &[(Suit::Circles, 5), (Suit::Circles, 5), (Suit::Circles, 5)],
        rows: &[],
        layout: TileGroupLayout::Flat,
        debuffed_visual: false,
    },
    TileGroup {
        label: "Zodiac ribbon",
        accent: color::TWILIGHT,
        tiles: &[
            (Suit::Characters, 1),
            (Suit::Characters, 9),
            (Suit::Characters, 5),
        ],
        rows: &[],
        layout: TileGroupLayout::Flat,
        debuffed_visual: false,
    },
    TileGroup {
        label: "Talisman",
        accent: color::MIST,
        tiles: &[(Suit::Dragon, 1), (Suit::Dragon, 1), (Suit::Bamboos, 6)],
        rows: &[],
        layout: TileGroupLayout::Flat,
        debuffed_visual: false,
    },
    TileGroup {
        label: "Tile pack",
        accent: color::CHAMPAGNE,
        tiles: &[
            (Suit::Bamboos, 2),
            (Suit::Circles, 3),
            (Suit::Characters, 4),
            (Suit::Bamboos, 8),
        ],
        rows: &[],
        layout: TileGroupLayout::Flat,
        debuffed_visual: false,
    },
];

const PART5_GROUPS: &[TileGroup] = &[
    TileGroup {
        label: "Boss-safe Full Hand",
        accent: color::TWILIGHT,
        tiles: &[],
        rows: BOSS_SAFE_FULL_HAND_ROWS,
        layout: TileGroupLayout::FullHand,
        debuffed_visual: false,
    },
    TileGroup {
        label: "Boss-safe Chiitoitsu",
        accent: color::CHAMPAGNE,
        tiles: &[],
        rows: BOSS_SAFE_CHIITOITSU_ROWS,
        layout: TileGroupLayout::Pairs,
        debuffed_visual: false,
    },
];

/// Part 5 — Structure And Scoring (0-based index into `PAGES`).
const TUTORIAL_PAGE_STRUCTURE: usize = 4;
/// Part 6 — Relics and bosses overview.
const TUTORIAL_PAGE_RELICS: usize = 5;
/// Part 8 — Tutorial Boss.
const TUTORIAL_PAGE_BOSS: usize = 7;
/// Part 7 — Shop overview.
const TUTORIAL_PAGE_SHOP: usize = 6;

const PAGES: &[TutorialPage] = &[
    TutorialPage {
        title: "Part 1 — The Three Suits",
        subtitle: "Numbered tiles belong to three suits: Bamboos, Circles, and Characters. Each suit runs from 1 to 9. Sequences and most scoring rules stay inside one suit at a time.",
        glossary: &[
            "Bamboos = green bamboo-stick suit",
            "Circles = dotted suit",
            "Characters = red chinese character suit",
            "1–9 = ranks within a suit",
        ],
        callout: Some("These are the tiles you will build melds from most often."),
        try_it_demo: false,
        groups: PART1_SUITS_GROUPS,
    },
    TutorialPage {
        title: "Part 2 — Simples & Terminals",
        subtitle: "Within each numbered suit, simples are ranks 2 through 8 — the middle tiles. Terminals are 1 and 9 at the ends. Many bosses and relics refer to “simples” or “terminals” as a group.",
        glossary: &[
            "Simples = ranks 2–8 in bamboo, circles, or characters",
            "Terminals = 1 or 9 in those suits",
            "Sequences use simples or terminals in order",
        ],
        callout: Some("Next: winds, dragons, honors, and flowers — different families of tiles."),
        try_it_demo: false,
        groups: PART1_NUMBER_GROUPS,
    },
    TutorialPage {
        title: "Part 3 — Winds, Dragons & Flowers",
        subtitle: "There are fours winds; dragons are the three red, green, and white tiles. Together they are honors (no bamboo, circles, or characters on those faces).",
        glossary: &[
            "Winds = East, South, West, North",
            "Dragons = Red, Green, White",
            "Honors = every wind and dragon tile",
            "Flowers = bonus tiles that act as wildcards",
        ],
        callout: Some(
            "Bosses and relics can target honors or terminals — make sure to read their descriptions",
        ),
        try_it_demo: false,
        groups: PART1_HONOR_GROUPS,
    },
    TutorialPage {
        title: "Part 4 — Tile Combinations",
        subtitle: "Tiles work like classic mahjong: pairs, triplets, and sequences are your basic groups. Any valid group you bank into your build area is called a meld. Start by learning two common winning shapes: Full Hand (four melds and one pair) and Chiitoitsu (seven pairs). Extra pattern bonuses are called yaku. A complete hand with no yaku is a Chicken Hand — legal, but barely worth scoring.",
        glossary: &[
            "Pair = 2 matching tiles",
            "Triplet = 3 matching tiles",
            "Sequence = 3 consecutive tiles, same suit",
            "Yaku = pattern bonus (raises mult)",
            "Chiitoitsu = seven pairs (Japanese name)",
            "Chicken Hand = valid hand, no yaku",
        ],
        callout: Some(
            "Focus on Full Hand and seven pairs (Chiitoitsu) first. They are the easiest ways to see what “finished” looks like.",
        ),
        try_it_demo: false,
        groups: PART1_GROUPS,
    },
    TutorialPage {
        title: "Part 5 — Structure And Scoring",
        subtitle: "Playing melds doesn't cause you to score in Mahjuro. Instead, it banks them into your structure. When you are ready, press Trigger to cash in. Chips come from the tiles and melds you banked. Mult comes mostly from yaku and relics. The final score is chips × mult.",
        glossary: &[
            "Structure = where banked melds sit until you cash in",
            "Play = bank selected melds",
            "Trigger = score your structure",
            "Chips = base value",
            "Mult = score multiplier",
            "Full Hand / Chiitoitsu = strong early goals",
        ],
        callout: Some(
            "Mult usually matters more than raw chips. Early on, land a real yaku before chasing exotic tiles.",
        ),
        try_it_demo: true,
        groups: PART2_GROUPS,
    },
    TutorialPage {
        title: "Part 6 — Relics And Bosses",
        subtitle: "Relics are passive upgrades for the whole run. Bosses add a special rule for one shrine (one scoring round). A debuff does not make tiles illegal — they can still complete melds, but may be worth less when scored. The Iconoclast debuffs honors, so bamboo, circles, and characters are the safer plan.",
        glossary: &[
            "Relic = passive run bonus",
            "Shrine = one round with a score target (Small, Big, or Boss)",
            "Boss = extra rule on the Boss shrine",
            "Debuff = tiles score for less, not gone",
            "Honors = wind and dragon tiles",
        ],
        callout: Some("Next you will browse the shop: buy upgrades, then face the tutorial boss."),
        try_it_demo: false,
        groups: PART3_GROUPS,
    },
    TutorialPage {
        title: "Part 7 — The Shop",
        subtitle: "Spend gold on relics (passive run bonuses), zodiac ribbons (level up a yaku for the run), talismans (stamp enhancements onto tiles), and tile packs (change what the wall contains). You can sell owned relics back for gold. When you are ready, Next Round continues the run.",
        glossary: &[
            "Relic = passive bonus for the run",
            "Zodiac ribbon = level one yaku",
            "Talisman = tile enhancement stamp",
            "Tile pack = wall modifier",
            "Sell = refund some gold",
            "Wall = the tiles you draw from",
        ],
        callout: Some(
            "Hover items to read what they do. You do not need to buy everything — pick what fits your plan.",
        ),
        try_it_demo: false,
        groups: PART4_GROUPS,
    },
    TutorialPage {
        title: "Part 8 — Tutorial Boss",
        subtitle: "The tutorial boss is The Iconoclast: honors (winds and dragons) are debuffed — they still form melds, but score for much less. Prefer bamboo, circles, and characters: triplets, sequences, Full Hand, or seven pairs (Chiitoitsu) still work. The deal is fair: you can miss the target, read the hint, and try again.",
        glossary: &[
            "Boss shrine = higher target + boss rule",
            "Debuff = less value from those tiles",
            "Honors = winds + dragons",
            "Full Hand / Chiitoitsu = your tutorial yakus",
        ],
        callout: Some(
            "After this lesson: open the shop, then press Next Round to enter the Boss shrine.",
        ),
        try_it_demo: true,
        groups: PART5_GROUPS,
    },
];

impl TutorialCampaignScene {
    pub fn new() -> Self {
        Self {
            page: 0,
            tree: TreeState::new(),
            try_it_phase: 0,
        }
    }

    fn page(&self) -> &'static TutorialPage {
        &PAGES[self.page.min(PAGES.len() - 1)]
    }

    fn try_it_demo_line(page_index: usize, phase: u8) -> Option<&'static str> {
        match (page_index, phase) {
            (TUTORIAL_PAGE_STRUCTURE, 0) => Some("Tap Play (bank), then Trigger (cash in)."),
            (TUTORIAL_PAGE_STRUCTURE, 1) => Some("Banked — structure is locked in."),
            (TUTORIAL_PAGE_STRUCTURE, 2) => Some("Demo: 4 chips × 3 mult = 12"),
            (TUTORIAL_PAGE_BOSS, 0) => Some("Tap Play (bank), then Trigger (cash in)."),
            (TUTORIAL_PAGE_BOSS, 1) => Some("Banked — boss debuff still applies."),
            (TUTORIAL_PAGE_BOSS, 2) => Some("Demo: 5 chips × 2 mult = 10"),
            _ => None,
        }
    }

    fn compute_try_it_layout(panel_x: f32, panel_w: f32, label_y: f32, scale: f32) -> TryItLayout {
        let btn_w = (150.0 * scale).max(100.0);
        let btn_h = (40.0 * scale).max(28.0);
        let gap = 12.0 * scale;
        let strip_y = label_y + 36.0 * scale;
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

    fn glossary_term_metrics(
        glossary: &[&'static str],
        term_w: f32,
        term_font: f32,
        scale: f32,
    ) -> (Vec<f32>, f32) {
        let mut heights = Vec::with_capacity(glossary.len());
        let mut total_h = 0.0;
        for term in glossary {
            let lines = widget::wrap_text(term, term_w, term_font);
            let term_h = lines.len().max(1) as f32 * term_font * 1.25;
            heights.push(term_h);
            total_h += term_h;
        }
        if !glossary.is_empty() {
            total_h += (glossary.len().saturating_sub(1) as f32) * 6.0 * scale;
        }
        (heights, total_h)
    }

    fn shop_preview_ribbon(
        center_x: f32,
        item_y: f32,
        h: f32,
        scale: f32,
    ) -> ZodiacRibbonPlacement {
        ZodiacRibbonPlacement {
            anchor_pos: [center_x, item_y + 2.0 * scale, h * 0.18],
            length: 46.0 * scale,
            width: 24.0 * scale,
            // Match the normal shop's wall-hung display pose.
            rotation_y_deg: 0.0,
            rotation_x_deg: 0.0,
            rotation_z_deg: 0.0,
            color: [1.0, 1.0, 1.0, 1.0],
            kind: Some(ZodiacKind::Dragon),
        }
    }

    fn shop_preview_talisman(center_x: f32, item_y: f32, h: f32, scale: f32) -> TalismanPlacement {
        TalismanPlacement {
            center_pos: [center_x, item_y + 22.0 * scale, h * 0.13],
            extents: [22.0 * scale, 32.0 * scale, 4.5 * scale],
            // Match the normal shop's upright wall display pose.
            rotation_y_deg: 0.0,
            rotation_x_deg: 0.0,
            rotation_z_deg: 0.0,
            color: [0.42, 0.82, 0.55, 1.0],
            kind: TalismanKind::Jade,
        }
    }

    fn shop_preview_pack(center_x: f32, item_y: f32, h: f32, scale: f32) -> PackPlacement {
        PackPlacement {
            world_pos: [center_x, item_y + 24.0 * scale, h * 0.14],
            half_extents: [18.0 * scale, 24.0 * scale, 5.5 * scale],
            color: [1.0, 1.0, 1.0, 1.0],
            kind: TilePackKind::CoinCache,
            // Match the normal shop's gentle shelf lean.
            rotation_x_deg: -5.0,
            rotation_y_deg: 0.0,
            pick_id: None,
        }
    }

    /// Matches `draw_frame` subtitle + tile row metrics so Try-it rects align with visuals.
    fn page_content_metrics(
        page: &TutorialPage,
        w: f32,
        h: f32,
        ui_scale: f32,
        _panel_x: f32,
        panel_y: f32,
        panel_w: f32,
    ) -> (f32, f32) {
        let scale = metrics::scene_scale(w, h, ui_scale);
        let subtitle_y = panel_y + 70.0 * scale;
        let subtitle_w = panel_w - 60.0 * scale;
        let subtitle_font = typography::size(typography::BODY, h, ui_scale).max(15.0);
        let subtitle_h = {
            let subtitle_lines = widget::wrap_text(page.subtitle, subtitle_w, subtitle_font);
            (subtitle_lines.len().max(1) as f32 * subtitle_font * 1.35)
                .max(70.0 * scale)
                .min(128.0 * scale)
        };
        let tile_area_y = subtitle_y + subtitle_h + 40.0 * scale;
        let label_y = tile_area_y + 74.0;
        (tile_area_y, label_y)
    }

    fn flat_items(&self, w: f32, h: f32, ui_scale: f32) -> Vec<FlatItem<TutorialNav>> {
        let scale = metrics::scene_scale(w, h, ui_scale);
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
        let (_, label_y) =
            Self::page_content_metrics(page, w, h, ui_scale, panel_x, panel_y, panel_w);

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
            let t = Self::compute_try_it_layout(panel_x, panel_w, label_y, scale);
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

    fn preview_tile_placements(
        page_index: usize,
        page: &TutorialPage,
        panel_x: f32,
        panel_w: f32,
        label_y: f32,
        scale: f32,
    ) -> Vec<ShowcaseTilePlacement> {
        let group_count = page.groups.len().max(1) as f32;
        let group_w = panel_w * 0.74 / group_count;
        let start_x = panel_x + panel_w * 0.13 + group_w * 0.5;
        let mut next_id = 30_000u32;
        let mut placements = Vec::new();

        for (group_idx, group) in page.groups.iter().enumerate() {
            if page_index == TUTORIAL_PAGE_RELICS && group_idx == 0 {
                continue;
            }
            let center_x = start_x + group_idx as f32 * group_w;
            let rows: Vec<&[(Suit, u8)]> = if group.rows.is_empty() {
                vec![group.tiles]
            } else {
                group.rows.to_vec()
            };
            let widest_row_units = rows
                .iter()
                .map(|row| {
                    let tiles = row.len() as f32;
                    let accent_gaps = match group.layout {
                        TileGroupLayout::Flat => 0.0,
                        TileGroupLayout::FullHand => {
                            (row.len().saturating_sub(1) / 3) as f32 * 0.24
                        }
                        TileGroupLayout::Pairs => (row.len().saturating_sub(1) / 2) as f32 * 0.24,
                    };
                    tiles + (tiles - 1.0) * 0.02 + accent_gaps
                })
                .fold(1.0, f32::max);
            let tile_size = ((group_w * 0.68) / widest_row_units).clamp(18.0 * scale, 34.0 * scale);
            let step = tile_size * 1.02;
            let cluster_gap = tile_size * 0.24;
            let row_gap = tile_size * 0.94;
            let base_row_y = label_y - tile_size * 0.80 - 8.0 * scale;
            let top_row_y = base_row_y - row_gap * (rows.len().saturating_sub(1) as f32);

            for (row_idx, row) in rows.iter().enumerate() {
                let extra_gaps = match group.layout {
                    TileGroupLayout::Flat => 0,
                    TileGroupLayout::FullHand => row.len().saturating_sub(1) / 3,
                    TileGroupLayout::Pairs => row.len().saturating_sub(1) / 2,
                };
                let total_w = tile_size
                    + (row.len().saturating_sub(1) as f32) * step
                    + extra_gaps as f32 * cluster_gap;
                let mut x = center_x - total_w * 0.5 + tile_size * 0.5;
                let row_y = top_row_y + row_idx as f32 * row_gap;
                for (idx, (suit, rank)) in row.iter().copied().enumerate() {
                    let mut tile = Tile::new(suit, rank, next_id);
                    tile.debuffed_visual = group.debuffed_visual;
                    placements.push(ShowcaseTilePlacement {
                        tile,
                        center_pos: [x, row_y, 0.0],
                        rotation: [0.0, 0.0, 0.0],
                        scale: 1.0,
                        size_px: tile_size,
                        brightness: 1.08,
                        selected: false,
                    });
                    next_id += 1;
                    x += step;
                    let end_of_cluster = match group.layout {
                        TileGroupLayout::Flat => false,
                        TileGroupLayout::FullHand => (idx + 1) % 3 == 0 && idx + 1 < row.len(),
                        TileGroupLayout::Pairs => (idx + 1) % 2 == 0 && idx + 1 < row.len(),
                    };
                    if end_of_cluster {
                        x += cluster_gap;
                    }
                }
            }
        }
        placements
    }
}

impl SceneBehavior for TutorialCampaignScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        let items = self.flat_items(ctx.layout.window_w, ctx.layout.window_h, ctx.ui_scale);
        let action = self.tree.update_flat(
            &items,
            TreeInput {
                actions: ctx.actions,
                button_clicks: ctx.button_clicks,
                cursor_pos: ctx.cursor_pos,
                window: (ctx.layout.window_w, ctx.layout.window_h),
                ui_scale: ctx.ui_scale,
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
                    if let Some(ref mut onboarding) = ctx.run.onboarding {
                        onboarding.phase = OnboardingPhase::Shop;
                    }
                    Some(Scene::Shop(ShopScene::new_tutorial(ctx.run)))
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
        let ui_scale = ctx.ui_scale;
        let scale = metrics::scene_scale(w, h, ui_scale);
        let page = self.page();

        let mut bg_quads = Vec::new();
        let mut fg_quads = Vec::new();
        let mut texts = Vec::new();
        let mut relic_icons = Vec::new();
        let mut ribbon_placements = Vec::new();
        let mut talisman_placements = Vec::new();
        let mut pack_placements = Vec::new();
        let mut showcase_tiles = Vec::new();
        let mut wood_tablet_placements: Vec<WoodTabletPlacement> = Vec::new();
        let mut mirror_placement: Option<MirrorPlacement> = None;
        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);
        frame.starfield();
        frame.golden_dust();
        let cam_scale = h / 1600.0;
        frame.camera_override = Some(CameraParams {
            eye: [0.0, 1960.0 * cam_scale, 220.0 * cam_scale],
            target: [0.0, 0.0, 40.0 * cam_scale],
            up: [0.0, 1.0, 0.0],
            fovy_deg: 45.0,
        });

        let panel_x = w * 0.06;
        let panel_y = h * 0.07;
        let panel_w = w * 0.88;
        let panel_h = h * 0.84;
        bg_quads.push(GpuInstance {
            rect: [panel_x, panel_y, panel_w, panel_h],
            color: color::MIDNIGHT,
        });
        bg_quads.push(GpuInstance {
            rect: [panel_x, panel_y, panel_w, (2.0 * scale).max(1.0)],
            color: color::BRASS,
        });
        bg_quads.push(GpuInstance {
            rect: [panel_x, panel_y, (2.0 * scale).max(1.0), panel_h],
            color: color::BRASS,
        });
        bg_quads.push(GpuInstance {
            rect: [
                panel_x + panel_w - (2.0 * scale).max(1.0),
                panel_y,
                (2.0 * scale).max(1.0),
                panel_h,
            ],
            color: color::BRASS,
        });
        bg_quads.push(GpuInstance {
            rect: [
                panel_x,
                panel_y + panel_h - (2.0 * scale).max(1.0),
                panel_w,
                (2.0 * scale).max(1.0),
            ],
            color: color::BRASS,
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
            font_px: Some(30.0 * scale),
            ..Default::default()
        });

        let subtitle_x = panel_x + 30.0 * scale;
        let subtitle_y = panel_y + 70.0 * scale;
        let subtitle_w = panel_w - 60.0 * scale;
        let subtitle_font = typography::size(typography::BODY, h, ui_scale).max(15.0);
        let subtitle_lines = widget::wrap_text(page.subtitle, subtitle_w, subtitle_font);
        let subtitle_h = (subtitle_lines.len().max(1) as f32 * subtitle_font * 1.35)
            .max(70.0 * scale)
            .min(128.0 * scale);
        widget::push_text_block(
            &mut texts,
            [subtitle_x, subtitle_y, subtitle_w, subtitle_h],
            page.subtitle,
            TextStyle {
                tier: typography::BODY,
                color: color::PARCHMENT,
                padding: 0.0,
                align: TextAlign::Center,
            },
            h,
            ui_scale,
        );

        let (_tile_area_y, label_y) =
            Self::page_content_metrics(page, w, h, ui_scale, panel_x, panel_y, panel_w);
        if self.page != TUTORIAL_PAGE_SHOP {
            showcase_tiles =
                Self::preview_tile_placements(self.page, page, panel_x, panel_w, label_y, scale);
        }
        let group_w = panel_w * 0.74 / page.groups.len().max(1) as f32;
        for (idx, group) in page.groups.iter().enumerate() {
            let gx = panel_x + panel_w * 0.13 + idx as f32 * group_w;
            fg_quads.push(GpuInstance {
                rect: [gx + group_w * 0.14, label_y, group_w * 0.72, 4.0 * scale],
                color: group.accent,
            });
            texts.push(TextLabel {
                rect: [gx, label_y + 10.0 * scale, group_w, 22.0 * scale],
                text: group.label.to_string(),
                color: color::PARCHMENT,
                align: TextAlign::Center,
                font_px: Some(15.0 * scale),
                ..Default::default()
            });
        }

        if self.page == TUTORIAL_PAGE_SHOP {
            let item_y = label_y - 56.0 * scale;
            let center_x =
                |idx: usize| panel_x + panel_w * 0.13 + idx as f32 * group_w + group_w * 0.5;
            let relic_size = 42.0 * scale;
            relic_icons.push(RelicIcon {
                rect: [
                    center_x(0) - relic_size * 0.5,
                    item_y + 2.0 * scale,
                    relic_size,
                    relic_size,
                ],
                relic_id: RelicId::MerchantsEye,
            });
            ribbon_placements.push(Self::shop_preview_ribbon(center_x(1), item_y, h, scale));
            talisman_placements.push(Self::shop_preview_talisman(center_x(2), item_y, h, scale));
            pack_placements.push(Self::shop_preview_pack(center_x(3), item_y, h, scale));
        } else if self.page == TUTORIAL_PAGE_RELICS {
            let item_y = label_y - 56.0 * scale;
            let center_x =
                |idx: usize| panel_x + panel_w * 0.13 + idx as f32 * group_w + group_w * 0.5;
            let relic_size = 48.0 * scale;
            relic_icons.push(RelicIcon {
                rect: [
                    center_x(0) - relic_size * 0.5,
                    item_y,
                    relic_size,
                    relic_size,
                ],
                relic_id: RelicId::MerchantsEye,
            });
        }

        let try_it_layout = page
            .try_it_demo
            .then(|| Self::compute_try_it_layout(panel_x, panel_w, label_y, scale));
        let glossary_y = if let Some(ref t) = try_it_layout {
            t.content_floor_y
                .min(panel_y + panel_h * 0.62)
                .max(label_y + 132.0 * scale)
        } else {
            (label_y + 148.0 * scale).min(panel_y + panel_h * 0.60)
        };

        if let Some(ref layout) = try_it_layout {
            let heading_y = layout.play_rect[1] - 22.0 * scale;
            let try_it_lift = (28.0 * scale).max(20.0);
            let try_it_world_z_py_nudge = 18.0 * scale;
            let play_focused = self.tree.focused() == Some(TutorialNav::TryPlay.id());
            let trigger_focused = self.tree.focused() == Some(TutorialNav::TryTrigger.id());
            let trigger_enabled = self.try_it_phase == 1;
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
            mirror_placement = Some(MirrorPlacement {
                world_pos: TableAnchorPx {
                    px: play_center_x,
                    py: play_center_y + try_it_world_z_py_nudge,
                    lift_y: try_it_lift,
                }
                .to_draw_cmd_triple(),
                extents: [mirror_diam, mirror_diam, mirror_diam],
                hover: if play_focused { 1.0 } else { 0.0 },
                rotation_x_deg: 36.0,
                rotation_z_deg: (wobble_t * 2.4).sin() * 7.5,
            });
            wood_tablet_placements.push(WoodTabletPlacement {
                world_pos: TableAnchorPx {
                    px: trigger_center_x,
                    py: trigger_center_y + try_it_world_z_py_nudge,
                    lift_y: try_it_lift,
                }
                .to_draw_cmd_triple(),
                extents: [
                    layout.trigger_rect[2],
                    (layout.trigger_rect[3] * 0.35).max(8.0),
                    layout.trigger_rect[3],
                ],
                label: "Trigger".to_string(),
                pressed: 0.0,
                hover: if trigger_focused && trigger_enabled {
                    1.0
                } else {
                    0.0
                },
                rotation_z_deg: 0.0,
                disabled: !trigger_enabled,
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
                font_px: Some(16.0 * scale),
                ..Default::default()
            });
            let note_w = (150.0 * scale).max(120.0);
            let note_h = (54.0 * scale).max(40.0);
            let note_x = (layout.play_rect[0] - note_w - 22.0 * scale).max(panel_x + 28.0 * scale);
            let note_y = layout.play_rect[1] - 10.0 * scale;
            fg_quads.push(GpuInstance {
                rect: [note_x, note_y, note_w, note_h],
                color: color::alpha(color::CHAMPAGNE, 0.16),
            });
            fg_quads.push(GpuInstance {
                rect: [note_x, note_y, 3.0 * scale, note_h],
                color: color::GOLD,
            });
            widget::push_text_block(
                &mut texts,
                [
                    note_x + 10.0 * scale,
                    note_y + 8.0 * scale,
                    note_w - 18.0 * scale,
                    note_h - 12.0 * scale,
                ],
                "Note: a bronze mirror is an old round metal mirror. Here, it is the \"Commit Meld\" button.",
                TextStyle {
                    tier: typography::CAPTION,
                    color: color::CHAMPAGNE,
                    padding: 0.0,
                    align: TextAlign::Left,
                },
                h,
                ui_scale,
            );
            if let Some(line) = Self::try_it_demo_line(self.page, self.try_it_phase) {
                texts.push(TextLabel {
                    rect: [
                        panel_x + 24.0 * scale,
                        layout.demo_line_y,
                        panel_w - 48.0 * scale,
                        22.0 * scale,
                    ],
                    text: line.to_string(),
                    color: color::CHAMPAGNE,
                    align: TextAlign::Center,
                    font_px: Some(15.0 * scale),
                    ..Default::default()
                });
            }
        }

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
            font_px: Some(18.0 * scale),
            ..Default::default()
        });
        let term_w = if page.try_it_demo {
            panel_w * 0.42
        } else {
            panel_w * 0.34
        };
        let glossary_floor = panel_y + panel_h - 86.0 * scale;
        let glossary_available_h = (glossary_floor - (glossary_y + 28.0 * scale)).max(0.0);
        let mut term_font = (14.0 * scale).max(12.0);
        let min_term_font = (11.0 * scale).max(10.0);
        let (mut term_heights, mut glossary_total_h) =
            Self::glossary_term_metrics(page.glossary, term_w, term_font, scale);
        while glossary_total_h > glossary_available_h && term_font > min_term_font {
            term_font = (term_font - 1.0).max(min_term_font);
            let (next_heights, next_total_h) =
                Self::glossary_term_metrics(page.glossary, term_w, term_font, scale);
            term_heights = next_heights;
            glossary_total_h = next_total_h;
        }
        let mut gy = glossary_y + 28.0 * scale;
        for (idx, term) in page.glossary.iter().enumerate() {
            let term_h = term_heights.get(idx).copied().unwrap_or(term_font * 1.25);
            widget::push_text_block(
                &mut texts,
                [panel_x + 36.0 * scale, gy, term_w, term_h],
                term,
                TextStyle {
                    tier: typography::CAPTION,
                    color: color::MIST,
                    padding: 0.0,
                    align: TextAlign::Left,
                },
                h,
                ui_scale,
            );
            gy += term_h + 6.0 * scale;
        }

        if let Some(callout) = page.callout {
            let callout_x = if page.try_it_demo {
                panel_x + panel_w * 0.54
            } else {
                panel_x + panel_w * 0.47
            };
            let callout_y = glossary_y + 6.0 * scale;
            let callout_w = if page.try_it_demo {
                panel_w * 0.36
            } else {
                panel_w * 0.45
            };
            let callout_font = typography::size(typography::BODY, h, ui_scale).max(15.0);
            let callout_lines = widget::wrap_text(callout, callout_w - 32.0 * scale, callout_font);
            let callout_h = (callout_lines.len().max(1) as f32 * callout_font * 1.3 + 36.0 * scale)
                .max(112.0 * scale);
            fg_quads.push(GpuInstance {
                rect: [callout_x, callout_y, callout_w, callout_h],
                color: color::alpha(color::OBSIDIAN, 0.85),
            });
            fg_quads.push(GpuInstance {
                rect: [callout_x, callout_y, 4.0 * scale, callout_h],
                color: color::GOLD,
            });
            widget::push_text_block(
                &mut texts,
                [
                    callout_x + 18.0 * scale,
                    callout_y + 14.0 * scale,
                    callout_w - 32.0 * scale,
                    callout_h - 28.0 * scale,
                ],
                callout,
                TextStyle {
                    tier: typography::BODY,
                    color: color::CHAMPAGNE,
                    padding: 0.0,
                    align: TextAlign::Left,
                },
                h,
                ui_scale,
            );
        }

        let items = self.flat_items(w, h, ui_scale);
        let mut buttons = Vec::new();
        for item in &items {
            if matches!(item.action, TutorialNav::TryPlay | TutorialNav::TryTrigger) {
                continue;
            }
            let (label, variant, state) = match item.action {
                TutorialNav::Next => {
                    let label = if self.page + 1 == PAGES.len() {
                        "Open Shop"
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
                item.rect,
                label,
                variant,
                state,
                crate::ui::input::UiAction::Confirm,
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
            focus_nav::push_focus_ring(rect, scale, &mut fg_quads);
        }

        frame.quads(bg_quads);
        if !showcase_tiles.is_empty() {
            frame.cmds.push(DrawCmd::ShowcaseTileBatch(showcase_tiles));
        }
        frame.relic_icons(relic_icons);
        if !ribbon_placements.is_empty() {
            frame.zodiac_batch(ribbon_placements);
        }
        if !talisman_placements.is_empty() {
            frame.talisman_batch(talisman_placements);
        }
        if !pack_placements.is_empty() {
            frame.pack_batch(pack_placements);
        }
        if let Some(mirror) = mirror_placement {
            frame.mirror(mirror);
        }
        if !wood_tablet_placements.is_empty() {
            frame.wood_tablet_batch(wood_tablet_placements);
        }
        // Broad, forgiving lighting for educational showcase objects.
        let light_y = h * 0.18;
        for &(lx, ly, intensity) in &[
            (panel_x + panel_w * 0.24, label_y - 54.0 * scale, 1.95),
            (panel_x + panel_w * 0.50, label_y - 72.0 * scale, 2.15),
            (panel_x + panel_w * 0.76, label_y - 54.0 * scale, 1.95),
            (panel_x + panel_w * 0.34, label_y + 10.0 * scale, 1.10),
            (panel_x + panel_w * 0.66, label_y + 10.0 * scale, 1.10),
        ] {
            frame.point_lights.push(PointLight {
                pos: [lx, ly, light_y],
                radius: h * 0.95,
                color: [1.0, 0.96, 0.88],
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
