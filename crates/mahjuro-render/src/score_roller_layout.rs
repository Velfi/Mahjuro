//! Gameplay score odometer roller slot layout — maps numeric values to authored
//! `roller` / `rollerN` mesh pivots in `gameplay.glb` and projects them to
//! screen anchors for score-popup fly targets.

use glam::Vec3;

use crate::draw_cmd::CameraParams;
use crate::gameplay_glb;
use crate::room_glb::RoomGlbCpu;
use crate::world_space::{object3d_pos_triple_for_world_center, LayoutAnchorPx};

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
    let significance = if value == 0 {
        0
    } else {
        value.ilog10() as i32
    };
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
pub fn collect_score_roller_pivots_doc(cpu: &RoomGlbCpu) -> [[f32; 3]; GAMEPLAY_SCORE_ROLLER_SLOT_COUNT] {
    let mut pivots = [[0.0; 3]; GAMEPLAY_SCORE_ROLLER_SLOT_COUNT];
    for env_prim in &cpu.environment_primitives {
        let Some(name) = env_prim.gltf_node_name.as_deref() else {
            continue;
        };
        let Some(raw_idx) = gameplay_score_roller_raw_index(name) else {
            continue;
        };
        let Some(slot) = gameplay_score_roller_slot_remap(raw_idx) else {
            continue;
        };
        if let Some(bind) = cpu.node_bind_poses.get(name) {
            pivots[slot] = bind.bind_world_doc.w_axis.truncate().to_array();
        }
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
}
