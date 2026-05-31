use super::*;
use crate::draw_cmd::{DrawCmd, SHOP_INSPECT_SUBJECT_ANIM_ID, UiFrame};
use crate::lit_mesh::{LitMeshInstance, MaterialKind, ShadowCasterUniform, ShadowGlobals, material_casts_shadow};
use crate::projected_light_shadow::{
    ProjectedShadowLightSetup, PunctualShadowBuild, build_punctual_shadow_setups,
    punctual_shadow_setups_changed,
};
use crate::room_gi_bake::RoomGiRoom;
use mahjuro_gfx_types::ShadowQuality;

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

    pub fn environment_bounds_doc(self) -> Option<crate::room_env_gltf::RoomEnvironmentBounds> {
        match self {
            Self::Gameplay => {
                crate::gameplay_glb::with_gameplay_glb_cpu(|o| o.and_then(|c| c.environment_bounds_doc))
            }
            Self::Shop => {
                crate::room_glb::with_shop_glb_cpu(|o| o.and_then(|c| c.environment_bounds_doc))
            }
            Self::Hallway => {
                crate::hallway_glb::with_hallway_glb_cpu(|o| o.and_then(|c| c.environment_bounds_doc))
            }
            Self::Staircase => {
                crate::staircase_glb::with_staircase_glb_cpu(|o| {
                    o.and_then(|c| c.environment_bounds_doc)
                })
            }
            Self::Archive => {
                crate::archive_glb::with_archive_glb_cpu(|o| o.and_then(|c| c.environment_bounds_doc))
            }
            Self::MainMenu => {
                crate::main_menu_glb::with_main_menu_glb_cpu(|o| {
                    o.and_then(|c| c.environment_bounds_doc)
                })
            }
        }
    }

    /// Fallback when the frame has no room-environment draw cmd yet.
    pub fn from_scene_key(key: &str) -> Option<Self> {
        match key {
            "shop" | "animation_lab" => Some(Self::Shop),
            "hallway" | "pick_chamber" => Some(Self::Hallway),
            "staircase" => Some(Self::Staircase),
            "archive" | "collection" => Some(Self::Archive),
            "main_menu" | "main_menu_exterior" => Some(Self::MainMenu),
            "gameplay" | "tutorial" | "game_over" | "roller_lab" | "cascade_lab" => {
                Some(Self::Gameplay)
            }
            _ => None,
        }
    }

    /// glTF node name for embedded punctual `light_index` when scene tags are missing.
    pub fn embedded_point_light_node_name(self, light_index: usize) -> Option<String> {
        let node = |cpu: &crate::room_glb::RoomGlbCpu| {
            cpu.embedded_point_lights
                .get(light_index)
                .map(|l| l.node_name.clone())
        };
        match self {
            Self::Gameplay => crate::gameplay_glb::with_gameplay_glb_cpu(|o| o.map(node)).flatten(),
            Self::Shop => crate::room_glb::with_shop_glb_cpu(|o| o.map(node)).flatten(),
            Self::Hallway => crate::hallway_glb::with_hallway_glb_cpu(|o| o.map(node)).flatten(),
            Self::Staircase => {
                crate::staircase_glb::with_staircase_glb_cpu(|o| o.map(node)).flatten()
            }
            Self::Archive => crate::archive_glb::with_archive_glb_cpu(|o| o.map(node)).flatten(),
            Self::MainMenu => {
                crate::main_menu_glb::with_main_menu_glb_cpu(|o| o.map(node)).flatten()
            }
        }
    }

    #[inline]
    pub fn has_embedded_spot_lights(self) -> bool {
        let has_spots = |cpu: &crate::room_glb::RoomGlbCpu| !cpu.embedded_spot_lights.is_empty();
        match self {
            Self::Gameplay => crate::gameplay_glb::with_gameplay_glb_cpu(|o| o.is_some_and(has_spots)),
            Self::Shop => crate::room_glb::with_shop_glb_cpu(|o| o.is_some_and(has_spots)),
            Self::Hallway => crate::hallway_glb::with_hallway_glb_cpu(|o| o.is_some_and(has_spots)),
            Self::Staircase => {
                crate::staircase_glb::with_staircase_glb_cpu(|o| o.is_some_and(has_spots))
            }
            Self::Archive => crate::archive_glb::with_archive_glb_cpu(|o| o.is_some_and(has_spots)),
            Self::MainMenu => {
                crate::main_menu_glb::with_main_menu_glb_cpu(|o| o.is_some_and(has_spots))
            }
        }
    }
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
) -> ShadowGlobals {
    let point_size = shadow_quality.point_map_size().max(1) as f32;
    let mut globals = ShadowGlobals::empty();
    globals.params = [
        if shadow_quality.active() { 1.0 } else { 0.0 },
        0.005,
        1.0 / point_size,
        0.0,
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
        0.0,
        if contact_ao_active { 1.0 } else { 0.0 },
        0.0,
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
        let active_room_env = ActiveRoomEnv::from_frame(frame)
            .or_else(|| self.active_scene_key.and_then(ActiveRoomEnv::from_scene_key));
        let bounds_doc = active_room_env.and_then(|e| e.environment_bounds_doc());
        let env_height_scale = active_room_env
            .map(|e| self.room_env_shadow_height_scale(e))
            .unwrap_or_else(|| self.active_frame_env().height_scale);
        let screen_w = self.size.width.max(1) as f32;
        let screen_h = self.size.height.max(1) as f32;
        let use_ray_plane = frame
            .showcase_render_hints
            .layout_uses_ray_plane(self.active_scene_key);
        let build = build_punctual_shadow_setups(
            frame,
            active_room_env,
            screen_w,
            screen_h,
            camera.h,
            env_height_scale,
            bounds_doc,
            frame.camera_override.as_ref(),
            use_ray_plane,
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
    ) {
        let globals = build_shadow_globals(
            shadow_quality,
            build,
            contact_ao_active,
            contact_ao_view_proj,
        );
        self.queue.write_buffer(
            &self.shadow_globals_buffer,
            0,
            bytemuck::bytes_of(&globals),
        );
    }

    /// Realtime shadows are punctual-only; spot lights are a content error.
    pub(super) fn warn_if_spot_lights_present(&self, frame: &UiFrame) {
        if frame.scene_lighting.spot_lights.is_empty() {
            return;
        }
        let active_room_env = active_room_env(frame)
            .or_else(|| self.active_scene_key.and_then(ActiveRoomEnv::from_scene_key));
        if !active_room_env.is_some_and(ActiveRoomEnv::has_embedded_spot_lights) {
            // Programmatic/runtime spot lights are valid; only flag glTF-authored spot lights.
            return;
        }
        static WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            log::error!(
                "scene submitted {} spot light(s); realtime spot shadows are unsupported (punctual-only shadow path)",
                frame.scene_lighting.spot_lights.len(),
            );
        }
    }
}

pub(super) struct Object3dShadowCtx<'a> {
    pub light_view_proj: [f32; 16],
    pub changed: &'a mut bool,
}

impl WgpuRenderer {
    pub(super) fn write_per_instance_shadow_casters(
        &mut self,
        _frame: &UiFrame,
        _camera: &CameraFrame,
        light_view_proj_arr: [f32; 16],
        tile_pick_models: &[(usize, glam::Mat4)],
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
            if !stg.casts_shadow {
                continue;
            }
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
            super::DrawKind::GltfCoin => {}
            super::DrawKind::Primitive(shape) => {
                if let Some(pool) = self.primitive_instances.get(&shape) {
                    rewrite!(pool);
                }
            }
        }
    }

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

/// Whether this catalog prop should cast into the live shadow map.
#[inline]
pub(super) fn object3d_casts_dynamic_shadow(
    active_room: Option<ActiveRoomEnv>,
    anim_id: u64,
) -> bool {
    let _ = anim_id;
    match active_room {
        None
        | Some(ActiveRoomEnv::Shop)
        | Some(ActiveRoomEnv::Hallway)
        | Some(ActiveRoomEnv::MainMenu)
        | Some(ActiveRoomEnv::Gameplay)
        | Some(ActiveRoomEnv::Staircase)
        | Some(ActiveRoomEnv::Archive) => true,
    }
}
