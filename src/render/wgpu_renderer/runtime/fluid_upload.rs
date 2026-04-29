use super::*;

impl WgpuRenderer {
    /// Configure the volumetric smoke grid bounds, inject scene-driven
    /// impulses (wind gusts, occluders, cursor trail), reallocate the
    /// offscreen target if the smoke detail or window size changed, and
    /// upload the per-frame camera uniform.
    ///
    /// Per-tile motion impulses are injected earlier in the tile loop —
    /// this method only handles the scene-level signals. Done before the
    /// encoder is created so all queued impulses are still pending when
    /// the smoke pass runs.
    pub(super) fn upload_fluid_frame(
        &mut self,
        frame: &UiFrame,
        camera: &CameraFrame,
        dt: f32,
        smoke_quality: crate::persistence::SmokeQuality,
        smoke_amount: crate::persistence::SmokeAmount,
    ) {
        let Some(ref mut fluid) = self.fluid else {
            return;
        };
        let w = camera.w;
        let h = camera.h;
        let view_proj = camera.view_proj;
        let cam_pos = camera.cam_pos;

        // Grid bounds: a box roughly enclosing the table with vertical
        // headroom for smoke to rise. World space is Z-up, so the grid
        // must be *tall in Z* — buoyancy and floor passes use world Z
        // for height_frac.
        let half_w = w * 0.75;
        let half_y = h * 0.75;
        let smoke_box_h = h * 0.75 + 12.0;
        let grid_min = glam::Vec3::new(-half_w, -half_y, -12.0);
        let grid_max = glam::Vec3::new(half_w, half_y, grid_min.z + 2.0 * smoke_box_h);
        fluid.set_grid_bounds(grid_min, grid_max);

        // Scene-driven wind gusts. Coordinates are layout pixels; route
        // them through `pixel_to_world` so the gust lands on the table.
        for g in frame.wind_gusts.iter() {
            let pos = pixel_to_world(w, h, g.center_px.0, g.center_px.1, g.lift);
            fluid.inject_impulse(
                pos,
                glam::Vec3::new(g.velocity[0], g.velocity[1], g.velocity[2]),
                g.radius,
                g.density * 0.35,
                0.0,
                0.0,
            );
        }

        // Opaque shadow casters (shop bugs, etc.). Same pixel→table
        // mapping as wind gusts so shadows land where the meshes visibly
        // are.
        let occluders: Vec<crate::render::fluid::BugOccluder> = frame
            .bug_occluders
            .iter()
            .map(|b| crate::render::fluid::BugOccluder {
                world_pos: pixel_to_world(w, h, b.center_px.0, b.center_px.1, b.lift),
                radius: b.radius,
                strength: b.strength,
            })
            .collect();
        fluid.set_occluders(&occluders);

        // Cursor → table-plane impulse trail. Unproject the screen
        // cursor, intersect z=5, then interpolate between the previous
        // and current world positions to inject a chain of small puffs
        // so the trail has no gaps at low frame rates or fast flicks.
        if let Some((cx, cy)) = frame.cursor_pos {
            // Gate on actual screen-space pointer motion. Without this,
            // a stationary cursor over an orbiting/swaying camera would
            // emit continuous puffs as the unprojected table-plane hit
            // drifts with the camera.
            let screen_moved = match self.prev_cursor_screen {
                Some((pcx, pcy)) => (cx - pcx).abs() > 0.01 || (cy - pcy).abs() > 0.01,
                None => false,
            };
            self.prev_cursor_screen = Some((cx, cy));
            let inv_vp = view_proj.inverse();
            let nx = (cx / w) * 2.0 - 1.0;
            let ny = 1.0 - (cy / h) * 2.0;
            let near = inv_vp * glam::Vec4::new(nx, ny, 0.0, 1.0);
            let far = inv_vp * glam::Vec4::new(nx, ny, 1.0, 1.0);
            let near3 = glam::Vec3::new(near.x / near.w, near.y / near.w, near.z / near.w);
            let far3 = glam::Vec3::new(far.x / far.w, far.y / far.w, far.z / far.w);
            let dir = (far3 - near3).normalize_or_zero();
            if dir.z.abs() > 1e-4 {
                let plane_z = 5.0;
                let t = (plane_z - near3.z) / dir.z;
                if t > 0.0 {
                    let hit = near3 + dir * t;
                    if let Some(prev) = self.prev_cursor_world {
                        let raw_delta = hit - prev;
                        let jump = raw_delta.length();
                        let win_scale = (h / 1080.0).max(0.5);
                        let max_jump = 42.0 * win_scale;
                        if screen_moved && jump.is_finite() && jump <= max_jump {
                            let speed_threshold = 0.4 * win_scale;
                            if jump > speed_threshold {
                                // Drop a line of overlapping gaussian puffs
                                // between the previous and current cursor
                                // world positions. The density-only sim
                                // transports them upward via its drift +
                                // curl field, so we just need to seed
                                // enough mass for a solid plume read.
                                let puff_radius = 18.0 * win_scale;
                                // Spacing below the radius so adjacent
                                // Gaussians overlap heavily (~e^-0.5 ≈ 60%
                                // at the midpoint), leaving no visible
                                // gaps along a fast flick. Cap raised
                                // from 8 so long drags still fill.
                                let step_size = puff_radius * 0.8;
                                let n_puffs = ((jump / step_size).ceil() as u32).clamp(1, 24);

                                // Perpendicular basis for in-plane jitter:
                                // table-plane is z=5, so XY are free axes.
                                let tangent = raw_delta.normalize_or_zero();
                                let perp = glam::Vec3::new(-tangent.y, tangent.x, 0.0);

                                // Wake-vortex strength scales with cursor
                                // speed — stronger flicks shed stronger
                                // eddies. Divide by dt so `speed` is in
                                // world units per second.
                                let speed = jump / dt.max(1.0 / 120.0);
                                let swirl_vel = (speed * 1.1).min(640.0 * win_scale);
                                // Small retrograde push so vortices sit
                                // behind the leading edge rather than
                                // racing ahead with the trail.
                                let retrograde = speed * 0.12;

                                use rand::RngExt;
                                let mut rng = rand::rng();
                                for i in 0..n_puffs {
                                    let frac = if n_puffs == 1 {
                                        1.0
                                    } else {
                                        (i as f32 + 1.0) / n_puffs as f32
                                    };
                                    let jitter_perp: f32 = rng.random_range(-1.0..1.0);
                                    let jitter_along: f32 = rng.random_range(-0.35..0.35);
                                    let jitter_z: f32 = rng.random_range(-0.4..0.4);
                                    let radius_mul: f32 = rng.random_range(0.75..1.25);
                                    let density_mul: f32 = rng.random_range(0.7..1.15);

                                    let center = prev
                                        + raw_delta * frac
                                        + perp * (jitter_perp * puff_radius * 0.35)
                                        + tangent * (jitter_along * step_size);
                                    let z_lift = glam::Vec3::new(
                                        0.0,
                                        0.0,
                                        (4.0 + jitter_z * 3.0) * win_scale,
                                    );
                                    fluid.inject_impulse(
                                        center + z_lift,
                                        glam::Vec3::ZERO,
                                        puff_radius * radius_mul,
                                        0.13 * density_mul * smoke_amount.density_mul(),
                                        0.0,
                                        0.0,
                                    );

                                    // Shed a counter-rotating vortex pair
                                    // per step, offset ±perp from the
                                    // trail. Alternate the leading side
                                    // per step so the wake staggers like
                                    // a Kármán vortex street instead of
                                    // reading as two parallel rails.
                                    let lead_sign = if i % 2 == 0 { 1.0 } else { -1.0 };
                                    let offset = puff_radius * 0.9;
                                    for side in [-1.0_f32, 1.0_f32] {
                                        let s = side * lead_sign;
                                        let pos = center + perp * (s * offset)
                                            - tangent * (offset * 0.4)
                                            + z_lift;
                                        let vel = perp * (s * swirl_vel) - tangent * retrograde;
                                        fluid.inject_impulse(
                                            pos,
                                            vel,
                                            puff_radius * 0.75 * radius_mul,
                                            0.07 * density_mul * smoke_amount.density_mul(),
                                            0.0,
                                            0.0,
                                        );
                                    }
                                }
                            }
                        }
                    }
                    self.prev_cursor_world = Some(hit);
                }
            }
        }

        // (Re)allocate the offscreen smoke target whenever the user
        // changes the detail dropdown OR the window resizes. Cheap
        // no-op when nothing changed. Reallocating invalidates the
        // render bgs (they bind offscreen views as TAA history inputs),
        // so `render_bgs_need_rebuild()` picks that up below.
        fluid.set_detail(&self.device, smoke_quality, &self.depth_copy_view);

        // Build/rebuild the volume render bind groups on first use,
        // after every depth-texture recreation (resize), and after any
        // offscreen reallocation. The smoke pass samples a SNAPSHOT of
        // the depth (`depth_copy_view`) copied between the pre-smoke
        // and post-smoke passes — the live `depth_view` would alias
        // the active depth attachment.
        if self.fluid_render_bg_dirty || fluid.render_bgs_need_rebuild() {
            fluid.rebuild_render_bind_group(
                &self.device,
                &self.depth_copy_view,
                &self.point_lights_buffer,
            );
            self.fluid_render_bg_dirty = false;
        }

        // Per-frame camera uniform consumed by the volume raymarch shader.
        fluid.upload_camera_uniform(&self.queue, view_proj, cam_pos, smoke_quality, smoke_amount);
    }
}
