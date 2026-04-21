//! Material Viewer — debug pushdown scene that renders one preview orb per
//! `MaterialKind`. The instance pool binds the 1×1 default relief texture, so
//! materials that would otherwise sample a per-asset heightmap render as their
//! base shading model with no displacement — the scene previews the material
//! itself, not any specific mesh's heightmap.
//!
//! Entered via the debug menu ("Material Viewer..."). Pops back to the
//! previous scene via `OverlayRequest::Pop` when the player presses Back/Escape.

use crate::render::draw_cmd::{CameraParams, Object3d, Object3dKind, UiFrame};
use crate::render::lit_mesh::{MaterialKind, MaterialParams};
use crate::render::theme::{color, typography};
use crate::render::wgpu_renderer::{GpuInstance, PointLight, TextAlign, TextLabel};
use crate::ui::input::UiAction;

use super::start_screen::StartScreenScene;
use super::{BackgroundId, ButtonDef, DrawCtx, Scene, SceneBehavior, SceneTransition, UpdateCtx};

const CLICK_BACK: u32 = 0xE001;

pub struct MaterialViewerScene {
    /// `true` when entered as an overlay from another scene. Controls whether
    /// Back pops the overlay stack or transitions to the start screen.
    has_suspended: bool,
}

impl MaterialViewerScene {
    pub fn new(has_suspended: bool) -> Self {
        Self { has_suspended }
    }

    fn go_back(&self, overlay_request: &mut Option<super::OverlayRequest>) -> SceneTransition {
        if self.has_suspended {
            *overlay_request = Some(super::OverlayRequest::Pop);
            None
        } else {
            Some(Scene::StartScreen(StartScreenScene::new()))
        }
    }
}

impl SceneBehavior for MaterialViewerScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        for &cid in ctx.button_clicks {
            if cid == CLICK_BACK {
                return self.go_back(ctx.overlay_request);
            }
        }
        for a in ctx.actions {
            if matches!(a, UiAction::Cancel | UiAction::Pause) {
                return self.go_back(ctx.overlay_request);
            }
        }
        None
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let ui_scale = ctx.ui_scale;
        let scale = (w.min(h)) / 600.0 * ui_scale;

        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);

        // ── Camera ────────────────────────────────────────────────
        // Front-facing camera sized to the window so pixel-space object
        // placements map directly to what the player sees. Matches the
        // meld guide's approach.
        let cam_scale = h / 1600.0;
        frame.camera_override = Some(CameraParams {
            eye: [0.0, -200.0 * cam_scale, 2040.0 * cam_scale],
            target: [0.0, -50.0 * cam_scale, 0.0],
            up: [0.0, 0.0, 1.0],
            fovy_deg: 45.0,
        });

        // ── Title ─────────────────────────────────────────────────
        let title_font = typography::size(typography::TITLE, h, ui_scale).max(24.0);
        let title_h = title_font * 1.6;
        let title_y = h * 0.04;
        frame.text(TextLabel {
            rect: [0.0, title_y, w, title_h],
            text: "Material Viewer".into(),
            color: color::CHAMPAGNE,
            align: TextAlign::Center,
            font_px: Some(title_font),
            ..Default::default()
        });

        // ── Grid of orbs ──────────────────────────────────────────
        let entries = material_entries();
        let cols = 5usize;
        let rows = entries.len().div_ceil(cols);

        // Layout the grid inside a central band that leaves room for title up
        // top and the Back button at the bottom.
        let grid_top = title_y + title_h + h * 0.04;
        let grid_bottom = h * 0.86;
        let grid_h = (grid_bottom - grid_top).max(1.0);
        let grid_w = w * 0.88;
        let grid_left = (w - grid_w) * 0.5;

        let cell_w = grid_w / cols as f32;
        let cell_h = grid_h / rows as f32;
        // Orb diameter sized to a comfortable fraction of cell width; caption
        // sits under the orb in pixel space.
        let orb_diameter = (cell_w.min(cell_h) * 0.62).max(40.0);
        let label_font = typography::size(typography::CAPTION, h, ui_scale).max(12.0);
        let label_h = label_font * 1.3;

        // Lights arranged above the grid so every orb gets similar illumination.
        for (dx, dy) in &[(0.25f32, 0.20f32), (0.75, 0.20), (0.50, 0.55)] {
            frame.point_lights.push(PointLight {
                pos: [w * dx, h * dy, h * 0.6],
                radius: h * 1.5,
                color: [1.0, 0.97, 0.90],
                intensity: 2.2,
            });
        }

        let mut orbs: Vec<Object3d> = Vec::with_capacity(entries.len());
        for (i, entry) in entries.iter().enumerate() {
            let col = i % cols;
            let row = i / cols;
            let cx = grid_left + (col as f32 + 0.5) * cell_w;
            // Shift the orb up inside its cell so the caption fits under it.
            let orb_cy = grid_top + row as f32 * cell_h + cell_h * 0.42;

            orbs.push(Object3d {
                pos: [cx, orb_cy, 0.0],
                extents: [orb_diameter, orb_diameter, orb_diameter],
                rotation: glam::Mat4::IDENTITY,
                color: [1.0, 1.0, 1.0, 1.0],
                kind: Object3dKind::MaterialOrb {
                    material: entry.material,
                },
                focusable: false,
                scene_shaded: true,
                own_light: None,
                hover_target: 0.0,
                anim_id: 0,
                arrange_name: None,
            });

            // Caption under the orb.
            let caption_y = orb_cy + orb_diameter * 0.58 + 6.0 * scale;
            frame.text(TextLabel {
                rect: [grid_left + col as f32 * cell_w, caption_y, cell_w, label_h],
                text: entry.label.into(),
                color: color::PARCHMENT,
                align: TextAlign::Center,
                font_px: Some(label_font),
                ..Default::default()
            });
        }
        frame.object3d_batch(orbs);

        // ── Back button ───────────────────────────────────────────
        let btn_font = typography::size(typography::BODY, h, ui_scale).max(16.0);
        let btn_h = (44.0 * scale).max(32.0);
        let btn_w = (160.0 * scale).max(100.0);
        let btn_y = h * 0.91;
        let btn_x = (w - btn_w) * 0.5;
        frame.quad(GpuInstance {
            rect: [btn_x, btn_y, btn_w, btn_h],
            color: color::OBSIDIAN,
        });
        frame.text(TextLabel {
            rect: [btn_x, btn_y, btn_w, btn_h],
            text: "Back".into(),
            color: color::CHAMPAGNE,
            align: TextAlign::Center,
            font_px: Some(btn_font),
            ..Default::default()
        });
        frame
            .buttons
            .push(ButtonDef::scene((btn_x, btn_y, btn_w, btn_h), CLICK_BACK));

        frame.window_title = "Mahjuro \u{2014} Material Viewer".into();
        frame
    }
}

struct MaterialEntry {
    label: &'static str,
    material: MaterialParams,
}

/// One entry per `MaterialKind`. `base_color` and specular values are chosen
/// to read cleanly on a sphere — they don't have to match how the game uses
/// the material in context.
fn material_entries() -> Vec<MaterialEntry> {
    use MaterialKind::*;
    let mk = |kind: MaterialKind, rgb: [f32; 3], spec_s: f32, spec_p: f32| MaterialParams {
        kind,
        base_color: [rgb[0], rgb[1], rgb[2], 1.0],
        specular_strength: spec_s,
        specular_power: spec_p,
    };
    vec![
        MaterialEntry {
            label: "Plain",
            material: mk(Plain, [0.85, 0.85, 0.85], 0.2, 32.0),
        },
        MaterialEntry {
            label: "Wax",
            material: MaterialParams::wax(),
        },
        MaterialEntry {
            label: "Wick",
            material: MaterialParams::wick(),
        },
        MaterialEntry {
            label: "Lacquered Wood",
            material: MaterialParams::lacquered_wood(),
        },
        MaterialEntry {
            label: "Lacq. Wood Flat",
            material: mk(LacqueredWoodFlat, [1.0, 1.0, 1.0], 0.55, 96.0),
        },
        MaterialEntry {
            label: "Metal",
            material: mk(Metal, [0.85, 0.70, 0.40], 1.0, 96.0),
        },
        MaterialEntry {
            label: "Water",
            material: mk(Water, [0.20, 0.40, 0.65], 0.8, 64.0),
        },
        MaterialEntry {
            label: "Foil",
            material: mk(Foil, [0.85, 0.85, 0.90], 1.0, 96.0),
        },
        MaterialEntry {
            label: "Glass",
            material: mk(Glass, [1.00, 0.92, 0.60], 1.0, 128.0),
        },
        MaterialEntry {
            label: "Enamel",
            material: mk(Enamel, [0.85, 0.25, 0.30], 0.9, 96.0),
        },
        MaterialEntry {
            label: "Jade",
            material: mk(Jade, [0.35, 0.70, 0.50], 0.6, 64.0),
        },
        MaterialEntry {
            label: "Moonstone",
            material: mk(Moonstone, [0.90, 0.92, 1.00], 0.8, 96.0),
        },
        MaterialEntry {
            label: "Pearl",
            material: mk(Pearl, [0.95, 0.92, 0.90], 0.7, 96.0),
        },
        MaterialEntry {
            label: "Gold Nugget",
            material: mk(GoldNugget, [0.95, 0.78, 0.30], 1.0, 96.0),
        },
        MaterialEntry {
            label: "Polychrome",
            material: mk(Polychrome, [0.80, 0.80, 0.85], 0.9, 96.0),
        },
        MaterialEntry {
            label: "Porcelain",
            material: mk(Porcelain, [0.95, 0.94, 0.92], 0.7, 128.0),
        },
    ]
}
