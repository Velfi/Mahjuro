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
use crate::render::wgpu_renderer::{
    GpuInstance, TextAlign, TextBlockVerticalAlign, TextLabel,
};
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

use super::content::t;
use super::TileGroup;
use super::tile_layout::yaku_shape_text;

/// Build tile groups for a yaku example hand. The rule string is [`yaku_shape_text`]
/// (journal plaque + table Rule column).
pub(crate) fn yaku_page(yk: YakuKind) -> (&'static str, Vec<TileGroup>) {
    let rule = yaku_shape_text(yk);
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
        YakuKind::Shousangen => meld_groups(&[
            (
                "Triplet",
                MeldKind::Triplet,
                Suit::Dragon,
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
                "Sequence",
                MeldKind::Sequence,
                Suit::Manzu,
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
            ("Pair", MeldKind::Pair, Suit::Dragon, &[3, 3], pair_color),
        ]),
        YakuKind::Daisangen => meld_groups(&[
            (
                "Triplet",
                MeldKind::Triplet,
                Suit::Dragon,
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
                "Triplet",
                MeldKind::Triplet,
                Suit::Dragon,
                &[3, 3, 3],
                trip_color,
            ),
            (
                "Sequence",
                MeldKind::Sequence,
                Suit::Manzu,
                &[4, 5, 6],
                seq_color,
            ),
            ("Pair", MeldKind::Pair, Suit::Pinzu, &[8, 8], pair_color),
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
                "Sequence",
                MeldKind::Sequence,
                Suit::Manzu,
                &[1, 2, 3],
                seq_color,
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
                "Sequence",
                MeldKind::Sequence,
                Suit::Souzu,
                &[7, 8, 9],
                seq_color,
            ),
            ("Pair", MeldKind::Pair, Suit::Pinzu, &[9, 9], pair_color),
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
            ("Pair", MeldKind::Pair, Suit::Manzu, &[1, 1], pair_color),
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
    (rule, groups)
}

/// `(label, kind, suit, ranks, accent)` descriptor for a single meld row.
pub(super) type MeldSpec = (&'static str, MeldKind, Suit, &'static [u8], [f32; 4]);

/// Build `TileGroup`s from a compact descriptor list. Assigns sequential tile
/// ids across all groups so the renderer treats each tile as unique.
pub(super) fn meld_groups(specs: &[MeldSpec]) -> Vec<TileGroup> {
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
