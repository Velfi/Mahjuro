//! Resolve gameplay spawn positions from [`gameplay.glb`](../../../assets/3d/gameplay.glb) empties.

use crate::render::draw_cmd::CameraParams;
use crate::render::gameplay_glb::{
    self, DISCARD_RIVER, HAND_TILES_LEFT, HAND_TILES_RIGHT, PLAY_MIRROR, PLAYER_DISCARD_TALLY,
    PLAYER_GOLD, PLAYER_PLAY_TALLY, PLAYER_RELIC_MARKERS, PLAYER_YAKU_JOURNAL, TILE_PLINTH_MARKERS,
};
use crate::render::room_glb::RoomGlbCpu;

/// Per-frame anchors derived from `gameplay.glb` marker nodes (screen space + lift).
pub struct GameplayGlbAnchors {
    pub hand_slots: Vec<(f32, f32, f32, f32)>,
    pub gold_anchor: [f32; 3],
    pub discard_btn_rect: (f32, f32, f32, f32),
    pub play_btn_rect: (f32, f32, f32, f32),
    pub journal_btn_rect: (f32, f32, f32, f32),
    pub journal_btn_cx: f32,
    pub discard_tally_anchor: [f32; 3],
    pub play_tally_anchor: [f32; 3],
    pub relic_anchors: Vec<[f32; 3]>,
    pub tile_plinth_anchors: Vec<[f32; 3]>,
}

fn marker_pair_screen_rect(
    w: f32,
    h: f32,
    cam: &CameraParams,
    env_h: f32,
    cpu: &RoomGlbCpu,
    left: &str,
    right: &str,
    slot_h: f32,
) -> Option<(f32, f32, f32, f32)> {
    let lw = gameplay_glb::gameplay_marker_world(h, env_h, cpu, left)?;
    let rw = gameplay_glb::gameplay_marker_world(h, env_h, cpu, right)?;
    let (lx, ly) = cam.project_world_to_screen(w, h, lw);
    let (rx, _) = cam.project_world_to_screen(w, h, rw);
    let left_x = lx.min(rx);
    let right_x = lx.max(rx);
    let strip_w = (right_x - left_x).max(8.0);
    Some((left_x, ly - slot_h * 0.5, strip_w, slot_h))
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
    if hand_len <= layout.hand_slots.len() {
        let center_offset = if hand_len < layout.hand_slots.len() {
            ((layout.hand_slots.len() - hand_len) as f32 * sw / hand_len as f32) * 0.5
        } else {
            0.0
        };
        let slot_w = sw / hand_len as f32;
        return (0..hand_len)
            .map(|i| (sx + center_offset + i as f32 * slot_w, sy, slot_w, sh))
            .collect();
    }
    let slot_w = sw / hand_len as f32;
    (0..hand_len)
        .map(|i| (sx + i as f32 * slot_w, sy, slot_w, sh))
        .collect()
}

/// Build anchors for the current window; returns `None` when the room GLB is absent.
pub fn resolve_gameplay_glb_anchors(
    layout: &crate::ui::layout::LayoutResult,
    hand_len: usize,
    w: f32,
    h: f32,
    cam: &CameraParams,
    env_h: f32,
) -> Option<GameplayGlbAnchors> {
    gameplay_glb::with_gameplay_glb_cpu(|cpu| {
        let cpu = cpu?;
        let slot_h = layout.hand_strip.h;
        let hand_strip = marker_pair_screen_rect(
            w, h, cam, env_h, cpu, HAND_TILES_LEFT, HAND_TILES_RIGHT, slot_h,
        )?;
        let hand_slots = hand_slots_from_markers(layout, hand_len, hand_strip);
        let gold_anchor = gameplay_glb::gameplay_marker_surface_anchor(w, h, env_h, cpu, PLAYER_GOLD)?;

        let bowl_d = (layout.mm(120.0)).max(48.0);
        let discard_btn_rect = gameplay_glb::gameplay_marker_screen_rect(
            w, h, cam, env_h, cpu, DISCARD_RIVER, bowl_d, bowl_d,
        )?;
        let play_btn_rect = gameplay_glb::gameplay_marker_screen_rect(
            w, h, cam, env_h, cpu, PLAY_MIRROR, bowl_d, bowl_d,
        )?;

        let journal = gameplay_glb::gameplay_marker_surface_anchor(w, h, env_h, cpu, PLAYER_YAKU_JOURNAL)?;
        let jw = layout.mm(70.0);
        let jh = layout.mm(32.0);
        let journal_btn_rect = (
            journal[0] - jw * 0.5,
            journal[1] - jh * 0.5,
            jw,
            jh,
        );
        let journal_btn_cx = journal[0];

        let discard_tally_anchor =
            gameplay_glb::gameplay_marker_surface_anchor(w, h, env_h, cpu, PLAYER_DISCARD_TALLY)?;
        let play_tally_anchor =
            gameplay_glb::gameplay_marker_surface_anchor(w, h, env_h, cpu, PLAYER_PLAY_TALLY)?;

        let relic_anchors: Vec<[f32; 3]> = PLAYER_RELIC_MARKERS
            .iter()
            .filter_map(|name| gameplay_glb::gameplay_marker_surface_anchor(w, h, env_h, cpu, name))
            .collect();

        let tile_plinth_anchors: Vec<[f32; 3]> = TILE_PLINTH_MARKERS
            .iter()
            .filter_map(|name| gameplay_glb::gameplay_marker_surface_anchor(w, h, env_h, cpu, name))
            .collect();

        Some(GameplayGlbAnchors {
            hand_slots,
            gold_anchor,
            discard_btn_rect: discard_btn_rect.into(),
            play_btn_rect: play_btn_rect.into(),
            journal_btn_rect,
            journal_btn_cx,
            discard_tally_anchor,
            play_tally_anchor,
            relic_anchors,
            tile_plinth_anchors,
        })
    })
}

