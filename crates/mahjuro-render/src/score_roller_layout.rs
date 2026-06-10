//! Gameplay score odometer roller slot layout — maps numeric values to authored
//! `roller` / `rollerN` mesh pivots in `gameplay.glb` and projects them to
//! screen anchors for score-popup fly targets.

use glam::Vec3;

use crate::draw_cmd::CameraParams;
use crate::gameplay_glb;
use crate::room_glb::RoomGlbCpu;
use crate::world_space::{LayoutAnchorPx, object3d_pos_triple_for_world_center};

pub const GAMEPLAY_SCORE_ROLLER_SLOT_COUNT: usize = 20;
pub const GAMEPLAY_SCORE_ROLLER_BANK_DIGITS: usize = 10;

/// Parse a gameplay env mesh name into a raw roller index (`roller` → 0, `roller3` → 3, …).
pub fn gameplay_score_roller_raw_index(name: &str) -> Option<usize> {
    let lower = name.to_ascii_lowercase();
    if !lower.contains("roller") {
        return None;
    }
    if lower == "roller" {
        return Some(0);
    }
    let mut end = lower.len();
    let bytes = lower.as_bytes();
    while end > 0 && bytes[end - 1].is_ascii_digit() {
        end -= 1;
    }
    if end == lower.len() {
        return None;
    }
    lower[end..].parse::<usize>().ok()
}

pub fn gameplay_score_roller_slot_remap(raw: usize) -> Option<usize> {
    if raw < GAMEPLAY_SCORE_ROLLER_SLOT_COUNT {
        Some(raw)
    } else {
        None
    }
}

/// Local slot within a 10-digit bank: 0 = 10⁹ column, 9 = ones column.
pub fn score_roller_local_slot_for_significance(significance: i32) -> usize {
    (9 - significance).clamp(0, 9) as usize
}

/// Local slot for the most significant non-zero digit of `value` (ones when zero).
pub fn score_roller_msd_local_slot(value: u64) -> usize {
    let significance = if value == 0 { 0 } else { value.ilog10() as i32 };
    score_roller_local_slot_for_significance(significance)
}

pub fn score_roller_slot_index(bank: usize, local_slot: usize) -> usize {
    bank * GAMEPLAY_SCORE_ROLLER_BANK_DIGITS + local_slot.min(9)
}

/// Full slot index for the most significant digit of `value` in `bank` (0 = score, 1 = target).
pub fn score_roller_msd_slot_for_value(value: u64, bank: usize) -> usize {
    score_roller_slot_index(bank, score_roller_msd_local_slot(value))
}

/// Roller wheel pivot positions in authored doc space (from node bind poses).
///
/// Uses [`RoomGlbCpu::node_bind_poses`] rather than environment mesh names so pivots
/// stay available after [`crate::room_glb::release_room_environment_primitives_cpu`].
pub fn collect_score_roller_pivots_doc(
    cpu: &RoomGlbCpu,
) -> [[f32; 3]; GAMEPLAY_SCORE_ROLLER_SLOT_COUNT] {
    let mut pivots = [[0.0; 3]; GAMEPLAY_SCORE_ROLLER_SLOT_COUNT];
    for (name, bind) in &cpu.node_bind_poses {
        let Some(raw_idx) = gameplay_score_roller_raw_index(name) else {
            continue;
        };
        let Some(slot) = gameplay_score_roller_slot_remap(raw_idx) else {
            continue;
        };
        pivots[slot] = bind.bind_world_doc.w_axis.truncate().to_array();
    }
    pivots
}

pub fn score_roller_slot_world(
    window_h: f32,
    env_height_scale: f32,
    cpu: &RoomGlbCpu,
    pivots_doc: &[[f32; 3]; GAMEPLAY_SCORE_ROLLER_SLOT_COUNT],
    slot: usize,
) -> Option<Vec3> {
    let pivot = *pivots_doc.get(slot)?;
    let room = crate::room_glb::room_env_model_matrix_from_cpu(window_h, env_height_scale, cpu);
    Some(room.transform_point3(Vec3::from_array(pivot)))
}

pub fn score_roller_slot_layout_anchor(
    window_w: f32,
    window_h: f32,
    _cam: &CameraParams,
    env_height_scale: f32,
    cpu: &RoomGlbCpu,
    pivots_doc: &[[f32; 3]; GAMEPLAY_SCORE_ROLLER_SLOT_COUNT],
    slot: usize,
) -> Option<LayoutAnchorPx> {
    let pivot = *pivots_doc.get(slot)?;
    if pivot == [0.0, 0.0, 0.0] {
        return None;
    }
    let world = score_roller_slot_world(window_h, env_height_scale, cpu, pivots_doc, slot)?;
    let triple = object3d_pos_triple_for_world_center(window_w, window_h, world);
    Some(LayoutAnchorPx {
        px: triple[0],
        py: triple[1],
        lift_z: triple[2],
    })
}

/// Screen AABB covering one score odometer bank (0 = round score, 1 = blind target).
pub fn score_roller_bank_screen_rect(
    window_w: f32,
    window_h: f32,
    cam: &CameraParams,
    env_height_scale: f32,
    cpu: &RoomGlbCpu,
    pivots_doc: &[[f32; 3]; GAMEPLAY_SCORE_ROLLER_SLOT_COUNT],
    bank: usize,
) -> Option<[f32; 4]> {
    if bank >= 2 {
        return None;
    }
    let base = bank * GAMEPLAY_SCORE_ROLLER_BANK_DIGITS;
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    let mut count = 0usize;
    for local in 0..GAMEPLAY_SCORE_ROLLER_BANK_DIGITS {
        let slot = base + local;
        if pivots_doc[slot] == [0.0, 0.0, 0.0] {
            continue;
        }
        let world = score_roller_slot_world(window_h, env_height_scale, cpu, pivots_doc, slot)?;
        let (sx, sy) = cam.project_world_to_screen(window_w, window_h, world);
        if !sx.is_finite() || !sy.is_finite() {
            continue;
        }
        min_x = min_x.min(sx);
        min_y = min_y.min(sy);
        max_x = max_x.max(sx);
        max_y = max_y.max(sy);
        count += 1;
    }
    if count == 0 {
        return None;
    }
    let span = (max_x - min_x).max(8.0);
    let digit_w = span / (count.saturating_sub(1).max(1) as f32);
    let pad_x = digit_w * 0.55;
    let pad_y = digit_w * 1.35;
    Some([
        min_x - pad_x,
        min_y - pad_y,
        (max_x - min_x) + pad_x * 2.0,
        (max_y - min_y) + pad_y * 2.0,
    ])
}

/// Screen anchor for the most significant score digit on the live-score roller bank.
pub fn try_resolve_score_reel_msd_anchor(
    window_w: f32,
    window_h: f32,
    cam: &CameraParams,
    env_height_scale: f32,
    running_score: u64,
) -> Option<LayoutAnchorPx> {
    gameplay_glb::with_gameplay_glb_cpu(|cpu| {
        let cpu = cpu?;
        let pivots = collect_score_roller_pivots_doc(cpu);
        let slot = score_roller_msd_slot_for_value(running_score, 0);
        score_roller_slot_layout_anchor(
            window_w,
            window_h,
            cam,
            env_height_scale,
            cpu,
            &pivots,
            slot,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn msd_slot_maps_digit_columns() {
        assert_eq!(score_roller_msd_local_slot(0), 9);
        assert_eq!(score_roller_msd_local_slot(5), 9);
        assert_eq!(score_roller_msd_local_slot(42), 8);
        assert_eq!(score_roller_msd_local_slot(1000), 6);
        assert_eq!(score_roller_msd_local_slot(2051), 6);
        assert_eq!(score_roller_msd_slot_for_value(1000, 0), 6);
    }

    #[test]
    fn roller_msd_anchor_uses_screen_projection() {
        use crate::gameplay_glb::{
            gameplay_camera_from_cpu, load_gameplay_glb_from_bytes, validate_gameplay_glb,
        };
        use crate::world_space::{object3d_pos_triple_for_world_center, pixel_to_world};
        let bytes = include_bytes!("../../../assets/3d/gameplay.glb");
        let cpu = validate_gameplay_glb(load_gameplay_glb_from_bytes(bytes).unwrap()).unwrap();
        let w = 1920.0f32;
        let h = 1080.0f32;
        let env_h = 1.0f32;
        let cam = gameplay_camera_from_cpu(&cpu, h, env_h).expect("camera");
        let score = 500u64;
        let anchor = try_resolve_score_reel_msd_anchor(w, h, &cam, env_h, score).expect("anchor");
        let pivots = collect_score_roller_pivots_doc(&cpu);
        let slot = score_roller_msd_slot_for_value(score, 0);
        let world = score_roller_slot_world(h, env_h, &cpu, &pivots, slot).expect("world");
        let triple = object3d_pos_triple_for_world_center(w, h, world);
        assert!(
            (anchor.px - triple[0]).abs() < 1.0 && (anchor.py - triple[1]).abs() < 1.0,
            "MSD anchor should use pixel_to_world round-trip triple"
        );
        let decoded = pixel_to_world(w, h, anchor.px, anchor.py, anchor.lift_z);
        let (proj_x, proj_y) = cam.project_world_to_screen(w, h, decoded);
        assert!(
            proj_x > 0.0 && proj_x < w && proj_y > 0.0 && proj_y < h,
            "MSD anchor should land on-screen, got ({}, {})",
            proj_x,
            proj_y
        );
    }

    #[test]
    fn roller_pivots_load_from_gameplay_glb() {
        use crate::gameplay_glb::{load_gameplay_glb_from_bytes, validate_gameplay_glb};
        let bytes = include_bytes!("../../../assets/3d/gameplay.glb");
        let cpu = load_gameplay_glb_from_bytes(bytes).expect("gameplay.glb");
        let cpu = validate_gameplay_glb(cpu).expect("validate");
        let pivots = collect_score_roller_pivots_doc(&cpu);
        let found = pivots.iter().filter(|p| **p != [0.0, 0.0, 0.0]).count();
        assert!(
            found >= 10,
            "expected score roller pivots in gameplay.glb, found {found}/{}",
            GAMEPLAY_SCORE_ROLLER_SLOT_COUNT
        );
        let slot = score_roller_msd_slot_for_value(500, 0);
        assert_ne!(
            pivots[slot],
            [0.0, 0.0, 0.0],
            "MSD slot for 500 should have a pivot"
        );
    }

    #[test]
    fn roller_name_parsing() {
        assert_eq!(gameplay_score_roller_raw_index("roller"), Some(0));
        assert_eq!(gameplay_score_roller_raw_index("Roller7"), Some(7));
        assert_eq!(gameplay_score_roller_raw_index("frame"), None);
    }

    #[test]
    fn roller_pivots_survive_environment_mesh_release() {
        use crate::gameplay_glb::{load_gameplay_glb_from_bytes, validate_gameplay_glb};
        use crate::room_glb::release_room_environment_primitives_cpu;
        let bytes = include_bytes!("../../../assets/3d/gameplay.glb");
        let mut cpu = validate_gameplay_glb(load_gameplay_glb_from_bytes(bytes).unwrap()).unwrap();
        let before = collect_score_roller_pivots_doc(&cpu);
        let before_count = before.iter().filter(|p| **p != [0.0, 0.0, 0.0]).count();
        assert!(before_count >= 10, "expected pivots before release, found {before_count}");
        release_room_environment_primitives_cpu(&mut cpu);
        assert!(cpu.environment_primitives.is_empty());
        let after = collect_score_roller_pivots_doc(&cpu);
        let after_count = after.iter().filter(|p| **p != [0.0, 0.0, 0.0]).count();
        assert_eq!(
            after_count, before_count,
            "roller pivots must survive env mesh CPU release"
        );
    }

    #[test]
    fn roller_bank_focus_rect_at_common_resolutions() {
        use crate::gameplay_glb::{
            gameplay_camera_from_cpu, load_gameplay_glb_from_bytes, validate_gameplay_glb, SCORE_FRAME,
            gameplay_marker_screen_rect_resolved,
        };
        let bytes = include_bytes!("../../../assets/3d/gameplay.glb");
        let cpu = validate_gameplay_glb(load_gameplay_glb_from_bytes(bytes).unwrap()).unwrap();
        let env_h = 1.0f32;
        for (w, h) in [(1920.0, 1080.0), (2560.0, 1440.0), (1280.0, 720.0), (1440.0, 900.0)] {
            let cam = gameplay_camera_from_cpu(&cpu, h, env_h).expect("camera");
            let frame = gameplay_marker_screen_rect_resolved(
                w, h, &cam, env_h, &cpu, SCORE_FRAME, 32.0, 16.0,
            )
            .expect("frame");
            let pivots = collect_score_roller_pivots_doc(&cpu);
            let score = score_roller_bank_screen_rect(w, h, &cam, env_h, &cpu, &pivots, 0);
            let target = score_roller_bank_screen_rect(w, h, &cam, env_h, &cpu, &pivots, 1);
            eprintln!(
                "{w}x{h}: frame_h={:.0} score={:?} target={:?}",
                frame[3],
                score.map(|s| format!("h={:.0}", s[3])),
                target.map(|t| format!("h={:.0}", t[3]))
            );
            let score = score.expect("score");
            let target = target.expect("target");
            assert!(
                score[3] <= frame[3] * 0.45,
                "{w}x{h} score bank too tall vs frame"
            );
            assert!(
                target[3] <= frame[3] * 0.45,
                "{w}x{h} target bank too tall vs frame"
            );
        }
    }

    #[test]
    fn roller_bank_focus_rect_matches_frame_height() {
        use crate::gameplay_glb::{
            gameplay_camera_from_cpu, load_gameplay_glb_from_bytes, validate_gameplay_glb, SCORE_FRAME,
            gameplay_marker_screen_rect_resolved,
        };
        let bytes = include_bytes!("../../../assets/3d/gameplay.glb");
        let cpu = validate_gameplay_glb(load_gameplay_glb_from_bytes(bytes).unwrap()).unwrap();
        let w = 1920.0f32;
        let h = 1080.0f32;
        let env_h = 1.0f32;
        let cam = gameplay_camera_from_cpu(&cpu, h, env_h).expect("camera");
        let frame = gameplay_marker_screen_rect_resolved(
            w, h, &cam, env_h, &cpu, SCORE_FRAME, 32.0, 16.0,
        )
        .expect("frame");
        let pivots = collect_score_roller_pivots_doc(&cpu);
        let score = score_roller_bank_screen_rect(w, h, &cam, env_h, &cpu, &pivots, 0)
            .expect("score bank");
        let target = score_roller_bank_screen_rect(w, h, &cam, env_h, &cpu, &pivots, 1);
        let found = pivots.iter().filter(|p| **p != [0.0, 0.0, 0.0]).count();
        eprintln!(
            "pivots={found} frame h={:.1} score h={:.1} target={:?}",
            frame[3],
            score[3],
            target.map(|t| format!("h={:.1}", t[3]))
        );
        assert!(target.is_some(), "target bank rect should resolve from pivots");
        assert!(
            score[3] <= frame[3] * 1.15,
            "score bank focus rect too tall: h={} frame_h={}",
            score[3],
            frame[3]
        );
    }
}
