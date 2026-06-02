//! NDJSON shadow diagnostics for debug sessions (session log path in workspace).

use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use glam::{Mat4, Vec3};

const LOG_PATH: &str = "/Users/zelda/Documents/Mahjuro/.cursor/debug-793696.log";
const SESSION_ID: &str = "793696";
const LOG_INTERVAL: Duration = Duration::from_secs(2);

static THROTTLE: Mutex<Option<Instant>> = Mutex::new(None);

pub(crate) fn agent_shadow_log(
    hypothesis_id: &str,
    location: &str,
    message: &str,
    data: serde_json::Value,
) {
    let mut guard = THROTTLE.lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    if guard
        .map(|last| now.duration_since(last) < LOG_INTERVAL)
        .unwrap_or(false)
    {
        return;
    }
    *guard = Some(now);
    drop(guard);

    let payload = serde_json::json!({
        "sessionId": SESSION_ID,
        "hypothesisId": hypothesis_id,
        "location": location,
        "message": message,
        "data": data,
        "timestamp": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0),
    });
    if let Ok(line) = serde_json::to_string(&payload) {
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(LOG_PATH) {
            let _ = writeln!(f, "{line}");
        }
    }
}

pub(crate) fn probe_baked_ao_at_world(
    lvp_cols: [f32; 16],
    ao_bytes: &[u8],
    width: u32,
    height: u32,
    world: Vec3,
) -> Option<(Vec3, [f32; 2], u8)> {
    probe_baked_ao_at_world_scaled(lvp_cols, None, ao_bytes, width, height, world, 1.0)
        .map(|p| (p.ndc, p.uv, p.ao))
}

pub(crate) struct BakedAoProbe {
    pub ndc: Vec3,
    pub uv: [f32; 2],
    pub ao: u8,
    pub baked_depth: Option<f32>,
    pub depth_delta: Option<f32>,
    pub ao_would_apply: bool,
}

pub(crate) fn probe_baked_ao_at_world_scaled(
    lvp_cols: [f32; 16],
    depth_bytes: Option<&[u8]>,
    ao_bytes: &[u8],
    width: u32,
    height: u32,
    world: Vec3,
    world_scale: f32,
) -> Option<BakedAoProbe> {
    let lvp = Mat4::from_cols_array(&lvp_cols);
    let scaled = world * world_scale;
    let clip = lvp * scaled.extend(1.0);
    if clip.w.abs() < 1e-8 {
        return None;
    }
    let ndc_v = clip.truncate() / clip.w;
    if ndc_v.z < 0.0 || ndc_v.z > 1.0 {
        return None;
    }
    let uv = [ndc_v.x * 0.5 + 0.5, ndc_v.y * -0.5 + 0.5];
    if uv[0] < 0.0 || uv[0] > 1.0 || uv[1] < 0.0 || uv[1] > 1.0 {
        return None;
    }
    let w = width as usize;
    let h = height as usize;
    let x = ((uv[0] * (w as f32 - 1.0)).round() as usize).min(w - 1);
    let y = ((uv[1] * (h as f32 - 1.0)).round() as usize).min(h - 1);
    let ao = *ao_bytes.get(y * w + x)?;
    let baked_depth = depth_bytes.and_then(|bytes| {
        let i = y * w + x;
        let chunk = bytes.get(i * 4..i * 4 + 4)?;
        Some(f32::from_le_bytes(chunk.try_into().ok()?))
    });
    let depth_delta = baked_depth.map(|d| (d - ndc_v.z).abs());
    let eps = crate::room_shadow_bake::CONTACT_AO_DEPTH_COHERENCE_EPS;
    let ao_would_apply = depth_delta.is_none_or(|d| d <= eps) && ao < 250;
    Some(BakedAoProbe {
        ndc: ndc_v,
        uv,
        ao,
        baked_depth,
        depth_delta,
        ao_would_apply,
    })
}

/// Shadow-caster axes for Z-up verification (world +Z should be view-up when possible).
pub(crate) fn shadow_caster_z_up_probe(light_world: Vec3, look_at: Vec3) -> serde_json::Value {
    use crate::projected_light_shadow::z_up_shadow_view_up;
    let forward = (look_at - light_world).normalize_or_zero();
    let view_up = z_up_shadow_view_up(forward);
    serde_json::json!({
        "light_world": light_world.to_array(),
        "look_at": look_at.to_array(),
        "forward": forward.to_array(),
        "view_up": view_up.to_array(),
        "uses_world_z_as_up": view_up.z.abs() > 0.99,
    })
}
