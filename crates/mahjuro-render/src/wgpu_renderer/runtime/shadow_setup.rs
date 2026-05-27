use super::*;
use crate::draw_cmd::SHOP_INSPECT_SUBJECT_ANIM_ID;
use crate::draw_cmd::{DrawCmd, UiFrame};
use crate::lit_mesh::{PunctualShadowSlotGpu, ShadowGlobals};
use crate::lit_mesh::{
    LitMeshInstance, MaterialKind, ShadowCasterUniform, material_casts_shadow,
};
use crate::punctual_shadow_atlas::{
    PUNCTUAL_SHADOW_ATLAS_SIZE, PUNCTUAL_SHADOW_TILE_SIZE, PunctualShadowLightSetup,
    gameplay_candle_punctual_shadow_setup,
};
use crate::room_gi_bake::{RoomGiRoom, room_gi_room_index};
use crate::punctual_shadow_atlas::MAX_PUNCTUAL_SHADOW_LIGHTS;

/// Which imported room mesh is active this frame (at most one is drawn).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ActiveRoomEnv {
    Shop,
    Hallway,
    Staircase,
    Archive,
    MainMenu,
    Gameplay,
}

#[inline]
pub fn active_room_env(frame: &UiFrame) -> Option<ActiveRoomEnv> {
    for cmd in &frame.cmds {
        match cmd {
            DrawCmd::ShopEnvironment => return Some(ActiveRoomEnv::Shop),
            DrawCmd::HallwayEnvironment => return Some(ActiveRoomEnv::Hallway),
            DrawCmd::StaircaseEnvironment => return Some(ActiveRoomEnv::Staircase),
            DrawCmd::ArchiveEnvironment => return Some(ActiveRoomEnv::Archive),
            DrawCmd::MainMenuEnvironment => return Some(ActiveRoomEnv::MainMenu),
            DrawCmd::GameplayEnvironment => return Some(ActiveRoomEnv::Gameplay),
            _ => {}
        }
    }
    None
}

impl ActiveRoomEnv {
    #[inline]
    pub fn to_room_gi(self) -> Option<RoomGiRoom> {
        match self {
            Self::Shop => Some(RoomGiRoom::Shop),
            Self::Hallway => Some(RoomGiRoom::Hallway),
            Self::Staircase => Some(RoomGiRoom::Staircase),
            Self::Archive => Some(RoomGiRoom::Archive),
            Self::MainMenu => Some(RoomGiRoom::MainMenu),
            Self::Gameplay => Some(RoomGiRoom::Gameplay),
        }
    }
}

pub(super) const SHADOW_MAP_SIZE: f32 = PUNCTUAL_SHADOW_ATLAS_SIZE as f32;

/// The active imported room has a loaded offline shadow field (`.msh`) on disk.
#[inline]
pub(super) fn room_has_baked_shadow_asset(
    env: ActiveRoomEnv,
    baked_room: Option<RoomGiRoom>,
) -> bool {
    env.to_room_gi()
        .zip(baked_room)
        .is_some_and(|(frame_room, loaded)| frame_room == loaded)
}

/// Gameplay uses per-candle shadow tiles instead of the single key-light map.
#[inline]
pub(super) fn gameplay_punctual_shadows_active(
    frame: &UiFrame,
    active_scene_key: Option<&str>,
) -> bool {
    active_scene_key == Some("gameplay")
        && frame.scene_lighting.embedded_gltf_punctual
        && active_room_env(frame) == Some(ActiveRoomEnv::Gameplay)
}

#[inline]
fn punctual_shadow_hash(lights: &[PunctualShadowLightSetup]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    lights.len().hash(&mut h);
    for light in lights {
        light.light_view_proj.to_cols_array().map(f32::to_bits).hash(&mut h);
        light.atlas_rect.map(f32::to_bits).hash(&mut h);
    }
    h.finish()
}

#[allow(dead_code)]
pub(super) struct PunctualShadowFrame {
    pub lights: Vec<PunctualShadowLightSetup>,
    pub changed: bool,
}

impl WgpuRenderer {
    pub(super) fn prepare_punctual_shadow_frame(
        &mut self,
        frame: &UiFrame,
        camera: &CameraFrame,
        shadows_enabled: bool,
    ) -> PunctualShadowFrame {
        if !(shadows_enabled && gameplay_punctual_shadows_active(frame, self.active_scene_key)) {
            self.punctual_shadow_lights.clear();
            return PunctualShadowFrame {
                lights: Vec::new(),
                changed: false,
            };
        }
        let env = self.active_frame_env();
        let lights = gameplay_candle_punctual_shadow_setup(camera.h, env.height_scale);
        let hash = punctual_shadow_hash(&lights);
        let changed = hash != self.cached_punctual_shadow_hash;
        self.cached_punctual_shadow_hash = hash;
        self.punctual_shadow_lights = lights.clone();
        PunctualShadowFrame { lights, changed }
    }

    pub(super) fn upload_punctual_shadow_globals(
        &self,
        shadows_enabled: bool,
        depth_bias: f32,
        lights: &[PunctualShadowLightSetup],
    ) {
        let tile_texel = 1.0 / PUNCTUAL_SHADOW_TILE_SIZE as f32;
        let count = lights.len().min(MAX_PUNCTUAL_SHADOW_LIGHTS) as f32;
        let mut globals = ShadowGlobals::empty_punctual();
        globals.params = [
            if shadows_enabled { 1.0 } else { 0.0 },
            depth_bias,
            1.0 / SHADOW_MAP_SIZE,
            0.0,
        ];
        globals.punctual_params = [
            count,
            tile_texel,
            if shadows_enabled && count > 0.0 {
                1.0
            } else {
                0.0
            },
            0.0,
        ];
        for (i, setup) in lights.iter().take(MAX_PUNCTUAL_SHADOW_LIGHTS).enumerate() {
            globals.punctual_lights[i] = PunctualShadowSlotGpu {
                light_view_proj: setup.light_view_proj.to_cols_array(),
                atlas_rect: setup.atlas_rect,
            };
        }
        self.queue.write_buffer(
            &self.shadow_globals_buffer,
            0,
            bytemuck::bytes_of(&globals),
        );
    }
}
#[inline]
pub(super) fn room_baked_shadow_loaded(
    slots: &[crate::wgpu_renderer::impl_room_shadow::RoomBakedShadowGpu;
         crate::room_gi_bake::ROOM_GI_ROOM_COUNT],
    env: ActiveRoomEnv,
) -> Option<RoomGiRoom> {
    let room = env.to_room_gi()?;
    let _ = &slots[room_gi_room_index(room)];
    Some(room)
}

/// Whether this room samples its offline `.msh` instead of the live room shadow pass.
///
/// Archive is excluded: the room GLB uses punctual lights only (`COLOR_0.a = 3` on all shell
/// meshes). Offline cubby-only bakes still mis-darken receivers when the asset grows (30+ prims).
#[inline]
pub(crate) fn room_env_uses_offline_baked_shadow(
    active_room: Option<ActiveRoomEnv>,
    baked_room: Option<RoomGiRoom>,
) -> bool {
    let Some(env) = active_room else {
        return false;
    };
    if env == ActiveRoomEnv::Archive {
        return false;
    }
    room_has_baked_shadow_asset(env, baked_room)
}

/// Room GLB must not be redrawn into the live 1024² map when offline baked sampling is active.
/// Offline capture (`room_shadow_capture_pending`) always redraws the shell once.
#[inline]
pub(super) fn skip_room_env_live_shadow_pass(
    active_room: Option<ActiveRoomEnv>,
    baked_room: Option<RoomGiRoom>,
    capturing_offline_bake: bool,
) -> bool {
    if capturing_offline_bake {
        return false;
    }
    room_env_uses_offline_baked_shadow(active_room, baked_room)
}

/// Whether this catalog prop should cast into the live shadow map.
#[inline]
pub(super) fn object3d_casts_dynamic_shadow(
    active_room: Option<ActiveRoomEnv>,
    anim_id: u64,
) -> bool {
    match active_room {
        None
        | Some(ActiveRoomEnv::Shop)
        | Some(ActiveRoomEnv::Hallway)
        | Some(ActiveRoomEnv::MainMenu)
        | Some(ActiveRoomEnv::Gameplay)
        | Some(ActiveRoomEnv::Staircase) => true,
        // Baked archive contact is static; the live map is for inspect orbit only.
        Some(ActiveRoomEnv::Archive) => anim_id == SHOP_INSPECT_SUBJECT_ANIM_ID,
    }
}

/// Per-frame shadow uniform target passed into [`WgpuRenderer::run_object3d_placement`].
pub(super) struct Object3dShadowCtx<'a> {
    pub light_view_proj: [f32; 16],
    pub changed: &'a mut bool,
}

impl WgpuRenderer {
    #[inline]
    pub(super) fn reset_shop_inspect_shadow_slot(&mut self) {
        self.shop_inspect_subject_shadow_slot = None;
    }

    #[inline]
    pub(super) fn register_placement_shadow_slot(
        &mut self,
        draw_kind: super::DrawKind,
        slot_i: usize,
    ) {
        if self.shadow_placement_anim_id == SHOP_INSPECT_SUBJECT_ANIM_ID {
            self.shop_inspect_subject_shadow_slot = Some((draw_kind, slot_i));
        }
    }

    #[inline]
    pub(super) fn placement_shadow_writes(&self, frame: &UiFrame) -> bool {
        frame.shop_inspect_shadow_target.is_none()
            || self.shadow_placement_anim_id == SHOP_INSPECT_SUBJECT_ANIM_ID
    }

    #[inline]
    pub(super) fn write_lit_mesh_shadow(
        &self,
        shadow: &mut Option<&mut Object3dShadowCtx<'_>>,
        inst: &LitMeshInstance,
        model: glam::Mat4,
        material: MaterialKind,
    ) {
        if let Some(shadow) = shadow.as_deref_mut()
            && object3d_casts_dynamic_shadow(
                self.placement_shadow_room,
                self.shadow_placement_anim_id,
            )
            && material_casts_shadow(material)
        {
            *shadow.changed |=
                inst.write_shadow_uniform(&self.queue, shadow.light_view_proj, model);
        }
    }

    /// Write per-primitive room-GLB shadow caster uniforms (`light_view_proj * model`).
    pub(super) fn write_room_env_shadow_caster(
        &self,
        gpu: &crate::wgpu_renderer::ShopEnvironmentGpu,
        light_view_proj: [f32; 16],
        base_model: glam::Mat4,
        anim_deltas: &rustc_hash::FxHashMap<usize, glam::Mat4>,
        changed: &mut bool,
    ) {
        for (pi, buf) in gpu.shadow_uniform_buffers.iter().enumerate() {
            let prim_model = if let Some(delta) = anim_deltas.get(&pi) {
                base_model * *delta
            } else {
                base_model
            };
            self.queue.write_buffer(
                buf,
                0,
                bytemuck::bytes_of(&ShadowCasterUniform {
                    light_view_proj,
                    model: prim_model.to_cols_array(),
                }),
            );
        }
        *changed = true;
    }
}

/// Table-scale shadow depth (world units along the key light).
const TABLE_SHADOW_SCENE_DEPTH: f32 = 80.0;
const TABLE_SHADOW_DEPTH_BIAS: f32 = 0.005;

#[inline]
fn frame_draws_room_environment(frame: &crate::draw_cmd::UiFrame) -> bool {
    frame.cmds.iter().any(|cmd| {
        matches!(
            cmd,
            DrawCmd::ShopEnvironment
                | DrawCmd::HallwayEnvironment
                | DrawCmd::StaircaseEnvironment
                | DrawCmd::ArchiveEnvironment
                | DrawCmd::MainMenuEnvironment
                | DrawCmd::GameplayEnvironment
        )
    })
}

#[allow(dead_code)] // light_view_proj kept available for future per-caster passes.
pub(super) struct ShadowFrame {
    pub light_view_proj: Mat4,
    pub light_view_proj_arr: [f32; 16],
}

impl WgpuRenderer {
    /// Build the directional shadow camera and upload the shared shadow
    /// globals uniform consumed by the shadow pre-pass + lit_mesh PCF tap.
    /// Returns the light view-proj so callers can write per-caster shadow
    /// uniforms in the same frame.
    pub(super) fn setup_shadow_frame(
        &self,
        camera: &CameraFrame,
        shadows_enabled: bool,
        frame: &crate::draw_cmd::UiFrame,
    ) -> ShadowFrame {
        // Anchor the shadow frustum to the same key direction the lit
        // shaders use. The orthographic frustum is sized to cover the play
        // area where casters live, not the full table — most of the table
        // is empty wood and would burn shadow texels for nothing.
        let key_dir = glam::Vec3::new(0.25, 1.0, 0.35).normalize();
        // Half-extents in world units. Generous so candles + relics on the
        // sides of the play area stay inside the frustum at any window
        // aspect.
        let room_frustum = frame_draws_room_environment(frame)
            || matches!(
                self.active_scene_key,
                Some(
                    "shop"
                        | "tile_pack_celebration"
                        | "pick_chamber"
                        | "main_menu_exterior"
                        | "collection"
                )
            );
        let inspect_subject = frame.shop_inspect_shadow_target.map(glam::Vec3::from_array);
        let (shadow_half_x, shadow_half_z, scene_height, depth_bias) = if inspect_subject.is_some()
        {
            let env_height_scale = if self.active_scene_key == Some("main_menu_exterior") {
                crate::main_menu_glb::main_menu_env_height_scale(
                    self.active_frame_env().height_scale,
                )
            } else {
                self.active_frame_env().height_scale
            };
            let extent = camera.h * env_height_scale;
            let half = extent * 0.11;
            let depth = extent * 0.42;
            (half, half, depth, TABLE_SHADOW_DEPTH_BIAS)
        } else if room_frustum {
            // Room GLBs are centered and scaled by `window_h` — extend the light
            // frustum in depth so shelf props cast contact shadows on the env.
            let env_height_scale = if self.active_scene_key == Some("main_menu_exterior") {
                crate::main_menu_glb::main_menu_env_height_scale(
                    self.active_frame_env().height_scale,
                )
            } else {
                self.active_frame_env().height_scale
            };
            let extent = camera.h * env_height_scale;
            let archive_offline_bake = self.room_shadow_capture_pending
                == Some(crate::room_gi_bake::RoomGiRoom::Archive);
            // Archive `.msh` only bakes cubby casters — tighter XY keeps texels on the grid.
            let (half_mul, depth_mul) = if archive_offline_bake {
                (0.44, 1.15)
            } else {
                (0.62, 1.45)
            };
            let half = extent * half_mul;
            let depth = extent * depth_mul;
            // PCF bias is in light clip Z (≈ texels), not world depth — scaling with the
            // room ortho span was ~20× too large and washed out GLB self-shadow.
            let bias = TABLE_SHADOW_DEPTH_BIAS;
            (half, half, depth, bias)
        } else {
            let half = (camera.w * 0.6).max(camera.h * 0.6);
            (
                half,
                half,
                TABLE_SHADOW_SCENE_DEPTH,
                TABLE_SHADOW_DEPTH_BIAS,
            )
        };
        // Light eye sits along +key_dir from the play-area center. Table scenes
        // keep a tight depth range for texel resolution; room scenes need more.
        let shadow_center = inspect_subject.unwrap_or(glam::Vec3::ZERO);
        let eye_dist = scene_height * 0.5;
        let shadow_eye = shadow_center + key_dir * eye_dist;
        let shadow_view = Mat4::look_at_rh(shadow_eye, shadow_center, glam::Vec3::Y);
        let shadow_proj = Mat4::orthographic_rh(
            -shadow_half_x,
            shadow_half_x,
            -shadow_half_z,
            shadow_half_z,
            0.1,
            scene_height,
        );
        let light_view_proj = shadow_proj * shadow_view;
        let light_view_proj_arr = light_view_proj.to_cols_array();
        // Tile-pack / zodiac showcase overlays: perspective shop camera on a
        // black void — the gameplay-table shadow frustum does not cover them,
        // so PCF reads as fully occluded and meshes vanish.
        let pack_celeb_black_void = (self.active_scene_key == Some("showcase")
            && frame.showcase_render_hints.tile_pack_celebration_tonemap
            && !frame_draws_room_environment(frame))
            || frame.showcase_render_hints.modal_relic_staging;
        let shadow_enabled_flag = if shadows_enabled && !pack_celeb_black_void {
            1.0_f32
        } else {
            0.0
        };
        let punctual = gameplay_punctual_shadows_active(frame, self.active_scene_key);
        if !punctual {
            let mut globals = ShadowGlobals::empty_punctual();
            globals.light_view_proj = light_view_proj_arr;
            globals.params = [shadow_enabled_flag, depth_bias, 1.0 / SHADOW_MAP_SIZE, 0.0];
            self.queue.write_buffer(
                &self.shadow_globals_buffer,
                0,
                bytemuck::bytes_of(&globals),
            );
        }
        ShadowFrame {
            light_view_proj,
            light_view_proj_arr,
        }
    }

    /// Per-instance shadow caster uniforms for the first (or key) light.
    pub(super) fn write_per_instance_shadow_casters(
        &mut self,
        _frame: &UiFrame,
        _camera: &CameraFrame,
        light_view_proj_arr: [f32; 16],
        tile_pick_models: &[(usize, Mat4)],
        shadow_uniforms_changed: &mut bool,
    ) {
        for (i, model) in tile_pick_models {
            if let Some(htg) = self.hand_tiles.get(*i) {
                self.queue.write_buffer(
                    &htg.shadow_uniform_buffer,
                    0,
                    bytemuck::bytes_of(&ShadowCasterUniform {
                        light_view_proj: light_view_proj_arr,
                        model: model.to_cols_array(),
                    }),
                );
                *shadow_uniforms_changed = true;
            }
        }
    }

    /// Rewrite every dynamic caster's shadow uniform for one punctual atlas light.
    pub(super) fn rewrite_shadow_casters_for_light(
        &self,
        light_view_proj_arr: [f32; 16],
        object3d_draw_list: &[(super::DrawKind, usize)],
        tile_pick_models: &[(usize, glam::Mat4)],
        showcase_tile_batches: &[&[super::ShowcaseTilePlacement]],
    ) {
        for &(kind, slot_i) in object3d_draw_list {
            self.rewrite_object3d_shadow_light(kind, slot_i, light_view_proj_arr);
        }
        for (i, model) in tile_pick_models {
            if let Some(htg) = self.hand_tiles.get(*i) {
                self.queue.write_buffer(
                    &htg.shadow_uniform_buffer,
                    0,
                    bytemuck::bytes_of(&ShadowCasterUniform {
                        light_view_proj: light_view_proj_arr,
                        model: model.to_cols_array(),
                    }),
                );
            }
        }
        let total_showcase: usize = showcase_tile_batches
            .iter()
            .map(|b| b.len())
            .sum::<usize>()
            .min(super::super::MAX_SHOWCASE_TILE_SLOTS);
        for slot_i in 0..total_showcase {
            let Some(stg) = self.showcase_tiles.get(slot_i) else {
                break;
            };
            self.queue.write_buffer(
                &stg.shadow_uniform_buffer,
                0,
                bytemuck::bytes_of(&ShadowCasterUniform {
                    light_view_proj: light_view_proj_arr,
                    model: stg.cached_shadow_caster.model,
                }),
            );
        }
    }

    fn rewrite_object3d_shadow_light(
        &self,
        kind: super::DrawKind,
        slot_i: usize,
        light_view_proj_arr: [f32; 16],
    ) {
        macro_rules! rewrite {
            ($pool:expr) => {
                if let Some(inst) = $pool.get(slot_i) {
                    inst.rewrite_shadow_light_view_proj(&self.queue, light_view_proj_arr);
                }
            };
        }
        match kind {
            super::DrawKind::YakuTablet => rewrite!(self.yaku_tablet_instances),
            super::DrawKind::WoodTablet => rewrite!(self.wood_tablet_instances),
            super::DrawKind::Book => rewrite!(self.book_instances),
            super::DrawKind::BookCover => rewrite!(self.book_cover_instances),
            super::DrawKind::Relic => rewrite!(self.relic_instances),
            super::DrawKind::BossIcon => rewrite!(self.ordeal_icon_instances),
            super::DrawKind::Pack => rewrite!(self.pack_instances),
            super::DrawKind::Ribbon => rewrite!(self.ribbon_instances),
            super::DrawKind::Talisman => rewrite!(self.talisman_instances),
            super::DrawKind::BugBody => rewrite!(self.bug_body_instances),
            super::DrawKind::BugWingL => rewrite!(self.bug_wing_instances),
            super::DrawKind::BugWingR => rewrite!(self.bug_wing_r_instances),
            super::DrawKind::BugWingBlurL => rewrite!(self.bug_wing_blur_instances),
            super::DrawKind::BugWingBlurR => rewrite!(self.bug_wing_blur_r_instances),
            super::DrawKind::Orb => rewrite!(self.orb_instances),
            super::DrawKind::Bowl => rewrite!(self.bowl_instances),
            super::DrawKind::Mirror => rewrite!(self.mirror_instances),
            super::DrawKind::TallyStickBase | super::DrawKind::TallyStickTip => {
                rewrite!(self.tally_stick_instances)
            }
            super::DrawKind::ExtrudedGlyph => rewrite!(self.extruded_glyph_instances),
            super::DrawKind::Primitive(shape) => {
                if let Some(pool) = self.primitive_instances.get(&shape) {
                    rewrite!(pool);
                }
            }
        }
    }
}
