use super::*;
use crate::scene_keys;

impl WgpuRenderer {
    /// Emissive-red overlay for main-menu `rain_hit_*` shells (rain debug menu).
    pub(super) fn write_debug_rain_hit_uniforms(&self, frame: &UiFrame, camera: &CameraFrame) {
        if !frame.debug_rain_hit_colliders {
            return;
        }
        if self.main_menu_rain_hit_debug_mesh.is_none() {
            return;
        }
        let Some(ref inst) = self.main_menu_rain_hit_debug_instance else {
            return;
        };
        let height = self.env_tune_for(scene_keys::MAIN_MENU).height_scale;
        let env_h = crate::main_menu_glb::main_menu_env_height_scale(height);
        let model = crate::main_menu_glb::with_main_menu_glb_cpu(|opt| {
            opt.map(|cpu| {
                crate::room_glb::room_env_model_matrix_from_cpu(camera.h, env_h, cpu)
            })
        })
        .unwrap_or_else(|| {
            let s = crate::room_glb::room_env_world_scale(camera.h, env_h);
            glam::Mat4::from_scale(glam::Vec3::splat(s))
        });
        let material = MaterialParams {
            kind: MaterialKind::Emissive,
            base_color: [1.0, 0.06, 0.06, 0.95],
            specular_strength: 2.5,
            specular_power: 0.0,
        };
        inst.write_uniform(&self.queue, camera.view_proj_arr, model, material);
    }

    pub(super) fn draw_debug_rain_hit_colliders(&self, pass: &mut wgpu::RenderPass<'_>) {
        let Some(ref mesh) = self.main_menu_rain_hit_debug_mesh else {
            return;
        };
        let Some(ref inst) = self.main_menu_rain_hit_debug_instance else {
            return;
        };
        if mesh.index_count == 0 {
            return;
        }
        pass.set_pipeline(&self.lit_mesh_pipeline);
        pass.set_bind_group(3, &self.lit_mesh_spot_ssr_bind_group, &[]);
        pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
        pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
        pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
        pass.set_index_buffer(
            mesh.index_buffer.slice(..),
            wgpu::IndexFormat::Uint32,
        );
        pass.set_bind_group(0, &inst.bind_group, &[]);
        pass.draw_indexed(0..mesh.index_count, 0, 0..1);
    }
}
