//! [`gameplay.glb`](../../../assets/3d/gameplay.glb) — authored gameplay table room.
//!
//! Spawn empties (Blender object names → glTF nodes):
//! - `hand_tiles_left` / `hand_tiles_right` — hand rack extent
//! - `structure_tiles_left` / `structure_tiles_right` — open-meld showcase strip
//! - `yaku_tablets_left` / `yaku_tablets_right` — yaku bone tablets
//! - `tile_plinth` / `.001` / `.002` — wall-tile plinths (dora / round wind / boss)
//! - `discard_river` / `play_mirror` — discard bowl + play mirror pick anchors
//! - `player_gold` — coin pile on dish
//! - `player_discard_tally` / `player_play_tally` — tally-stick fans
//! - `player_relic` … `player_relic.004` — five relic slots
//! - `player_consumables` / `.001` — talisman + ribbon dishes
//! - `player_yaku_journal` — journal book anchor
//!
//! Static geometry (table, dishes, candles, score plaque, cash-in control, …) draws from the
//! environment mesh; dynamic props still spawn at the markers above.
//!
//! Export **without Draco**. Decodes through [`crate::render::room_env_gltf`].

use std::sync::RwLock;

use glam::Vec3;

use crate::render::draw_cmd::CameraParams;
use crate::render::flame_volume::{FlameEmitter, shop_gltf_flame_emitter_scale, shop_gltf_wick_from_light};
use crate::render::room_env_gltf::{RoomEnvWalkHooks, RoomMeshPolicy};
use crate::render::room_glb::{self, RoomEnvLightingTune, RoomGlbCpu, load_room_glb_from_bytes};
use crate::render::wgpu_renderer::{PointLight, SpotLight};
use crate::render::world_space::surface_anchor_from_world_xyz;

pub const HAND_TILES_LEFT: &str = "hand_tiles_left";
pub const HAND_TILES_RIGHT: &str = "hand_tiles_right";
pub const STRUCTURE_TILES_LEFT: &str = "structure_tiles_left";
pub const STRUCTURE_TILES_RIGHT: &str = "structure_tiles_right";
pub const YAKU_TABLETS_LEFT: &str = "yaku_tablets_left";
pub const YAKU_TABLETS_RIGHT: &str = "yaku_tablets_right";
pub const DISCARD_RIVER: &str = "discard_river";
pub const PLAY_MIRROR: &str = "play_mirror";
pub const PLAYER_GOLD: &str = "player_gold";
pub const PLAYER_DISCARD_TALLY: &str = "player_discard_tally";
pub const PLAYER_PLAY_TALLY: &str = "player_play_tally";
pub const PLAYER_YAKU_JOURNAL: &str = "player_yaku_journal";
pub const BTN_CASH_IN: &str = "btn_cash_in";
pub const GAMEPLAY_CAMERA_NODE: &str = "Camera";

pub const PLAYER_RELIC_MARKERS: [&str; 5] = [
    "player_relic",
    "player_relic.001",
    "player_relic.002",
    "player_relic.003",
    "player_relic.004",
];

pub const PLAYER_CONSUMABLE_MARKERS: [&str; 2] = [
    "player_consumables",
    "player_consumables.001",
];

pub const TILE_PLINTH_MARKERS: [&str; 3] = ["tile_plinth", "tile_plinth.001", "tile_plinth.002"];

/// Linear HDR exposure multiplier when embedded punctual lights are active.
pub const GAMEPLAY_ENV_LINEAR_EXPOSURE_MUL: f32 = 1.0;
pub const GAMEPLAY_ENV_AMBIENT_SCALE_MIN: f32 = 0.06;

enum GameplayGlbCache {
    Uninit,
    Ready(Option<Box<RoomGlbCpu>>),
}

static GAMEPLAY_GLB_CPU: RwLock<GameplayGlbCache> = RwLock::new(GameplayGlbCache::Uninit);

fn ensure_gameplay_glb_loaded() {
    let mut w = GAMEPLAY_GLB_CPU.write().unwrap_or_else(|e| e.into_inner());
    if !matches!(*w, GameplayGlbCache::Uninit) {
        return;
    }
    let ready = if let Some(file) = crate::asset_path::get("3d/gameplay.glb") {
        match load_gameplay_glb_from_bytes(&file.data) {
            Ok(cpu) => {
                log::debug!(
                    "gameplay.glb: {} marker(s), {} draw primitive(s)",
                    cpu.markers.len(),
                    cpu.environment_primitives.len(),
                );
                Some(cpu)
            }
            Err(e) => {
                log::error!("gameplay.glb failed to load: {e:#}");
                None
            }
        }
    } else {
        log::debug!("gameplay.glb not embedded — gameplay uses procedural table");
        None
    };
    *w = GameplayGlbCache::Ready(ready.map(Box::new));
}

/// `true` when `gameplay.glb` loaded and has drawable environment geometry.
pub fn gameplay_room_draw_ready() -> bool {
    with_gameplay_glb_cpu(|opt| opt.is_some_and(|c| !c.environment_primitives.is_empty()))
}

pub fn with_gameplay_glb_cpu<R>(f: impl FnOnce(Option<&RoomGlbCpu>) -> R) -> R {
    ensure_gameplay_glb_loaded();
    let g = GAMEPLAY_GLB_CPU.read().unwrap_or_else(|e| e.into_inner());
    match &*g {
        GameplayGlbCache::Ready(Some(cpu)) => f(Some(cpu)),
        GameplayGlbCache::Ready(None) => f(None),
        GameplayGlbCache::Uninit => {
            log::warn!("gameplay.glb cache still Uninit after ensure — treating as absent");
            f(None)
        }
    }
}

pub fn release_gameplay_environment_cpu_sources_after_gpu_upload() {
    let mut g = GAMEPLAY_GLB_CPU.write().unwrap_or_else(|e| e.into_inner());
    if let GameplayGlbCache::Ready(Some(cpu)) = &mut *g {
        room_glb::release_room_environment_primitives_cpu(cpu);
    }
}

#[inline]
fn is_gameplay_spawn_marker(name: &str) -> bool {
    matches!(
        name,
        HAND_TILES_LEFT
            | HAND_TILES_RIGHT
            | STRUCTURE_TILES_LEFT
            | STRUCTURE_TILES_RIGHT
            | YAKU_TABLETS_LEFT
            | YAKU_TABLETS_RIGHT
            | DISCARD_RIVER
            | PLAY_MIRROR
            | PLAYER_GOLD
            | PLAYER_DISCARD_TALLY
            | PLAYER_PLAY_TALLY
            | PLAYER_YAKU_JOURNAL
            | BTN_CASH_IN
    ) || PLAYER_RELIC_MARKERS.contains(&name)
        || PLAYER_CONSUMABLE_MARKERS.contains(&name)
        || TILE_PLINTH_MARKERS.contains(&name)
}

#[derive(Copy, Clone)]
struct GameplayRoomWalkHooks;

impl RoomEnvWalkHooks for GameplayRoomWalkHooks {
    fn is_marker(&self, name: &str) -> bool {
        is_gameplay_spawn_marker(name)
    }

    fn mesh_policy(&self, name: &str) -> RoomMeshPolicy {
        if is_gameplay_spawn_marker(name) {
            RoomMeshPolicy::SkipDrawCollisionIfMarker
        } else if name == BTN_CASH_IN {
            RoomMeshPolicy::EnvironmentDrawWithCollision
        } else {
            RoomMeshPolicy::EnvironmentDraw
        }
    }

    fn log_asset_label(&self) -> &'static str {
        "gameplay.glb"
    }
}

pub fn load_gameplay_glb_from_bytes(data: &[u8]) -> anyhow::Result<RoomGlbCpu> {
    load_room_glb_from_bytes(
        data,
        "gltf::import_slice(gameplay.glb)",
        "gameplay.glb has no scenes",
        &GameplayRoomWalkHooks,
    )
}

/// World-space marker origin (centered room basis).
pub fn gameplay_marker_world(
    window_h: f32,
    env_height_scale: f32,
    cpu: &RoomGlbCpu,
    name: &str,
) -> Option<Vec3> {
    let t = room_glb::marker_translation(cpu, name)?;
    let s = room_glb::room_env_world_scale(window_h, env_height_scale);
    Some(t * s)
}

/// Packed [`crate::render::draw_cmd::WorldSurfaceAnchor`] for spawning dynamic props.
pub fn gameplay_marker_surface_anchor(
    window_w: f32,
    window_h: f32,
    env_height_scale: f32,
    cpu: &RoomGlbCpu,
    name: &str,
) -> Option<[f32; 3]> {
    let world = gameplay_marker_world(window_h, env_height_scale, cpu, name)?;
    Some(surface_anchor_from_world_xyz(window_w, window_h, world))
}

/// Screen center + minimum hit size for a spawn empty (cursor / focus).
pub fn gameplay_marker_screen_rect(
    win_w: f32,
    win_h: f32,
    cam: &CameraParams,
    env_height_scale: f32,
    cpu: &RoomGlbCpu,
    name: &str,
    min_rw: f32,
    min_rh: f32,
) -> Option<[f32; 4]> {
    let tw = gameplay_marker_world(win_h, env_height_scale, cpu, name)?;
    let (cx, cy) = cam.project_world_to_screen(win_w, win_h, tw);
    Some([cx - min_rw * 0.5, cy - min_rh * 0.5, min_rw, min_rh])
}

/// Embedded perspective camera (`Camera` node), scaled like the room mesh.
pub fn gameplay_camera_from_glb_if_present(
    window_h: f32,
    env_height_scale: f32,
) -> Option<CameraParams> {
    with_gameplay_glb_cpu(|opt| {
        let cpu = opt?;
        let center_doc = cpu
            .environment_bounds_doc
            .map(|b| b.center())
            .unwrap_or(Vec3::ZERO);
        cpu.embedded_cameras_by_name
            .get(GAMEPLAY_CAMERA_NODE)
            .copied()
            .or(cpu.embedded_perspective_camera)
            .map(|c| c.to_camera_params(window_h, env_height_scale, center_doc))
    })
}

pub fn gameplay_glb_has_embedded_lights() -> bool {
    with_gameplay_glb_cpu(|opt| {
        opt.is_some_and(crate::render::room_gltf_punctual::room_glb_has_embedded_lights)
    })
}

pub fn gameplay_embedded_point_lights_runtime(
    w: f32,
    h: f32,
    env_h: f32,
    tune: &RoomEnvLightingTune,
    flame_time_s: f32,
    lamp_flicker: f32,
) -> Vec<PointLight> {
    with_gameplay_glb_cpu(|opt| {
        opt.map(|cpu| {
            crate::render::room_gltf_punctual::embedded_point_lights_runtime(
                cpu,
                w,
                h,
                env_h,
                tune,
                crate::render::room_gltf_punctual::RoomPunctualProfile::ShopCandles {
                    flame_time_s,
                    lamp_flicker,
                },
                "gameplay.glb",
            )
        })
        .unwrap_or_default()
    })
}

pub fn gameplay_embedded_spot_lights_runtime(
    w: f32,
    h: f32,
    env_h: f32,
    tune: &RoomEnvLightingTune,
) -> Vec<SpotLight> {
    with_gameplay_glb_cpu(|opt| {
        opt.map(|cpu| {
            crate::render::room_gltf_punctual::embedded_spot_lights_runtime(
                cpu, w, h, env_h, tune, "gameplay.glb",
            )
        })
        .unwrap_or_default()
    })
}

/// Procedural flame particles at each `light_candle*` punctual in `gameplay.glb`.
pub fn gameplay_gltf_candle_flame_emitters(
    h: f32,
    env_h: f32,
    lamp_flicker: f32,
) -> Vec<FlameEmitter> {
    with_gameplay_glb_cpu(|opt| {
        let Some(cpu) = opt else {
            return Vec::new();
        };
        let s = room_glb::room_env_world_scale(h, env_h);
        let center_doc = cpu
            .environment_bounds_doc
            .map(|b| b.center())
            .unwrap_or(Vec3::ZERO);
        let flame_scale = shop_gltf_flame_emitter_scale(s);
        cpu.embedded_point_lights
            .iter()
            .filter(|l| l.is_candle)
            .enumerate()
            .map(|(i, l)| {
                let light_world = (l.pos_doc - center_doc) * s;
                let seed = (i as u32)
                    .wrapping_mul(0x9E37_79B9)
                    .wrapping_add(0xC01A_C0FF);
                let scale_jitter = 0.88 + (seed & 0x3f) as f32 / 63.0 * 0.24;
                let emitter_scale = flame_scale * scale_jitter;
                let world = shop_gltf_wick_from_light(light_world, emitter_scale);
                let phase = (seed as f32 * 2.328_306e-10).fract();
                FlameEmitter {
                    wick_world: world,
                    scale: emitter_scale,
                    wind: glam::Vec2::ZERO,
                    brightness: lamp_flicker,
                    phase,
                    flicker_amp: crate::render::flame_volume::SHOP_CANDLE_FLICKER_AMP,
                }
            })
            .collect()
    })
}
