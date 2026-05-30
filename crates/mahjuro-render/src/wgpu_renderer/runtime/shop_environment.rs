use super::camera::CameraFrame;
use super::*;

struct GameplayScoreRollerDrive {
    drives: [f64; 2],
}

/// Seconds between each +1× speed step while rollers spin.
const GAMEPLAY_SCORE_ROLLER_RAMP_INTERVAL_SECS: f64 = 2.0;
const GAMEPLAY_SCORE_ROLLER_MIN_SPEED: f64 = 35.0;
const GAMEPLAY_SCORE_ROLLER_MAX_SPEED: f64 = 25000.0;

pub(crate) fn gameplay_score_roller_speed_multiplier(elapsed_secs: f64) -> f64 {
    1.0 + (elapsed_secs / GAMEPLAY_SCORE_ROLLER_RAMP_INTERVAL_SECS).floor()
}

/// Loop SFX playback speed — same 2s tiers as the visual drive, but only +2 semitones
/// per tier instead of matching the full mechanical multiplier.
const GAMEPLAY_SCORE_ROLLER_LOOP_SPIN_IN_SECS: f64 = 0.5;
const GAMEPLAY_SCORE_ROLLER_LOOP_SPIN_IN_SEMITONES: f64 = -3.0;

pub(crate) fn gameplay_score_roller_loop_speed_multiplier(elapsed_secs: f64) -> f64 {
    if elapsed_secs < GAMEPLAY_SCORE_ROLLER_LOOP_SPIN_IN_SECS {
        return 2.0_f64.powf(GAMEPLAY_SCORE_ROLLER_LOOP_SPIN_IN_SEMITONES / 12.0);
    }
    let tier = (elapsed_secs / GAMEPLAY_SCORE_ROLLER_RAMP_INTERVAL_SECS).floor();
    2.0_f64.powf(tier * 2.0 / 12.0)
}

pub(crate) fn gameplay_score_roller_bank_moving(
    initialized: &[bool; 2],
    drive_values: &[f64; 2],
    goal: &[f64; 2],
) -> bool {
    (0..2).any(|bank| {
        initialized[bank] && (goal[bank] - drive_values[bank]).abs() > 1e-5
    })
}

/// Remap a roller wheel's continuous phase so digits stay visually locked near
/// integer steps. Lower-order digits leave fractional residue on higher wheels
/// (score 42 → tens wheel at 4.2); cubically scaling the fractional rotation by
/// distance from the nearest snap point keeps those reads unambiguous while
/// still allowing a quick whip through the transition band during spins.
fn gameplay_score_roller_slot_wheel_phase(slot: usize, drives: &[f64; 2]) -> f64 {
    let bank = if slot < 10 { 0 } else { 1 };
    let local_slot = slot % 10;
    let significance = 9_i32 - local_slot as i32;
    let turns = drives[bank] / 10_f64.powi(significance);
    turns.rem_euclid(10.0)
}

fn gameplay_score_roller_visual_phase(phase: f64) -> f64 {
    let base = phase.floor();
    let frac = phase - base;
    if frac < 1e-9 {
        return base;
    }
    // 1.0 on a digit snap point, 0.0 halfway between digits.
    let dist_to_nearest = frac.min(1.0 - frac);
    let closeness = 1.0 - dist_to_nearest * 2.0;
    let visual_frac = frac * (1.0 - closeness).powi(3);
    base + visual_frac
}

struct GltfRoomEnvUniformParams<'a> {
    frame: &'a crate::draw_cmd::UiFrame,
    camera: &'a CameraFrame,
    env_scene_key: &'static str,
    embedded_gltf_punctual: bool,
    hallway_env: bool,
    staircase_env: bool,
    archive_env: bool,
    main_menu_env: bool,
    bloom_linear_hdr_output: bool,
    model: Mat4,
    gpu: &'a ShopEnvironmentGpu,
    shadow_upload: Option<([f32; 16], &'a mut bool)>,
    prim_deltas: &'a rustc_hash::FxHashMap<usize, glam::Mat4>,
}

impl WgpuRenderer {
    fn shop_eyeball_prim_indices_for_draw(&self) -> Vec<usize> {
        if !self.shop_eyeball_prim_indices.is_empty() {
            return self.shop_eyeball_prim_indices.clone();
        }
        self.shop_gltf_anim
            .clip_prim_bindings
            .get("eyeball_travel")
            .map(|b| b.iter().map(|(pi, _)| *pi).collect())
            .unwrap_or_default()
    }

    fn shop_gltf_anim_prim_deltas(
        &self,
        frame: &crate::draw_cmd::UiFrame,
    ) -> rustc_hash::FxHashMap<usize, glam::Mat4> {
        if frame.shop_gltf_anim_samples.is_empty() {
            return rustc_hash::FxHashMap::default();
        }
        let deltas = self
            .shop_gltf_anim
            .resolve_prim_deltas(&frame.shop_gltf_anim_samples);
        if deltas.is_empty() && !self.shop_gltf_anim_missing_clip_warned.replace(true) {
            log::warn!(
                "shop glTF anim: playback requested but no clip/primitive bindings matched"
            );
        } else if !deltas.is_empty() {
            self.shop_gltf_anim_missing_clip_warned.set(false);
        }
        deltas
    }

    fn gameplay_score_roller_drive(
        &self,
        frame: &crate::draw_cmd::UiFrame,
    ) -> Option<GameplayScoreRollerDrive> {
        let (score, target) = frame.gameplay_score_roller_values?;
        let dt = self.frame_dt.clamp(1.0 / 480.0, 0.100) as f64;
        let goal = [score as f64, target as f64];
        let mut drive_values = self.gameplay_score_roller_drive_values.borrow_mut();
        let mut initialized = self.gameplay_score_roller_drive_initialized.borrow_mut();
        let mut roll_elapsed = self.gameplay_score_roller_roll_elapsed.borrow_mut();
        let was_rolling =
            gameplay_score_roller_bank_moving(&initialized, &drive_values, &goal);
        if was_rolling {
            *roll_elapsed += dt;
        }
        let speed_multiplier = gameplay_score_roller_speed_multiplier(*roll_elapsed);
        for bank in 0..2 {
            let target_value = goal[bank];
            if !initialized[bank] {
                drive_values[bank] = target_value;
                initialized[bank] = true;
                continue;
            }
            let diff = target_value - drive_values[bank];
            if diff.abs() <= 1e-5 {
                drive_values[bank] = target_value;
                continue;
            }
            let speed_units_per_sec = (diff
                .abs()
                .clamp(GAMEPLAY_SCORE_ROLLER_MIN_SPEED, GAMEPLAY_SCORE_ROLLER_MAX_SPEED)
                * speed_multiplier)
                .min(GAMEPLAY_SCORE_ROLLER_MAX_SPEED);
            let step = speed_units_per_sec * dt;
            if diff > 0.0 {
                drive_values[bank] = (drive_values[bank] + step).min(target_value);
            } else {
                drive_values[bank] = (drive_values[bank] - step).max(target_value);
            }
        }
        if !gameplay_score_roller_bank_moving(&initialized, &drive_values, &goal) {
            *roll_elapsed = 0.0;
        }
        let drives = *drive_values;
        let still_rolling =
            gameplay_score_roller_bank_moving(&initialized, &drive_values, &goal);
        drop(roll_elapsed);
        drop(initialized);
        drop(drive_values);
        if was_rolling && !still_rolling {
            *self.gameplay_score_roller_stopped.borrow_mut() = true;
        }
        Some(GameplayScoreRollerDrive { drives })
    }

    fn gameplay_score_roller_slot_angle(slot: usize, drive: &GameplayScoreRollerDrive) -> f32 {
        let wheel_phase = gameplay_score_roller_slot_wheel_phase(slot, &drive.drives);
        let visual_phase = gameplay_score_roller_visual_phase(wheel_phase);
        (visual_phase as f32) * (-std::f32::consts::TAU / 10.0)
    }

    fn gameplay_score_roller_frame_state(
        &self,
        frame: &crate::draw_cmd::UiFrame,
    ) -> rustc_hash::FxHashMap<usize, glam::Mat4> {
        let Some(drive) = self.gameplay_score_roller_drive(frame) else {
            return rustc_hash::FxHashMap::default();
        };
        let count = self
            .gameplay_score_roller_prim_groups
            .len()
            .min(self.gameplay_score_roller_pivots_doc.len())
            .min(self.gameplay_score_roller_axes_doc.len())
            .min(20);
        let mut deltas = rustc_hash::FxHashMap::default();
        if count == 0 {
            return deltas;
        }

        for slot in 0..count {
            let Some(prim_group) = self.gameplay_score_roller_prim_groups.get(slot) else {
                continue;
            };
            if prim_group.is_empty() {
                continue;
            }
            let angle = Self::gameplay_score_roller_slot_angle(slot, &drive);
            let pivot = glam::Vec3::from_array(self.gameplay_score_roller_pivots_doc[slot]);
            let axis_raw = glam::Vec3::from_array(self.gameplay_score_roller_axes_doc[slot]);
            let axis = if axis_raw.length_squared() > 1e-6 {
                axis_raw.normalize()
            } else {
                glam::Vec3::X
            };
            let delta = if angle.abs() >= 1e-6 {
                Mat4::from_translation(pivot)
                    * Mat4::from_axis_angle(axis, angle)
                    * Mat4::from_translation(-pivot)
            } else {
                Mat4::IDENTITY
            };
            for &pi in prim_group {
                if angle.abs() >= 1e-6 {
                    deltas.insert(pi, delta);
                }
            }
        }
        deltas
    }

    fn draw_gltf_room_env_meshes(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        frame: &crate::draw_cmd::UiFrame,
        prims: &[TilePrimitiveGpu],
        gpu: &ShopEnvironmentGpu,
        room_hdr_mrt_emissive: bool,
        skip_prim: impl Fn(usize) -> bool,
    ) {
        if prims.is_empty() {
            return;
        }
        pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
        pass.set_bind_group(2, self.room_shadow_sample_bind_group(), &[]);
        pass.set_bind_group(3, &self.spot_lights_bind_group, &[]);
        for blend_phase in [false, true] {
            let mut last_pi: Option<usize> = None;
            let mut last_key = None;
            for (pi, prim) in prims.iter().enumerate() {
                if skip_prim(pi) {
                    continue;
                }
                let use_blend = prim.pipeline_key.is_blend();
                if use_blend != blend_phase {
                    continue;
                }
                let draw_key = prim.pipeline_key;
                if last_key != Some(draw_key) {
                    let pipe = if frame.uses_room_glb_shader() {
                        if room_hdr_mrt_emissive {
                            self.shop_env_pipeline_mrt(draw_key)
                        } else {
                            self.shop_env_pipeline(draw_key)
                        }
                    } else {
                        self.tile_glb_pipeline(draw_key)
                    };
                    pass.set_pipeline(pipe);
                    last_key = Some(draw_key);
                }
                if last_pi != Some(pi) {
                    pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
                    pass.set_index_buffer(prim.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                    last_pi = Some(pi);
                }
                let Some(bg) = gpu.bind_groups.get(pi) else {
                    continue;
                };
                pass.set_bind_group(0, bg, &[]);
                pass.draw_indexed(0..prim.index_count, 0, 0..1);
            }
        }
    }

    /// Draw [`shop.glb`] environment primitives through `room_glb.wgsl` / `tile_3d.wgsl`
    /// (same routing as [`RenderOp::ShopEnvironment`]).
    pub(super) fn draw_shop_environment_meshes(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        frame: &crate::draw_cmd::UiFrame,
        room_hdr_mrt_emissive: bool,
    ) {
        let Some(ref gpu) = self.shop_environment else {
            return;
        };
        let eyeball_only = frame.shop_env_eyeball_only;
        let eyeball_indices = self.shop_eyeball_prim_indices_for_draw();
        self.draw_gltf_room_env_meshes(
            pass,
            frame,
            &self.shop_env_primitives,
            gpu,
            room_hdr_mrt_emissive,
            |pi| eyeball_only && !eyeball_indices.is_empty() && !eyeball_indices.contains(&pi),
        );
    }

    /// Draw [`gameplay.glb`] table room.
    pub(super) fn draw_gameplay_environment_meshes(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        frame: &crate::draw_cmd::UiFrame,
        room_hdr_mrt_emissive: bool,
    ) {
        let Some(ref gpu) = self.gameplay_environment else {
            return;
        };
        let skip = |pi: usize| self.gameplay_env_skip_cash_in_prim(pi, frame);
        self.draw_gltf_room_env_meshes(
            pass,
            frame,
            &self.gameplay_env_primitives,
            gpu,
            room_hdr_mrt_emissive,
            skip,
        );
    }

    /// Draw [`hallway.glb`] (pick-blind room).
    pub(super) fn draw_hallway_environment_meshes(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        frame: &crate::draw_cmd::UiFrame,
        room_hdr_mrt_emissive: bool,
    ) {
        let Some(ref gpu) = self.hallway_environment else {
            return;
        };
        self.draw_gltf_room_env_meshes(
            pass,
            frame,
            &self.hallway_env_primitives,
            gpu,
            room_hdr_mrt_emissive,
            |_| false,
        );
    }

    /// Draw [`staircase.glb`] (post-ordeal interstitial).
    pub(super) fn draw_staircase_environment_meshes(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        frame: &crate::draw_cmd::UiFrame,
        room_hdr_mrt_emissive: bool,
    ) {
        let Some(ref gpu) = self.staircase_environment else {
            return;
        };
        self.draw_gltf_room_env_meshes(
            pass,
            frame,
            &self.staircase_env_primitives,
            gpu,
            room_hdr_mrt_emissive,
            |_| false,
        );
    }

    /// Draw [`archive.glb`] Archive room.
    pub(super) fn draw_archive_environment_meshes(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        frame: &crate::draw_cmd::UiFrame,
        room_hdr_mrt_emissive: bool,
    ) {
        let Some(ref gpu) = self.archive_environment else {
            return;
        };
        self.draw_gltf_room_env_meshes(
            pass,
            frame,
            &self.archive_env_primitives,
            gpu,
            room_hdr_mrt_emissive,
            |pi| self.archive_env_skip_archive_prim(pi, frame),
        );
    }

    fn write_gltf_room_env_uniforms<'a>(&self, p: GltfRoomEnvUniformParams<'a>) {
        let GltfRoomEnvUniformParams {
            frame,
            camera,
            env_scene_key,
            embedded_gltf_punctual,
            hallway_env,
            staircase_env,
            archive_env,
            main_menu_env,
            bloom_linear_hdr_output,
            model,
            gpu,
            shadow_upload,
            prim_deltas,
        } = p;
        let env_tune = self.env_tune_for(env_scene_key);
        let height_scale = if main_menu_env {
            crate::main_menu_glb::main_menu_env_height_scale(env_tune.height_scale)
        } else {
            env_tune.height_scale
        };
        let s = crate::room_glb::room_env_world_scale(camera.h, height_scale);
        let inv_doc_scale = if embedded_gltf_punctual {
            1.0 / s.max(1e-6)
        } else {
            0.0
        };
        let mut hdr_tonemap = self.tile_hdr_tonemap(frame);
        // `bloom_linear_hdr_output` is informational only — the emissive pre-pass
        // uses the same uniforms. Main-menu pride rainbow reuses `hdr_tonemap.w`
        // as scene time for moon/star swirl meshes tagged in their PBR uniform.
        let _ = bloom_linear_hdr_output;
        hdr_tonemap[3] = if main_menu_env
            && crate::main_menu_glb::main_menu_pride_rainbow_active(
                self.main_menu_pride_rainbow_debug,
            )
        {
            self.creation_time.elapsed().as_secs_f32()
        } else {
            0.0
        };
        let (exposure, ambient_x) = if embedded_gltf_punctual {
            let gameplay_table = matches!(env_scene_key, "gameplay" | "tutorial");
            // `gameplay.glb` room + tiles share `ROOM_GLB_LINEAR_EXPOSURE_BASE` × this mul via `tile_hdr_tonemap`.
            let mut e = env_tune.linear_exposure
                * crate::room_glb::ROOM_GLB_LINEAR_EXPOSURE_BASE;
            if gameplay_table {
                e *= crate::gameplay_glb::GAMEPLAY_ENV_LINEAR_EXPOSURE_MUL;
            }
            let mut a = env_tune.ambient_scale;
            if hallway_env {
                e *= crate::hallway_glb::HALLWAY_ENV_LINEAR_EXPOSURE_MUL;
                a = a.max(crate::hallway_glb::HALLWAY_ENV_AMBIENT_SCALE_MIN);
            }
            if staircase_env {
                e *= crate::staircase_glb::STAIRCASE_ENV_LINEAR_EXPOSURE_MUL;
                a = a.max(crate::staircase_glb::STAIRCASE_ENV_AMBIENT_SCALE_MIN);
            }
            if archive_env {
                e *= crate::archive_glb::ARCHIVE_ENV_LINEAR_EXPOSURE_MUL;
                a = a.max(crate::archive_glb::ARCHIVE_ENV_AMBIENT_SCALE_MIN);
            }
            if main_menu_env {
                e *= crate::main_menu_glb::MAIN_MENU_ENV_LINEAR_EXPOSURE_MUL;
                a = a.max(crate::main_menu_glb::MAIN_MENU_ENV_AMBIENT_SCALE_MIN);
            }
            // Gameplay/tutorial GLB: windowless interior — authored ambient only (usually 0).
            if !gameplay_table && !hallway_env && !staircase_env && !archive_env && !main_menu_env {
                a = a.max(crate::room_glb::SHOP_ENV_DIELECTRIC_AMBIENT_MIN);
            }
            (e, a)
        } else {
            (0.0, 0.0)
        };
        let uniform = CameraUniform {
            view_proj: camera.view_proj_arr,
            model: model.to_cols_array(),
            base_color_factor: [
                1.0,
                if frame.shop_env_unlit_debug { 1.0 } else { 0.0 },
                0.0,
                crate::tile_body::TEXTURED_BASE_MAP_BODY_KIND,
            ],
            cam_pos: camera.cam_pos.to_array(),
            tile_seed: if frame.shop_env_unlit_debug {
                1.0
            } else {
                exposure
            },
            decal_atlas_uv: [
                if frame.shop_env_unlit_debug {
                    1.0
                } else {
                    ambient_x
                },
                inv_doc_scale,
                env_tune.gltf_emissive_scale,
                1.0,
            ],
            hdr_tonemap,
        };
        for (pi, buf) in gpu.uniform_buffers.iter().enumerate() {
            let prim_model = if let Some(delta) = prim_deltas.get(&pi) {
                model * *delta
            } else {
                model
            };
            let mut u = uniform;
            u.model = prim_model.to_cols_array();
            self.queue.write_buffer(buf, 0, bytemuck::bytes_of(&u));
        }
        if let Some((lvp, changed)) = shadow_upload {
            self.write_room_env_shadow_caster(gpu, lvp, model, prim_deltas, changed);
        }
    }

    /// Depth-only draws for imported room GLB opaque primitives.
    pub(super) fn draw_gltf_room_env_shadow(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        prims: &[TilePrimitiveGpu],
        gpu: &ShopEnvironmentGpu,
        skip_prim: impl Fn(usize) -> bool,
    ) {
        if prims.is_empty() {
            return;
        }
        pass.set_pipeline(&self.shadow_pipeline_room_env);
        pass.set_bind_group(1, &gpu.shadow_warp_bind_group, &[]);
        for (pi, prim) in prims.iter().enumerate() {
            if skip_prim(pi) || prim.pipeline_key.is_blend() || prim.index_count == 0 {
                continue;
            }
            if prim.vertex_buffer.size() == 0 || prim.index_buffer.size() == 0 {
                continue;
            }
            let Some(bg) = gpu.shadow_bind_groups.get(pi) else {
                continue;
            };
            pass.set_bind_group(0, bg, &[]);
            pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
            pass.set_index_buffer(prim.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..prim.index_count, 0, 0..1);
        }
    }

    pub(super) fn draw_shop_environment_shadow(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        frame: &crate::draw_cmd::UiFrame,
    ) {
        let Some(ref gpu) = self.shop_environment else {
            return;
        };
        let eyeball_only = frame.shop_env_eyeball_only;
        let eyeball_indices = self.shop_eyeball_prim_indices_for_draw();
        self.draw_gltf_room_env_shadow(
            pass,
            &self.shop_env_primitives,
            gpu,
            |pi| eyeball_only && !eyeball_indices.is_empty() && !eyeball_indices.contains(&pi),
        );
    }

    pub(super) fn write_shop_environment_uniforms(
        &self,
        frame: &crate::draw_cmd::UiFrame,
        camera: &CameraFrame,
        bloom_linear_hdr_output: bool,
        shadow_upload: Option<([f32; 16], &mut bool)>,
    ) {
        let Some(ref gpu) = self.shop_environment else {
            return;
        };
        let height = self.env_tune_for("shop").height_scale;
        let s = crate::room_glb::room_env_world_scale(camera.h, height);
        let model = crate::room_glb::with_shop_glb_cpu(|opt| {
            opt.map(|cpu| {
                crate::room_glb::room_env_model_matrix_from_cpu(camera.h, height, cpu)
            })
        })
        .unwrap_or_else(|| Mat4::from_scale(glam::Vec3::splat(s)));
        let prim_deltas = self.shop_gltf_anim_prim_deltas(frame);
        self.write_gltf_room_env_uniforms(GltfRoomEnvUniformParams {
            frame,
            camera,
            env_scene_key: "shop",
            embedded_gltf_punctual: frame.scene_lighting.embedded_gltf_punctual,
            hallway_env: false,
            staircase_env: false,
            archive_env: false,
            main_menu_env: false,
            bloom_linear_hdr_output,
            model,
            gpu,
            shadow_upload,
            prim_deltas: &prim_deltas,
        });
    }

    pub(super) fn write_gameplay_environment_uniforms(
        &self,
        frame: &crate::draw_cmd::UiFrame,
        camera: &CameraFrame,
        bloom_linear_hdr_output: bool,
        shadow_upload: Option<([f32; 16], &mut bool)>,
    ) {
        let Some(ref gpu) = self.gameplay_environment else {
            return;
        };
        let env_key = if self.active_scene_key == Some("tutorial") {
            "tutorial"
        } else {
            "gameplay"
        };
        let height = self.env_tune_for(env_key).height_scale;
        let s = crate::room_glb::room_env_world_scale(camera.h, height);
        let model = crate::gameplay_glb::with_gameplay_glb_cpu(|opt| {
            opt.map(|cpu| {
                crate::room_glb::room_env_model_matrix_from_cpu(camera.h, height, cpu)
            })
        })
        .unwrap_or_else(|| Mat4::from_scale(glam::Vec3::splat(s)));
        let prim_deltas = self.gameplay_score_roller_frame_state(frame);
        self.write_gltf_room_env_uniforms(GltfRoomEnvUniformParams {
            frame,
            camera,
            env_scene_key: env_key,
            embedded_gltf_punctual: frame.scene_lighting.embedded_gltf_punctual,
            hallway_env: false,
            archive_env: false,
            main_menu_env: false,
            bloom_linear_hdr_output,
            model,
            gpu,
            shadow_upload,
            staircase_env: false,
            prim_deltas: &prim_deltas,
        });
    }

    pub(super) fn write_hallway_environment_uniforms(
        &self,
        frame: &crate::draw_cmd::UiFrame,
        camera: &CameraFrame,
        bloom_linear_hdr_output: bool,
        shadow_upload: Option<([f32; 16], &mut bool)>,
    ) {
        let Some(ref gpu) = self.hallway_environment else {
            return;
        };
        let height = self.env_tune_for("pick_chamber").height_scale;
        let s = crate::room_glb::room_env_world_scale(camera.h, height);
        let model = crate::hallway_glb::with_hallway_glb_cpu(|opt| {
            opt.map(|cpu| {
                crate::room_glb::room_env_model_matrix_from_cpu(
                    camera.h,
                    height,
                    cpu,
                )
            })
        })
        .unwrap_or_else(|| Mat4::from_scale(glam::Vec3::splat(s)));
        let prim_deltas = rustc_hash::FxHashMap::default();
        self.write_gltf_room_env_uniforms(GltfRoomEnvUniformParams {
            frame,
            camera,
            env_scene_key: "pick_chamber",
            embedded_gltf_punctual: frame.scene_lighting.embedded_gltf_punctual,
            hallway_env: true,
            staircase_env: false,
            archive_env: false,
            main_menu_env: false,
            bloom_linear_hdr_output,
            model,
            gpu,
            shadow_upload,
            prim_deltas: &prim_deltas,
        });
        let mut dist = frame.hallway_distortion.unwrap_or_default();
        dist.time_pulse[0] = self.creation_time.elapsed().as_secs_f32();
        self.queue
            .write_buffer(&gpu.distortion_buffer, 0, bytemuck::bytes_of(&dist));
    }

    pub(super) fn write_staircase_environment_uniforms(
        &self,
        frame: &crate::draw_cmd::UiFrame,
        camera: &CameraFrame,
        bloom_linear_hdr_output: bool,
        shadow_upload: Option<([f32; 16], &mut bool)>,
    ) {
        let Some(ref gpu) = self.staircase_environment else {
            return;
        };
        let height = self.env_tune_for("staircase").height_scale;
        let s = crate::room_glb::room_env_world_scale(camera.h, height);
        let model = crate::staircase_glb::with_staircase_glb_cpu(|opt| {
            opt.map(|cpu| {
                crate::room_glb::room_env_model_matrix_from_cpu(
                    camera.h,
                    height,
                    cpu,
                )
            })
        })
        .unwrap_or_else(|| Mat4::from_scale(glam::Vec3::splat(s)));
        let prim_deltas = rustc_hash::FxHashMap::default();
        self.write_gltf_room_env_uniforms(GltfRoomEnvUniformParams {
            frame,
            camera,
            env_scene_key: "staircase",
            embedded_gltf_punctual: frame.scene_lighting.embedded_gltf_punctual,
            hallway_env: false,
            staircase_env: true,
            archive_env: false,
            main_menu_env: false,
            bloom_linear_hdr_output,
            model,
            gpu,
            shadow_upload,
            prim_deltas: &prim_deltas,
        });
    }

    /// Rasterize archive browse / inspect copy into per-board decal atlases (`@binding(3)`).
    pub(super) fn sync_archive_description_decal_texture(
        &mut self,
        frame: &crate::draw_cmd::UiFrame,
    ) {
        self.sync_archive_sign_decal_texture(frame);
        self.sync_archive_inspect_plaque_decal_texture(frame);
    }

    fn sync_archive_sign_decal_texture(&mut self, frame: &crate::draw_cmd::UiFrame) {
        let Some(gpu) = self.archive_environment.as_ref() else {
            return;
        };
        let Some(tex) = gpu.archive_sign_decal_texture.as_ref() else {
            return;
        };
        use crate::archive_glb::archive_sign_description_decal_extents;
        use crate::decal::{PLAQUE_DECAL_HEIGHT, decal_dimensions};
        use crate::primitive::DecalLayout;

        let layout = DecalLayout::Fit {
            target_short_edge: PLAQUE_DECAL_HEIGHT,
        };
        let (dw, dh) = decal_dimensions(&layout, archive_sign_description_decal_extents());
        let key = match frame.archive_sign_description_decal_text.as_ref() {
            None => u64::MAX,
            Some(t) => super::super::tablet_label_hash(t, dw, dh),
        };
        if key == self.archive_sign_decal_upload_key {
            return;
        }
        self.archive_sign_decal_upload_key = key;
        self.upload_archive_decal_texture(tex, dw, dh, frame.archive_sign_description_decal_text.as_deref());
    }

    fn sync_archive_inspect_plaque_decal_texture(&mut self, frame: &crate::draw_cmd::UiFrame) {
        let Some(gpu) = self.archive_environment.as_ref() else {
            return;
        };
        let Some(tex) = gpu.archive_inspect_plaque_decal_texture.as_ref() else {
            return;
        };
        use crate::archive_glb::archive_inspect_plaque_decal_extents;
        use crate::decal::{PLAQUE_DECAL_HEIGHT, decal_dimensions};
        use crate::primitive::DecalLayout;

        let layout = DecalLayout::Fit {
            target_short_edge: PLAQUE_DECAL_HEIGHT,
        };
        let (dw, dh) = decal_dimensions(&layout, archive_inspect_plaque_decal_extents());
        let key = match frame.archive_inspect_plaque_decal_text.as_ref() {
            None => u64::MAX,
            Some(t) => super::super::tablet_label_hash(t, dw, dh),
        };
        if key == self.archive_inspect_plaque_decal_upload_key {
            return;
        }
        self.archive_inspect_plaque_decal_upload_key = key;
        self.upload_archive_decal_texture(tex, dw, dh, frame.archive_inspect_plaque_decal_text.as_deref());
    }

    fn upload_archive_decal_texture(
        &self,
        tex: &wgpu::Texture,
        dw: u32,
        dh: u32,
        text: Option<&str>,
    ) {
        use crate::decal::{PlaqueDecalStyle, rasterize_plaque_decal_styled};

        let rgba = match text {
            None => vec![0u8; (dw * dh * 4) as usize],
            Some(text) => rasterize_plaque_decal_styled(
                text,
                self.ui_font.as_ref(),
                self.emoji_font.as_ref(),
                dw,
                dh,
                PlaqueDecalStyle::WalnutInkOnLight,
            ),
        };
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * dw),
                rows_per_image: Some(dh),
            },
            wgpu::Extent3d {
                width: dw,
                height: dh,
                depth_or_array_layers: 1,
            },
        );
    }

    pub(super) fn write_archive_environment_uniforms(
        &self,
        frame: &crate::draw_cmd::UiFrame,
        camera: &CameraFrame,
        bloom_linear_hdr_output: bool,
        shadow_upload: Option<([f32; 16], &mut bool)>,
    ) {
        let Some(ref gpu) = self.archive_environment else {
            return;
        };
        let height = self.env_tune_for("collection").height_scale;
        let s = crate::room_glb::room_env_world_scale(camera.h, height);
        let model = crate::archive_glb::with_archive_glb_cpu(|opt| {
            opt.map(|cpu| {
                crate::room_glb::room_env_model_matrix_from_cpu(
                    camera.h,
                    height,
                    cpu,
                )
            })
        })
        .unwrap_or_else(|| Mat4::from_scale(glam::Vec3::splat(s)));
        let prim_deltas = rustc_hash::FxHashMap::default();
        self.write_gltf_room_env_uniforms(GltfRoomEnvUniformParams {
            frame,
            camera,
            env_scene_key: "collection",
            embedded_gltf_punctual: frame.scene_lighting.embedded_gltf_punctual,
            hallway_env: false,
            staircase_env: false,
            archive_env: true,
            main_menu_env: false,
            bloom_linear_hdr_output,
            model,
            gpu,
            shadow_upload,
            prim_deltas: &prim_deltas,
        });
    }

    /// Draw [`main_menu.glb`] hub waterfront.
    pub(super) fn draw_main_menu_environment_meshes(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        frame: &crate::draw_cmd::UiFrame,
        room_hdr_mrt_emissive: bool,
    ) {
        let Some(ref gpu) = self.main_menu_environment else {
            return;
        };
        self.draw_gltf_room_env_meshes(
            pass,
            frame,
            &self.main_menu_env_primitives,
            gpu,
            room_hdr_mrt_emissive,
            |_| false,
        );
    }

    pub(super) fn write_main_menu_environment_uniforms(
        &self,
        frame: &crate::draw_cmd::UiFrame,
        camera: &CameraFrame,
        bloom_linear_hdr_output: bool,
        shadow_upload: Option<([f32; 16], &mut bool)>,
    ) {
        let Some(ref gpu) = self.main_menu_environment else {
            return;
        };
        let height = self.env_tune_for("main_menu_exterior").height_scale;
        let env_h =
            crate::main_menu_glb::main_menu_env_height_scale(height);
        let s = crate::room_glb::room_env_world_scale(camera.h, env_h);
        let model = crate::main_menu_glb::with_main_menu_glb_cpu(|opt| {
            opt.map(|cpu| {
                crate::room_glb::room_env_model_matrix_from_cpu(camera.h, env_h, cpu)
            })
        })
        .unwrap_or_else(|| Mat4::from_scale(glam::Vec3::splat(s)));
        let prim_deltas = rustc_hash::FxHashMap::default();
        self.write_gltf_room_env_uniforms(GltfRoomEnvUniformParams {
            frame,
            camera,
            env_scene_key: "main_menu_exterior",
            embedded_gltf_punctual: frame.scene_lighting.embedded_gltf_punctual,
            hallway_env: false,
            staircase_env: false,
            archive_env: false,
            main_menu_env: true,
            bloom_linear_hdr_output,
            model,
            gpu,
            shadow_upload,
            prim_deltas: &prim_deltas,
        });
    }

    /// Upload shop collision AABBs for per-punctual ray occlusion in `room_glb.wgsl`.
    pub(super) fn write_shop_room_punctual_occluders(&self, camera: &CameraFrame) {
        if self.shop_env_collision_meshes.is_empty() {
            return;
        }
        let height = self.env_tune_for("shop").height_scale;
        let model = crate::room_glb::with_shop_glb_cpu(|opt| {
            opt.map(|cpu| {
                crate::room_glb::room_env_model_matrix_from_cpu(
                    camera.h,
                    height,
                    cpu,
                )
            })
        })
        .unwrap_or_else(|| {
            let s = crate::room_glb::room_env_world_scale(
                camera.h,
                self.env_tune_for("shop").height_scale,
            );
            glam::Mat4::from_scale(glam::Vec3::splat(s))
        });
        let occ =
            TileOccludersBuf::from_room_collision_meshes(model, &self.shop_env_collision_meshes);
        self.queue
            .write_buffer(&self.tile_occluders_buffer, 0, bytemuck::bytes_of(&occ));
    }

    /// Upload main-menu roof AABBs so porch punctuals respect `rooflet` / main roof shells.
    pub(super) fn write_main_menu_room_punctual_occluders(&self, camera: &CameraFrame) {
        if self.main_menu_env_collision_meshes.is_empty() {
            return;
        }
        let height = self.env_tune_for("main_menu_exterior").height_scale;
        let env_h =
            crate::main_menu_glb::main_menu_env_height_scale(height);
        let model = crate::main_menu_glb::with_main_menu_glb_cpu(|opt| {
            opt.map(|cpu| {
                crate::room_glb::room_env_model_matrix_from_cpu(camera.h, env_h, cpu)
            })
        })
        .unwrap_or_else(|| {
            let s = crate::room_glb::room_env_world_scale(camera.h, env_h);
            glam::Mat4::from_scale(glam::Vec3::splat(s))
        });
        let occ = TileOccludersBuf::from_room_collision_meshes(
            model,
            &self.main_menu_env_collision_meshes,
        );
        self.queue
            .write_buffer(&self.tile_occluders_buffer, 0, bytemuck::bytes_of(&occ));
    }

    /// Lit pass: hide authored cash-in control when structure cannot be scored.
    #[inline]
    fn gameplay_env_skip_cash_in_prim(
        &self,
        pi: usize,
        frame: &crate::draw_cmd::UiFrame,
    ) -> bool {
        !frame.gameplay_cash_in_button_visible && self.gameplay_cash_in_prim_indices.contains(&pi)
    }

    #[inline]
    fn archive_env_is_description_sign_prim(&self, pi: usize) -> bool {
        self.archive_sign_left_prim_idx == Some(pi) || self.archive_sign_right_prim_idx == Some(pi)
    }

    #[inline]
    fn archive_env_is_inspect_plaque_prim(&self, pi: usize) -> bool {
        self.archive_inspect_plaque_prim_idx == Some(pi)
    }

    /// Lit pass: draw only the active description board (opposite the focus ref).
    #[inline]
    pub(super) fn archive_env_skip_archive_prim(
        &self,
        pi: usize,
        frame: &crate::draw_cmd::UiFrame,
    ) -> bool {
        self.archive_env_skip_description_prim(pi, frame)
            || self.archive_env_skip_inspect_plaque_prim(pi, frame)
            || self.archive_env_skip_page_button_prim(pi, frame)
    }

    #[inline]
    fn archive_env_skip_page_button_prim(
        &self,
        pi: usize,
        frame: &crate::draw_cmd::UiFrame,
    ) -> bool {
        if self.archive_page_left_prim_indices.contains(&pi) {
            return !frame.archive_page_left_visible;
        }
        if self.archive_page_right_prim_indices.contains(&pi) {
            return !frame.archive_page_right_visible;
        }
        false
    }

    /// Lit pass: draw only the active description board (opposite the focus ref); hide both
    /// boards while item inspect owns the decal on `inspect_plaque`.
    #[inline]
    pub(super) fn archive_env_skip_description_prim(
        &self,
        pi: usize,
        frame: &crate::draw_cmd::UiFrame,
    ) -> bool {
        if !self.archive_env_is_description_sign_prim(pi) {
            return false;
        }
        if frame.archive_inspect_plaque_visible {
            return true;
        }
        match frame.archive_description_sign_use_left {
            Some(true) => self.archive_sign_right_prim_idx == Some(pi),
            Some(false) => self.archive_sign_left_prim_idx == Some(pi),
            _ => false,
        }
    }

    /// Lit pass: `inspect_plaque` only while item inspect is active.
    #[inline]
    pub(super) fn archive_env_skip_inspect_plaque_prim(
        &self,
        pi: usize,
        frame: &crate::draw_cmd::UiFrame,
    ) -> bool {
        self.archive_env_is_inspect_plaque_prim(pi) && !frame.archive_inspect_plaque_visible
    }

    /// Shadow pre-pass / offline bake: never cast from either sign — they are flat
    /// decal boards and project hard silhouettes onto the featured pedestal.
    #[inline]
    pub(super) fn archive_env_skip_room_shadow_caster(&self, pi: usize) -> bool {
        self.archive_env_shadow_caster_mask
            .get(pi)
            .is_some_and(|casts| !*casts)
    }

    /// Shadow pre-pass / offline bake: skip primitives opted out at glTF load.
    #[inline]
    pub(super) fn gameplay_env_skip_room_shadow_caster(&self, pi: usize) -> bool {
        self.gameplay_env_shadow_caster_mask
            .get(pi)
            .is_some_and(|casts| !*casts)
    }
}

#[cfg(test)]
mod gameplay_score_roller_tests {
    use super::{
        gameplay_score_roller_loop_speed_multiplier, gameplay_score_roller_speed_multiplier,
        gameplay_score_roller_visual_phase, GAMEPLAY_SCORE_ROLLER_LOOP_SPIN_IN_SEMITONES,
        GAMEPLAY_SCORE_ROLLER_RAMP_INTERVAL_SECS,
    };

    #[test]
    fn speed_multiplier_steps_every_two_seconds() {
        assert_eq!(gameplay_score_roller_speed_multiplier(0.0), 1.0);
        assert_eq!(
            gameplay_score_roller_speed_multiplier(GAMEPLAY_SCORE_ROLLER_RAMP_INTERVAL_SECS - 1e-6),
            1.0
        );
        assert_eq!(
            gameplay_score_roller_speed_multiplier(GAMEPLAY_SCORE_ROLLER_RAMP_INTERVAL_SECS),
            2.0
        );
        assert_eq!(
            gameplay_score_roller_speed_multiplier(GAMEPLAY_SCORE_ROLLER_RAMP_INTERVAL_SECS * 2.5),
            3.0
        );
    }

    #[test]
    fn loop_speed_multiplier_climbs_gently() {
        let spin_in =
            2.0_f64.powf(GAMEPLAY_SCORE_ROLLER_LOOP_SPIN_IN_SEMITONES / 12.0);
        assert!((gameplay_score_roller_loop_speed_multiplier(0.0) - spin_in).abs() < 1e-6);
        assert!((gameplay_score_roller_loop_speed_multiplier(0.49) - spin_in).abs() < 1e-6);
        assert!((gameplay_score_roller_loop_speed_multiplier(0.5) - 1.0).abs() < 1e-6);
        let at_2s = gameplay_score_roller_loop_speed_multiplier(GAMEPLAY_SCORE_ROLLER_RAMP_INTERVAL_SECS);
        assert!((at_2s - 2.0_f64.powf(2.0 / 12.0)).abs() < 1e-6);
        assert!(at_2s < gameplay_score_roller_speed_multiplier(GAMEPLAY_SCORE_ROLLER_RAMP_INTERVAL_SECS));
    }

    #[test]
    fn visual_phase_snaps_fractional_carry_residue() {
        // Score 42 leaves the tens wheel at 4.2 from continuous division.
        let snapped = gameplay_score_roller_visual_phase(4.2);
        assert!(
            (snapped - 4.0).abs() < 0.05,
            "expected ~4.0, got {snapped}"
        );
    }

    #[test]
    fn visual_phase_preserves_integer_steps() {
        assert!((gameplay_score_roller_visual_phase(2.0) - 2.0).abs() < 1e-6);
        assert!((gameplay_score_roller_visual_phase(7.0) - 7.0).abs() < 1e-6);
    }


    #[test]
    fn slot_wheel_phase_ones_tracks_drive() {
        use super::gameplay_score_roller_slot_wheel_phase;
        let drives = [42.0, 500.0];
        assert!((gameplay_score_roller_slot_wheel_phase(9, &drives) - 2.0).abs() < 1e-6);
        assert!((gameplay_score_roller_slot_wheel_phase(8, &drives) - 4.2).abs() < 1e-6);
    }
}
