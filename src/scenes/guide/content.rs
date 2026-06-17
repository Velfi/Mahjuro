#![allow(unused_imports)]
use crate::core::hand::{MeldKind, validate_selection};
use crate::core::memorial_talisman::MemorialTalismanKind;
use crate::core::progression::PlayerProgress;
use crate::core::relic::{RelicId, all_relic_defs, relic_visual};
use crate::core::tag::TagKind;
use crate::core::talisman::TalismanKind;
use crate::core::tile::{Suit, Tile};
use crate::core::tile_pack::{PACK_ASPECT_W_OVER_H, TilePackKind};
use crate::core::yaku::{YakuKind, detect_yaku_with_wind};
use crate::core::zodiac::ZodiacKind;
use crate::persistence::TilePreset;
use crate::render::consumable_prop_scale::for_sale_talisman_tablet_extent;
use crate::render::decal::load_ui_font;
use crate::render::doc_tile_camera::{DOC_TILE_ROTATION, doc_tile_camera};
use crate::render::draw_cmd::{
    CameraParams, DrawCmd, ImageQuad, ImageQuadSource, Object3d, Object3dKind, SceneLighting,
    ShowcaseTilePlacement, UiFrame, camera_facing_euler_xyz_rad,
};
use crate::render::gameplay_glb;
use crate::render::scene_keys;
use crate::render::showcase_tile_layout::{
    ShowcaseTileLabelGaps, showcase_tile_group_label_anchor, showcase_tile_merge_projected_group,
};
use crate::render::table_transform::{
    compose_rotation_euler, mat4_to_euler_xyz_rad, rot_euler_xyz_rad,
};
use crate::render::theme::{ButtonState, ButtonVariant, color, metrics, typography};
use crate::render::vocabulary_colors::{GlossaryMode, text_effect_for_glossary_tint};
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextBlockVerticalAlign, TextLabel};
use crate::render::world_space::{
    object3d_pos_triple_for_world_center, world_on_camera_ray_plane_z,
};
use crate::ui::chart_primitives::{ChartClip, push_yaku_pill, yaku_pill_width};
use crate::ui::clip::intersect_rect;
use crate::ui::controller_hints::screen_footer_reserve;
use crate::ui::styled_text;
use crate::ui::styled_text::push_keyword_label;
use crate::ui::temptation_icons::temptation_icon_source;
use crate::ui::widget::{self, wrap_text};
use crate::ui::widget_tree::{FlatItem, FocusId, TreeState};

use crate::scenes::archive_career::{yaku_pill_face, yaku_pill_ink, yaku_pill_rim};
use crate::scenes::economy_intro_copy;
use crate::scenes::flowers_intro_copy;
use crate::scenes::header_chrome::{HeaderChromeMetrics, HeaderTitleLayout};
use crate::scenes::melds_intro_copy;
use crate::scenes::scoring_intro_copy;
use crate::scenes::tanuki_tips_intro_copy;
use crate::scenes::tiles_intro_copy;
use crate::scenes::yaku_intro_copy;
use crate::scenes::{BackgroundId, DrawCtx};

use glam::{Mat4, Quat, Vec3};

use super::yaku_chunk_for_page;
use super::yaku_page::meld_groups;
use super::{
    PAGE_ECONOMY, PAGE_FLOWERS, PAGE_MELDS, PAGE_SCORING, PAGE_TANUKI_TIPS, PAGE_TILES, PAGE_YAKU,
    YAKU_PAGE_START,
};

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

pub(super) fn tile_group(label: &'static str, tiles: Vec<Tile>, accent: [f32; 4]) -> TileGroup {
    TileGroup {
        label,
        tiles,
        accent,
        subtitle: None,
        framed: false,
    }
}

pub(super) fn tile_group_with_subtitle(
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

pub(super) fn tile_group_framed(
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
pub(super) struct MeldLabel {
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) w: f32,
    pub(super) underline_y: f32,
    pub(super) text: String,
    pub(super) color: [f32; 4],
}

/// Per-tile yaw wobble for invalid guide examples (melds invalid-sequence pattern).
pub(super) const GUIDE_INVALID_TILE_WOBBLE: [f32; 3] = [0.14, -0.11, 0.09];

pub(super) fn guide_example_is_invalid(group: &TileGroup) -> bool {
    group.label.starts_with("Invalid")
}

pub(super) fn guide_invalid_tile_rotation(tile_i: usize) -> [f32; 3] {
    let wobble = GUIDE_INVALID_TILE_WOBBLE[tile_i % GUIDE_INVALID_TILE_WOBBLE.len()];
    [
        DOC_TILE_ROTATION[0],
        DOC_TILE_ROTATION[1],
        DOC_TILE_ROTATION[2] + wobble,
    ]
}

/// Convenience tile constructor.
pub(super) fn t(suit: Suit, rank: u8, id: u32) -> Tile {
    Tile::new(suit, rank, id)
}

pub(super) fn suit_ranks(suit: Suit, id_base: u32) -> Vec<Tile> {
    (1..=9u8)
        .map(|rank| t(suit, rank, id_base + rank as u32 - 1))
        .collect()
}

/// Optional in-universe margin scrawl for a page. Rendered below the tile
/// area in faded italic to feel like a player's aside left on the guide.
pub(super) fn page_graffiti(page: usize) -> Option<&'static str> {
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
pub(crate) fn page_content(
    page: usize,
    progress: &PlayerProgress,
) -> (&'static str, Vec<TileGroup>) {
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
                    "+5 Fu",
                    vec![t(Suit::Pinzu, 5, 9)],
                    Suit::Pinzu.keyword_color(),
                ),
                tile_group_with_subtitle(
                    "Red Dragon",
                    "+12 Fu",
                    vec![t(Suit::Dragon, 1, 10)],
                    Suit::Dragon.keyword_color(),
                ),
                tile_group_with_subtitle(
                    "Flower",
                    "+0 Fu",
                    vec![t(Suit::Flower, 1, 11)],
                    Suit::Flower.keyword_color(),
                ),
                {
                    let mut tile = t(Suit::Pinzu, 1, 12);
                    tile.debuffed_visual = true;
                    tile_group_with_subtitle("Debuffed tile", "+0 Fu", vec![tile], color::STONE)
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
