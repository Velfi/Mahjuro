use super::*;

impl WgpuRenderer {
    /// Draw the 2D screen-space rectangle outline around the projected
    /// AABB of the currently selected debug-arrange object, plus the two
    /// clamp-band hint lines that tell the user why a nudge can't move
    /// the object any further.
    ///
    /// No-op when nothing is selected. Pushes one or two `RenderOp::QuadBatch`
    /// entries onto `ops` and the matching vertex buffers onto
    /// `quad_buffers`.
    pub(super) fn push_arrange_bbox_overlay(
        &self,
        frame: &UiFrame,
        camera: &CameraFrame,
        quad_buffers: &mut Vec<wgpu::Buffer>,
        ops: &mut Vec<RenderOp>,
    ) {
        let Some(ref ov) = self.debug_arrange_override else {
            return;
        };
        let w = camera.w;
        let h = camera.h;
        let aabb = self
            .last_debug_pickables
            .iter()
            .find(|(n, _, _, _)| n == &ov.name)
            .map(|(_, m, h, o)| (*m, *h, *o))
            .or_else(|| {
                self.last_debug_trimesh_pickables
                    .iter()
                    .find(|(n, _, _)| n == &ov.name)
                    .map(|(_, m, mesh)| match mesh {
                        TrimeshRef::LampBody => {
                            (*m, self.lamp_body_local_half, self.lamp_body_local_center_y)
                        }
                    })
            });
        if let Some((model, half, center_y)) = aabb {
            let [rx, ry, rw, rh] =
                camera.project_aabb_rect(model, [half.x, half.y, half.z], center_y);
            let t = (h * 0.003).max(2.0); // border thickness in pixels
            let color = [1.0_f32, 0.85, 0.25, 0.9]; // gold
            let border_quads: [GpuInstance; 4] = [
                // top
                GpuInstance {
                    rect: [rx, ry, rw, t],
                    color,
                },
                // bottom
                GpuInstance {
                    rect: [rx, ry + rh - t, rw, t],
                    color,
                },
                // left
                GpuInstance {
                    rect: [rx, ry, t, rh],
                    color,
                },
                // right
                GpuInstance {
                    rect: [rx + rw - t, ry, t, rh],
                    color,
                },
            ];
            let buf = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("arrange-bbox"),
                    contents: bytemuck::cast_slice(&border_quads),
                    usage: wgpu::BufferUsages::VERTEX,
                });
            let buf_idx = quad_buffers.len();
            quad_buffers.push(buf);
            ops.push(RenderOp::QuadBatch { buf_idx, count: 4 });
        }

        // Clamp-band hint for the selected pickable. Two thin lines at
        // the clamp walls — dim gold when the current `center_frac` is
        // inside the band, red-thick on whichever wall is currently
        // pinning it. Tells the user at a glance why a nudge isn't
        // moving the object any further.
        if let Some(clamp) = frame.arrange_clamps.iter().find(|c| c.name == ov.name) {
            use crate::render::draw_cmd::ClampAxis;
            let dim = [1.0_f32, 0.85, 0.25, 0.35];
            let pin = [1.0_f32, 0.30, 0.25, 0.95];
            let line_t = (h * 0.0018).max(1.5);
            let pin_t = line_t * 3.0;
            let below = clamp.center_frac < clamp.lo_frac;
            let above = clamp.center_frac > clamp.hi_frac;
            let (lo_color, lo_thick) = if below { (pin, pin_t) } else { (dim, line_t) };
            let (hi_color, hi_thick) = if above { (pin, pin_t) } else { (dim, line_t) };
            let clamp_quads: [GpuInstance; 2] = match clamp.axis {
                ClampAxis::Horizontal => {
                    let lo_px = clamp.lo_frac * w;
                    let hi_px = clamp.hi_frac * w;
                    [
                        GpuInstance {
                            rect: [lo_px - lo_thick * 0.5, 0.0, lo_thick, h],
                            color: lo_color,
                        },
                        GpuInstance {
                            rect: [hi_px - hi_thick * 0.5, 0.0, hi_thick, h],
                            color: hi_color,
                        },
                    ]
                }
            };
            let buf = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("arrange-clamp"),
                    contents: bytemuck::cast_slice(&clamp_quads),
                    usage: wgpu::BufferUsages::VERTEX,
                });
            let buf_idx = quad_buffers.len();
            quad_buffers.push(buf);
            ops.push(RenderOp::QuadBatch { buf_idx, count: 2 });
        }
    }
}
