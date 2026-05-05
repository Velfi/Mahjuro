use super::*;

impl WgpuRenderer {
    /// Write per-axis instance uniforms for the +X/+Y/+Z debug overlay.
    /// Three thin coloured boxes anchored at the camera look target so the
    /// user can read both axis and sign. No-op when `frame.debug_axes` is
    /// false. The companion text labels at each tip are still emitted from
    /// `render()` because they thread into the shared text/op vectors.
    pub(super) fn write_debug_axes_uniforms(&self, frame: &UiFrame, camera: &CameraFrame) {
        if !frame.debug_axes {
            return;
        }
        // Length: a chunky fraction of screen height so the bars are
        // visible against the table from the default camera.
        let length = camera.h * 0.35;
        let thickness = (camera.h * 0.012).max(4.0);
        let origin = camera.look_target;
        let axes: [(glam::Vec3, glam::Vec3, [f32; 4]); 3] = [
            // +X — red
            (
                glam::Vec3::X,
                glam::Vec3::new(length, thickness, thickness),
                [1.6, 0.10, 0.10, 1.0],
            ),
            // +Y — green
            (
                glam::Vec3::Y,
                glam::Vec3::new(thickness, length, thickness),
                [0.10, 1.6, 0.10, 1.0],
            ),
            // +Z — blue
            (
                glam::Vec3::Z,
                glam::Vec3::new(thickness, thickness, length),
                [0.20, 0.40, 1.8, 1.0],
            ),
        ];
        for (i, (axis_dir, scale, color)) in axes.iter().enumerate() {
            // Center the box halfway down the positive axis so its -end
            // sits at `origin` and its +end sticks out by `length`.
            let center = origin + *axis_dir * (length * 0.5);
            let model = translate_rot_scale(center, Mat4::IDENTITY, *scale);
            let material = MaterialParams {
                kind: MaterialKind::Plain,
                base_color: *color,
                specular_strength: 0.0,
                specular_power: 8.0,
            };
            if let Some(inst) = self.debug_axes_instances.get(i) {
                inst.write_uniform(&self.queue, camera.view_proj_arr, model, material);
            }
        }
    }
}
