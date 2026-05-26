//! Resolve gameplay spawn poses from [`gameplay.glb`](../../../assets/3d/gameplay.glb) empties.

use crate::render::draw_cmd::{CameraParams, Object3d};
use crate::render::gameplay_glb::{
    self, BTN_CASH_IN, DISCARD_RIVER, GameplayMarkerPose, HAND_TILES_LEFT, HAND_TILES_RIGHT,
    PLAY_MIRROR, PLAYER_CONSUMABLE_MARKERS, PLAYER_DISCARD_TALLY, PLAYER_GOLD, PLAYER_PLAY_TALLY,
    PLAYER_RELIC_MARKERS, STRUCTURE_TILES_LEFT, STRUCTURE_TILES_RIGHT, TILE_PLINTH_MARKERS,
    YAKU_TABLETS_LEFT, YAKU_TABLETS_RIGHT,
    gameplay_discard_river_model_screen_rect, gameplay_play_mirror_model_screen_rect,
};
use crate::render::room_glb::RoomGlbCpu;

/// Hand rack slot: surface anchor, slot width (px), rotation (XYZ rad from GLB markers).
pub type HandWorldSlot = ([f32; 3], f32, [f32; 3]);
use crate::render::theme::color;
use crate::render::wgpu_renderer::{TextAlign, TextLabel};

/// Pixel heights for projecting `structure_*` / `yaku_tablets_*` marker pairs.
pub fn gameplay_hud_strip_heights(
    window_h: f32,
    layout_scale: f32,
    showcase_present: bool,
) -> (f32, f32, f32) {
    let yaku_panel_h = (33.0 * layout_scale).max(24.0).min(window_h * 0.10);
    let structure_tag_h = if showcase_present {
        (17.0 * layout_scale).max(14.0)
    } else {
        0.0
    };
    let structure_meld_h = if showcase_present {
        (46.0 * layout_scale).max(38.0)
    } else {
        0.0
    };
    (yaku_panel_h, structure_tag_h, structure_meld_h)
}

#[inline]
pub fn screen_rect_center(rect: (f32, f32, f32, f32)) -> (f32, f32) {
    (rect.0 + rect.2 * 0.5, rect.1 + rect.3 * 0.5)
}

/// Discard / play / cash-in hit rects projected with the **same** camera used for this frame's
/// render (`camera_override` after FOV pop, etc.). [`resolve_gameplay_glb_anchors`] runs earlier
/// with a pre-pop camera; call this once the final [`CameraParams`] are known.
pub fn reproject_action_button_rects(
    win_w: f32,
    win_h: f32,
    cam: &CameraParams,
    env_h: f32,
    layout_scale: f32,
    anchors: &GameplayGlbAnchors,
) -> ((f32, f32, f32, f32), (f32, f32, f32, f32), (f32, f32, f32, f32)) {
    let discard = gameplay_discard_river_model_screen_rect(win_w, win_h, cam, &anchors.discard_river_pick);
    let play = gameplay_play_mirror_model_screen_rect(win_w, win_h, cam, &anchors.play_mirror_pick);
    let cash_in_w = (96.0 * layout_scale).max(72.0);
    let cash_in_h = (36.0 * layout_scale).max(24.0);
    let cash_in = gameplay_glb::with_gameplay_glb_cpu(|opt| {
        let cpu = opt?;
        gameplay_glb::gameplay_marker_screen_rect_resolved(
            win_w,
            win_h,
            cam,
            env_h,
            cpu,
            BTN_CASH_IN,
            cash_in_w,
            cash_in_h,
        )
        .ok()
    })
    .map(Into::into)
    .unwrap_or(anchors.cash_in_btn_rect);
    (discard.into(), play.into(), cash_in)
}

/// Best-effort anchors for tooltips / popups outside `draw_frame` (no `DrawCtx`).
pub fn try_resolve_gameplay_glb_anchors(
    layout: &crate::ui::layout::LayoutResult,
    hand_len: usize,
    showcase_present: bool,
    env_height_scale: f32,
) -> Option<GameplayGlbAnchors> {
    let layout_scale = (layout.window_w.min(layout.window_h)) / 600.0;
    let (yaku_panel_h, _, _) =
        gameplay_hud_strip_heights(layout.window_h, layout_scale, showcase_present);
    let cam = crate::render::gameplay_glb::gameplay_camera_from_glb_if_present(
        layout.window_h,
        env_height_scale,
    )?;
    resolve_gameplay_glb_anchors(
        layout,
        hand_len,
        layout.window_w,
        layout.window_h,
        &cam,
        env_height_scale,
        yaku_panel_h,
    )
    .ok()
}

/// Per-frame anchors derived from validated `gameplay.glb` marker nodes.
pub struct GameplayGlbAnchors {
    pub hand_slots: Vec<(f32, f32, f32, f32)>,
    pub hand_world_slots: Vec<HandWorldSlot>,
    /// `hand_tiles_left` / `hand_tiles_right` — rotation is lerped per slot; scale from the left.
    pub hand_marker_poses: [GameplayMarkerPose; 2],
    pub gold_pose: GameplayMarkerPose,
    pub play_tally_pose: GameplayMarkerPose,
    pub discard_tally_pose: GameplayMarkerPose,
    pub relic_poses: [GameplayMarkerPose; 5],
    pub tile_plinth_poses: [GameplayMarkerPose; 3],
    pub cash_in_btn_rect: (f32, f32, f32, f32),
    /// `structure_tiles_left` / `structure_tiles_right` — per-tile anchor + rotation lerped along the row.
    pub structure_marker_poses: [GameplayMarkerPose; 2],
    /// `yaku_tablets_left` / `yaku_tablets_right` — per-tablet anchor + rotation lerped along the row.
    pub yaku_marker_poses: [GameplayMarkerPose; 2],
    /// Screen bounds for yaku focus / tooltips (derived from [`Self::yaku_marker_poses`]).
    pub yaku_tablet_strip: (f32, f32, f32, f32),
    pub consumable_poses: [GameplayMarkerPose; 2],
    /// Procedural meshes + pick proxies at GLB marker empties (env pass skips marker geometry).
    pub discard_river_pick: Object3d,
    pub play_mirror_pick: Object3d,
    pub journal_pick: Object3d,
}

fn require_marker_pair_screen_rect(
    w: f32,
    h: f32,
    cam: &CameraParams,
    env_h: f32,
    cpu: &RoomGlbCpu,
    left: &str,
    right: &str,
    slot_h: f32,
) -> anyhow::Result<(f32, f32, f32, f32)> {
    let lw = gameplay_glb::require_gameplay_marker_world(h, env_h, cpu, left)?;
    let rw = gameplay_glb::require_gameplay_marker_world(h, env_h, cpu, right)?;
    let (lx, ly) = cam.project_world_to_screen(w, h, lw);
    let (rx, _) = cam.project_world_to_screen(w, h, rw);
    let left_x = lx.min(rx);
    let right_x = lx.max(rx);
    let strip_w = (right_x - left_x).max(8.0);
    Ok((left_x, ly - slot_h * 0.5, strip_w, slot_h))
}

/// Lerp parameter along `hand_tiles_left` → `hand_tiles_right` for slot `index`.
fn hand_marker_lerp_t(hand_len: usize, ref_count: usize, index: usize) -> f32 {
    if hand_len == 1 {
        return 0.5;
    }
    let ref_n = ref_count.max(2);
    if hand_len <= ref_count {
        let center = (ref_count - hand_len) as f32 * 0.5;
        (center + index as f32) / (ref_n - 1) as f32
    } else {
        index as f32 / (hand_len - 1) as f32
    }
}

fn hand_slots_from_markers(
    layout: &crate::ui::layout::LayoutResult,
    hand_len: usize,
    strip: (f32, f32, f32, f32),
) -> Vec<(f32, f32, f32, f32)> {
    if hand_len == 0 {
        return Vec::new();
    }
    let (sx, sy, sw, sh) = strip;
    let ref_count = layout.hand_slots.len().max(1);
    if hand_len <= ref_count {
        let slot_w = sw / ref_count as f32;
        let center_offset = if hand_len < ref_count {
            ((ref_count - hand_len) as f32 * slot_w) * 0.5
        } else {
            0.0
        };
        return (0..hand_len)
            .map(|i| (sx + center_offset + i as f32 * slot_w, sy, slot_w, sh))
            .collect();
    }
    let slot_w = sw / hand_len as f32;
    (0..hand_len)
        .map(|i| (sx + i as f32 * slot_w, sy, slot_w, sh))
        .collect()
}

fn require_hand_world_slots_from_markers(
    hand_len: usize,
    ref_count: usize,
    hand_marker_poses: &[GameplayMarkerPose; 2],
) -> Vec<HandWorldSlot> {
    if hand_len == 0 {
        return Vec::new();
    }
    let a_l = hand_marker_poses[0].anchor;
    let a_r = hand_marker_poses[1].anchor;
    let strip_w = gameplay_glb::marker_pair_span_px(a_l, a_r);
    let ref_n = ref_count.max(1);
    let slot_w_px = if hand_len <= ref_count {
        strip_w / ref_n as f32
    } else {
        strip_w / hand_len as f32
    };
    let rot_l = hand_marker_poses[0].rotation_rad;
    let rot_r = hand_marker_poses[1].rotation_rad;

    (0..hand_len)
        .map(|i| {
            let t = hand_marker_lerp_t(hand_len, ref_count, i);
            let anchor = gameplay_glb::lerp_marker_anchor(a_l, a_r, t);
            let rotation = gameplay_glb::lerp_marker_rotation_rad(rot_l, rot_r, t);
            (anchor, slot_w_px, rotation)
        })
        .collect()
}

fn collect_marker_poses<const N: usize>(
    w: f32,
    h: f32,
    env_h: f32,
    cpu: &RoomGlbCpu,
    names: &[&str; N],
) -> anyhow::Result<[GameplayMarkerPose; N]> {
    let mut out = [GameplayMarkerPose {
        anchor: [0.0; 3],
        rotation_rad: [0.0; 3],
        scale: GameplayMarkerPose::UNIT_SCALE,
    }; N];
    for (slot, name) in out.iter_mut().zip(names.iter()) {
        *slot = gameplay_glb::require_gameplay_marker_pose(w, h, env_h, cpu, name)?;
    }
    Ok(out)
}

/// Build anchors for the current window. Errors when any required empty is absent.
pub fn resolve_gameplay_glb_anchors(
    layout: &crate::ui::layout::LayoutResult,
    hand_len: usize,
    w: f32,
    h: f32,
    cam: &CameraParams,
    env_h: f32,
    yaku_panel_h: f32,
) -> anyhow::Result<GameplayGlbAnchors> {
    gameplay_glb::with_gameplay_glb_cpu(|cpu| {
        let cpu = cpu.ok_or_else(|| {
            anyhow::anyhow!("gameplay.glb anchors requested but room GLB is not loaded")
        })?;
        resolve_gameplay_glb_anchors_from_cpu(
            cpu,
            layout,
            hand_len,
            w,
            h,
            cam,
            env_h,
            yaku_panel_h,
        )
    })
}

fn resolve_gameplay_glb_anchors_from_cpu(
    cpu: &RoomGlbCpu,
    layout: &crate::ui::layout::LayoutResult,
    hand_len: usize,
    w: f32,
    h: f32,
    cam: &CameraParams,
    env_h: f32,
    yaku_panel_h: f32,
) -> anyhow::Result<GameplayGlbAnchors> {
    let slot_h = layout.hand_strip.h;
    let hand_strip = require_marker_pair_screen_rect(
        w,
        h,
        cam,
        env_h,
        cpu,
        HAND_TILES_LEFT,
        HAND_TILES_RIGHT,
        slot_h,
    )?;
    let hand_slots = hand_slots_from_markers(layout, hand_len, hand_strip);
    let hand_marker_poses = [
        gameplay_glb::require_gameplay_marker_pose(w, h, env_h, cpu, HAND_TILES_LEFT)?,
        gameplay_glb::require_gameplay_marker_pose(w, h, env_h, cpu, HAND_TILES_RIGHT)?,
    ];
    let hand_ref_count = layout.hand_slots.len().max(1);
    let hand_world_slots =
        require_hand_world_slots_from_markers(hand_len, hand_ref_count, &hand_marker_poses);
    let gold_pose = gameplay_glb::require_gameplay_marker_pose(w, h, env_h, cpu, PLAYER_GOLD)?;

    let layout_scale = (w.min(h)) / 600.0;
    let bowl_d = (120.0 * layout_scale).max(48.0);
    let discard_btn_fallback_rect = gameplay_glb::gameplay_marker_screen_rect_resolved(
        w,
        h,
        cam,
        env_h,
        cpu,
        DISCARD_RIVER,
        bowl_d,
        bowl_d,
    )?;
    let play_btn_fallback_rect = gameplay_glb::gameplay_marker_screen_rect_resolved(
        w,
        h,
        cam,
        env_h,
        cpu,
        PLAY_MIRROR,
        bowl_d,
        bowl_d,
    )?;

    let play_tally_pose =
        gameplay_glb::require_gameplay_marker_pose(w, h, env_h, cpu, PLAYER_PLAY_TALLY)?;
    let discard_tally_pose =
        gameplay_glb::require_gameplay_marker_pose(w, h, env_h, cpu, PLAYER_DISCARD_TALLY)?;

    let relic_poses = collect_marker_poses(w, h, env_h, cpu, &PLAYER_RELIC_MARKERS)?;
    let tile_plinth_poses = collect_marker_poses(w, h, env_h, cpu, &TILE_PLINTH_MARKERS)?;
    let consumable_poses = collect_marker_poses(w, h, env_h, cpu, &PLAYER_CONSUMABLE_MARKERS)?;

    let structure_marker_poses = [
        gameplay_glb::require_gameplay_marker_pose(w, h, env_h, cpu, STRUCTURE_TILES_LEFT)?,
        gameplay_glb::require_gameplay_marker_pose(w, h, env_h, cpu, STRUCTURE_TILES_RIGHT)?,
    ];
    let yaku_marker_poses = [
        gameplay_glb::require_gameplay_marker_pose(w, h, env_h, cpu, YAKU_TABLETS_LEFT)?,
        gameplay_glb::require_gameplay_marker_pose(w, h, env_h, cpu, YAKU_TABLETS_RIGHT)?,
    ];
    let yaku_tablet_strip = gameplay_glb::marker_pair_screen_rect_from_poses(
        &yaku_marker_poses[0],
        &yaku_marker_poses[1],
        yaku_panel_h,
    );

    let cash_in_w = (96.0 * layout_scale).max(72.0);
    let cash_in_h = (36.0 * layout_scale).max(24.0);
    let cash_in_btn_rect = gameplay_glb::gameplay_marker_screen_rect_resolved(
        w,
        h,
        cam,
        env_h,
        cpu,
        BTN_CASH_IN,
        cash_in_w,
        cash_in_h,
    )?;

    let discard_river_pick =
        gameplay_glb::gameplay_pick_discard_river(w, h, env_h, cpu, discard_btn_fallback_rect)?;
    let play_mirror_pick =
        gameplay_glb::gameplay_pick_play_mirror(w, h, env_h, cpu, play_btn_fallback_rect)?;
    let journal_pick = gameplay_glb::gameplay_pick_journal_book(w, h, env_h, cpu, 0.0)?;

    Ok(GameplayGlbAnchors {
        hand_slots,
        hand_world_slots,
        hand_marker_poses,
        gold_pose,
        play_tally_pose,
        discard_tally_pose,
        relic_poses,
        tile_plinth_poses,
        cash_in_btn_rect: cash_in_btn_rect.into(),
        structure_marker_poses,
        yaku_marker_poses,
        yaku_tablet_strip,
        consumable_poses,
        discard_river_pick,
        play_mirror_pick,
        journal_pick,
    })
}

/// Screen center of relic slot `idx` from `player_relic` empties.
pub fn relic_tray_screen_center(
    w: f32,
    h: f32,
    env_height_scale: f32,
    idx: usize,
) -> anyhow::Result<(f32, f32)> {
    let name = PLAYER_RELIC_MARKERS
        .get(idx)
        .copied()
        .ok_or_else(|| anyhow::anyhow!("relic slot index {idx} out of range"))?;
    gameplay_glb::with_gameplay_glb_cpu(|cpu| {
        let cpu = cpu.ok_or_else(|| anyhow::anyhow!("gameplay.glb not loaded"))?;
        let pose = gameplay_glb::require_gameplay_marker_pose(w, h, env_height_scale, cpu, name)?;
        Ok((pose.anchor[0], pose.anchor[1]))
    })
}

/// `player_gold` surface anchor for coin pile / flying coins.
pub fn resolve_player_gold_anchor(
    w: f32,
    h: f32,
    env_height_scale: f32,
) -> anyhow::Result<[f32; 3]> {
    resolve_player_gold_pose(w, h, env_height_scale).map(|p| p.anchor)
}

/// `player_gold` spawn pose for coin pile / flying coins.
pub fn resolve_player_gold_pose(
    w: f32,
    h: f32,
    env_height_scale: f32,
) -> anyhow::Result<GameplayMarkerPose> {
    gameplay_glb::with_gameplay_glb_cpu(|cpu| {
        let cpu = cpu.ok_or_else(|| anyhow::anyhow!("gameplay.glb not loaded"))?;
        gameplay_glb::require_gameplay_marker_pose(w, h, env_height_scale, cpu, PLAYER_GOLD)
    })
}

/// Minimal error frame when `gameplay.glb` failed validation.
pub fn gameplay_glb_error_frame(
    layout: &crate::ui::layout::LayoutResult,
    message: &str,
) -> crate::render::draw_cmd::UiFrame {
    use crate::render::draw_cmd::UiFrame;
    use crate::scenes::BackgroundId;

    let mut frame = UiFrame::new();
    frame.background(BackgroundId::Black);
    let w = layout.window_w;
    let h = layout.window_h;
    let mut label = TextLabel::default();
    label.rect = [w * 0.08, h * 0.35, w * 0.84, h * 0.3];
    label.text = format!("gameplay.glb failed to load:\n{message}");
    label.color = color::alpha(color::CHAMPAGNE, 0.95);
    label.font_px = Some((h * 0.028).max(18.0));
    label.align = TextAlign::Center;
    frame.texts(vec![label]);
    frame
}

#[cfg(test)]
mod tests {
    use crate::render::gameplay_glb::{self, load_gameplay_glb_from_bytes, validate_gameplay_glb};
    use crate::render::world_space::layout_anchor_to_world;

    #[test]
    fn hand_marker_lerp_t_centers_short_hands() {
        assert!((super::hand_marker_lerp_t(10, 14, 0) - 2.0 / 13.0).abs() < 1e-4);
        assert!((super::hand_marker_lerp_t(10, 14, 9) - 11.0 / 13.0).abs() < 1e-4);
        assert!((super::hand_marker_lerp_t(14, 14, 0) - 0.0).abs() < 1e-4);
        assert!((super::hand_marker_lerp_t(14, 14, 13) - 1.0).abs() < 1e-4);
        assert!((super::hand_marker_lerp_t(1, 14, 0) - 0.5).abs() < 1e-4);
    }

    #[test]
    fn gameplay_hand_slots_project_on_screen() {
        let bytes = include_bytes!("../../../assets/3d/gameplay.glb");
        let cpu = load_gameplay_glb_from_bytes(bytes).expect("gameplay.glb");
        let cpu = validate_gameplay_glb(cpu).expect("required gameplay.glb spawn empties");
        let w = 1920.0_f32;
        let h = 1080.0_f32;
        let env_h = 1.0_f32;
        let layout = {
            let mut solver = crate::ui::layout::UiLayout::new();
            solver.solve(w, h)
        };
        let cam = gameplay_glb::require_gameplay_camera_from_cpu(&cpu, h, env_h)
            .expect("embedded gameplay camera");
        let hand_len = 14;
        let anchors = super::resolve_gameplay_glb_anchors_from_cpu(
            &cpu, &layout, hand_len, w, h, &cam, env_h, 48.0,
        )
        .expect("anchors");
        assert_eq!(anchors.hand_world_slots.len(), hand_len, "hand slots");
        let left = anchors.hand_marker_poses[0].anchor;
        let right = anchors.hand_marker_poses[1].anchor;
        for (i, &(center, slot_w, rot)) in anchors.hand_world_slots.iter().enumerate() {
            assert!(
                center[0].is_finite() && center[1].is_finite() && center[2].is_finite(),
                "slot {i} finite"
            );
            assert!(
                center[0] >= -w && center[0] <= w * 2.0 && center[1] >= -h && center[1] <= h * 2.0,
                "slot {i} on screen-ish: {:?}",
                center
            );
            assert!(slot_w > 0.0, "slot {i} width");
            assert!(
                rot.iter().all(|r| r.is_finite()),
                "slot {i} marker rotation finite: {rot:?}",
            );
            let world =
                layout_anchor_to_world(w, h, Some(&cam), center[0], center[1], center[2], false);
            assert!(world.z.is_finite(), "slot {i} finite world z: {world:?}");
        }
        let (first, _, _) = anchors.hand_world_slots.first().unwrap();
        let (last, _, _) = anchors.hand_world_slots.last().unwrap();
        for (a, b) in [(first, left), (last, right)] {
            assert!(
                (a[0] - b[0]).abs() < 0.5 && (a[1] - b[1]).abs() < 0.5 && (a[2] - b[2]).abs() < 0.5,
                "end slot matches marker anchor: slot {a:?} marker {b:?}",
            );
        }
        assert!(
            last[0] > first[0] || last[1] != first[1],
            "hand strip spans markers"
        );
        assert!(anchors.hand_world_slots[0].1 > 0.0, "slot width");

        let short = super::resolve_gameplay_glb_anchors_from_cpu(
            &cpu, &layout, 10, w, h, &cam, env_h, 48.0,
        )
        .expect("anchors");
        let (short_first, short_w, _) = short.hand_world_slots.first().unwrap();
        let (short_last, _, _) = short.hand_world_slots.last().unwrap();
        let full_w = anchors.hand_world_slots[0].1;
        assert!(
            (short_w - full_w).abs() < 0.01,
            "short hand keeps reference slot width"
        );
        assert!(
            (short_first[0] - left[0]).abs() > 0.05,
            "short hand does not start at left marker"
        );
        assert!(
            (short_last[0] - right[0]).abs() > 0.05,
            "short hand does not end at right marker"
        );
        assert!(
            anchors.hand_marker_poses[0]
                .scale
                .iter()
                .all(|s| s.is_finite() && *s > 0.0),
            "hand marker scale finite positive: {:?}",
            anchors.hand_marker_poses[0].scale
        );
    }
}
