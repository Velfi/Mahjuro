use super::*;

#[allow(dead_code)] // up_v/fov_y/aspect/view_mat/proj are exposed for downstream passes.
pub(super) struct CameraFrame {
    pub cam_pos: glam::Vec3,
    pub look_target: glam::Vec3,
    pub up_v: glam::Vec3,
    pub fov_y: f32,
    pub aspect: f32,
    pub view_mat: Mat4,
    pub proj: Mat4,
    pub view_proj: Mat4,
    pub view_proj_arr: [f32; 16],
    pub w: f32,
    pub h: f32,
}

impl CameraFrame {
    pub(super) fn build(frame: &UiFrame, size: crate::physical_size::PhysicalSize) -> Self {
        let w = size.width.max(1) as f32;
        let h = size.height.max(1) as f32;
        let aspect = w / h;
        let (cam_pos, look_target, fov_y) = if let Some(ref c) = frame.camera_override {
            (
                glam::Vec3::from_array(c.eye),
                glam::Vec3::from_array(c.target),
                c.fovy_deg.to_radians(),
            )
        } else {
            let c = crate::render::draw_cmd::CameraParams::default_table_camera(h);
            (
                glam::Vec3::from_array(c.eye),
                glam::Vec3::from_array(c.target),
                c.fovy_deg.to_radians(),
            )
        };
        let up_v = frame
            .camera_override
            .as_ref()
            .map(|c| glam::Vec3::from_array(c.up))
            .unwrap_or(glam::Vec3::Z);
        let view_mat = Mat4::look_at_rh(cam_pos, look_target, up_v);
        let proj = Mat4::perspective_rh(fov_y, aspect, 1.0, h * 12.0);
        let view_proj = proj * view_mat;
        let view_proj_arr = view_proj.to_cols_array();
        Self {
            cam_pos,
            look_target,
            up_v,
            fov_y,
            aspect,
            view_mat,
            proj,
            view_proj,
            view_proj_arr,
            w,
            h,
        }
    }

    /// Project a world position to integer-ish screen pixels for use in 2D
    /// overlay quads (selection halos, hint pulses, hover arrows).
    pub(super) fn project_to_screen(&self, world: glam::Vec3) -> (f32, f32) {
        let clip = self.view_proj * glam::Vec4::new(world.x, world.y, world.z, 1.0);
        let inv_w = 1.0 / clip.w.max(1e-6);
        let nx = clip.x * inv_w;
        let ny = clip.y * inv_w;
        let sx = (nx * 0.5 + 0.5) * self.w;
        let sy = (1.0 - (ny * 0.5 + 0.5)) * self.h;
        (sx, sy)
    }

    /// Project the unit cube `[-0.5, 0.5]³` under `model` to a screen-space
    /// rect. Used for primitives whose mesh AABB matches the unit cube.
    pub(super) fn project_unit_cube_rect(&self, model: Mat4) -> [f32; 4] {
        self.project_aabb_rect(model, [0.5, 0.5, 0.5], 0.0)
    }

    /// Project a world-space AABB to a screen-space rect `[x, y, w, h]`.
    /// `model` transforms local-space corners; `half` is the half-extents
    /// in local space; `center_y` shifts the local-space box along Y
    /// (some meshes are centered, others sit on their base).
    pub(super) fn project_aabb_rect(&self, model: Mat4, half: [f32; 3], center_y: f32) -> [f32; 4] {
        let corners = [
            glam::Vec3::new(-half[0], center_y - half[1], -half[2]),
            glam::Vec3::new(half[0], center_y - half[1], -half[2]),
            glam::Vec3::new(-half[0], center_y + half[1], -half[2]),
            glam::Vec3::new(half[0], center_y + half[1], -half[2]),
            glam::Vec3::new(-half[0], center_y - half[1], half[2]),
            glam::Vec3::new(half[0], center_y - half[1], half[2]),
            glam::Vec3::new(-half[0], center_y + half[1], half[2]),
            glam::Vec3::new(half[0], center_y + half[1], half[2]),
        ];
        let mut mn_x = f32::INFINITY;
        let mut mn_y = f32::INFINITY;
        let mut mx_x = f32::NEG_INFINITY;
        let mut mx_y = f32::NEG_INFINITY;
        for c in corners {
            let w = model.transform_point3(c);
            let (sx, sy) = self.project_to_screen(w);
            mn_x = mn_x.min(sx);
            mn_y = mn_y.min(sy);
            mx_x = mx_x.max(sx);
            mx_y = mx_y.max(sy);
        }
        [mn_x, mn_y, mx_x - mn_x, mx_y - mn_y]
    }
}

/// Which [`SsrGlobals.felt`] row to upload for shop item inspect (`lit_mesh` physical HDR).
#[derive(Clone, Copy)]
pub(super) enum ShopInspectLitMeshFelt {
    /// Matches storeroom proportion — shelf props + coins + journal before the hero mesh.
    Dim,
    /// Full inspect exposure for the focused prop.
    Subject,
}

impl WgpuRenderer {
    /// Shop-style ACES tonemap knobs for `tile_3d` / `tile_outline` (`CameraUniform.hdr_tonemap`)
    /// and `lit_mesh` (`SsrGlobals.felt`). Same `ShopEnvLightingTune` as the room.
    pub(super) fn tile_hdr_tonemap(&self, frame: &crate::render::draw_cmd::UiFrame) -> [f32; 4] {
        use crate::render::draw_cmd::DrawCmd;
        let k = self.active_scene_key;
        let h = frame.showcase_render_hints;
        let table_like = matches!(
            k,
            Some("gameplay") | Some("tutorial") | Some("pick_blind") | Some("collection")
        ) || (k == Some("showcase") && h.collection_tonemap_context);
        let shop_scene =
            k == Some("shop") || (k == Some("showcase") && h.shop_tonemap_and_lit_mesh_context);
        let tile_pack_celebration = k == Some("tile_pack_celebration")
            || (k == Some("showcase") && h.tile_pack_celebration_tonemap);
        if shop_scene && frame.shop_inspect_lit_mesh_hdr {
            let linear_hdr = self.shop_env_linear_exposure
                * crate::render::shop_glb::SHOP_INSPECT_LIT_MESH_HDR_LINEAR_MUL;
            let ambient = (self.shop_env_ambient_scale
                + crate::render::shop_glb::SHOP_INSPECT_LIT_MESH_AMBIENT)
                .min(0.30);
            return [1.0, linear_hdr, ambient, 0.0];
        }
        if tile_pack_celebration {
            let ambient = (self.shop_env_ambient_scale * 0.45)
                .max(crate::render::shop_glb::TILE_PACK_CELEBRATION_LIT_MESH_AMBIENT_MIN);
            return [
                1.0,
                self.shop_env_linear_exposure
                    * crate::render::shop_glb::TILE_PACK_CELEBRATION_HDR_LINEAR_EXPOSURE,
                ambient,
                0.0,
            ];
        }
        // Shop applies a heavy linear HDR divisor so bright `Shop.glb` fills land in
        // range. Showcase tiles alone (e.g. headless pack-celebration isolation)
        // use ordinary tile shading — same /512 crush makes them vanish.
        let shop_showcase_without_env = shop_scene
            && frame
                .cmds
                .iter()
                .any(|c| matches!(c, DrawCmd::ShowcaseTileBatch(_)))
            && !frame
                .cmds
                .iter()
                .any(|c| matches!(c, DrawCmd::ShopEnvironment | DrawCmd::HallwayEnvironment));
        if shop_showcase_without_env {
            let linear_hdr = self.shop_env_linear_exposure;
            return [1.0, linear_hdr, self.shop_env_ambient_scale, 0.0];
        }
        if !(shop_scene || table_like) {
            return [0.0; 4];
        }
        let linear_hdr = self.shop_env_linear_exposure
            * if shop_scene {
                crate::render::shop_glb::SHOP_ENV_LINEAR_EXPOSURE_BASE
            } else {
                crate::render::shop_glb::GAMEPLAY_TABLE_HDR_LINEAR_MUL
            };
        let ambient = if shop_scene {
            self.shop_env_ambient_scale
        } else {
            self.shop_env_ambient_scale
                .max(crate::render::shop_glb::GAMEPLAY_TABLE_AMBIENT_MIN)
        };
        [1.0, linear_hdr, ambient, 0.0]
    }

    fn lit_mesh_ssr_globals(
        &self,
        cam: &CameraFrame,
        ssr_enabled: bool,
        frame: &crate::render::draw_cmd::UiFrame,
        shop_inspect_felt: Option<ShopInspectLitMeshFelt>,
    ) -> SsrGlobals {
        let tm = self.tile_hdr_tonemap(frame);
        let felt_y = tm[0];
        let mut felt_z = if felt_y > 0.5 { tm[1] } else { 0.0 };
        let mut felt_w = if felt_y > 0.5 { tm[2] } else { 0.0 };
        let shop_like = self.active_scene_key == Some("shop")
            || (self.active_scene_key == Some("showcase")
                && frame
                    .showcase_render_hints
                    .shop_tonemap_and_lit_mesh_context);
        if shop_like && frame.shop_inspect_lit_mesh_hdr {
            if let Some(phase) = shop_inspect_felt {
                match phase {
                    ShopInspectLitMeshFelt::Dim => {
                        felt_z = tm[1] * crate::render::shop_glb::SHOP_INSPECT_ENV_VS_LIT_LINEAR;
                        felt_w = tm[2] * crate::render::shop_glb::SHOP_INSPECT_ENV_VS_LIT_AMBIENT;
                        if felt_y <= 0.5 {
                            felt_z = 0.0;
                            felt_w = 0.0;
                        }
                    }
                    ShopInspectLitMeshFelt::Subject => {
                        felt_z = if felt_y > 0.5 { tm[1] } else { 0.0 };
                        felt_w = if felt_y > 0.5 { tm[2] } else { 0.0 };
                    }
                }
            }
        }
        let shop_lit_hdr = shop_like && frame.scene_lighting.embedded_gltf_punctual;
        let shop_punctual_inv_doc = if shop_lit_hdr {
            let s =
                crate::render::shop_glb::shop_env_world_scale(cam.h, self.shop_env_height_scale);
            1.0 / s.max(1e-6)
        } else {
            0.0
        };
        let ssr_max_distance = cam.h * 2.0;
        let ssr_stride = cam.h * 0.04;
        let ssr_max_steps = 24.0;
        SsrGlobals {
            inv_view_proj: cam.view_proj.inverse().to_cols_array(),
            view_proj: cam.view_proj_arr,
            view_pos: [cam.cam_pos.x, cam.cam_pos.y, cam.cam_pos.z, 1.0],
            params: [
                if ssr_enabled { 1.0 } else { 0.0 },
                ssr_max_distance,
                ssr_stride,
                ssr_max_steps,
            ],
            felt: [self.felt_shader_lod, felt_y, felt_z, felt_w],
            shop_punctual: [shop_punctual_inv_doc, 0.0, 0.0, 0.0],
        }
    }

    pub(super) fn upload_camera_uniforms(
        &self,
        cam: &CameraFrame,
        ssr_enabled: bool,
        frame: &crate::render::draw_cmd::UiFrame,
    ) {
        let shop_like = self.active_scene_key == Some("shop")
            || (self.active_scene_key == Some("showcase")
                && frame
                    .showcase_render_hints
                    .shop_tonemap_and_lit_mesh_context);
        let shop_inspect_felt = if shop_like && frame.shop_inspect_lit_mesh_hdr {
            Some(ShopInspectLitMeshFelt::Dim)
        } else {
            None
        };
        let g = self.lit_mesh_ssr_globals(cam, ssr_enabled, frame, shop_inspect_felt);
        self.queue
            .write_buffer(&self.lit_mesh_ssr_buffer, 0, bytemuck::bytes_of(&g));

        self.queue.write_buffer(
            &self.flame_view_buffer,
            0,
            bytemuck::bytes_of(&FlameViewUniform {
                view_proj: cam.view_proj_arr,
                view_pos: [cam.cam_pos.x, cam.cam_pos.y, cam.cam_pos.z, 1.0],
            }),
        );
    }

    pub(super) fn upload_shop_inspect_lit_mesh_subject_ssr(
        &self,
        cam: &CameraFrame,
        ssr_enabled: bool,
        frame: &crate::render::draw_cmd::UiFrame,
    ) {
        let g = self.lit_mesh_ssr_globals(
            cam,
            ssr_enabled,
            frame,
            Some(ShopInspectLitMeshFelt::Subject),
        );
        self.queue
            .write_buffer(&self.lit_mesh_ssr_buffer, 0, bytemuck::bytes_of(&g));
    }
}
