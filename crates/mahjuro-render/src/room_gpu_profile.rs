//! Opt-in metrics for room GLB CPU decode and main-thread GPU upload.
//!
//! Enable with `MAHJURO_STARTUP_PROFILE=1`.
//! Logs one `log::info!` line per completed decode/upload with payload sizes and
//! the previous frame's `dt` (hitch proxy). Also feeds [`crate::startup_profile`]
//! and [`crate::cpu_profiler`] scopes when those sessions are active.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::room_env_gltf::RoomEnvPrimitiveCpu;
use crate::tile_glb::{LoadedPrimitive, Vertex3dTex};

static ENABLED: OnceLock<bool> = OnceLock::new();

/// True when room GPU profiling env vars are set.
pub fn enabled() -> bool {
    *ENABLED.get_or_init(|| std::env::var_os("MAHJURO_STARTUP_PROFILE").is_some())
}

/// CPU-side bytes that will be copied to the GPU during room environment upload.
#[derive(Clone, Copy, Debug, Default)]
pub struct RoomCpuUploadPayload {
    pub primitives: u32,
    pub vertex_bytes: u64,
    pub index_bytes: u64,
    /// Unique texture slots (albedo / normal / MR / emissive), counting only maps present.
    pub texture_slots: u32,
    pub texture_bytes: u64,
}

impl RoomCpuUploadPayload {
    pub fn total_bytes(&self) -> u64 {
        self.vertex_bytes + self.index_bytes + self.texture_bytes
    }
}

pub fn count_cpu_payload(primitives: &[RoomEnvPrimitiveCpu]) -> RoomCpuUploadPayload {
    let mut out = RoomCpuUploadPayload {
        primitives: primitives.len() as u32,
        ..Default::default()
    };
    for env in primitives {
        count_loaded_primitive(&env.mesh, &mut out);
    }
    out
}

fn count_loaded_primitive(prim: &LoadedPrimitive, out: &mut RoomCpuUploadPayload) {
    out.vertex_bytes += (prim.vertices.len() * std::mem::size_of::<Vertex3dTex>()) as u64;
    out.index_bytes += (prim.indices.len() * std::mem::size_of::<u32>()) as u64;
    count_texture_slot(
        prim.albedo_rgba.as_ref(),
        prim.albedo_mip_chain.as_deref().map(|c| c.as_slice()),
        out,
    );
    count_texture_slot(
        prim.normal_rgba.as_ref(),
        prim.normal_mip_chain.as_deref().map(|c| c.as_slice()),
        out,
    );
    count_texture_slot(
        prim.metallic_roughness_rgba.as_ref(),
        prim.metallic_roughness_mip_chain
            .as_deref()
            .map(|c| c.as_slice()),
        out,
    );
    count_texture_slot(
        prim.emissive_rgba.as_ref(),
        prim.emissive_mip_chain.as_deref().map(|c| c.as_slice()),
        out,
    );
}

fn count_texture_slot(
    rgba: Option<&(Vec<u8>, u32, u32)>,
    mip_chain: Option<&[(Vec<u8>, u32, u32)]>,
    out: &mut RoomCpuUploadPayload,
) {
    let Some((base, _, _)) = rgba else {
        return;
    };
    out.texture_slots += 1;
    out.texture_bytes += texture_slot_bytes(base, mip_chain);
}

fn texture_slot_bytes(base: &[u8], mip_chain: Option<&[(Vec<u8>, u32, u32)]>) -> u64 {
    match mip_chain {
        Some(chain) if !chain.is_empty() => chain.iter().map(|(b, _, _)| b.len() as u64).sum(),
        _ => base.len() as u64,
    }
}

/// Record background-thread GLB decode duration (called when the prefetch worker finishes).
pub fn record_cpu_decode(room: &'static str, elapsed: Duration) {
    if !enabled() {
        return;
    }
    let ms = elapsed.as_secs_f64() * 1000.0;
    if let Some(scope) = cpu_decode_startup_scope(room) {
        crate::startup_profile::record(scope, elapsed);
    }
    log::info!("room gpu profile: {room} CPU decode — {ms:.1} ms (worker thread)");
}

fn cpu_decode_startup_scope(room: &str) -> Option<&'static str> {
    match room {
        "shop.glb" => Some("room.cpu.shop"),
        "archive.glb" => Some("room.cpu.archive"),
        "hallway.glb" => Some("room.cpu.hallway"),
        "gameplay.glb" => Some("room.cpu.gameplay"),
        _ => None,
    }
}

/// Run a main-thread GPU upload and log metrics when profiling is enabled.
pub fn measure_gpu_upload(
    room: &'static str,
    startup_scope: &'static str,
    payload: RoomCpuUploadPayload,
    frame_dt_ms: f32,
    upload: impl FnOnce(),
) {
    let _cpu = crate::cpu_profiler::scope(startup_scope);
    let _startup = crate::startup_profile::scope(startup_scope);
    let t0 = Instant::now();
    upload();
    let gpu_ms = t0.elapsed().as_secs_f64() * 1000.0;
    if !enabled() {
        return;
    }
    let mb = payload.total_bytes() as f64 / (1024.0 * 1024.0);
    let hitch = if frame_dt_ms >= 33.0 {
        "HITCH"
    } else if frame_dt_ms >= 20.0 {
        "slow"
    } else {
        "ok"
    };
    log::info!(
        "room gpu profile: {room} GPU upload — {gpu_ms:.1} ms | {prims} prims | \
         {v_mb:.2} MiB verts+idx | {tex_slots} tex slots {tex_mb:.2} MiB | {total_mb:.2} MiB CPU payload | \
         prev frame dt {frame_dt_ms:.1} ms ({hitch})",
        prims = payload.primitives,
        v_mb = (payload.vertex_bytes + payload.index_bytes) as f64 / (1024.0 * 1024.0),
        tex_slots = payload.texture_slots,
        tex_mb = payload.texture_bytes as f64 / (1024.0 * 1024.0),
        total_mb = mb,
    );
}
