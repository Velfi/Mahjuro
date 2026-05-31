//! Runtime punctual shadow diagnostics (`MAHJURO_SHADOW_PROBE=1`).

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use glam::{Mat4, Vec3};

use super::shadow_setup::ActiveRoomEnv;
use crate::projected_light_shadow::PunctualShadowBuild;

const LOG_INTERVAL: Duration = Duration::from_secs(2);

pub(crate) fn shadow_probe_enabled() -> bool {
    static CACHED: OnceLock<bool> = OnceLock::new();
    *CACHED.get_or_init(|| {
        std::env::var("MAHJURO_SHADOW_PROBE")
            .map(|v| !v.is_empty() && v != "0")
            .unwrap_or(false)
    })
}

#[derive(Clone, Debug)]
struct ShadowProbeCpuSummary {
    punctual_count: usize,
    caster_count: usize,
    active_env: Option<ActiveRoomEnv>,
    scene_key: Option<String>,
    layer_index: u32,
    source_light_index: u32,
    center_ndc: [f32; 3],
    center_uv: [f32; 2],
    center_in_frustum: bool,
}

fn world_to_ndc(world: Vec3, view_proj: Mat4) -> glam::Vec3 {
    let clip = view_proj * world.extend(1.0);
    clip.truncate() / clip.w.max(1e-8)
}

fn ndc_uv(ndc: glam::Vec3) -> glam::Vec2 {
    glam::Vec2::new(ndc.x * 0.5 + 0.5, ndc.y * -0.5 + 0.5)
}

fn center_in_frustum(ndc: glam::Vec3, uv: glam::Vec2) -> bool {
    ndc.z >= 0.0
        && ndc.z <= 1.0
        && uv.x >= 0.0
        && uv.x <= 1.0
        && uv.y >= 0.0
        && uv.y <= 1.0
}

fn cpu_summary(
    build: &PunctualShadowBuild,
    punctual_count: usize,
    active_env: Option<ActiveRoomEnv>,
    scene_key: Option<&str>,
) -> Option<ShadowProbeCpuSummary> {
    let first = build.casters.first()?;
    let ndc = world_to_ndc(Vec3::ZERO, first.light_view_proj);
    let uv = ndc_uv(ndc);
    Some(ShadowProbeCpuSummary {
        punctual_count,
        caster_count: build.casters.len(),
        active_env,
        scene_key: scene_key.map(str::to_string),
        layer_index: first.layer_index,
        source_light_index: first.source_light_index,
        center_ndc: ndc.to_array(),
        center_uv: uv.to_array(),
        center_in_frustum: center_in_frustum(ndc, uv),
    })
}

pub(crate) struct ShadowProbeStaging {
    buffer: wgpu::Buffer,
    byte_len: u64,
    width: u32,
    height: u32,
    summary: ShadowProbeCpuSummary,
}

impl crate::wgpu_renderer::WgpuRenderer {
    /// After the shadow pre-pass: copy layer 0 depth for min/max readback (next frame tail).
    pub(super) fn schedule_shadow_probe_copy(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        build: &PunctualShadowBuild,
        punctual_count: usize,
        active_env: Option<ActiveRoomEnv>,
    ) -> Option<ShadowProbeStaging> {
        if !shadow_probe_enabled() {
            return None;
        }
        let caster_count = build.casters.len();
        let should_log = caster_count != self.shadow_probe_last_caster_count
            || self.shadow_probe_last_log.elapsed() >= LOG_INTERVAL;
        if !should_log {
            return None;
        }
        self.shadow_probe_last_caster_count = caster_count;

        let summary = if let Some(summary) = cpu_summary(
            build,
            punctual_count,
            active_env,
            self.active_scene_key,
        ) {
            summary
        } else if punctual_count > 0 {
            ShadowProbeCpuSummary {
                punctual_count,
                caster_count: 0,
                active_env,
                scene_key: self.active_scene_key.map(str::to_string),
                layer_index: 0,
                source_light_index: 0,
                center_ndc: [0.0; 3],
                center_uv: [0.0; 2],
                center_in_frustum: false,
            }
        } else {
            return None;
        };

        if build.casters.is_empty() {
            log_shadow_probe_cpu(&summary, None);
            self.shadow_probe_last_log = Instant::now();
            return None;
        }

        let layer = summary.layer_index;
        let width = self.point_shadow_array.size;
        let height = width;
        let byte_len = (width as u64) * (height as u64) * 4;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shadow-probe-staging"),
            size: byte_len,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.point_shadow_array.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: 0,
                    y: 0,
                    z: layer,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * width),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        Some(ShadowProbeStaging {
            buffer,
            byte_len,
            width,
            height,
            summary,
        })
    }

    pub(super) fn finalize_shadow_probe(&mut self, staging: ShadowProbeStaging) {
        let slice = staging.buffer.slice(..staging.byte_len);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        match rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                log::warn!("shadow_probe: depth map failed: {e:?}");
                return;
            }
            Err(_) => {
                log::warn!("shadow_probe: map channel closed");
                return;
            }
        }
        let mapped = slice.get_mapped_range();
        let depth_stats = depth_layer_stats(&mapped, staging.width, staging.height);
        drop(mapped);
        staging.buffer.unmap();
        log_shadow_probe_cpu(&staging.summary, Some(depth_stats));
        self.shadow_probe_last_log = Instant::now();
    }
}

#[derive(Clone, Copy, Debug)]
struct DepthLayerStats {
    min: f32,
    max: f32,
    /// Share of texels with depth clearly below the cleared-far value (1.0).
    covered_frac: f32,
}

fn depth_layer_stats(mapped: &[u8], width: u32, height: u32) -> DepthLayerStats {
    let pixels = (width * height) as usize;
    let mut min = f32::INFINITY;
    let mut max = f32::NEG_INFINITY;
    let mut covered = 0u32;
    for i in 0..pixels {
        let off = i * 4;
        if off + 4 > mapped.len() {
            break;
        }
        let d = f32::from_le_bytes(mapped[off..off + 4].try_into().unwrap());
        if d.is_finite() {
            min = min.min(d);
            max = max.max(d);
            if d < 0.999 {
                covered += 1;
            }
        }
    }
    if !min.is_finite() {
        min = 1.0;
        max = 1.0;
    }
    DepthLayerStats {
        min,
        max,
        covered_frac: covered as f32 / pixels.max(1) as f32,
    }
}

fn log_shadow_probe_cpu(summary: &ShadowProbeCpuSummary, depth: Option<DepthLayerStats>) {
    match depth {
        Some(d) => log::info!(
            "shadow_probe: punctual={} casters={} env={:?} scene={:?} \
             layer={} light_idx={} center_ndc={:?} center_uv={:?} in_frustum={} \
             depth_layer min={:.4} max={:.4} covered={:.1}%",
            summary.punctual_count,
            summary.caster_count,
            summary.active_env,
            summary.scene_key,
            summary.layer_index,
            summary.source_light_index,
            summary.center_ndc,
            summary.center_uv,
            summary.center_in_frustum,
            d.min,
            d.max,
            d.covered_frac * 100.0,
        ),
        None => log::info!(
            "shadow_probe: punctual={} casters={} env={:?} scene={:?} (no depth readback)",
            summary.punctual_count,
            summary.caster_count,
            summary.active_env,
            summary.scene_key,
        ),
    }
}
