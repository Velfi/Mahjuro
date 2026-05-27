//! [`gameplay.glb`](../../../assets/3d/gameplay.glb) — authored gameplay table room.
//!
//! Spawn empties (Blender object names → glTF nodes):
//! - `hand_tiles_left` / `hand_tiles_right` — hand rack extent
//! - `structure_tiles_left` / `structure_tiles_right` — structure tile strip bounds
//! - `yaku_tablets_left` / `yaku_tablets_right` — yaku bone tablet row bounds
//! - `tile_plinth` / `.001` / `.002` — dora / round-wind / boss indicator tiles
//! - `discard_river` / `play_mirror` — procedural discard bowl + play mirror at marker empties
//! - `player_gold` — coin pile spawn
//! - `player_discard_tally` / `player_play_tally` — tally-stick fan spawns
//! - `btn_cash_in` / `label_cash_in` — authored cash-in control (env mesh + label)
//! - `player_relic` … `player_relic.004` — relic medallion slots
//! - `player_consumables` / `.001` — consumable dish slots
//! - `player_yaku_journal` — journal book anchor (opens yaku journal on click)
//! - `default` — embedded glTF perspective camera (not a spawn empty)
//!
//! Static geometry (table, dishes, candles, score plaque, …) draws from the environment mesh.
//! Dynamic props spawn at the markers above. Missing required empties fail load — no procedural fallback.
//!
//! Export **without Draco**. Decodes through [`crate::room_env_gltf`].

use std::sync::RwLock;

use glam::{EulerRot, Mat4, Quat, Vec2, Vec3};

use crate::draw_cmd::{CameraParams, Object3d, Object3dKind};
use crate::flame_volume::{
    FlameEmitter, shop_gltf_flame_emitter_scale, shop_gltf_wick_from_light,
};
use crate::mirror_mesh::{MIRROR_LOCAL_CENTER_Y, MIRROR_LOCAL_HALF};
use crate::river_mesh::{RIVER_LOCAL_CENTER_Y, RIVER_LOCAL_HALF};
use crate::room_env_gltf::{RoomEnvWalkHooks, RoomMeshPolicy};
use crate::room_glb::{self, RoomEnvLightingTune, RoomGlbCpu, load_room_glb_from_bytes};
use crate::table_transform::{
    compose_rotation_euler, mat4_to_euler_xyz_rad, rot_euler_xyz_rad, translate_rot_scale,
};
use crate::wgpu_renderer::{PointLight, SpotLight};
use crate::world_space::{pixel_to_world, surface_anchor_from_world_xyz};

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
/// Authored cash-in button mesh in `gameplay.glb` (draw + pick collision).
pub const BTN_CASH_IN: &str = "btn_cash_in";
/// Engraved label mesh parented to the cash-in control.
pub const LABEL_CASH_IN: &str = "label_cash_in";
pub const GAMEPLAY_CAMERA_NODE: &str = "default";
const GAMEPLAY_ACTION_PICK_SHRINK_MUL: f32 = 0.125;
const GAMEPLAY_DISCARD_RIVER_SIZE_MUL: f32 = 1.5;

/// Per-room linear HDR multiplier for `gameplay.glb` **and** showcase tiles when
/// [`DrawCmd::GameplayEnvironment`] is active — same path as hallway/archive in
/// [`crate::wgpu_renderer::runtime::camera::WgpuRenderer::tile_hdr_tonemap`].
pub const GAMEPLAY_ENV_LINEAR_EXPOSURE_MUL: f32 = 1.0;

pub const PLAYER_RELIC_MARKERS: [&str; 5] = [
    "player_relic",
    "player_relic.001",
    "player_relic.002",
    "player_relic.003",
    "player_relic.004",
];

pub const PLAYER_CONSUMABLE_MARKERS: [&str; 2] = ["player_consumables", "player_consumables.001"];

pub const TILE_PLINTH_MARKERS: [&str; 3] = ["tile_plinth", "tile_plinth.001", "tile_plinth.002"];

/// Every spawn empty that must exist in a shippable `gameplay.glb`.
pub const REQUIRED_GAMEPLAY_MARKER_NODES: &[&str] = &[
    HAND_TILES_LEFT,
    HAND_TILES_RIGHT,
    STRUCTURE_TILES_LEFT,
    STRUCTURE_TILES_RIGHT,
    YAKU_TABLETS_LEFT,
    YAKU_TABLETS_RIGHT,
    DISCARD_RIVER,
    PLAY_MIRROR,
    PLAYER_GOLD,
    PLAYER_DISCARD_TALLY,
    PLAYER_PLAY_TALLY,
    PLAYER_YAKU_JOURNAL,
];

enum GameplayGlbCache {
    Uninit,
    Missing,
    Invalid(String),
    Ready(Box<RoomGlbCpu>),
}

static GAMEPLAY_GLB_CPU: RwLock<GameplayGlbCache> = RwLock::new(GameplayGlbCache::Uninit);

/// Whether [`gameplay.glb`] is absent, invalid, or ready for the authored table path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GameplayGlbLoadState {
    Missing,
    Invalid(String),
    Ready,
}

pub fn gameplay_glb_load_state() -> GameplayGlbLoadState {
    ensure_gameplay_glb_loaded();
    match &*GAMEPLAY_GLB_CPU.read().unwrap_or_else(|e| e.into_inner()) {
        GameplayGlbCache::Uninit => GameplayGlbLoadState::Missing,
        GameplayGlbCache::Missing => GameplayGlbLoadState::Missing,
        GameplayGlbCache::Invalid(msg) => GameplayGlbLoadState::Invalid(msg.clone()),
        GameplayGlbCache::Ready(_) => GameplayGlbLoadState::Ready,
    }
}

fn ensure_gameplay_glb_loaded() {
    let mut w = GAMEPLAY_GLB_CPU.write().unwrap_or_else(|e| e.into_inner());
    match &*w {
        GameplayGlbCache::Uninit => {}
        GameplayGlbCache::Ready(cpu) if room_glb::room_glb_cpu_needs_environment_mesh_reload(cpu) => {
            *w = GameplayGlbCache::Uninit;
        }
        _ if !matches!(*w, GameplayGlbCache::Uninit) => return,
        _ => {}
    }
    *w = if let Some(file) = mahjuro_assets::asset_path::get("3d/gameplay.glb") {
        match load_gameplay_glb_from_bytes(&file.data) {
            Ok(cpu) => match validate_gameplay_glb(cpu) {
                Ok(cpu) => GameplayGlbCache::Ready(cpu),
                Err(e) => {
                    log::error!("gameplay.glb rejected: {e:#}");
                    GameplayGlbCache::Invalid(e.to_string())
                }
            },
            Err(e) => {
                log::error!("gameplay.glb failed to decode: {e:#}");
                GameplayGlbCache::Invalid(e.to_string())
            }
        }
    } else {
        log::error!("gameplay.glb not embedded");
        GameplayGlbCache::Missing
    };
}

pub fn validate_gameplay_glb(cpu: RoomGlbCpu) -> anyhow::Result<Box<RoomGlbCpu>> {
    if cpu.environment_primitives.is_empty() {
        anyhow::bail!("gameplay.glb has no drawable environment geometry");
    }
    for name in REQUIRED_GAMEPLAY_MARKER_NODES {
        if !cpu.markers.contains_key(*name) {
            anyhow::bail!("gameplay.glb missing required empty `{name}`");
        }
    }
    for name in PLAYER_RELIC_MARKERS {
        if !cpu.markers.contains_key(name) {
            anyhow::bail!("gameplay.glb missing required empty `{name}`");
        }
    }
    for name in PLAYER_CONSUMABLE_MARKERS {
        if !cpu.markers.contains_key(name) {
            anyhow::bail!("gameplay.glb missing required empty `{name}`");
        }
    }
    for name in TILE_PLINTH_MARKERS {
        if !cpu.markers.contains_key(name) {
            anyhow::bail!("gameplay.glb missing required empty `{name}`");
        }
    }
    if !cpu.marker_mesh_bounds_doc.contains_key(BTN_CASH_IN) {
        anyhow::bail!("gameplay.glb missing authored cash-in mesh `{BTN_CASH_IN}`");
    }
    if gameplay_embedded_camera_doc(&cpu).is_none() {
        anyhow::bail!(
            "gameplay.glb missing glTF perspective camera node `{GAMEPLAY_CAMERA_NODE}` (assign a Camera object in Blender, export as glTF camera)"
        );
    }
    for prim in &cpu.environment_primitives {
        if let Some(name) = prim.gltf_node_name.as_deref()
            && is_gameplay_unexportable_mesh(name)
        {
            anyhow::bail!(
                "gameplay.glb exports Unexportables mesh `{name}` — disable the Unexportables collection for glTF export"
            );
        }
    }
    log::debug!(
        "gameplay.glb: {} marker(s), {} draw primitive(s)",
        cpu.markers.len(),
        cpu.environment_primitives.len(),
    );
    Ok(Box::new(cpu))
}

pub fn with_gameplay_glb_cpu<R>(f: impl FnOnce(Option<&RoomGlbCpu>) -> R) -> R {
    ensure_gameplay_glb_loaded();
    let g = GAMEPLAY_GLB_CPU.read().unwrap_or_else(|e| e.into_inner());
    match &*g {
        GameplayGlbCache::Ready(cpu) => f(Some(cpu)),
        GameplayGlbCache::Missing | GameplayGlbCache::Invalid(_) => f(None),
        GameplayGlbCache::Uninit => {
            log::warn!("gameplay.glb cache still Uninit after ensure — treating as absent");
            f(None)
        }
    }
}

pub fn release_gameplay_environment_cpu_sources_after_gpu_upload() {
    let mut g = GAMEPLAY_GLB_CPU.write().unwrap_or_else(|e| e.into_inner());
    if let GameplayGlbCache::Ready(cpu) = &mut *g {
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
    ) || PLAYER_RELIC_MARKERS.contains(&name)
        || PLAYER_CONSUMABLE_MARKERS.contains(&name)
        || TILE_PLINTH_MARKERS.contains(&name)
}

/// Blender **Unexportables** collection: layout-preview meshes that must not be in the
/// glTF export (tiles, relics, score plaque parts, action props, …).
#[inline]
fn is_gameplay_unexportable_mesh(name: &str) -> bool {
    name.starts_with("score_counter")
        || name.starts_with("hand_tile")
        || name.starts_with("Relic_")
        || name.starts_with("Ribbon_")
        || name.starts_with("Talisman_")
        || name.starts_with("YakuTablet_")
        || name.starts_with("TallyStick")
        || name.starts_with("Book")
        || name.starts_with("Bowl_")
        || name == "gold coins"
        || matches!(name, "Cash In" | "Text" | "player_relic.005")
}

#[inline]
fn is_gameplay_env_button_node(name: &str) -> bool {
    name == BTN_CASH_IN
}

/// Static env meshes that should not cast into the punctual shadow atlas (or the
/// shared key-light map). The table is a receiver; candles are light sources;
/// flat dishes and engraved UI read as harsh rectangular pools.
#[inline]
pub fn gameplay_prim_casts_room_shadow(node_name: Option<&str>) -> bool {
    match node_name {
        None => true,
        Some("table") | Some("candles") | Some("candle_wicks") => false,
        Some("consumables_dish") | Some("gold_dish") => false,
        Some(n) if n.starts_with("label_") || n.starts_with("btn_") => false,
        Some(_) => true,
    }
}

#[derive(Copy, Clone)]
struct GameplayRoomWalkHooks;

impl RoomEnvWalkHooks for GameplayRoomWalkHooks {
    fn is_marker(&self, name: &str) -> bool {
        is_gameplay_spawn_marker(name)
    }

    fn forbid_env_mesh(&self, name: &str) -> bool {
        is_gameplay_unexportable_mesh(name)
    }

    fn mesh_policy(&self, name: &str) -> RoomMeshPolicy {
        if is_gameplay_spawn_marker(name) {
            RoomMeshPolicy::SkipDrawCollisionIfMarker
        } else if is_gameplay_env_button_node(name) {
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

pub fn require_gameplay_marker_world(
    window_h: f32,
    env_height_scale: f32,
    cpu: &RoomGlbCpu,
    name: &str,
) -> anyhow::Result<Vec3> {
    gameplay_marker_world(window_h, env_height_scale, cpu, name)
        .ok_or_else(|| anyhow::anyhow!("gameplay.glb missing required empty `{name}` (runtime)"))
}

/// Position, rotation, and scale for a spawn empty in one resolve.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GameplayMarkerPose {
    /// Packed surface anchor `(pixel_x, pixel_y, lift)`.
    pub anchor: [f32; 3],
    /// Euler XYZ radians in world space (same as [`Object3d::rotation`]).
    pub rotation_rad: [f32; 3],
    /// Non-uniform scale from the glTF node (`room * node` decomposition).
    pub scale: [f32; 3],
}

impl GameplayMarkerPose {
    pub const UNIT_SCALE: [f32; 3] = [1.0, 1.0, 1.0];

    #[inline]
    pub fn rotation_deg(&self) -> [f32; 3] {
        [
            self.rotation_rad[0].to_degrees(),
            self.rotation_rad[1].to_degrees(),
            self.rotation_rad[2].to_degrees(),
        ]
    }

    /// Uniform scale for sizing props (geometric mean of the three axes).
    #[inline]
    pub fn uniform_scale(&self) -> f32 {
        let [x, y, z] = self.scale;
        (x * y * z).cbrt()
    }

    /// Dimensionless Blender-empty scale for pixel-footprint sizing.
    ///
    /// [`Self::scale`] is in world units because procedural meshes use it for
    /// extents. Showcase tile `size_px` is already in renderer world/pixel
    /// units, so it should only honor the empty's authored scale, not the
    /// room's window-height scale a second time.
    #[inline]
    pub fn uniform_author_scale(&self, window_h: f32, env_height_scale: f32) -> f32 {
        let room_scale = room_glb::room_env_world_scale(window_h, env_height_scale).max(1e-8);
        self.uniform_scale() / room_scale
    }
}

/// Lerp packed surface anchors between two marker empties.
#[inline]
pub fn lerp_marker_anchor(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + t * (b[0] - a[0]),
        a[1] + t * (b[1] - a[1]),
        a[2] + t * (b[2] - a[2]),
    ]
}

/// Screen-space distance between two marker anchors (for strip sizing).
#[inline]
pub fn marker_pair_span_px(a_l: [f32; 3], a_r: [f32; 3]) -> f32 {
    ((a_r[0] - a_l[0]).powi(2) + (a_r[1] - a_l[1]).powi(2))
        .sqrt()
        .max(8.0)
}

/// Axis-aligned screen rect spanning two marker anchors (focus rects / tooltips).
pub fn marker_pair_screen_rect_from_poses(
    left: &GameplayMarkerPose,
    right: &GameplayMarkerPose,
    slot_h: f32,
) -> (f32, f32, f32, f32) {
    let left_x = left.anchor[0].min(right.anchor[0]);
    let cy = (left.anchor[1] + right.anchor[1]) * 0.5;
    let w = marker_pair_span_px(left.anchor, right.anchor);
    (left_x, cy - slot_h * 0.5, w, slot_h)
}

/// Slerp two marker euler triples (XYZ radians).
pub fn lerp_marker_rotation_rad(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let qa = Quat::from_euler(EulerRot::XYZ, a[0], a[1], a[2]);
    let qb = Quat::from_euler(EulerRot::XYZ, b[0], b[1], b[2]);
    mat4_to_euler_xyz_rad(Mat4::from_quat(qa.slerp(qb, t.clamp(0.0, 1.0))))
}

/// Offset a packed surface anchor along the marker's local **+X** in screen pixels.
pub fn showcase_anchor_spread_px(
    w: f32,
    h: f32,
    cam: &CameraParams,
    anchor: [f32; 3],
    rotation_rad: [f32; 3],
    offset_px: f32,
) -> [f32; 3] {
    use crate::table_transform::rot_euler_xyz_rad;
    use crate::world_space::layout_anchor_to_world;

    let world = layout_anchor_to_world(w, h, Some(cam), anchor[0], anchor[1], anchor[2], true);
    let rot = rot_euler_xyz_rad(rotation_rad[0], rotation_rad[1], rotation_rad[2]);
    let local_x = rot.transform_vector3(Vec3::X);
    let (px0, py0) = cam.project_world_to_screen(w, h, world);
    let (px1, py1) = cam.project_world_to_screen(w, h, world + local_x);
    let dir = Vec2::new(px1 - px0, py1 - py0);
    let len = dir.length();
    if len < 1e-4 {
        return [anchor[0] + offset_px, anchor[1], anchor[2]];
    }
    let dir = dir / len;
    [
        anchor[0] + dir.x * offset_px,
        anchor[1] + dir.y * offset_px,
        anchor[2],
    ]
}

/// Rotation (XYZ radians) and scale from a marker's world matrix.
pub fn gameplay_marker_rotation_scale(
    window_h: f32,
    env_height_scale: f32,
    cpu: &RoomGlbCpu,
    name: &str,
) -> anyhow::Result<([f32; 3], [f32; 3])> {
    let room = room_glb::room_env_model_matrix_from_cpu(window_h, env_height_scale, cpu);
    let node = cpu
        .marker_node_transform_doc(name)
        .ok_or_else(|| anyhow::anyhow!("gameplay.glb missing marker `{name}`"))?;
    let world_uniform = room_glb::room_env_world_scale(window_h, env_height_scale);
    let node_scale = Vec3::new(
        node.x_axis.truncate().length(),
        node.y_axis.truncate().length(),
        node.z_axis.truncate().length(),
    );
    let scale = [
        node_scale.x * world_uniform,
        node_scale.y * world_uniform,
        node_scale.z * world_uniform,
    ];
    let inv_node_scale = Mat4::from_scale(Vec3::new(
        1.0 / node_scale.x.max(1e-8),
        1.0 / node_scale.y.max(1e-8),
        1.0 / node_scale.z.max(1e-8),
    ));
    let (_, rot, _) = (room * node * inv_node_scale).to_scale_rotation_translation();
    Ok((mat4_to_euler_xyz_rad(Mat4::from_quat(rot)), scale))
}

/// Full spawn pose: surface anchor + rotation + scale.
pub fn resolve_gameplay_marker_pose(
    window_w: f32,
    window_h: f32,
    env_height_scale: f32,
    cpu: &RoomGlbCpu,
    name: &str,
) -> anyhow::Result<GameplayMarkerPose> {
    let anchor =
        require_gameplay_marker_surface_anchor(window_w, window_h, env_height_scale, cpu, name)?;
    let (rotation_rad, scale) =
        gameplay_marker_rotation_scale(window_h, env_height_scale, cpu, name)?;
    Ok(GameplayMarkerPose {
        anchor,
        rotation_rad,
        scale,
    })
}

pub fn require_gameplay_marker_pose(
    window_w: f32,
    window_h: f32,
    env_height_scale: f32,
    cpu: &RoomGlbCpu,
    name: &str,
) -> anyhow::Result<GameplayMarkerPose> {
    resolve_gameplay_marker_pose(window_w, window_h, env_height_scale, cpu, name)
}

/// Packed [`crate::draw_cmd::WorldSurfaceAnchor`] for spawning dynamic props.
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

pub fn require_gameplay_marker_surface_anchor(
    window_w: f32,
    window_h: f32,
    env_height_scale: f32,
    cpu: &RoomGlbCpu,
    name: &str,
) -> anyhow::Result<[f32; 3]> {
    gameplay_marker_surface_anchor(window_w, window_h, env_height_scale, cpu, name)
        .ok_or_else(|| anyhow::anyhow!("gameplay.glb missing required empty `{name}` (runtime)"))
}

#[inline]
fn scale_extents(extents: [f32; 3], scale: [f32; 3]) -> [f32; 3] {
    [
        extents[0] * scale[0],
        extents[1] * scale[1],
        extents[2] * scale[2],
    ]
}

#[inline]
fn scale_extents_uniform(extents: [f32; 3], mul: f32) -> [f32; 3] {
    [extents[0] * mul, extents[1] * mul, extents[2] * mul]
}

#[inline]
pub fn rotate_marker_pose_x_180(rotation_rad: [f32; 3]) -> [f32; 3] {
    compose_rotation_euler(
        rot_euler_xyz_rad(rotation_rad[0], rotation_rad[1], rotation_rad[2]),
        [180.0, 0.0, 0.0],
    )
}

/// Pick-ray proxy for the discard river — position/rotation from the `discard_river` empty.
pub fn gameplay_pick_discard_river(
    window_w: f32,
    window_h: f32,
    env_height_scale: f32,
    cpu: &RoomGlbCpu,
    _screen_rect: [f32; 4],
) -> anyhow::Result<Object3d> {
    let pose =
        require_gameplay_marker_pose(window_w, window_h, env_height_scale, cpu, DISCARD_RIVER)?;
    let extents = scale_extents_uniform(
        scale_extents(
            [
                RIVER_LOCAL_HALF[0] * 2.0,
                RIVER_LOCAL_HALF[1] * 2.0,
                RIVER_LOCAL_HALF[2] * 2.0,
            ],
            pose.scale,
        ),
        GAMEPLAY_ACTION_PICK_SHRINK_MUL * GAMEPLAY_DISCARD_RIVER_SIZE_MUL,
    );
    Ok(Object3d {
        pos: pose.anchor,
        extents,
        rotation: rotate_marker_pose_x_180(pose.rotation_rad),
        color: [1.0, 1.0, 1.0, 1.0],
        kind: Object3dKind::Bowl,
        hover_target: 0.0,
        anim_id: 1,
    })
}

/// Pick-ray proxy for the journal book — position/rotation from `player_yaku_journal`.
pub fn gameplay_pick_journal_book(
    window_w: f32,
    window_h: f32,
    env_height_scale: f32,
    cpu: &RoomGlbCpu,
    open_amount: f32,
) -> anyhow::Result<Object3d> {
    use crate::scene_glue::{BOOK_SPINE_THICKNESS_MM, book_cover_face_extents_xy};
    use mahjuro_types::shop_pick::YAKU_JOURNAL_BOOK_PICK_ID;

    let pose = require_gameplay_marker_pose(
        window_w,
        window_h,
        env_height_scale,
        cpu,
        PLAYER_YAKU_JOURNAL,
    )?;
    let (face_w, face_h) = book_cover_face_extents_xy(window_w, 1.0);
    let spine_mm = layout_mm(window_h, BOOK_SPINE_THICKNESS_MM);
    let face_h_safe = face_h.max(1e-6);
    let extents = scale_extents_uniform(
        scale_extents(
            [face_w / face_h_safe, spine_mm / face_h_safe, 1.0],
            pose.scale,
        ),
        GAMEPLAY_ACTION_PICK_SHRINK_MUL,
    );
    Ok(Object3d {
        pos: pose.anchor,
        extents,
        rotation: rotate_marker_pose_x_180(pose.rotation_rad),
        color: [1.0, 1.0, 1.0, 1.0],
        kind: Object3dKind::Book {
            spine_label: std::borrow::Cow::Borrowed("Journal"),
            pick_id: Some(YAKU_JOURNAL_BOOK_PICK_ID),
            open_amount,
        },
        hover_target: 0.0,
        anim_id: 0,
    })
}

#[inline]
fn layout_mm(window_h: f32, mm: f32) -> f32 {
    mm * (window_h / 2104.0)
}

/// Pick-ray proxy for the play mirror — position/rotation from the `play_mirror` empty.
pub fn gameplay_pick_play_mirror(
    window_w: f32,
    window_h: f32,
    env_height_scale: f32,
    cpu: &RoomGlbCpu,
    _screen_rect: [f32; 4],
) -> anyhow::Result<Object3d> {
    let pose =
        require_gameplay_marker_pose(window_w, window_h, env_height_scale, cpu, PLAY_MIRROR)?;
    let extents = scale_extents_uniform(
        scale_extents(
            [
                MIRROR_LOCAL_HALF[0] * 2.0,
                MIRROR_LOCAL_HALF[1] * 2.0,
                MIRROR_LOCAL_HALF[2] * 2.0,
            ],
            pose.scale,
        ),
        GAMEPLAY_ACTION_PICK_SHRINK_MUL,
    );
    Ok(Object3d {
        pos: pose.anchor,
        extents,
        rotation: rotate_marker_pose_x_180(pose.rotation_rad),
        color: [1.0, 1.0, 1.0, 1.0],
        kind: Object3dKind::Mirror,
        hover_target: 0.0,
        anim_id: 2,
    })
}

fn project_object3d_aabb_rect(
    win_w: f32,
    win_h: f32,
    cam: &CameraParams,
    obj: &Object3d,
    half: [f32; 3],
    center_y: f32,
    lift_half_y: bool,
) -> [f32; 4] {
    let center_lift = if lift_half_y {
        obj.pos[2] + obj.extents[1] * 0.5
    } else {
        obj.pos[2]
    };
    let center = pixel_to_world(win_w, win_h, obj.pos[0], obj.pos[1], center_lift);
    let model = translate_rot_scale(center, obj.rotation_matrix(), Vec3::from(obj.extents));
    project_model_aabb_rect(win_w, win_h, cam, model, half, center_y)
}

fn project_model_aabb_rect(
    win_w: f32,
    win_h: f32,
    cam: &CameraParams,
    model: Mat4,
    half: [f32; 3],
    center_y: f32,
) -> [f32; 4] {
    let corners = [
        Vec3::new(-half[0], center_y - half[1], -half[2]),
        Vec3::new(half[0], center_y - half[1], -half[2]),
        Vec3::new(-half[0], center_y + half[1], -half[2]),
        Vec3::new(half[0], center_y + half[1], -half[2]),
        Vec3::new(-half[0], center_y - half[1], half[2]),
        Vec3::new(half[0], center_y - half[1], half[2]),
        Vec3::new(-half[0], center_y + half[1], half[2]),
        Vec3::new(half[0], center_y + half[1], half[2]),
    ];
    let mut mn_x = f32::INFINITY;
    let mut mn_y = f32::INFINITY;
    let mut mx_x = f32::NEG_INFINITY;
    let mut mx_y = f32::NEG_INFINITY;
    for c in corners {
        let world = model.transform_point3(c);
        let (sx, sy) = cam.project_world_to_screen(win_w, win_h, world);
        mn_x = mn_x.min(sx);
        mn_y = mn_y.min(sy);
        mx_x = mx_x.max(sx);
        mx_y = mx_y.max(sy);
    }
    [mn_x, mn_y, mx_x - mn_x, mx_y - mn_y]
}

/// Project the spawned discard river model at its GLB marker pose.
pub fn gameplay_discard_river_model_screen_rect(
    win_w: f32,
    win_h: f32,
    cam: &CameraParams,
    obj: &Object3d,
) -> [f32; 4] {
    project_object3d_aabb_rect(
        win_w,
        win_h,
        cam,
        obj,
        RIVER_LOCAL_HALF,
        RIVER_LOCAL_CENTER_Y,
        true,
    )
}

/// Project the spawned play mirror model at its GLB marker pose.
pub fn gameplay_play_mirror_model_screen_rect(
    win_w: f32,
    win_h: f32,
    cam: &CameraParams,
    obj: &Object3d,
) -> [f32; 4] {
    project_object3d_aabb_rect(
        win_w,
        win_h,
        cam,
        obj,
        MIRROR_LOCAL_HALF,
        MIRROR_LOCAL_CENTER_Y,
        true,
    )
}

/// Project the yaku journal book pick proxy (unit-cube local bounds, center lifted on Y).
pub fn gameplay_journal_book_screen_rect(
    win_w: f32,
    win_h: f32,
    cam: &CameraParams,
    obj: &Object3d,
) -> [f32; 4] {
    project_object3d_aabb_rect(win_w, win_h, cam, obj, [0.5, 0.5, 0.5], 0.0, true)
}

/// Project a named marker's mesh AABB to screen pixels when the GLB node carries authoring
/// geometry; otherwise fall back to the empty origin with pixel `min_rw` × `min_rh`.
pub fn gameplay_marker_screen_rect_resolved(
    win_w: f32,
    win_h: f32,
    cam: &CameraParams,
    env_height_scale: f32,
    cpu: &RoomGlbCpu,
    name: &str,
    min_rw_px: f32,
    min_rh_px: f32,
) -> anyhow::Result<[f32; 4]> {
    use crate::room_glb::{MarkerScreenRectParams, screen_rect_for_marker_mesh_bounds};
    let params = MarkerScreenRectParams {
        win_w,
        win_h,
        cam,
        env_height_scale,
        cpu,
        node_name: name,
        min_rw: min_rw_px,
        min_rh: min_rh_px,
    };
    if let Some(rect) = screen_rect_for_marker_mesh_bounds(&params) {
        return Ok(rect);
    }
    require_gameplay_marker_screen_rect(
        win_w,
        win_h,
        cam,
        env_height_scale,
        cpu,
        name,
        min_rw_px,
        min_rh_px,
    )
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

pub fn require_gameplay_marker_screen_rect(
    win_w: f32,
    win_h: f32,
    cam: &CameraParams,
    env_height_scale: f32,
    cpu: &RoomGlbCpu,
    name: &str,
    min_rw: f32,
    min_rh: f32,
) -> anyhow::Result<[f32; 4]> {
    gameplay_marker_screen_rect(
        win_w,
        win_h,
        cam,
        env_height_scale,
        cpu,
        name,
        min_rw,
        min_rh,
    )
    .ok_or_else(|| anyhow::anyhow!("gameplay.glb missing required empty `{name}` (runtime)"))
}

/// Billboards a [`Object3dKind::BossIcon`] toward the gameplay camera (mesh cap faces local **+Y**),
/// then spins 90° about local **+Y** so atlas art reads upright on the plinth.
pub fn gameplay_boss_icon_rotation(
    window_w: f32,
    window_h: f32,
    cam: &CameraParams,
    anchor: [f32; 3],
) -> [f32; 3] {
    use crate::draw_cmd::camera_facing_euler_xyz_rad;
    use crate::world_space::layout_anchor_to_world;

    let center = layout_anchor_to_world(
        window_w,
        window_h,
        Some(cam),
        anchor[0],
        anchor[1],
        anchor[2],
        false,
    );
    let eye = Vec3::from_array(cam.eye);
    let mut forward = eye - center;
    if forward.length_squared() < 1e-8 {
        let e = camera_facing_euler_xyz_rad(cam.eye, cam.target);
        return compose_rotation_euler(
            rot_euler_xyz_rad(e[0], e[1], e[2]),
            [0.0, 90.0, 0.0],
        );
    }
    forward = forward.normalize();
    if forward.y.abs() > 0.97 {
        let e = camera_facing_euler_xyz_rad(cam.eye, cam.target);
        return compose_rotation_euler(
            rot_euler_xyz_rad(e[0], e[1], e[2]),
            [0.0, 90.0, 0.0],
        );
    }
    let q = Quat::from_rotation_arc(Vec3::Y, forward);
    compose_rotation_euler(Mat4::from_quat(q.normalize()), [0.0, 90.0, 0.0])
}

/// Boss ordeal token at `tile_plinth.002` — same footprint as plinth showcase tiles.
pub fn gameplay_boss_ordeal_object3d(
    plinth: &GameplayMarkerPose,
    window_w: f32,
    window_h: f32,
    env_height_scale: f32,
    cam: &CameraParams,
    icon_size_px: f32,
    kind: mahjuro_core::core::ordeal_kind::OrdealKind,
    glow: f32,
) -> Object3d {
    let scale = icon_size_px * plinth.uniform_author_scale(window_h, env_height_scale);
    let rotation = gameplay_boss_icon_rotation(window_w, window_h, cam, plinth.anchor);
    Object3d {
        pos: plinth.anchor,
        extents: [scale, scale * 0.04, scale],
        rotation,
        color: [1.0, 1.0, 1.0, 0.98],
        kind: Object3dKind::BossIcon {
            kind,
            glow,
            pick_id: None,
        },
        hover_target: 0.0,
        anim_id: 0,
    }
}

/// Screen-space hit / focus rect for the boss ordeal icon at `tile_plinth.002`.
pub fn gameplay_boss_ordeal_screen_rect(
    plinth: &GameplayMarkerPose,
    window_w: f32,
    window_h: f32,
    env_height_scale: f32,
    cam: &CameraParams,
    icon_size_px: f32,
) -> [f32; 4] {
    use crate::world_space::layout_anchor_to_world;

    let icon_px = icon_size_px * plinth.uniform_author_scale(window_h, env_height_scale);
    let half = icon_px * 0.5;
    let center = layout_anchor_to_world(
        window_w,
        window_h,
        Some(cam),
        plinth.anchor[0],
        plinth.anchor[1],
        plinth.anchor[2],
        false,
    );
    let eye = Vec3::from_array(cam.eye);
    let target = Vec3::from_array(cam.target);
    let mut forward = eye - center;
    if forward.length_squared() < 1e-8 {
        forward = target - eye;
    }
    forward = forward.normalize();
    let world_up = Vec3::Z;
    let mut right = forward.cross(world_up);
    if right.length_squared() < 1e-8 {
        right = Vec3::X;
    } else {
        right = right.normalize();
    }
    let up = right.cross(forward).normalize();

    let mut mn_x = f32::INFINITY;
    let mut mn_y = f32::INFINITY;
    let mut mx_x = f32::NEG_INFINITY;
    let mut mx_y = f32::NEG_INFINITY;
    for offset in [
        (-half, -half),
        (half, -half),
        (-half, half),
        (half, half),
    ] {
        let world = center + right * offset.0 + up * offset.1;
        let (sx, sy) = cam.project_world_to_screen(window_w, window_h, world);
        mn_x = mn_x.min(sx);
        mn_y = mn_y.min(sy);
        mx_x = mx_x.max(sx);
        mx_y = mx_y.max(sy);
    }
    [mn_x, mn_y, mx_x - mn_x, mx_y - mn_y]
}

fn gameplay_embedded_camera_doc(cpu: &RoomGlbCpu) -> Option<room_glb::RoomGlbEmbeddedCamera> {
    cpu.embedded_cameras_by_name
        .get(GAMEPLAY_CAMERA_NODE)
        .copied()
}

/// Embedded glTF perspective camera (`default` node), scaled like the room mesh.
pub fn gameplay_camera_from_cpu(
    cpu: &RoomGlbCpu,
    window_h: f32,
    env_height_scale: f32,
) -> Option<CameraParams> {
    let center_doc = cpu
        .environment_bounds_doc
        .map(|b| b.center())
        .unwrap_or(Vec3::ZERO);
    gameplay_embedded_camera_doc(cpu)
        .map(|c| c.to_camera_params(window_h, env_height_scale, center_doc))
}

/// Embedded glTF perspective camera (`default` node), scaled like the room mesh.
pub fn gameplay_camera_from_glb_if_present(
    window_h: f32,
    env_height_scale: f32,
) -> Option<CameraParams> {
    with_gameplay_glb_cpu(|opt| {
        opt.and_then(|cpu| gameplay_camera_from_cpu(cpu, window_h, env_height_scale))
    })
}

pub fn require_gameplay_camera(
    window_h: f32,
    env_height_scale: f32,
) -> anyhow::Result<CameraParams> {
    gameplay_camera_from_glb_if_present(window_h, env_height_scale).ok_or_else(|| {
        anyhow::anyhow!(
            "gameplay.glb missing glTF perspective camera node `{GAMEPLAY_CAMERA_NODE}`"
        )
    })
}

#[cfg(test)]
pub fn require_gameplay_camera_from_cpu(
    cpu: &RoomGlbCpu,
    window_h: f32,
    env_height_scale: f32,
) -> anyhow::Result<CameraParams> {
    gameplay_camera_from_cpu(cpu, window_h, env_height_scale).ok_or_else(|| {
        anyhow::anyhow!(
            "gameplay.glb missing glTF perspective camera node `{GAMEPLAY_CAMERA_NODE}`"
        )
    })
}

pub fn gameplay_glb_has_embedded_lights() -> bool {
    with_gameplay_glb_cpu(|opt| {
        opt.is_some_and(crate::room_gltf_punctual::room_glb_has_embedded_lights)
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
            crate::room_gltf_punctual::embedded_point_lights_runtime(
                cpu,
                w,
                h,
                env_h,
                tune,
                crate::room_gltf_punctual::RoomPunctualProfile::ShopCandles {
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
            crate::room_gltf_punctual::embedded_spot_lights_runtime(
                cpu,
                w,
                h,
                env_h,
                tune,
                "gameplay.glb",
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
                    flicker_amp: crate::flame_volume::SHOP_CANDLE_FLICKER_AMP,
                }
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unexportable_mesh_names_are_recognized() {
        for name in [
            "score_counter.001",
            "hand_tile_0",
            "Relic_0_31",
            "Bowl_0_57",
            "gold coins",
        ] {
            assert!(
                is_gameplay_unexportable_mesh(name),
                "{name} should be forbidden"
            );
        }
        assert!(!is_gameplay_unexportable_mesh("table"));
        assert!(!is_gameplay_unexportable_mesh("btn_cash_in"));
        assert!(!is_gameplay_unexportable_mesh("CandleWax_6_29"));
    }

    #[test]
    fn gameplay_room_shadow_caster_policy() {
        assert!(!gameplay_prim_casts_room_shadow(Some("table")));
        assert!(!gameplay_prim_casts_room_shadow(Some("candles")));
        assert!(!gameplay_prim_casts_room_shadow(Some("candle_wicks")));
        assert!(!gameplay_prim_casts_room_shadow(Some("gold_dish")));
        assert!(!gameplay_prim_casts_room_shadow(Some("label_cash_in")));
        assert!(gameplay_prim_casts_room_shadow(Some("plinth_dora")));
        assert!(gameplay_prim_casts_room_shadow(Some("plinth_ordeal")));
    }

    #[test]
    fn shipped_gameplay_glb_loads_with_required_markers_and_no_unexportables() {
        let bytes = include_bytes!("../../../assets/3d/gameplay.glb");
        let cpu = load_gameplay_glb_from_bytes(bytes).expect("decode gameplay.glb");
        validate_gameplay_glb(cpu).expect("valid gameplay.glb");
    }
}
