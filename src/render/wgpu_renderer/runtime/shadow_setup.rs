use super::*;
use crate::render::draw_cmd::SHOP_INSPECT_SUBJECT_ANIM_ID;
use crate::render::draw_cmd::{DrawCmd, UiFrame};
use crate::render::lit_mesh::{
    LitMeshInstance, MaterialKind, ShadowCasterUniform, material_casts_shadow,
};
use crate::render::room_gi_bake::{RoomGiRoom, room_gi_room_index};

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

pub(super) const SHADOW_MAP_SIZE: f32 = 1024.0;

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

/// Loaded offline shadow field for the active room env, if any.
#[inline]
pub(super) fn room_baked_shadow_loaded(
    slots: &[Option<crate::render::wgpu_renderer::impl_room_shadow::RoomBakedShadowGpu>;
         crate::render::room_gi_bake::ROOM_GI_ROOM_COUNT],
    env: ActiveRoomEnv,
) -> Option<RoomGiRoom> {
    let room = env.to_room_gi()?;
    let idx = room_gi_room_index(room);
    slots[idx].as_ref()?;
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

    /// Write the shared room-GLB shadow caster uniform (`light_view_proj * model`).
    pub(super) fn write_room_env_shadow_caster(
        &self,
        gpu: &crate::render::wgpu_renderer::ShopEnvironmentGpu,
        light_view_proj: [f32; 16],
        model: glam::Mat4,
        changed: &mut bool,
    ) {
        self.queue.write_buffer(
            &gpu.shadow_uniform_buffer,
            0,
            bytemuck::bytes_of(&ShadowCasterUniform {
                light_view_proj,
                model: model.to_cols_array(),
            }),
        );
        *changed = true;
    }
}

/// Table-scale shadow depth (world units along the key light).
const TABLE_SHADOW_SCENE_DEPTH: f32 = 80.0;
const TABLE_SHADOW_DEPTH_BIAS: f32 = 0.005;

#[inline]
fn frame_draws_room_environment(frame: &crate::render::draw_cmd::UiFrame) -> bool {
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
        frame: &crate::render::draw_cmd::UiFrame,
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
                crate::render::main_menu_glb::main_menu_env_height_scale(
                    self.room_gltf_height_scale,
                )
            } else {
                self.room_gltf_height_scale
            };
            let extent = camera.h * env_height_scale;
            let half = extent * 0.11;
            let depth = extent * 0.42;
            (half, half, depth, TABLE_SHADOW_DEPTH_BIAS)
        } else if room_frustum {
            // Room GLBs are centered and scaled by `window_h` — extend the light
            // frustum in depth so shelf props cast contact shadows on the env.
            let env_height_scale = if self.active_scene_key == Some("main_menu_exterior") {
                crate::render::main_menu_glb::main_menu_env_height_scale(
                    self.room_gltf_height_scale,
                )
            } else {
                self.room_gltf_height_scale
            };
            let extent = camera.h * env_height_scale;
            let archive_offline_bake = self.room_shadow_capture_pending
                == Some(crate::render::room_gi_bake::RoomGiRoom::Archive);
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
        let pack_celeb_black_void = self.active_scene_key == Some("showcase")
            && frame.showcase_render_hints.tile_pack_celebration_tonemap
            && !frame_draws_room_environment(frame);
        let shadow_enabled_flag = if shadows_enabled && !pack_celeb_black_void {
            1.0_f32
        } else {
            0.0
        };
        self.queue.write_buffer(
            &self.shadow_globals_buffer,
            0,
            bytemuck::bytes_of(&ShadowGlobals {
                light_view_proj: light_view_proj_arr,
                params: [shadow_enabled_flag, depth_bias, 1.0 / SHADOW_MAP_SIZE, 0.0],
                room_baked_light_view_proj: glam::Mat4::IDENTITY.to_cols_array(),
            }),
        );
        ShadowFrame {
            light_view_proj,
            light_view_proj_arr,
        }
    }

    /// Per-instance shadow caster uniforms. Mirrors the model matrices
    /// written into the main lit-mesh + hand-tile uniforms so the shadow
    /// pre-pass can re-render the same geometry from the light's POV. Table
    /// is excluded — it's a flat receiver, not a caster. Returns nothing —
    /// writes to per-instance shadow uniform buffers via `self.queue`.
    pub(super) fn write_per_instance_shadow_casters(
        &mut self,
        _frame: &UiFrame,
        _camera: &CameraFrame,
        light_view_proj_arr: [f32; 16],
        tile_pick_models: &[(usize, Mat4)],
        shadow_uniforms_changed: &mut bool,
    ) {
        // Object3d / primitive shadow uniforms are written during
        // [`run_object3d_placement`] so model matrices match the lit pass.
        // Hand tile shadow uniforms — pull each tile's model matrix from
        // `tile_pick_models` (snapshot of the per-tile model written above).
        for (i, model) in tile_pick_models.iter() {
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
}
