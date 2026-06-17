use super::*;
use crate::draw_cmd::{DrawCmd, UiFrame};
use crate::lit_mesh::{
    LitMeshInstance, MaterialKind, ShadowCasterUniform, ShadowGlobals, material_casts_shadow,
};
use crate::projected_light_shadow::{
    ProjectedShadowLightSetup, PunctualShadowBuild, build_punctual_shadow_setups,
    punctual_shadow_setups_changed,
};
use crate::room_gi_bake::RoomGiRoom;
use crate::scene_keys;
use mahjuro_gfx_types::ShadowQuality;

/// Which imported room mesh is active this frame (at most one is drawn).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ActiveRoomEnv {
    Shop,
    Hallway,
    Stairway,
    Archive,
    MainMenu,
    Gameplay,
    ShadowTest,
}

#[inline]
pub fn active_room_env(frame: &UiFrame) -> Option<ActiveRoomEnv> {
    for cmd in &frame.cmds {
        match cmd {
            DrawCmd::ShopEnvironment => return Some(ActiveRoomEnv::Shop),
            DrawCmd::HallwayEnvironment => return Some(ActiveRoomEnv::Hallway),
            DrawCmd::StaircaseEnvironment => return Some(ActiveRoomEnv::Stairway),
            DrawCmd::ArchiveEnvironment => return Some(ActiveRoomEnv::Archive),
            DrawCmd::MainMenuEnvironment => return Some(ActiveRoomEnv::MainMenu),
            DrawCmd::GameplayEnvironment => return Some(ActiveRoomEnv::Gameplay),
            DrawCmd::ShadowTestEnvironment => return Some(ActiveRoomEnv::ShadowTest),
            _ => {}
        }
    }
    None
}

impl ActiveRoomEnv {
    /// Runtime `.msh` contact-AO bake for this room, if one should be sampled.
    pub fn baked_shadow_room(self) -> Option<RoomGiRoom> {
        match self {
            Self::Shop => Some(RoomGiRoom::Shop),
            Self::Hallway => Some(RoomGiRoom::Hallway),
            Self::Stairway => Some(RoomGiRoom::Stairway),
            Self::MainMenu => Some(RoomGiRoom::MainMenu),
            Self::Gameplay => Some(RoomGiRoom::Gameplay),
            Self::Archive => Some(RoomGiRoom::Archive),
            Self::ShadowTest => None,
        }
    }

    pub fn environment_bounds_doc(self) -> Option<crate::room_env_gltf::RoomEnvironmentBounds> {
        match self {
            Self::Gameplay => crate::gameplay_glb::with_gameplay_glb_cpu(|o| {
                o.and_then(|c| c.environment_bounds_doc)
            }),
            Self::Shop => {
                crate::room_glb::with_shop_glb_cpu(|o| o.and_then(|c| c.environment_bounds_doc))
            }
            Self::Hallway => crate::hallway_glb::with_hallway_glb_cpu(|o| {
                o.and_then(|c| c.environment_bounds_doc)
            }),
            Self::Stairway => crate::staircase_glb::with_staircase_glb_cpu(|o| {
                o.and_then(|c| c.environment_bounds_doc)
            }),
            Self::Archive => crate::archive_glb::with_archive_glb_cpu(|o| {
                o.and_then(|c| c.environment_bounds_doc)
            }),
            Self::MainMenu => crate::main_menu_glb::with_main_menu_glb_cpu(|o| {
                o.and_then(|c| c.environment_bounds_doc)
            }),
            Self::ShadowTest => crate::shadow_test_room_glb::with_shadow_test_room_glb_cpu(|o| {
                o.and_then(|c| c.environment_bounds_doc)
            }),
        }
    }

    /// Fallback when the frame has no room-environment draw cmd yet.
    pub fn from_scene_key(key: &str) -> Option<Self> {
        match key {
            scene_keys::SHOP | "animation_lab" => Some(Self::Shop),
            scene_keys::HALLWAY => Some(Self::Hallway),
            scene_keys::STAIRWAY => Some(Self::Stairway),
            scene_keys::ARCHIVE => Some(Self::Archive),
            scene_keys::MAIN_MENU => Some(Self::MainMenu),
            scene_keys::GAMEPLAY
            | scene_keys::VICTORY
            | scene_keys::DEFEAT
            | "tutorial"
            | "roller_lab"
            | "cascade_lab" => Some(Self::Gameplay),
            scene_keys::SHADOW_AO_LAB => None,
            // Legacy aliases (tuning overrides, screenshot CLI, old saves).
            "pick_chamber" | "pick_blind" => Some(Self::Hallway),
            "staircase" => Some(Self::Stairway),
            "collection" => Some(Self::Archive),
            "main_menu_exterior" => Some(Self::MainMenu),
            "game_over" => Some(Self::Gameplay),
            _ => None,
        }
    }
}

#[inline]
pub fn active_baked_shadow_room(frame: &UiFrame, scene_key: Option<&str>) -> Option<RoomGiRoom> {
    active_room_env(frame)
        .or_else(|| scene_key.and_then(ActiveRoomEnv::from_scene_key))
        .and_then(ActiveRoomEnv::baked_shadow_room)
}

#[inline]
pub fn skip_room_env_live_shadow_pass(
    active_room_env: ActiveRoomEnv,
    active_baked_room: Option<RoomGiRoom>,
) -> bool {
    active_room_env
        .baked_shadow_room()
        .is_some_and(|room| Some(room) == active_baked_room)
}

pub(super) struct ProjectedShadowFrame {
    pub build: PunctualShadowBuild,
    pub changed: bool,
    pub first_light_view_proj: [f32; 16],
}

impl ProjectedShadowFrame {
    #[inline]
    pub fn casters(&self) -> &[ProjectedShadowLightSetup] {
        &self.build.casters
    }
}

pub(crate) fn build_shadow_globals(
    shadow_quality: ShadowQuality,
    build: &PunctualShadowBuild,
    contact_ao_active: bool,
    contact_ao_view_proj: [f32; 16],
    contact_ao_world_scale: f32,
    dynamic_receiver_shadow_strength: f32,
) -> ShadowGlobals {
    let point_size = shadow_quality.point_map_size().max(1) as f32;
    let mut globals = ShadowGlobals::empty();
    let point_texel = 1.0 / point_size;
    let pcf_texel = if crate::lit_mesh_profile::pcf_single_tap() {
        -1.0
    } else {
        match shadow_quality {
            ShadowQuality::Off => -1.0,
            ShadowQuality::Low => -1.0,
            // Negative texel size selects 4-tap PCF in `projected_shadow.wgsl`.
            ShadowQuality::Medium => -point_texel,
            // Positive texel size selects 9-tap PCF (Visuals → High).
            ShadowQuality::High => point_texel,
        }
    };
    globals.params = [
        if shadow_quality.active() { 1.0 } else { 0.0 },
        0.005,
        pcf_texel,
        dynamic_receiver_shadow_strength.clamp(0.0, 1.0),
    ];
    for caster in &build.casters {
        let layer = caster.layer_index as usize;
        if layer < globals.point_view_proj.len() {
            globals.point_view_proj[layer] = caster.light_view_proj.to_cols_array();
        }
    }
    for (i, layer) in build.light_index_to_layer.iter().enumerate().take(16) {
        let block = i / 4;
        let slot = i % 4;
        globals.point_light_layer[block][slot] = *layer as f32;
    }
    globals.counts = [
        build.casters.len() as f32,
        if contact_ao_active {
            crate::room_shadow_bake::CONTACT_AO_DEPTH_COHERENCE_EPS
        } else {
            0.0
        },
        if contact_ao_active { 1.0 } else { 0.0 },
        if contact_ao_active {
            contact_ao_world_scale
        } else {
            0.0
        },
    ];
    globals.contact_ao_view_proj = contact_ao_view_proj;
    globals
}

impl WgpuRenderer {
    pub(super) fn prepare_projected_shadow_frame(
        &mut self,
        frame: &UiFrame,
        camera: &CameraFrame,
        shadow_quality: ShadowQuality,
    ) -> ProjectedShadowFrame {
        if !shadow_quality.active() {
            self.projected_shadow_lights.clear();
            return ProjectedShadowFrame {
                build: PunctualShadowBuild::empty(),
                changed: false,
                first_light_view_proj: glam::Mat4::IDENTITY.to_cols_array(),
            };
        }
        let active_room_env = ActiveRoomEnv::from_frame(frame).or_else(|| {
            self.active_scene_key
                .and_then(ActiveRoomEnv::from_scene_key)
        });
        let bounds_doc = active_room_env.and_then(|e| e.environment_bounds_doc());
        let focus_shop_dynamic_region = active_room_env.is_some_and(|env| {
            matches!(env, ActiveRoomEnv::Shop)
                && skip_room_env_live_shadow_pass(env, self.active_room_baked_shadow)
        });
        let env_height_scale = active_room_env
            .map(|e| self.room_env_shadow_height_scale(e))
            .unwrap_or_else(|| self.active_frame_env().height_scale);
        // Match [`PointLightsBuf::from_scene_punctual`] — same window dims as [`CameraFrame`].
        let screen_w = camera.w;
        let screen_h = camera.h;
        let use_ray_plane = frame
            .showcase_render_hints
            .layout_uses_ray_plane(self.active_scene_key);
        let build = build_punctual_shadow_setups(
            frame,
            active_room_env,
            screen_w,
            screen_h,
            screen_h,
            env_height_scale,
            bounds_doc,
            frame.camera_override.as_ref(),
            use_ray_plane,
            focus_shop_dynamic_region,
        );
        let (hash, changed) =
            punctual_shadow_setups_changed(&build, self.cached_projected_shadow_hash);
        self.cached_projected_shadow_hash = hash;
        self.projected_shadow_lights = build.casters.clone();
        if build.casters.is_empty() && !frame.scene_lighting.punctual.is_empty() {
            static WARNED: std::sync::atomic::AtomicBool =
                std::sync::atomic::AtomicBool::new(false);
            if !WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                log::warn!(
                    "punctual shadow setup produced 0 casters for {} punctual light(s) \
                     (active room env: {:?}, scene key: {:?}) — check scene policy / glTF node tags",
                    frame.scene_lighting.punctual.len(),
                    active_room_env,
                    self.active_scene_key,
                );
            }
        }
        let first_light_view_proj = build
            .casters
            .first()
            .map(|l| l.light_view_proj.to_cols_array())
            .unwrap_or(glam::Mat4::IDENTITY.to_cols_array());
        ProjectedShadowFrame {
            build,
            changed,
            first_light_view_proj,
        }
    }

    pub(super) fn upload_projected_shadow_globals(
        &self,
        shadow_quality: ShadowQuality,
        build: &PunctualShadowBuild,
        contact_ao_active: bool,
        contact_ao_view_proj: [f32; 16],
        window_h: f32,
        dynamic_receiver_shadow_strength: f32,
    ) {
        let ao_scale = if self.active_lab_baked_shadow {
            crate::shadow_ao_lab::CONTACT_AO_WORLD_SCALE
        } else if contact_ao_active {
            crate::room_shadow_bake::contact_ao_world_scale_ratio(window_h)
        } else {
            1.0
        };
        let globals = build_shadow_globals(
            shadow_quality,
            build,
            contact_ao_active,
            contact_ao_view_proj,
            ao_scale,
            dynamic_receiver_shadow_strength,
        );
        self.queue
            .write_buffer(&self.shadow_globals_buffer, 0, bytemuck::bytes_of(&globals));
    }
}

pub(super) struct Object3dShadowCtx<'a> {
    pub light_view_proj: [f32; 16],
    pub changed: &'a mut bool,
}

impl WgpuRenderer {
    pub(super) fn rewrite_shadow_casters_for_light(
        &self,
        light_view_proj_arr: [f32; 16],
        object3d_draw_list: &[(super::DrawKind, usize)],
        _tile_pick_models: &[(usize, glam::Mat4)],
        _showcase_tile_batches: &[&[super::ShowcaseTilePlacement]],
    ) {
        self.queue.write_buffer(
            &self.tile_shadow_frame_uniform_buffer,
            0,
            bytemuck::bytes_of(&ShadowCasterUniform {
                light_view_proj: light_view_proj_arr,
                model: glam::Mat4::IDENTITY.to_cols_array(),
                normal_model: glam::Mat4::IDENTITY.to_cols_array(),
            }),
        );
        for &(kind, slot_i) in object3d_draw_list {
            self.rewrite_object3d_shadow_light(kind, slot_i, light_view_proj_arr);
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
            super::DrawKind::TallyStickPlay | super::DrawKind::TallyStickDiscard => {
                rewrite!(self.tally_stick_instances)
            }
            super::DrawKind::ExtrudedGlyph => rewrite!(self.extruded_glyph_instances),
            super::DrawKind::GltfCoin => {}
            super::DrawKind::Primitive(shape) => {
                if let Some(pool) = self.primitive_instances.get(&shape) {
                    rewrite!(pool);
                }
            }
        }
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
            && material_casts_shadow(material)
        {
            *shadow.changed |=
                inst.write_shadow_uniform(&self.queue, shadow.light_view_proj, model);
        }
    }

    pub(crate) fn write_room_env_shadow_caster(
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
                    normal_model: crate::table_transform::normal_matrix_from_model(prim_model)
                        .to_cols_array(),
                }),
            );
        }
        *changed = true;
    }
}

/// Static room shells skip the live depth pre-pass when an offline `.msh` bake is active,
/// or for room-specific exceptions that should not cast live projected shadows.
#[inline]
pub fn room_env_shadow_upload_active(
    shadow_quality_active: bool,
    skip_live_room_env_shadow: bool,
) -> bool {
    shadow_quality_active && !skip_live_room_env_shadow
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_uses_baked_shadow_like_other_static_rooms() {
        assert_eq!(
            ActiveRoomEnv::Archive.baked_shadow_room(),
            Some(RoomGiRoom::Archive)
        );
        assert!(!skip_room_env_live_shadow_pass(
            ActiveRoomEnv::Archive,
            None
        ));
        assert!(skip_room_env_live_shadow_pass(
            ActiveRoomEnv::Archive,
            Some(RoomGiRoom::Archive)
        ));
    }

    #[test]
    fn baked_rooms_skip_static_live_shadow_only_when_their_bake_is_active() {
        assert!(!skip_room_env_live_shadow_pass(ActiveRoomEnv::Shop, None));
        assert!(skip_room_env_live_shadow_pass(
            ActiveRoomEnv::Shop,
            Some(RoomGiRoom::Shop)
        ));
        assert!(!skip_room_env_live_shadow_pass(
            ActiveRoomEnv::Shop,
            Some(RoomGiRoom::Hallway)
        ));
    }
}
