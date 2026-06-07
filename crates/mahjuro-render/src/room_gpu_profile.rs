//! Opt-in metrics for room GLB CPU decode and main-thread GPU upload.
//!
//! Enable with `MAHJURO_STARTUP_PROFILE=1`.
//! Logs one `log::info!` line per completed decode/upload with payload sizes and
//! the previous frame's `dt` (hitch proxy). Also feeds [`crate::startup_profile`]
//! and [`crate::cpu_profiler`] scopes when those sessions are active.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::gltf_helpers::GltfSamplerCpu;
use crate::room_env_gltf::{RoomEnvPrimitiveCpu, RoomTextureSourceMeta, RoomTextureUsageClass};
use crate::tile_glb::{LoadedPrimitive, Vertex3dTex};
use rustc_hash::FxHashMap;

static ENABLED: OnceLock<bool> = OnceLock::new();
static LAST_TEXTURE_AUDITS: OnceLock<Mutex<FxHashMap<&'static str, RoomTextureAuditSummary>>> =
    OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RuntimePhase {
    #[default]
    StartupBlocking,
    LoadingScreen,
    SplashNonInteractive,
    MenuInteractive,
    GameplayInteractive,
}

impl RuntimePhase {
    pub fn label(self) -> &'static str {
        match self {
            Self::StartupBlocking => "startup_blocking",
            Self::LoadingScreen => "loading_screen",
            Self::SplashNonInteractive => "splash_non_interactive",
            Self::MenuInteractive => "menu_interactive",
            Self::GameplayInteractive => "gameplay_interactive",
        }
    }

    #[inline]
    fn is_interactive(self) -> bool {
        matches!(self, Self::MenuInteractive | Self::GameplayInteractive)
    }
}

/// True when room GPU profiling env vars are set.
pub fn enabled() -> bool {
    *ENABLED.get_or_init(|| std::env::var_os("MAHJURO_STARTUP_PROFILE").is_some())
}

pub fn frame_timing_tag(frame_dt_ms: f32, phase: RuntimePhase) -> &'static str {
    if !phase.is_interactive() {
        return "preload";
    }
    if frame_dt_ms >= 33.0 {
        "HITCH"
    } else if frame_dt_ms >= 20.0 {
        "slow"
    } else {
        "ok"
    }
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
        prim.albedo_rgba.as_deref(),
        prim.albedo_mip_chain.as_deref().map(|c| c.as_slice()),
        out,
    );
    count_texture_slot(
        prim.normal_rgba.as_deref(),
        prim.normal_mip_chain.as_deref().map(|c| c.as_slice()),
        out,
    );
    count_texture_slot(
        prim.metallic_roughness_rgba.as_deref(),
        prim.metallic_roughness_mip_chain
            .as_deref()
            .map(|c| c.as_slice()),
        out,
    );
    count_texture_slot(
        prim.emissive_rgba.as_deref(),
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

pub fn texture_slot_bytes(base: &[u8], mip_chain: Option<&[(Vec<u8>, u32, u32)]>) -> u64 {
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
    phase: RuntimePhase,
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
    let hitch = frame_timing_tag(frame_dt_ms, phase);
    log::info!(
        "room gpu profile: {room} GPU upload — {gpu_ms:.1} ms | {prims} prims | \
         {v_mb:.2} MiB verts+idx | {tex_slots} tex slots {tex_mb:.2} MiB | {total_mb:.2} MiB decoded_cpu_payload | \
         prev frame dt {frame_dt_ms:.1} ms ({hitch}, phase={phase})",
        prims = payload.primitives,
        v_mb = (payload.vertex_bytes + payload.index_bytes) as f64 / (1024.0 * 1024.0),
        tex_slots = payload.texture_slots,
        tex_mb = payload.texture_bytes as f64 / (1024.0 * 1024.0),
        total_mb = mb,
        phase = phase.label(),
    );
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RoomTextureAuditSummary {
    pub material_texture_refs: u32,
    pub unique_image_sources: u32,
    pub unique_dedupe_keys: u32,
    pub deduped_refs: u32,
    pub total_decoded_cpu_bytes: u64,
    pub total_gpu_bytes_estimate_bytes: u64,
    pub total_gpu_bytes_reference_bytes: u64,
}

fn last_texture_audits() -> &'static Mutex<FxHashMap<&'static str, RoomTextureAuditSummary>> {
    LAST_TEXTURE_AUDITS.get_or_init(|| Mutex::new(FxHashMap::default()))
}

pub fn last_room_texture_audit(room: &'static str) -> Option<RoomTextureAuditSummary> {
    last_texture_audits()
        .lock()
        .ok()
        .and_then(|m| m.get(room).copied())
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct SamplerClassKey {
    wrap_s: u8,
    wrap_t: u8,
    mag: u8,
    min: u8,
}

impl SamplerClassKey {
    fn from_sampler(s: GltfSamplerCpu) -> Self {
        Self {
            wrap_s: wrap_tag(s.wrap_s),
            wrap_t: wrap_tag(s.wrap_t),
            mag: mag_tag(s.mag_filter),
            min: min_tag(s.min_filter),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct TextureAuditDedupeKey {
    source_identity: String,
    usage: RoomTextureUsageClass,
    sampler_class: SamplerClassKey,
    mip_policy: bool,
    format_tag: u8,
    content_hash: u64,
    width: u32,
    height: u32,
}

#[derive(Clone, Debug)]
struct TextureAuditRow {
    key: TextureAuditDedupeKey,
    material_name: Option<String>,
    material_index: Option<usize>,
    source_label: String,
    source_format: &'static str,
    usage: RoomTextureUsageClass,
    width: u32,
    height: u32,
    mip_count: usize,
    decoded_cpu_bytes: u64,
    gpu_bytes_estimate: u64,
}

fn wrap_tag(w: gltf::texture::WrappingMode) -> u8 {
    use gltf::texture::WrappingMode::*;
    match w {
        ClampToEdge => 0,
        MirroredRepeat => 1,
        Repeat => 2,
    }
}

fn mag_tag(f: Option<gltf::texture::MagFilter>) -> u8 {
    use gltf::texture::MagFilter::*;
    match f {
        Some(Nearest) => 1,
        Some(Linear) => 2,
        None => 0,
    }
}

fn min_tag(f: Option<gltf::texture::MinFilter>) -> u8 {
    use gltf::texture::MinFilter::*;
    match f {
        Some(Nearest) => 1,
        Some(Linear) => 2,
        Some(NearestMipmapNearest) => 3,
        Some(LinearMipmapNearest) => 4,
        Some(NearestMipmapLinear) => 5,
        Some(LinearMipmapLinear) => 6,
        None => 0,
    }
}

fn texture_format_tag(format: wgpu::TextureFormat) -> u8 {
    match format {
        wgpu::TextureFormat::Rgba8UnormSrgb => 0,
        wgpu::TextureFormat::Rgba8Unorm => 1,
        _ => 2,
    }
}

fn content_hash_bytes(base: &[u8], mip_chain: Option<&[(Vec<u8>, u32, u32)]>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = rustc_hash::FxHasher::default();
    base.hash(&mut hasher);
    if let Some(chain) = mip_chain {
        for (level, (bytes, w, h)) in chain.iter().enumerate() {
            level.hash(&mut hasher);
            w.hash(&mut hasher);
            h.hash(&mut hasher);
            bytes.hash(&mut hasher);
        }
    }
    hasher.finish()
}

fn dedupe_key_fingerprint(key: &TextureAuditDedupeKey) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = rustc_hash::FxHasher::default();
    key.hash(&mut hasher);
    hasher.finish()
}

fn gpu_bytes_estimate_rgba8(
    width: u32,
    height: u32,
    mips: bool,
    known_chain: Option<&[(Vec<u8>, u32, u32)]>,
) -> u64 {
    if let Some(chain) = known_chain
        && !chain.is_empty()
    {
        return chain.iter().map(|(bytes, _, _)| bytes.len() as u64).sum();
    }
    if !mips {
        return (width as u64) * (height as u64) * 4;
    }
    let levels = crate::gltf_helpers::mip_level_count(width, height);
    let mut total = 0u64;
    let mut w = width.max(1);
    let mut h = height.max(1);
    for _ in 0..levels {
        total = total.saturating_add((w as u64) * (h as u64) * 4);
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    total
}

fn source_identity_for_slot(
    source_meta: Option<&RoomTextureSourceMeta>,
    usage: RoomTextureUsageClass,
    width: u32,
    height: u32,
    content_hash: u64,
) -> String {
    if let Some(meta) = source_meta {
        return meta.source_identity.clone();
    }
    format!(
        "generated:{}:{}x{}:0x{content_hash:016x}",
        usage.label(),
        width,
        height
    )
}

fn collect_texture_audit_row(
    rows: &mut Vec<TextureAuditRow>,
    env_prim: &RoomEnvPrimitiveCpu,
    usage: RoomTextureUsageClass,
    rgba: Option<&(Vec<u8>, u32, u32)>,
    mip_chain: Option<&[(Vec<u8>, u32, u32)]>,
) {
    let Some((base, width, height)) = rgba else {
        return;
    };
    let mips = crate::gltf_helpers::wants_mipmaps(env_prim.mesh.sampler.min_filter);
    let content_hash = content_hash_bytes(base, mip_chain);
    let source_meta = env_prim.texture_sources.for_usage(usage);
    let source_label = source_identity_for_slot(source_meta, usage, *width, *height, content_hash);
    let source_format = source_meta.and_then(|m| m.source_format).unwrap_or("unknown");
    let key = TextureAuditDedupeKey {
        source_identity: source_label.clone(),
        usage,
        sampler_class: SamplerClassKey::from_sampler(env_prim.mesh.sampler),
        mip_policy: mips,
        format_tag: texture_format_tag(usage.gpu_format()),
        content_hash,
        width: *width,
        height: *height,
    };
    let decoded_cpu_bytes = texture_slot_bytes(base, mip_chain);
    let gpu_bytes_estimate = gpu_bytes_estimate_rgba8(*width, *height, mips, mip_chain);
    let mip_count = if mips {
        mip_chain
            .map(|chain| chain.len().max(1))
            .unwrap_or_else(|| crate::gltf_helpers::mip_level_count(*width, *height) as usize)
    } else {
        1
    };
    rows.push(TextureAuditRow {
        key,
        material_name: env_prim.material_name.clone(),
        material_index: env_prim.material_index,
        source_label,
        source_format,
        usage,
        width: *width,
        height: *height,
        mip_count,
        decoded_cpu_bytes,
        gpu_bytes_estimate,
    });
}

pub fn log_room_texture_audit(
    room: &'static str,
    primitives: &[RoomEnvPrimitiveCpu],
) -> RoomTextureAuditSummary {
    let mut rows = Vec::new();
    for env_prim in primitives {
        collect_texture_audit_row(
            &mut rows,
            env_prim,
            RoomTextureUsageClass::BaseColorSrgb,
            env_prim.mesh.albedo_rgba.as_deref(),
            env_prim.mesh.albedo_mip_chain.as_deref().map(|c| c.as_slice()),
        );
        collect_texture_audit_row(
            &mut rows,
            env_prim,
            RoomTextureUsageClass::NormalLinear,
            env_prim.mesh.normal_rgba.as_deref(),
            env_prim.mesh.normal_mip_chain.as_deref().map(|c| c.as_slice()),
        );
        collect_texture_audit_row(
            &mut rows,
            env_prim,
            RoomTextureUsageClass::MetallicRoughnessLinear,
            env_prim.mesh.metallic_roughness_rgba.as_deref(),
            env_prim
                .mesh
                .metallic_roughness_mip_chain
                .as_deref()
                .map(|c| c.as_slice()),
        );
        collect_texture_audit_row(
            &mut rows,
            env_prim,
            RoomTextureUsageClass::EmissiveSrgb,
            env_prim.mesh.emissive_rgba.as_deref(),
            env_prim.mesh.emissive_mip_chain.as_deref().map(|c| c.as_slice()),
        );
    }

    let mut key_to_refs: FxHashMap<TextureAuditDedupeKey, u32> = FxHashMap::default();
    let mut key_to_gpu_bytes: FxHashMap<TextureAuditDedupeKey, u64> = FxHashMap::default();
    let mut unique_sources: FxHashMap<String, ()> = FxHashMap::default();
    let mut total_cpu = 0u64;
    let mut total_gpu_refs = 0u64;
    for row in &rows {
        *key_to_refs.entry(row.key.clone()).or_insert(0) += 1;
        key_to_gpu_bytes
            .entry(row.key.clone())
            .or_insert(row.gpu_bytes_estimate);
        unique_sources.entry(row.source_label.clone()).or_insert(());
        total_cpu = total_cpu.saturating_add(row.decoded_cpu_bytes);
        total_gpu_refs = total_gpu_refs.saturating_add(row.gpu_bytes_estimate);
    }
    let deduped_refs = key_to_refs
        .values()
        .map(|&r| r.saturating_sub(1))
        .sum::<u32>();
    let total_gpu_unique = key_to_gpu_bytes.values().copied().sum::<u64>();
    let summary = RoomTextureAuditSummary {
        material_texture_refs: rows.len() as u32,
        unique_image_sources: unique_sources.len() as u32,
        unique_dedupe_keys: key_to_refs.len() as u32,
        deduped_refs,
        total_decoded_cpu_bytes: total_cpu,
        total_gpu_bytes_estimate_bytes: total_gpu_unique,
        total_gpu_bytes_reference_bytes: total_gpu_refs,
    };
    if let Ok(mut map) = last_texture_audits().lock() {
        map.insert(room, summary);
    }
    if !enabled() {
        return summary;
    }

    log::info!("room texture audit: {room}");
    log::info!(
        "  material texture refs: {}",
        summary.material_texture_refs
    );
    log::info!("  unique image sources: {}", summary.unique_image_sources);
    log::info!("  unique dedupe keys: {}", summary.unique_dedupe_keys);
    log::info!("  deduped refs: {}", summary.deduped_refs);
    log::info!(
        "  total decoded CPU bytes: {:.2} MiB",
        summary.total_decoded_cpu_bytes as f64 / (1024.0 * 1024.0)
    );
    log::info!(
        "  total GPU bytes estimate: {:.2} MiB (deduped), {:.2} MiB (raw refs)",
        summary.total_gpu_bytes_estimate_bytes as f64 / (1024.0 * 1024.0),
        summary.total_gpu_bytes_reference_bytes as f64 / (1024.0 * 1024.0),
    );
    log::info!("  top 20 textures by GPU bytes:");

    let mut ranked = rows;
    ranked.sort_by(|a, b| {
        b.gpu_bytes_estimate
            .cmp(&a.gpu_bytes_estimate)
            .then_with(|| b.decoded_cpu_bytes.cmp(&a.decoded_cpu_bytes))
    });
    for (idx, row) in ranked.into_iter().take(20).enumerate() {
        let refs = key_to_refs.get(&row.key).copied().unwrap_or(1);
        let deduped = refs > 1;
        let mat = row.material_name.as_deref().unwrap_or("-");
        let mat_idx = row
            .material_index
            .map(|i| i.to_string())
            .unwrap_or_else(|| "-".to_string());
        log::info!(
            "    {:02}. mat={}#{} slot={} src={} key=0x{:016x} {}x{} mips={} src_fmt={} gpu_fmt={:?} decoded={:.2} MiB gpu={:.2} MiB refs={} deduped={}",
            idx + 1,
            mat,
            mat_idx,
            row.usage.label(),
            row.source_label,
            dedupe_key_fingerprint(&row.key),
            row.width,
            row.height,
            row.mip_count,
            row.source_format,
            row.usage.gpu_format(),
            row.decoded_cpu_bytes as f64 / (1024.0 * 1024.0),
            row.gpu_bytes_estimate as f64 / (1024.0 * 1024.0),
            refs,
            deduped,
        );
    }
    summary
}

pub fn log_room_residency_after_upload(
    room: &'static str,
    phase: RuntimePhase,
    packed_asset_bytes_read: u64,
    decoded_cpu_payload_bytes: u64,
    staging_upload_bytes: u64,
    gpu_resident_estimate_bytes: u64,
    decoded_cpu_payload_retained_bytes: u64,
    staging_retained_bytes: u64,
    raw_source_retained_bytes: u64,
    device: &wgpu::Device,
) {
    if !enabled() {
        return;
    }
    let report = device.generate_allocator_report();
    let (allocated, reserved) = report
        .as_ref()
        .map(|r| (Some(r.total_allocated_bytes), Some(r.total_reserved_bytes)))
        .unwrap_or((None, None));
    log::info!(
        "room residency after upload: {room} (phase={})",
        phase.label()
    );
    log::info!(
        "  packed_asset_bytes_read={:.2} MiB decoded_cpu_payload_bytes={:.2} MiB staging_upload_bytes={:.2} MiB gpu_resident_estimate_bytes={:.2} MiB",
        packed_asset_bytes_read as f64 / (1024.0 * 1024.0),
        decoded_cpu_payload_bytes as f64 / (1024.0 * 1024.0),
        staging_upload_bytes as f64 / (1024.0 * 1024.0),
        gpu_resident_estimate_bytes as f64 / (1024.0 * 1024.0),
    );
    match (allocated, reserved) {
        (Some(a), Some(r)) => log::info!(
            "  allocator_allocated_bytes={:.2} MiB allocator_reserved_bytes={:.2} MiB",
            a as f64 / (1024.0 * 1024.0),
            r as f64 / (1024.0 * 1024.0),
        ),
        _ => log::info!(
            "  allocator_allocated_bytes=unavailable allocator_reserved_bytes=unavailable"
        ),
    }
    log::info!(
        "  cpu decoded payload retained: {:.2} MiB staging retained: {:.2} MiB raw source retained: {:.2} MiB",
        decoded_cpu_payload_retained_bytes as f64 / (1024.0 * 1024.0),
        staging_retained_bytes as f64 / (1024.0 * 1024.0),
        raw_source_retained_bytes as f64 / (1024.0 * 1024.0),
    );
}
