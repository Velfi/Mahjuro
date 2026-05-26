//! Player discard animation: lift from hand, staggered arc into the discard river,
//! persist until the next discard. River placement follows the `discard_river` GLB empty.

use std::time::{Duration, Instant};

use crate::core::tile::Tile;
use crate::game::cascade::CascadeTuning;
use crate::persistence::TilePreset;
use crate::render::draw_cmd::{Object3d, ShowcaseTilePlacement};
use crate::render::river_mesh::{
    RIVER_TILE_FLOW_T_MAX, RIVER_TILE_FLOW_T_MIN, river_surface_local,
};
use crate::render::table_transform::{mat4_to_euler_xyz_rad, translate_rot_scale};
use crate::render::world_space::{pixel_to_world, surface_anchor_from_world_xyz};
use crate::scenes::gameplay::GameplayScene;
use crate::scenes::gameplay::glb_anchors;
use glam::Vec3;
use rand::RngExt;

/// Discard tiles in the river — smaller than hand slots, lying face-up on the water.
const RIVER_TILE_SIZE_FRAC: f32 = 0.26;
/// Half of a Chinese tile thickness (15 mm), scaled to river tile size — centers sit in the stream.
const RIVER_TILE_HALF_THICKNESS_MM: f32 = 7.5;
/// Bury slightly into the water plane so tiles read submerged, not on the rocky lip.
const RIVER_TILE_SINK_FRAC: f32 = 0.3;
/// Clear gap between consecutive river tiles as a fraction of each tile's long edge.
const RIVER_TILE_GAP_FRAC: f32 = 0.14;
const RIVER_ARC_LUT_SAMPLES: usize = 128;

/// One landing slot on the discard river.
#[derive(Clone, Copy, Debug)]
pub struct RiverDiscardSlot {
    pub center: [f32; 3],
    pub rotation: [f32; 3],
    pub size_px: f32,
}

/// Per-tile phase within a discard batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiscardAnimPhase {
    /// Waiting for this tile's stagger offset.
    Pending,
    /// Rising above the hand slot.
    Lift,
    /// Curved path into the river.
    Flight,
    /// Small settle on the water surface.
    Landing,
    /// Resting in the river until the next discard clears the pile.
    Settled,
}

/// One tile traveling from hand → river.
#[derive(Clone, Debug)]
pub struct DiscardAnimTile {
    pub tile: Tile,
    pub start_center: [f32; 3],
    pub start_size_px: f32,
    pub river_center: [f32; 3],
    pub river_size_px: f32,
    /// Flow-axis parameter on the river mesh used to re-resolve pose each frame.
    pub flow_t: f32,
    /// Lateral lane across the stream when the pile is crowded (`-1` / `0` / `+1`).
    pub stream_lane: f32,
    pub hand_rotation: [f32; 3],
    pub river_rotation: [f32; 3],
    pub start_delay_ms: u64,
    pub phase: DiscardAnimPhase,
    /// Set when the tile first enters [`DiscardAnimPhase::Settled`].
    pub settled_at: Option<Instant>,
}

/// Active batch spawned on [`UiAction::CommitDiscard`].
#[derive(Clone, Debug)]
pub struct DiscardAnimationBatch {
    pub started_at: Instant,
    pub tiles: Vec<DiscardAnimTile>,
}

/// Tiles resting in the discard river between discards.
#[derive(Clone, Debug)]
pub struct RiverSettledTile {
    pub tile: Tile,
    pub center_pos: [f32; 3],
    pub size_px: f32,
    pub flow_t: f32,
    pub stream_lane: f32,
    pub rotation: [f32; 3],
}

/// Previous river pile animating downward before despawn.
#[derive(Clone, Debug)]
pub struct RiverSinkBatch {
    pub started_at: Instant,
    pub tiles: Vec<RiverSettledTile>,
}

impl DiscardAnimationBatch {
    pub fn is_complete(&self, now: Instant, tuning: &CascadeTuning) -> bool {
        self.tiles
            .iter()
            .all(|t| tile_phase_at(t, self.started_at, now, tuning) == DiscardAnimPhase::Settled)
    }

    pub fn total_duration(&self, tuning: &CascadeTuning) -> Duration {
        let max_start_delay = self
            .tiles
            .iter()
            .map(|t| t.start_delay_ms)
            .max()
            .unwrap_or(0);
        Duration::from_millis(
            max_start_delay
                + tuning.discard_lift_ms
                + tuning.discard_flight_ms
                + tuning.discard_landing_ms,
        )
    }
}

/// Assign per-tile randomized start delays that stay tightly clustered.
fn random_start_delays(count: usize, stagger_ms: u64) -> Vec<u64> {
    if count == 0 {
        return Vec::new();
    }
    if count == 1 {
        return vec![0];
    }
    // Keep starts clustered, but allow up to a 200ms launch spread.
    let spread_ms = stagger_ms.clamp(8, 200);
    let mut rng = rand::rng();
    let mut delays: Vec<u64> = (0..count)
        .map(|_| rng.random_range(0..=spread_ms))
        .collect();
    // Normalize so at least one tile starts immediately.
    let min_delay = delays.iter().copied().min().unwrap_or(0);
    for delay in &mut delays {
        *delay = delay.saturating_sub(min_delay);
    }
    delays
}

fn ease_in_out_cubic(t: f32) -> f32 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

fn hand_slot_center(
    hand_world_slots: &[super::glb_anchors::HandWorldSlot],
    slot: usize,
) -> super::glb_anchors::HandWorldSlot {
    hand_world_slots[slot]
}

pub(crate) fn bowl_model_matrix(window_w: f32, window_h: f32, bowl: &Object3d) -> glam::Mat4 {
    let center = pixel_to_world(
        window_w,
        window_h,
        bowl.pos[0],
        bowl.pos[1],
        bowl.pos[2] + bowl.extents[1] * 0.5,
    );
    translate_rot_scale(center, bowl.rotation_matrix(), Vec3::from(bowl.extents))
}

fn river_surface_normal_world(model: glam::Mat4) -> Vec3 {
    let mut normal = model.transform_vector3(Vec3::new(0.0, 1.0, 0.0));
    if normal.length_squared() < 1e-10 {
        normal = Vec3::Z;
    } else {
        normal = normal.normalize();
    }
    normal
}

fn river_flow_tangent_world(model: glam::Mat4, t_flow: f32) -> Vec3 {
    let eps = 0.012;
    let t0 = (t_flow - eps).clamp(0.0, 1.0);
    let t1 = (t_flow + eps).clamp(0.0, 1.0);
    let p0 = model.transform_point3(river_surface_local(t0));
    let p1 = model.transform_point3(river_surface_local(t1));
    (p1 - p0).normalize()
}

/// Face-up on the river surface; long edge follows flow in the bowl's local frame.
fn river_tile_rotation_at(model: glam::Mat4, t_flow: f32) -> [f32; 3] {
    let normal = river_surface_normal_world(model);

    let mut tangent = river_flow_tangent_world(model, t_flow);
    tangent -= normal * tangent.dot(normal);
    if tangent.length_squared() < 1e-10 {
        tangent = model.transform_vector3(Vec3::new(1.0, 0.0, 0.0));
        tangent -= normal * tangent.dot(normal);
    }
    if tangent.length_squared() < 1e-10 {
        tangent = Vec3::X;
    } else {
        tangent = tangent.normalize();
    }

    let bitangent = normal.cross(tangent).normalize();
    // Showcase path: `base_rotation * tile_mesh_local_to_world` — columns map world X/Y/Z
    // to short / long / face-normal axes after the fixed tile basis.
    let base_rotation = glam::Mat4::from_cols(
        bitangent.extend(0.0),
        tangent.extend(0.0),
        normal.extend(0.0),
        glam::Vec4::new(0.0, 0.0, 0.0, 1.0),
    );
    // Same 180° flip as hand tiles (`Rz(π)`) so the painted face reads upright to the player.
    let upright = glam::Mat4::from_axis_angle(normal, std::f32::consts::PI) * base_rotation;
    mat4_to_euler_xyz_rad(upright)
}

fn river_tile_center_world(
    bowl_model: glam::Mat4,
    flow_t: f32,
    tile_size_px: f32,
    stream_lane: f32,
    layout: &crate::ui::layout::LayoutResult,
) -> Vec3 {
    let mut surface = bowl_model.transform_point3(river_surface_local(flow_t));
    let normal = river_surface_normal_world(bowl_model);
    if stream_lane.abs() > 1e-4 {
        let mut tangent = river_flow_tangent_world(bowl_model, flow_t);
        tangent -= normal * tangent.dot(normal);
        if tangent.length_squared() > 1e-10 {
            tangent = tangent.normalize();
            let bitangent = normal.cross(tangent).normalize();
            surface += bitangent * stream_lane * river_tile_long_span_px(tile_size_px) * 0.5;
        }
    }
    let half_thick = layout.mm(RIVER_TILE_HALF_THICKNESS_MM * RIVER_TILE_SIZE_FRAC);
    let sit = half_thick * (1.0 - RIVER_TILE_SINK_FRAC);
    surface + normal * sit
}

/// Resolve one slot against the live bowl model (includes arrange + hover from last frame).
pub fn resolve_river_slot(
    bowl_model: glam::Mat4,
    flow_t: f32,
    tile_size_px: f32,
    stream_lane: f32,
    window_w: f32,
    window_h: f32,
    layout: &crate::ui::layout::LayoutResult,
) -> RiverDiscardSlot {
    let world = river_tile_center_world(bowl_model, flow_t, tile_size_px, stream_lane, layout);
    RiverDiscardSlot {
        center: surface_anchor_from_world_xyz(window_w, window_h, world),
        rotation: river_tile_rotation_at(bowl_model, flow_t),
        size_px: tile_size_px * RIVER_TILE_SIZE_FRAC,
    }
}

/// Projected long-edge span of one river tile in world XY (matches showcase scale).
fn river_tile_long_span_px(tile_size_px: f32) -> f32 {
    let preset = TilePreset::Chinese;
    tile_size_px * RIVER_TILE_SIZE_FRAC * 0.85 * preset.face_long_ratio()
}

fn river_tile_min_center_spacing(tile_size_px: f32) -> f32 {
    river_tile_long_span_px(tile_size_px) * (1.0 + RIVER_TILE_GAP_FRAC)
}

fn centerline_world_xy(bowl_model: glam::Mat4, t_flow: f32) -> glam::Vec2 {
    let p = bowl_model.transform_point3(river_surface_local(t_flow));
    glam::Vec2::new(p.x, p.y)
}

struct RiverArcLengthLut {
    /// Flow parameters from `t_min` to `t_max`, inclusive.
    ts: Vec<f32>,
    /// Cumulative arc length in world XY from `ts[0]`.
    cum_len: Vec<f32>,
}

fn build_river_arc_lut(bowl_model: glam::Mat4, t_min: f32, t_max: f32) -> RiverArcLengthLut {
    let n = RIVER_ARC_LUT_SAMPLES.max(2);
    let mut ts = Vec::with_capacity(n);
    let mut cum_len = Vec::with_capacity(n);
    let mut prev = centerline_world_xy(bowl_model, t_min);
    ts.push(t_min);
    cum_len.push(0.0);
    for i in 1..n {
        let t = t_min + (t_max - t_min) * (i as f32 / (n - 1) as f32);
        let p = centerline_world_xy(bowl_model, t);
        let seg = (p - prev).length();
        cum_len.push(cum_len[i - 1] + seg);
        ts.push(t);
        prev = p;
    }
    RiverArcLengthLut { ts, cum_len }
}

fn flow_at_arc_offset(lut: &RiverArcLengthLut, arc_offset: f32) -> f32 {
    let total = *lut.cum_len.last().unwrap_or(&0.0);
    let s = arc_offset.clamp(0.0, total);
    if s <= 0.0 {
        return lut.ts[0];
    }
    if s >= total {
        return lut.ts.last().copied().unwrap_or(RIVER_TILE_FLOW_T_MAX);
    }
    let idx = lut
        .cum_len
        .partition_point(|&c| c < s)
        .clamp(1, lut.ts.len() - 1);
    let s0 = lut.cum_len[idx - 1];
    let s1 = lut.cum_len[idx];
    let t0 = lut.ts[idx - 1];
    let t1 = lut.ts[idx];
    let u = if (s1 - s0).abs() < 1e-8 {
        0.0
    } else {
        (s - s0) / (s1 - s0)
    };
    t0 + (t1 - t0) * u
}

#[derive(Clone, Copy, Debug)]
pub struct RiverFlowPlacement {
    pub flow_t: f32,
    pub stream_lane: f32,
}

fn stream_lane_for_index(i: usize, crowded: bool) -> f32 {
    if !crowded {
        0.0
    } else if i.is_multiple_of(2) {
        -1.0
    } else {
        1.0
    }
}

/// Flow placements along the centerline with arc-length spacing and a minimum gap.
fn flow_placements_arc_spaced(
    count: usize,
    bowl_model: glam::Mat4,
    tile_size_px: f32,
) -> (Vec<RiverFlowPlacement>, bool) {
    if count == 0 {
        return (Vec::new(), false);
    }
    if count == 1 {
        return (
            vec![RiverFlowPlacement {
                flow_t: (RIVER_TILE_FLOW_T_MIN + RIVER_TILE_FLOW_T_MAX) * 0.5,
                stream_lane: 0.0,
            }],
            false,
        );
    }

    let min_sep = river_tile_min_center_spacing(tile_size_px);
    let needed = min_sep * (count - 1) as f32;

    let try_span = |t_min: f32, t_max: f32| -> (RiverArcLengthLut, f32) {
        let lut = build_river_arc_lut(bowl_model, t_min, t_max);
        let len = *lut.cum_len.last().unwrap_or(&0.0);
        (lut, len)
    };

    let (lut, total_len) = {
        let (lut_inset, len_inset) = try_span(RIVER_TILE_FLOW_T_MIN, RIVER_TILE_FLOW_T_MAX);
        if needed <= len_inset {
            (lut_inset, len_inset)
        } else {
            try_span(0.0, 1.0)
        }
    };

    let crowded = needed > total_len;

    if total_len < 1e-6 {
        return (
            (0..count)
                .map(|i| RiverFlowPlacement {
                    flow_t: if count == 1 {
                        0.5
                    } else {
                        i as f32 / (count - 1) as f32
                    },
                    stream_lane: stream_lane_for_index(i, crowded),
                })
                .collect(),
            crowded,
        );
    }

    let (start_arc, end_arc) = if needed <= total_len {
        let margin = (total_len - needed) * 0.5;
        (margin, margin + needed)
    } else {
        (0.0, total_len)
    };

    let placements = (0..count)
        .map(|i| {
            let frac = i as f32 / (count - 1) as f32;
            let arc = start_arc + (end_arc - start_arc) * frac;
            RiverFlowPlacement {
                flow_t: flow_at_arc_offset(&lut, arc),
                stream_lane: stream_lane_for_index(i, crowded),
            }
        })
        .collect();
    (placements, crowded)
}

/// Flow-axis parameters spaced along the channel without overlapping footprints.
pub fn flow_params_for_tiles(
    count: usize,
    bowl_model: glam::Mat4,
    tile_size_px: f32,
) -> Vec<RiverFlowPlacement> {
    flow_placements_arc_spaced(count, bowl_model, tile_size_px).0
}

/// Evenly space slots along the river centerline with per-slot rotation and size.
pub fn river_slots_for_discard(
    window_w: f32,
    window_h: f32,
    bowl: &Object3d,
    count: usize,
    tile_size_px: f32,
    layout: &crate::ui::layout::LayoutResult,
) -> Vec<RiverDiscardSlot> {
    let model = bowl_model_matrix(window_w, window_h, bowl);
    flow_placements_arc_spaced(count, model, tile_size_px)
        .0
        .into_iter()
        .map(|slot| {
            resolve_river_slot(
                model,
                slot.flow_t,
                tile_size_px,
                slot.stream_lane,
                window_w,
                window_h,
                layout,
            )
        })
        .collect()
}

fn tile_phase_at(
    tile: &DiscardAnimTile,
    batch_start: Instant,
    now: Instant,
    tuning: &CascadeTuning,
) -> DiscardAnimPhase {
    let elapsed_ms = now.saturating_duration_since(batch_start).as_millis() as u64;
    if elapsed_ms < tile.start_delay_ms {
        return DiscardAnimPhase::Pending;
    }
    let local_ms = elapsed_ms - tile.start_delay_ms;
    if local_ms < tuning.discard_lift_ms {
        DiscardAnimPhase::Lift
    } else if local_ms < tuning.discard_lift_ms + tuning.discard_flight_ms {
        DiscardAnimPhase::Flight
    } else if local_ms
        < tuning.discard_lift_ms + tuning.discard_flight_ms + tuning.discard_landing_ms
    {
        DiscardAnimPhase::Landing
    } else {
        DiscardAnimPhase::Settled
    }
}

fn tile_local_t(
    tile: &DiscardAnimTile,
    batch_start: Instant,
    now: Instant,
    tuning: &CascadeTuning,
    phase: DiscardAnimPhase,
) -> f32 {
    let elapsed_ms = now.saturating_duration_since(batch_start).as_millis() as u64;
    let local_ms = elapsed_ms.saturating_sub(tile.start_delay_ms) as f32;
    match phase {
        DiscardAnimPhase::Lift => (local_ms / tuning.discard_lift_ms.max(1) as f32).clamp(0.0, 1.0),
        DiscardAnimPhase::Flight => {
            let start = tuning.discard_lift_ms as f32;
            let dur = tuning.discard_flight_ms.max(1) as f32;
            ((local_ms - start) / dur).clamp(0.0, 1.0)
        }
        DiscardAnimPhase::Landing => {
            let start = (tuning.discard_lift_ms + tuning.discard_flight_ms) as f32;
            let dur = tuning.discard_landing_ms.max(1) as f32;
            ((local_ms - start) / dur).clamp(0.0, 1.0)
        }
        _ => 1.0,
    }
}

/// Sample animated center, scale, and phase for one in-flight tile.
pub fn sample_discard_tile(
    tile: &DiscardAnimTile,
    batch_start: Instant,
    now: Instant,
    tuning: &CascadeTuning,
) -> Option<(DiscardAnimPhase, [f32; 3], f32)> {
    let phase = tile_phase_at(tile, batch_start, now, tuning);
    if phase == DiscardAnimPhase::Pending {
        return None;
    }
    let t = ease_in_out_cubic(tile_local_t(tile, batch_start, now, tuning, phase));
    const LIFT_MM: f32 = 28.0;
    let lift_px = |layout_scale: f32| layout_scale * LIFT_MM * 0.15;

    let (center, scale) = match phase {
        DiscardAnimPhase::Lift => {
            let lift = lift_px(1.0) * t;
            (
                [
                    tile.start_center[0],
                    tile.start_center[1],
                    tile.start_center[2] + lift,
                ],
                1.0 + 0.06 * t,
            )
        }
        DiscardAnimPhase::Flight => {
            let u = t;
            let one_u = 1.0 - u;
            let apex = [
                (tile.start_center[0] + tile.river_center[0]) * 0.5,
                tile.start_center[1].min(tile.river_center[1]) - 90.0,
                tile.start_center[2].max(tile.river_center[2]) + lift_px(1.0) * 1.4,
            ];
            let cx = one_u * one_u * tile.start_center[0]
                + 2.0 * one_u * u * apex[0]
                + u * u * tile.river_center[0];
            let cy = one_u * one_u * tile.start_center[1]
                + 2.0 * one_u * u * apex[1]
                + u * u * tile.river_center[1];
            let cz = one_u * one_u * tile.start_center[2]
                + 2.0 * one_u * u * apex[2]
                + u * u * tile.river_center[2];
            ([cx, cy, cz], 1.0 - 0.08 * u)
        }
        DiscardAnimPhase::Landing => {
            let bounce = (t * std::f32::consts::PI).sin() * lift_px(1.0) * 0.35;
            (
                [
                    tile.river_center[0],
                    tile.river_center[1],
                    tile.river_center[2] + bounce * (1.0 - t),
                ],
                0.92 + 0.08 * t,
            )
        }
        DiscardAnimPhase::Settled => (tile.river_center, 1.0),
        DiscardAnimPhase::Pending => unreachable!(),
    };
    Some((phase, center, scale))
}

/// Move the current river pile into a sink animation (replaced on next discard).
fn start_river_sink(scene: &mut GameplayScene, now: Instant) {
    if scene.river_settled_tiles.is_empty() {
        return;
    }
    scene.river_sink_batch = Some(RiverSinkBatch {
        started_at: now,
        tiles: std::mem::take(&mut scene.river_settled_tiles),
    });
}

fn sample_river_sink_tile(
    tile: &RiverSettledTile,
    started_at: Instant,
    now: Instant,
    sink_ms: u64,
) -> Option<([f32; 3], f32, f32)> {
    let elapsed_ms = now.saturating_duration_since(started_at).as_millis() as u64;
    if elapsed_ms >= sink_ms {
        return None;
    }
    let u = ease_in_out_cubic((elapsed_ms as f32 / sink_ms.max(1) as f32).clamp(0.0, 1.0));
    let sink_depth = tile.size_px * 0.42 * u;
    let scale = 1.0 - 0.58 * u;
    let brightness = 1.0 - 0.7 * u;
    Some((
        [
            tile.center_pos[0],
            tile.center_pos[1],
            tile.center_pos[2] - sink_depth,
        ],
        scale,
        brightness,
    ))
}

/// Start a discard animation batch; sinks any previous river pile first.
pub fn begin_discard_batch(
    scene: &mut GameplayScene,
    layout: &crate::ui::layout::LayoutResult,
    env_height_scale: f32,
    selected_indices: &[usize],
    tiles: &[Tile],
    now: Instant,
) {
    start_river_sink(scene, now);
    if selected_indices.is_empty() || tiles.is_empty() {
        scene.active_discard_anim = None;
        return;
    }

    let w = layout.window_w;
    let h = layout.window_h;
    let cam = match crate::render::gameplay_glb::require_gameplay_camera(h, env_height_scale) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("discard batch camera: {e}");
            scene.active_discard_anim = None;
            return;
        }
    };
    let hand_len = selected_indices
        .iter()
        .copied()
        .max()
        .map(|i| i + 1)
        .unwrap_or(0);
    let anchors = match glb_anchors::resolve_gameplay_glb_anchors(
        layout,
        hand_len,
        w,
        h,
        &cam,
        env_height_scale,
        0.0,
        0.0,
    ) {
        Ok(a) => a,
        Err(e) => {
            log::warn!("discard batch anchors: {e}");
            scene.active_discard_anim = None;
            return;
        }
    };
    let bowl = anchors.discard_river_pick.clone();
    let hand_world_slots = anchors.hand_world_slots;
    let hand_scale_mul = anchors.hand_marker_poses[0].uniform_author_scale(h, env_height_scale);
    let reference_size_px = selected_indices
        .iter()
        .map(|&slot| hand_slot_center(&hand_world_slots, slot).1)
        .fold(0.0_f32, f32::max)
        * hand_scale_mul;
    let bowl_model = bowl_model_matrix(layout.window_w, layout.window_h, &bowl);
    let flow_ts = flow_params_for_tiles(tiles.len(), bowl_model, reference_size_px);
    let river_slots = river_slots_for_discard(
        layout.window_w,
        layout.window_h,
        &bowl,
        tiles.len(),
        reference_size_px,
        layout,
    );

    let mut anim_tiles = Vec::with_capacity(tiles.len());
    let start_delays =
        random_start_delays(tiles.len(), scene.cached_cascade_tuning.discard_stagger_ms);
    for (seq, &tile) in tiles.iter().enumerate() {
        let slot = selected_indices[seq];
        let (start_center, start_size_px, hand_rotation) =
            hand_slot_center(&hand_world_slots, slot);
        let start_size_px = start_size_px * hand_scale_mul;
        let river = river_slots[seq];
        anim_tiles.push(DiscardAnimTile {
            tile,
            start_center,
            start_size_px,
            river_center: river.center,
            river_size_px: river.size_px,
            flow_t: flow_ts[seq].flow_t,
            stream_lane: flow_ts[seq].stream_lane,
            hand_rotation,
            river_rotation: river.rotation,
            start_delay_ms: start_delays[seq],
            phase: DiscardAnimPhase::Pending,
            settled_at: None,
        });
    }

    scene.active_discard_anim = Some(DiscardAnimationBatch {
        started_at: now,
        tiles: anim_tiles,
    });
}

fn tick_river_sink(scene: &mut GameplayScene, now: Instant, tuning: &CascadeTuning) {
    let Some(batch) = &scene.river_sink_batch else {
        return;
    };
    let done = now.saturating_duration_since(batch.started_at).as_millis()
        >= tuning.discard_river_sink_ms as u128;
    if done {
        scene.river_sink_batch = None;
    }
}

/// Advance discard animation; moves landed tiles into [`GameplayScene::river_settled_tiles`].
pub fn tick_discard_animation(scene: &mut GameplayScene, now: Instant, tuning: &CascadeTuning) {
    tick_river_sink(scene, now, tuning);

    let Some(batch) = scene.active_discard_anim.as_mut() else {
        return;
    };
    let batch_start = batch.started_at;
    for tile in &mut batch.tiles {
        let phase = tile_phase_at(tile, batch_start, now, tuning);
        if phase == DiscardAnimPhase::Settled && tile.settled_at.is_none() {
            tile.settled_at = Some(now);
            tile.phase = DiscardAnimPhase::Settled;
            scene.river_settled_tiles.push(RiverSettledTile {
                tile: tile.tile,
                center_pos: tile.river_center,
                size_px: tile.river_size_px,
                flow_t: tile.flow_t,
                stream_lane: tile.stream_lane,
                rotation: tile.river_rotation,
            });
        } else {
            tile.phase = phase;
        }
    }
    if batch.is_complete(now, tuning) {
        scene.active_discard_anim = None;
    }
}

pub fn discard_animation_active(scene: &GameplayScene) -> bool {
    scene.active_discard_anim.is_some() || scene.river_sink_batch.is_some()
}

/// Build 3D placements for the previous river pile while it sinks away.
pub fn sinking_placements(
    scene: &GameplayScene,
    now: Instant,
    tuning: &CascadeTuning,
    bowl_model: Option<glam::Mat4>,
    layout: &crate::ui::layout::LayoutResult,
    window_w: f32,
    window_h: f32,
    run: &crate::game::run::RunState,
) -> Vec<ShowcaseTilePlacement> {
    let Some(batch) = &scene.river_sink_batch else {
        return Vec::new();
    };
    batch
        .tiles
        .iter()
        .filter_map(|t| {
            let (base_center, rotation, size_px) = river_pose(
                bowl_model,
                t.flow_t,
                t.stream_lane,
                t.size_px / RIVER_TILE_SIZE_FRAC,
                layout,
                window_w,
                window_h,
                t.center_pos,
                t.rotation,
            );
            let sink_tile = RiverSettledTile {
                center_pos: base_center,
                rotation,
                size_px,
                ..t.clone()
            };
            let (center, scale, brightness) = sample_river_sink_tile(
                &sink_tile,
                batch.started_at,
                now,
                tuning.discard_river_sink_ms,
            )?;
            Some(ShowcaseTilePlacement {
                tile: GameplayScene::display_tile(t.tile, run),
                center_pos: center,
                rotation,
                scale,
                size_px: size_px * scale,
                brightness,
                selected: false,
                hovered: false,
                outline: false,
                glow: false,
                glow_color: None,
                pick_id: None,
                overlay_rect_group: None,
            })
        })
        .collect()
}

fn river_pose(
    bowl_model: Option<glam::Mat4>,
    flow_t: f32,
    stream_lane: f32,
    size_px: f32,
    layout: &crate::ui::layout::LayoutResult,
    window_w: f32,
    window_h: f32,
    fallback_center: [f32; 3],
    fallback_rotation: [f32; 3],
) -> ([f32; 3], [f32; 3], f32) {
    let Some(model) = bowl_model else {
        return (
            fallback_center,
            fallback_rotation,
            size_px * RIVER_TILE_SIZE_FRAC,
        );
    };
    let slot = resolve_river_slot(
        model,
        flow_t,
        size_px,
        stream_lane,
        window_w,
        window_h,
        layout,
    );
    (slot.center, slot.rotation, slot.size_px)
}

/// Build 3D placements for in-flight discard tiles (not yet settled).
pub fn in_flight_placements(
    scene: &GameplayScene,
    now: Instant,
    tuning: &CascadeTuning,
    bowl_model: Option<glam::Mat4>,
    layout: &crate::ui::layout::LayoutResult,
    window_w: f32,
    window_h: f32,
    run: &crate::game::run::RunState,
) -> Vec<ShowcaseTilePlacement> {
    let Some(batch) = &scene.active_discard_anim else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for tile in &batch.tiles {
        let Some((phase, center, scale)) = sample_discard_tile(tile, batch.started_at, now, tuning)
        else {
            continue;
        };
        if phase == DiscardAnimPhase::Settled {
            continue;
        }
        let (river_center, river_rotation, river_size) =
            if matches!(phase, DiscardAnimPhase::Lift | DiscardAnimPhase::Flight) {
                (center, tile.hand_rotation, tile.start_size_px * scale)
            } else {
                let (rc, rr, rs) = river_pose(
                    bowl_model,
                    tile.flow_t,
                    tile.stream_lane,
                    tile.start_size_px,
                    layout,
                    window_w,
                    window_h,
                    tile.river_center,
                    tile.river_rotation,
                );
                // Blend landing target toward resolved river pose.
                let u = if phase == DiscardAnimPhase::Landing {
                    ease_in_out_cubic(tile_local_t(tile, batch.started_at, now, tuning, phase))
                } else {
                    1.0
                };
                let cx = center[0] + (rc[0] - center[0]) * u;
                let cy = center[1] + (rc[1] - center[1]) * u;
                let cz = center[2] + (rc[2] - center[2]) * u;
                ([cx, cy, cz], rr, rs * scale)
            };
        let rotation = if matches!(phase, DiscardAnimPhase::Lift | DiscardAnimPhase::Flight) {
            tile.hand_rotation
        } else {
            river_rotation
        };
        out.push(ShowcaseTilePlacement {
            tile: GameplayScene::display_tile(tile.tile, run),
            center_pos: river_center,
            rotation,
            scale,
            size_px: river_size,
            brightness: 1.0,
            selected: false,
            hovered: false,
            outline: false,
            glow: false,
            glow_color: None,
            pick_id: None,
            overlay_rect_group: None,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tile::{Suit, Tile};
    use crate::render::draw_cmd::Object3dKind;
    use std::time::Instant;

    fn test_tile(id: u32) -> Tile {
        Tile::new(Suit::Manzu, 1, id)
    }

    #[test]
    fn river_surface_local_spreads_along_channel() {
        let a = river_surface_local(RIVER_TILE_FLOW_T_MIN);
        let b = river_surface_local(RIVER_TILE_FLOW_T_MAX);
        assert!((a.x - b.x).abs() > 0.5);
    }

    #[test]
    fn arc_spaced_18_tiles_keep_min_gap() {
        use crate::ui::layout::{LayoutResult, Rect};

        let layout = LayoutResult {
            window_w: 800.0,
            window_h: 600.0,
            score_panel: Rect {
                x: 0.0,
                y: 0.0,
                w: 100.0,
                h: 50.0,
            },
            modifier_strip: Rect {
                x: 0.0,
                y: 50.0,
                w: 100.0,
                h: 30.0,
            },
            hand_strip: Rect {
                x: 128.0,
                y: 400.0,
                w: 544.0,
                h: 80.0,
            },
            hand_slots: vec![Rect {
                x: 128.0,
                y: 400.0,
                w: 34.0,
                h: 80.0,
            }],
        };
        let bowl = Object3d {
            pos: [400.0, 420.0, 44.0],
            extents: [140.0, 74.0, 81.0],
            rotation: [std::f32::consts::FRAC_PI_2, 0.0, 0.0],
            color: [1.0; 4],
            kind: Object3dKind::Bowl,
            hover_target: 0.0,
            anim_id: 1,
        };
        let tile_size_px = 80.0;
        let model = bowl_model_matrix(800.0, 600.0, &bowl);
        let (placements, crowded) = flow_placements_arc_spaced(18, model, tile_size_px);
        assert_eq!(placements.len(), 18);
        assert!(crowded, "18 tiles should exceed inset arc capacity");

        for w in placements.windows(2) {
            assert_ne!(
                w[0].stream_lane, w[1].stream_lane,
                "crowded piles should stagger across the stream"
            );
        }

        let long_span = river_tile_long_span_px(tile_size_px);
        let slots = river_slots_for_discard(800.0, 600.0, &bowl, 18, tile_size_px, &layout);
        assert_eq!(slots.len(), 18);
        for w in slots.windows(2) {
            let p0 = pixel_to_world(800.0, 600.0, w[0].center[0], w[0].center[1], w[0].center[2]);
            let p1 = pixel_to_world(800.0, 600.0, w[1].center[0], w[1].center[1], w[1].center[2]);
            let d = glam::Vec2::new(p1.x - p0.x, p1.y - p0.y).length();
            assert!(
                d >= long_span * 0.82,
                "staggered tiles should stay separated (got {d:.1}, need ~{long_span:.1})"
            );
        }
    }

    #[test]
    fn river_slots_spread_along_flow_axis() {
        use crate::render::draw_cmd::Object3dKind;
        use crate::ui::layout::{LayoutResult, Rect};

        let layout = LayoutResult {
            window_w: 800.0,
            window_h: 600.0,
            score_panel: Rect {
                x: 0.0,
                y: 0.0,
                w: 100.0,
                h: 50.0,
            },
            modifier_strip: Rect {
                x: 0.0,
                y: 50.0,
                w: 100.0,
                h: 30.0,
            },
            hand_strip: Rect {
                x: 128.0,
                y: 400.0,
                w: 544.0,
                h: 80.0,
            },
            hand_slots: vec![Rect {
                x: 128.0,
                y: 400.0,
                w: 34.0,
                h: 80.0,
            }],
        };
        let bowl = Object3d {
            pos: [400.0, 420.0, 44.0],
            extents: [140.0, 74.0, 81.0],
            rotation: [std::f32::consts::FRAC_PI_2, 0.0, 0.0],
            color: [1.0; 4],
            kind: Object3dKind::Bowl,
            hover_target: 0.0,
            anim_id: 1,
        };
        let slots = river_slots_for_discard(800.0, 600.0, &bowl, 4, 80.0, &layout);
        assert_eq!(slots.len(), 4);
        assert!((slots[0].size_px - 80.0 * RIVER_TILE_SIZE_FRAC).abs() < 0.01);
        let spread_x = (slots[3].center[0] - slots[0].center[0]).abs();
        assert!(
            spread_x > 40.0,
            "tiles should spread along river length (x)"
        );
        let mut path_len = 0.0_f32;
        for w in slots.windows(2) {
            let dx = w[1].center[0] - w[0].center[0];
            let dy = w[1].center[1] - w[0].center[1];
            path_len += (dx * dx + dy * dy).sqrt();
            assert!(
                dx * dx + dy * dy > 100.0,
                "consecutive slots should be separated"
            );
            assert!(
                (w[0].rotation[2] - w[1].rotation[2]).abs() > 0.05
                    || (w[0].center[0] - w[1].center[0]).abs() > 20.0,
                "adjacent tiles align to local flow heading"
            );
        }
        assert!(
            path_len > spread_x * 0.9,
            "slots should span most of the channel"
        );
    }

    #[test]
    fn randomized_start_delays_are_tightly_clustered() {
        let stagger_ms = 600;
        let delays = random_start_delays(12, stagger_ms);
        assert_eq!(delays.len(), 12);
        assert_eq!(delays.iter().copied().min(), Some(0));
        let spread = delays.iter().copied().max().unwrap_or(0);
        assert!(spread <= 200);
    }

    #[test]
    fn start_delayed_tiles_enter_lift_after_delay() {
        let tuning = CascadeTuning::default();
        let now = Instant::now();
        let tile = |delay_ms: u64| DiscardAnimTile {
            tile: test_tile(0),
            start_center: [0.0, 0.0, 0.0],
            start_size_px: 40.0,
            river_center: [100.0, 100.0, 10.0],
            river_size_px: 35.0,
            flow_t: 0.5,
            stream_lane: 0.0,
            hand_rotation: [0.0, 0.0, 0.0],
            river_rotation: [0.0, 0.0, 0.0],
            start_delay_ms: delay_ms,
            phase: DiscardAnimPhase::Pending,
            settled_at: None,
        };
        let batch = DiscardAnimationBatch {
            started_at: now,
            tiles: vec![tile(0), tile(45)],
        };
        let t0 = tile_phase_at(&batch.tiles[0], now, now, &tuning);
        let t1 = tile_phase_at(&batch.tiles[1], now, now, &tuning);
        assert_eq!(t0, DiscardAnimPhase::Lift);
        assert_eq!(t1, DiscardAnimPhase::Pending);
        let mid = now + Duration::from_millis(45 + tuning.discard_lift_ms / 2);
        assert_eq!(
            tile_phase_at(&batch.tiles[1], now, mid, &tuning),
            DiscardAnimPhase::Lift
        );
    }

    #[test]
    fn river_sink_finishes_after_duration() {
        let tuning = CascadeTuning::default();
        let now = Instant::now();
        let tile = RiverSettledTile {
            tile: test_tile(0),
            center_pos: [100.0, 200.0, 30.0],
            size_px: 40.0,
            flow_t: 0.5,
            stream_lane: 0.0,
            rotation: [0.0; 3],
        };
        let mid = sample_river_sink_tile(&tile, now, now, tuning.discard_river_sink_ms);
        assert!(mid.is_some());
        let end = now + Duration::from_millis(tuning.discard_river_sink_ms);
        assert!(sample_river_sink_tile(&tile, now, end, tuning.discard_river_sink_ms).is_none());
    }

    #[test]
    fn batch_total_duration_includes_stagger() {
        let tuning = CascadeTuning::default();
        let batch = DiscardAnimationBatch {
            started_at: Instant::now(),
            tiles: vec![
                DiscardAnimTile {
                    tile: test_tile(0),
                    start_center: [0.0; 3],
                    start_size_px: 40.0,
                    river_center: [1.0; 3],
                    river_size_px: 35.0,
                    flow_t: 0.35,
                    stream_lane: 0.0,
                    hand_rotation: [0.0; 3],
                    river_rotation: [0.0; 3],
                    start_delay_ms: 12,
                    phase: DiscardAnimPhase::Pending,
                    settled_at: None,
                },
                DiscardAnimTile {
                    tile: test_tile(0),
                    start_center: [0.0; 3],
                    start_size_px: 40.0,
                    river_center: [1.0; 3],
                    river_size_px: 35.0,
                    flow_t: 0.65,
                    stream_lane: 0.0,
                    hand_rotation: [0.0; 3],
                    river_rotation: [0.0; 3],
                    start_delay_ms: 45,
                    phase: DiscardAnimPhase::Pending,
                    settled_at: None,
                },
            ],
        };
        let expected =
            45 + tuning.discard_lift_ms + tuning.discard_flight_ms + tuning.discard_landing_ms;
        assert_eq!(batch.total_duration(&tuning).as_millis(), expected as u128);
    }
}

/// Build 3D placements for tiles resting in the discard river.
pub fn settled_placements(
    scene: &GameplayScene,
    bowl_model: Option<glam::Mat4>,
    layout: &crate::ui::layout::LayoutResult,
    window_w: f32,
    window_h: f32,
    run: &crate::game::run::RunState,
) -> Vec<ShowcaseTilePlacement> {
    scene
        .river_settled_tiles
        .iter()
        .map(|t| {
            let (center, rotation, size_px) = river_pose(
                bowl_model,
                t.flow_t,
                t.stream_lane,
                t.size_px / RIVER_TILE_SIZE_FRAC,
                layout,
                window_w,
                window_h,
                t.center_pos,
                t.rotation,
            );
            (center, rotation, size_px, t.tile)
        })
        .map(
            |(center_pos, rotation, size_px, tile)| ShowcaseTilePlacement {
                tile: GameplayScene::display_tile(tile, run),
                center_pos,
                rotation,
                scale: 1.0,
                size_px,
                brightness: 1.0,
                selected: false,
                hovered: false,
                outline: false,
                glow: false,
                glow_color: None,
                pick_id: None,
                overlay_rect_group: None,
            },
        )
        .collect()
}
