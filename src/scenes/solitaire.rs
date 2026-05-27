//! Mahjong solitaire (Shanghai-style) — pair-matching mode accessible from the
//! start screen. Independent of the main run; does not touch RunState.
//!
//! Layout: classic 144-tile turtle across 5 layers (87 / 36 / 16 / 4 / 1).
//! Tile set is the full standard mahjong distribution including the 4 flowers
//! and 4 seasons (each unique, but matching within their group).
//! A tile is "free" when nothing rests directly on top of it AND it has at
//! least one open side (no left or no right neighbor on its own layer).

use crate::core::tile::{Suit, Tile};
use crate::render::draw_cmd::{CameraParams, DrawCmd, ShowcaseTilePlacement, UiFrame};
use crate::render::theme::color;
use crate::render::wgpu_renderer::{GpuInstance, PointLight, TextLabel};
use crate::ui::input::UiAction;

use super::main_menu_exterior::MainMenuExteriorScene;
use super::{BackgroundId, ButtonDef, DrawCtx, Scene, SceneBehavior, SceneTransition, UpdateCtx};

// ── Layout definition ──────────────────────────────────────────────

/// Layer 0 (the body) — column ranges per row. Sums to 87.
/// Cols are in a 16-wide grid; rows 0..8.
const BODY_ROWS: [(i32, i32); 8] = [
    (2, 14), // row 0: 12
    (4, 12), // row 1:  8
    (3, 13), // row 2: 10
    (1, 15), // row 3: 14 (ears)
    (1, 14), // row 4: 13 (left ear + tail)
    (3, 13), // row 5: 10
    (4, 12), // row 6:  8
    (2, 14), // row 7: 12
];
// 12 + 8 + 10 + 14 + 13 + 10 + 8 + 12 = 87

const GRID_COLS: i32 = 16;
const GRID_ROWS: i32 = 8;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Face {
    /// Standard tile (Manzu/Souzu/Pinzu/Wind/Dragon).
    Standard(Suit, u8),
    /// Bonus flower 1..=4. All flowers match each other.
    Flower(u8),
    /// Bonus season 1..=4. All seasons match each other.
    Season(u8),
}

impl Face {
    fn matches(self, other: Face) -> bool {
        match (self, other) {
            (Face::Standard(s1, r1), Face::Standard(s2, r2)) => s1 == s2 && r1 == r2,
            (Face::Flower(_), Face::Flower(_)) => true,
            (Face::Season(_), Face::Season(_)) => true,
            _ => false,
        }
    }

    fn to_tile(self, id: u32) -> Tile {
        match self {
            Face::Standard(suit, rank) => Tile::new(suit, rank, id),
            Face::Flower(n) => Tile::new(Suit::Flower, n, id),
            Face::Season(n) => Tile::new(Suit::Season, n, id),
        }
    }
}

#[derive(Clone, Copy)]
struct Slot {
    layer: usize,
    col: i32,
    row: i32,
    face: Option<Face>,
}

pub struct SolitaireScene {
    slots: Vec<Slot>,
    selected: Option<usize>,
    pairs_matched: u32,
    finished: Option<Finished>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Finished {
    Won,
    Stuck,
}

const CLICK_NEW_GAME: u32 = 1_000_000;
const CLICK_BACK: u32 = 1_000_001;

impl SolitaireScene {
    pub fn new() -> Self {
        let mut scene = Self {
            slots: Vec::new(),
            selected: None,
            pairs_matched: 0,
            finished: None,
        };
        scene.deal();
        scene
    }

    /// Build all 144 turtle slots and assign a shuffled standard tile set.
    fn deal(&mut self) {
        let mut slots: Vec<Slot> = Vec::new();

        // Layer 0 — body (87 tiles).
        for (r, &(cs, ce)) in BODY_ROWS.iter().enumerate() {
            for col in cs..ce {
                slots.push(Slot {
                    layer: 0,
                    col,
                    row: r as i32,
                    face: None,
                });
            }
        }
        // Layer 1 — 6×6 (36).
        for row in 1..7 {
            for col in 5..11 {
                slots.push(Slot {
                    layer: 1,
                    col,
                    row,
                    face: None,
                });
            }
        }
        // Layer 2 — 4×4 (16).
        for row in 2..6 {
            for col in 6..10 {
                slots.push(Slot {
                    layer: 2,
                    col,
                    row,
                    face: None,
                });
            }
        }
        // Layer 3 — 2×2 (4).
        for row in 3..5 {
            for col in 7..9 {
                slots.push(Slot {
                    layer: 3,
                    col,
                    row,
                    face: None,
                });
            }
        }
        // Layer 4 — single cap.
        slots.push(Slot {
            layer: 4,
            col: 7,
            row: 3,
            face: None,
        });

        debug_assert_eq!(slots.len(), 144);

        let mut faces = standard_faces();
        debug_assert_eq!(faces.len(), 144);

        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0xDEAD_BEEF);
        shuffle(&mut faces, seed);

        for (slot, face) in slots.iter_mut().zip(faces.into_iter()) {
            slot.face = Some(face);
        }

        self.slots = slots;
        self.selected = None;
        self.pairs_matched = 0;
        self.finished = None;
    }

    /// True if any other present tile is in a strictly higher layer at the
    /// same (col, row). Used both to gate selection and to suppress label
    /// rendering for covered tiles.
    fn covered_by_top(&self, idx: usize) -> bool {
        let s = self.slots[idx];
        self.slots
            .iter()
            .any(|o| o.face.is_some() && o.layer > s.layer && o.col == s.col && o.row == s.row)
    }

    fn is_free(&self, idx: usize) -> bool {
        let s = self.slots[idx];
        if s.face.is_none() {
            return false;
        }
        if self.covered_by_top(idx) {
            return false;
        }
        let left_blocked = self.slots.iter().any(|o| {
            o.face.is_some() && o.layer == s.layer && o.row == s.row && o.col == s.col - 1
        });
        let right_blocked = self.slots.iter().any(|o| {
            o.face.is_some() && o.layer == s.layer && o.row == s.row && o.col == s.col + 1
        });
        !left_blocked || !right_blocked
    }

    fn try_match(&mut self, idx: usize) {
        if !self.is_free(idx) {
            return;
        }
        let Some(picked) = self.slots[idx].face else {
            return;
        };
        match self.selected {
            None => self.selected = Some(idx),
            Some(prev) if prev == idx => self.selected = None,
            Some(prev) => {
                let prev_face = match self.slots[prev].face {
                    Some(f) => f,
                    None => {
                        self.selected = Some(idx);
                        return;
                    }
                };
                if picked.matches(prev_face) {
                    self.slots[prev].face = None;
                    self.slots[idx].face = None;
                    self.pairs_matched += 1;
                    self.selected = None;
                    self.check_end_state();
                } else {
                    self.selected = Some(idx);
                }
            }
        }
    }

    fn check_end_state(&mut self) {
        if self.slots.iter().all(|s| s.face.is_none()) {
            self.finished = Some(Finished::Won);
            return;
        }
        let free: Vec<usize> = (0..self.slots.len()).filter(|&i| self.is_free(i)).collect();
        for i in 0..free.len() {
            for j in (i + 1)..free.len() {
                let a = self.slots[free[i]].face.unwrap();
                let b = self.slots[free[j]].face.unwrap();
                if a.matches(b) {
                    return;
                }
            }
        }
        self.finished = Some(Finished::Stuck);
    }
}

impl SceneBehavior for SolitaireScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        for &id in ctx.button_clicks {
            if id == CLICK_NEW_GAME {
                self.deal();
            } else if id == CLICK_BACK {
                return Some(Scene::MainMenuExterior(MainMenuExteriorScene::new()));
            } else if (id as usize) < self.slots.len() {
                self.try_match(id as usize);
            }
        }
        for a in ctx.actions {
            match a {
                UiAction::Cancel | UiAction::Pause => {
                    return Some(Scene::MainMenuExterior(MainMenuExteriorScene::new()));
                }
                _ => {}
            }
        }
        None
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let scale = (w.min(h)) / 600.0;

        let mut frame = UiFrame::new();

        // ── Paper/linen background ────────────────────────────────
        frame.background(BackgroundId::Black);
        frame.quad(GpuInstance {
            rect: [0.0, 0.0, w, h],
            color: [0.92, 0.89, 0.82, 1.0], // warm parchment, user: 0});

        // Top-down-ish camera so the board reads as a flat layout with
        // visible 3D stacking between layers.
        // Scale camera to viewport so the board fills the screen at any
        // resolution.  The fixed values looked right at ~1200px height;
        // scale proportionally from that reference.
        let cam_scale = h / 1600.0;
        frame.camera_override = Some(CameraParams {
            eye: [0.0, -200.0 * cam_scale, 2040.0 * cam_scale],
            target: [0.0, -50.0 * cam_scale, 0.0],
            up: [0.0, 0.0, 1.0],
            fovy_deg: 45.0,
            clip_near: None,
            clip_far: None,
        });
        frame.showcase_render_hints.layout_use_ray_plane_z = true;

        // Bright, broad overhead lights — the tile shader has no ambient,
        // so without these the 3D tiles would be pitch black. Lights must
        // be close to the tile plane (world_y ≈ layer heights) with a
        // radius large enough to cover the board plus vertical distance.
        let light_y = h * 0.20; // just above the top layer
        for &(lx, ly) in &[
            (w * 0.25, h * 0.30),
            (w * 0.75, h * 0.30),
            (w * 0.50, h * 0.50),
            (w * 0.25, h * 0.70),
            (w * 0.75, h * 0.70),
        ] {
            frame.scene_lighting.push_smooth(PointLight {
                pos: [lx, ly, light_y],
                radius: h * 1.2,
                color: color::rgb(color::PARCHMENT), // warm daylight
                intensity: 2.2,
            });
        }

        let mut buttons = Vec::new();

        // ── Header ────────────────────────────────────────────────
        let title_h = (28.0 * scale).max(18.0);
        let title_y = h * 0.02;
        frame.text(TextLabel {
            rect: [0.0, title_y, w, title_h],
            text: "Mahjong Solitaire".into(),
            color: [0.25, 0.22, 0.18, 1.0], // dark ink
            ..Default::default()
        });
        let score_h = (18.0 * scale).max(12.0);
        let score_y = title_y + title_h + h * 0.005;
        let remaining = self.slots.iter().filter(|s| s.face.is_some()).count();
        frame.text(TextLabel {
            rect: [0.0, score_y, w, score_h],
            text: format!(
                "Pairs: {}   |   Tiles left: {}",
                self.pairs_matched, remaining
            ),
            color: [0.45, 0.42, 0.38, 0.9], // muted warm gray
            ..Default::default()
        });

        // ── Board area ────────────────────────────────────────────
        let board_top = score_y + score_h + h * 0.025;
        let board_bottom = h - h * 0.10;
        let board_h = board_bottom - board_top;
        let board_w = w * 0.96;
        let board_x = (w - board_w) * 0.5;

        // Tile size — fit grid into the board area. No 2D layer offset
        // needed; the 3D world_y_lift handles stacking visually.
        let cell_w = board_w / GRID_COLS as f32;
        let cell_h = board_h / GRID_ROWS as f32;
        let target_aspect = 1.50; // canonical Chinese mahjong 30×20 mm
        let (tile_w, tile_h) = {
            let by_w = (cell_w, cell_w * target_aspect);
            let by_h = (cell_h / target_aspect, cell_h);
            if by_w.1 <= cell_h { by_w } else { by_h }
        };

        let used_w = tile_w * GRID_COLS as f32;
        let used_h = tile_h * GRID_ROWS as f32;
        let origin_x = board_x + (board_w - used_w) * 0.5;
        let origin_y = board_top + (board_h - used_h) * 0.5;

        // World-space height per layer for 3D stacking.
        let layer_lift = h * 0.04;

        // ── Build 3D tile placements ──────────────────────────────
        let mut order: Vec<usize> = (0..self.slots.len()).collect();
        order.sort_by_key(|&i| self.slots[i].layer);

        let mut placements = Vec::new();

        for i in order {
            let slot = self.slots[i];
            let Some(face) = slot.face else { continue };

            let px = origin_x + slot.col as f32 * tile_w + tile_w * 0.5;
            let py = origin_y + slot.row as f32 * tile_h + tile_h * 0.5;
            let lift = slot.layer as f32 * layer_lift;
            let free = self.is_free(i);

            placements.push(ShowcaseTilePlacement {
                tile: face.to_tile(i as u32),
                center_pos: [px, py, lift],
                rotation: [0.0, 0.0, std::f32::consts::PI],
                scale: 1.0,
                size_px: tile_w,
                brightness: if free { 1.0 } else { 0.55 },
                selected: self.selected == Some(i),
                hovered: false,
                outline: false,
                glow: false,
                glow_color: None,
                pick_id: None,
            });

            // Hit-test button for free tiles.
            if free && self.finished.is_none() {
                buttons.push(ButtonDef::scene(
                    (px - tile_w * 0.5, py - tile_h * 0.5, tile_w, tile_h),
                    i as u32,
                ));
            }
        }

        frame.cmds.push(DrawCmd::ShowcaseTileBatch(placements));

        // ── Footer buttons ────────────────────────────────────────
        let btn_h = (28.0 * scale).max(20.0);
        let btn_w = (140.0 * scale).min(w * 0.32);
        let btn_y = h - btn_h - h * 0.025;

        let back_x = w * 0.04;
        frame.quad(GpuInstance {
            rect: [back_x, btn_y, btn_w, btn_h],
            color: [0.60, 0.30, 0.25, 0.90], // muted red-brown, user: 0});
        frame.text(TextLabel {
            rect: [back_x, btn_y, btn_w, btn_h],
            text: "< Back".into(),
            color: color::PARCHMENT,
            ..Default::default()
        });
        buttons.push(ButtonDef::scene((back_x, btn_y, btn_w, btn_h), CLICK_BACK));

        let new_x = w - btn_w - w * 0.04;
        frame.quad(GpuInstance {
            rect: [new_x, btn_y, btn_w, btn_h],
            color: [0.35, 0.55, 0.35, 0.90], // sage green, user: 0});
        frame.text(TextLabel {
            rect: [new_x, btn_y, btn_w, btn_h],
            text: "New Deal".into(),
            color: color::PARCHMENT,
            ..Default::default()
        });
        buttons.push(ButtonDef::scene(
            (new_x, btn_y, btn_w, btn_h),
            CLICK_NEW_GAME,
        ));

        let hint_h = (14.0 * scale).max(10.0);
        let hint_y = btn_y - hint_h - (4.0 * scale);
        frame.text(TextLabel {
            rect: [0.0, hint_y, w, hint_h],
            text: "Match pairs of identical free tiles.".into(),
            color: [0.50, 0.48, 0.42, 0.7], // warm medium gray
            ..Default::default()
        });

        // ── End-state banner ──────────────────────────────────────
        if let Some(state) = self.finished {
            let banner_w = (340.0 * scale).min(w * 0.7);
            let banner_h = (110.0 * scale).max(64.0);
            let bx = (w - banner_w) * 0.5;
            let by = (h - banner_h) * 0.5;
            frame.quad(GpuInstance {
                rect: [bx - 4.0, by - 4.0, banner_w + 8.0, banner_h + 8.0],
                color: [0.85, 0.75, 0.20, 0.90], user: 0});
            frame.quad(GpuInstance {
                rect: [bx, by, banner_w, banner_h],
                color: color::alpha(color::WALNUT_INK, 0.97), user: 0});
            let (title, subtitle) = match state {
                Finished::Won => ("You Win!", "All tiles cleared."),
                Finished::Stuck => ("No Moves", "No matching free pairs remain."),
            };
            let title_font = (24.0 * scale).max(16.0);
            frame.text(TextLabel {
                rect: [bx, by + banner_h * 0.18, banner_w, title_font],
                text: title.into(),
                color: color::PARCHMENT,
                ..Default::default()
            });
            let sub_font = (14.0 * scale).max(10.0);
            frame.text(TextLabel {
                rect: [bx, by + banner_h * 0.55, banner_w, sub_font],
                text: subtitle.into(),
                color: [0.85, 0.85, 0.95, 1.0],
                ..Default::default()
            });
        }

        frame.buttons = buttons;
        frame.window_title = format!("Mahjuro — Solitaire ({} pairs)", self.pairs_matched);
        frame
    }
}

// ── Helpers ────────────────────────────────────────────────────────

/// Build the full 144-tile mahjong distribution: 108 numbered + 16 winds + 12
/// dragons + 4 flowers + 4 seasons.
fn standard_faces() -> Vec<Face> {
    let mut faces = Vec::with_capacity(144);
    for suit in [Suit::Manzu, Suit::Souzu, Suit::Pinzu] {
        for rank in 1..=9u8 {
            for _ in 0..4 {
                faces.push(Face::Standard(suit, rank));
            }
        }
    }
    for rank in 1..=4u8 {
        for _ in 0..4 {
            faces.push(Face::Standard(Suit::Wind, rank));
        }
    }
    for rank in 1..=3u8 {
        for _ in 0..4 {
            faces.push(Face::Standard(Suit::Dragon, rank));
        }
    }
    for n in 1..=4u8 {
        faces.push(Face::Flower(n));
    }
    for n in 1..=4u8 {
        faces.push(Face::Season(n));
    }
    faces
}

/// Fisher–Yates shuffle backed by a tiny LCG (no extra crate dependency).
fn shuffle<T>(slice: &mut [T], seed: u64) {
    let mut state = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    for i in (1..slice.len()).rev() {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let j = ((state >> 33) as usize) % (i + 1);
        slice.swap(i, j);
    }
}
