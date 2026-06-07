//! Gameplay debug visibility toggles (Debug → Overlays → HUD Visibility…).

use crate::render::draw_cmd::{DrawCmd, Object3dKind, ShowcaseTilePlacement, TileOverlayRectGroup};

/// Per-layer hide flags for the gameplay scene. `true` = hidden.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DebugVisibility {
    pub hide_environment: bool,
    pub hide_candle_lights: bool,
    pub hide_hand_tiles: bool,
    pub hide_structure_tiles: bool,
    pub hide_discard_tiles: bool,
    pub hide_plinth_tiles: bool,
    pub hide_relics: bool,
    pub hide_yen_pile: bool,
    pub hide_yen_label: bool,
    pub hide_wall_hud: bool,
    pub hide_score_readout: bool,
    pub hide_play_tally_fan: bool,
    pub hide_discard_tally_fan: bool,
    pub hide_score_popups: bool,
    pub hide_cascade_hud: bool,
    pub hide_yaku_tablets: bool,
    pub hide_discard_bowl: bool,
    pub hide_play_mirror: bool,
    pub hide_journal: bool,
    pub hide_wood_tablets: bool,
    pub hide_consumables: bool,
    pub hide_flying_coins: bool,
    pub hide_boss_icon: bool,
}

pub const GAMEPLAY_VIS_ROW_COUNT: usize = 23;
pub const GAMEPLAY_VIS_VISIBLE_ROWS: usize = 10;

impl DebugVisibility {
    pub fn flag_mut(&mut self, row: usize) -> Option<&mut bool> {
        Some(match row {
            0 => &mut self.hide_environment,
            1 => &mut self.hide_candle_lights,
            2 => &mut self.hide_hand_tiles,
            3 => &mut self.hide_structure_tiles,
            4 => &mut self.hide_discard_tiles,
            5 => &mut self.hide_plinth_tiles,
            6 => &mut self.hide_relics,
            7 => &mut self.hide_yen_pile,
            8 => &mut self.hide_yen_label,
            9 => &mut self.hide_wall_hud,
            10 => &mut self.hide_score_readout,
            11 => &mut self.hide_play_tally_fan,
            12 => &mut self.hide_discard_tally_fan,
            13 => &mut self.hide_score_popups,
            14 => &mut self.hide_cascade_hud,
            15 => &mut self.hide_yaku_tablets,
            16 => &mut self.hide_discard_bowl,
            17 => &mut self.hide_play_mirror,
            18 => &mut self.hide_journal,
            19 => &mut self.hide_wood_tablets,
            20 => &mut self.hide_consumables,
            21 => &mut self.hide_flying_coins,
            22 => &mut self.hide_boss_icon,
            _ => return None,
        })
    }

    pub fn label_checked(&self, row: usize) -> (&'static str, bool) {
        match row {
            0 => ("Room (gameplay.glb)", self.hide_environment),
            1 => ("Candle lights + flames", self.hide_candle_lights),
            2 => ("Hand tiles", self.hide_hand_tiles),
            3 => ("Structure showcase tiles", self.hide_structure_tiles),
            4 => ("Discard river tiles", self.hide_discard_tiles),
            5 => ("Plinth tiles (dora / wind)", self.hide_plinth_tiles),
            6 => ("Relics", self.hide_relics),
            7 => ("Gold coin pile", self.hide_yen_pile),
            8 => ("Gold amount label", self.hide_yen_label),
            9 => ("Wall tiles HUD", self.hide_wall_hud),
            10 => ("Score readout (0 / N)", self.hide_score_readout),
            11 => ("Play tally fan", self.hide_play_tally_fan),
            12 => ("Discard tally fan", self.hide_discard_tally_fan),
            13 => ("Score popups (+50, ×3)", self.hide_score_popups),
            14 => ("Cascade HUD glyphs", self.hide_cascade_hud),
            15 => ("Yaku tablets", self.hide_yaku_tablets),
            16 => ("Discard bowl", self.hide_discard_bowl),
            17 => ("Play mirror", self.hide_play_mirror),
            18 => ("Yaku journal book", self.hide_journal),
            19 => ("Wood tablets (cash in)", self.hide_wood_tablets),
            20 => ("Consumables (ribbon / talisman)", self.hide_consumables),
            21 => ("Flying coins", self.hide_flying_coins),
            22 => ("Boss ordeal icon", self.hide_boss_icon),
            _ => ("", false),
        }
    }

    pub fn any_hide(&self) -> bool {
        (0..GAMEPLAY_VIS_ROW_COUNT).any(|i| self.label_checked(i).1)
    }

    #[inline]
    pub fn hide_object3d_kind(&self, kind: &Object3dKind) -> bool {
        match kind {
            Object3dKind::Relic { .. } => self.hide_relics,
            Object3dKind::YakuTablet { .. } => self.hide_yaku_tablets,
            Object3dKind::WoodTablet { .. } => self.hide_wood_tablets,
            Object3dKind::Book { .. } => self.hide_journal,
            Object3dKind::Bowl => self.hide_discard_bowl,
            Object3dKind::Mirror { .. } => self.hide_play_mirror,
            Object3dKind::TallyFan { kind, .. } => match kind {
                crate::render::draw_cmd::TallyFanKind::Draws => self.hide_play_tally_fan,
                crate::render::draw_cmd::TallyFanKind::Discards => self.hide_discard_tally_fan,
            },
            Object3dKind::ZodiacRibbon { .. }
            | Object3dKind::Talisman { .. }
            | Object3dKind::MemorialTalisman { .. } => self.hide_consumables,
            Object3dKind::Primitive { shape, .. } => {
                use crate::render::primitive::MeshId;
                match shape {
                    MeshId::Coin => self.hide_yen_pile,
                    MeshId::Cylinder => self.hide_yen_pile,
                    _ => false,
                }
            }
            _ => false,
        }
    }
}

/// Drop hidden gameplay draw cmds after the scene builds its frame.
pub fn filter_gameplay_frame_cmds(
    frame: &mut crate::render::draw_cmd::UiFrame,
    vis: &DebugVisibility,
) {
    if !vis.any_hide() {
        return;
    }

    if vis.hide_environment {
        frame
            .cmds
            .retain(|c| !matches!(c, DrawCmd::GameplayEnvironment));
    }

    if vis.hide_candle_lights {
        frame.scene_lighting.punctual.clear();
        frame.scene_lighting.clear_spot_lights();
        frame.scene_lighting.embedded_gltf_punctual = false;
        frame.candle_light_count = 0;
        frame.procedural_flame_emitters.clear();
    }

    frame.cmds.retain_mut(|cmd| match cmd {
        DrawCmd::ShowcaseTileBatch(batch) => {
            if vis.hide_hand_tiles || vis.hide_discard_tiles || vis.hide_plinth_tiles {
                batch.placements.retain(|p| !showcase_tile_hidden(p, vis));
            }
            !batch.placements.is_empty()
        }
        DrawCmd::Object3d(obj) => !vis.hide_object3d_kind(&obj.kind),
        DrawCmd::Object3dBatch(batch) => {
            if vis.any_hide() {
                batch.retain(|o| !vis.hide_object3d_kind(&o.kind));
            }
            !batch.is_empty()
        }
        _ => true,
    });
}

fn showcase_tile_hidden(p: &ShowcaseTilePlacement, vis: &DebugVisibility) -> bool {
    match p.overlay_rect_group {
        Some(TileOverlayRectGroup::DoraTiles) | Some(TileOverlayRectGroup::RoundWindTiles) => {
            vis.hide_plinth_tiles
        }
        None => {
            if p.pick_id.is_some() {
                vis.hide_hand_tiles
            } else {
                vis.hide_discard_tiles
            }
        }
    }
}
