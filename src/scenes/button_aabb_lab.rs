//! Debug scene: projected button hit rects (mesh AABB vs model AABB).
//!
//! Entered from Debug → Labs → Button AABB Lab…

use crate::render::archive_glb::{
    self, ARCHIVE_TAB_BUTTON_NODES, BTN_MAIN_MENU, BTN_PAGE_LEFT, BTN_PAGE_RIGHT, BTN_SWITCH_SAVE,
};
use crate::render::draw_cmd::{CameraParams, Object3d, UiFrame};
use crate::render::gameplay_glb::{
    self, BTN_CASH_IN, DISCARD_RIVER, PLAY_MIRROR, gameplay_discard_river_model_screen_rect,
    gameplay_play_mirror_model_screen_rect,
};
use crate::render::room_glb::{
    self, MarkerScreenRectParams, marker_mesh_bounds_reference_object3d,
    screen_rect_for_marker_mesh_bounds,
};
use crate::render::theme::color;
use crate::render::theme::{metrics, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use crate::ui::controller_hints::{HintStyle, back_footer_row, push_screen_footer_hint};
use crate::ui::input::UiAction;

use super::{
    BackgroundId, ButtonDef, DrawCtx, SceneBehavior, SceneIntent, SceneTransition, UpdateCtx,
};

const CLICK_BACK: u32 = 0xE020;
const CLICK_PROBE_BASE: u32 = 0xE100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RoomPreset {
    Archive,
    Gameplay,
}

impl RoomPreset {
    fn label(self) -> &'static str {
        match self {
            Self::Archive => "Archive",
            Self::Gameplay => "Gameplay",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Archive => Self::Gameplay,
            Self::Gameplay => Self::Archive,
        }
    }
}

#[derive(Clone, Copy)]
struct ButtonProbe {
    label: &'static str,
    node: &'static str,
    min_rw: f32,
    min_rh: f32,
    /// When true, also project the spawned pick proxy mesh (discard river / play mirror).
    show_model_aabb: bool,
}

fn archive_probes(w: f32, h: f32) -> Vec<ButtonProbe> {
    let mut list = vec![
        ButtonProbe {
            label: "Main menu",
            node: BTN_MAIN_MENU,
            min_rw: w * 0.11,
            min_rh: h * 0.052,
            show_model_aabb: false,
        },
        ButtonProbe {
            label: "Switch save",
            node: BTN_SWITCH_SAVE,
            min_rw: w * 0.14,
            min_rh: h * 0.052,
            show_model_aabb: false,
        },
        ButtonProbe {
            label: "Page left",
            node: BTN_PAGE_LEFT,
            min_rw: w * 0.08,
            min_rh: h * 0.08,
            show_model_aabb: false,
        },
        ButtonProbe {
            label: "Page right",
            node: BTN_PAGE_RIGHT,
            min_rw: w * 0.08,
            min_rh: h * 0.08,
            show_model_aabb: false,
        },
    ];
    const TAB_LABELS: [&str; 5] = ["Relics", "Talismans", "Yaku", "Bosses", "Chronicle"];
    for (&node, &label) in ARCHIVE_TAB_BUTTON_NODES.iter().zip(TAB_LABELS.iter()) {
        list.push(ButtonProbe {
            label,
            node,
            min_rw: w * 0.09,
            min_rh: h * 0.06,
            show_model_aabb: false,
        });
    }
    list
}

fn gameplay_probes(w: f32, h: f32, layout_scale: f32) -> Vec<ButtonProbe> {
    let bowl_d = (120.0 * layout_scale).max(48.0);
    let cash_in_w = (96.0 * layout_scale).max(72.0);
    let cash_in_h = (36.0 * layout_scale).max(24.0);
    let _ = (w, h);
    vec![
        ButtonProbe {
            label: "Discard river",
            node: DISCARD_RIVER,
            min_rw: bowl_d,
            min_rh: bowl_d,
            show_model_aabb: true,
        },
        ButtonProbe {
            label: "Play mirror",
            node: PLAY_MIRROR,
            min_rw: bowl_d,
            min_rh: bowl_d,
            show_model_aabb: true,
        },
        ButtonProbe {
            label: "Cash in",
            node: BTN_CASH_IN,
            min_rw: cash_in_w,
            min_rh: cash_in_h,
            show_model_aabb: false,
        },
    ]
}

struct ResolvedProbe {
    label: &'static str,
    mesh: Option<[f32; 4]>,
    model: Option<[f32; 4]>,
    /// Lit mesh matching the mesh AABB (green reference cube).
    mesh_ref: Option<Object3d>,
    /// Spawned pick proxy (gameplay discard river / play mirror).
    model_ref: Option<Object3d>,
}

const MESH_REF_TINT: [f32; 4] = [0.25, 0.85, 0.45, 0.35];
const MODEL_REF_TINT: [f32; 4] = [0.95, 0.95, 0.98, 1.0];

pub struct ButtonAabbLabScene {
    has_suspended: bool,
    room: RoomPreset,
}

impl ButtonAabbLabScene {
    pub fn new(has_suspended: bool) -> Self {
        Self {
            has_suspended,
            room: RoomPreset::Archive,
        }
    }

    fn go_back(&self, overlay_request: &mut Option<super::OverlayRequest>) -> SceneTransition {
        if self.has_suspended {
            *overlay_request = Some(super::OverlayRequest::Pop);
            None
        } else {
            Some(SceneIntent::MainMenu)
        }
    }
}

impl SceneBehavior for ButtonAabbLabScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        for &cid in ctx.button_clicks {
            if cid == CLICK_BACK {
                return self.go_back(ctx.overlay_request);
            }
            if (CLICK_PROBE_BASE..CLICK_PROBE_BASE + 32).contains(&cid) {
                log::debug!("Button AABB lab: clicked probe id {cid:#x}");
            }
        }
        for a in ctx.actions {
            match a {
                UiAction::Cancel | UiAction::Pause => {
                    return self.go_back(ctx.overlay_request);
                }
                UiAction::FocusNext | UiAction::FocusDown => {
                    self.room = self.room.next();
                }
                UiAction::FocusPrev | UiAction::FocusUp => {
                    self.room = self.room.next();
                }
                _ => {}
            }
        }
        None
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let scale = metrics::scene_scale(w, h);
        let env_h = ctx.room_gltf_height_scale.max(0.01);

        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);

        let cam = match self.room {
            RoomPreset::Archive => archive_glb::archive_camera_base(w, h, env_h),
            RoomPreset::Gameplay => {
                gameplay_glb::require_gameplay_camera(h, env_h).unwrap_or_else(|e| {
                    log::error!("{e:#}");
                    CameraParams {
                        eye: [0.0, -h * 1.15, h * 0.48],
                        target: [0.0, h * 0.02, h * 0.12],
                        up: [0.0, 0.0, 1.0],
                        projection: crate::render::draw_cmd::CameraProjection::Perspective { fovy_deg: 50.0 },
                        clip_near: None,
                        clip_far: None,
                    }
                })
            }
        };
        frame.camera_override = Some(cam);

        match self.room {
            RoomPreset::Archive => {
                frame.archive_environment();
                frame.archive_page_left_visible = true;
                frame.archive_page_right_visible = true;
                let room_glb = archive_glb::archive_glb_has_embedded_lights();
                frame.scene_lighting.embedded_gltf_punctual = room_glb;
                frame.scene_lighting.room_glb_brdf = true;
                frame.scene_lighting.clear_spot_lights();
                if room_glb {
                    let (punctual, nodes) =
                        crate::render::room_gltf_punctual::tagged_to_scene_punctual(
                            archive_glb::archive_embedded_point_lights_runtime_tagged(
                                w,
                                h,
                                env_h,
                                &ctx.room_env_for(crate::render::scene_keys::ARCHIVE).0,
                            ),
                        );
                    frame.scene_lighting.punctual = punctual;
                    frame.scene_lighting.punctual_gltf_nodes = nodes;
                }
            }
            RoomPreset::Gameplay => {
                frame.gameplay_environment();
                frame.gameplay_cash_in_button_visible = true;
                let room_glb = gameplay_glb::gameplay_glb_has_embedded_lights();
                frame.scene_lighting.embedded_gltf_punctual = room_glb;
                frame.scene_lighting.room_glb_brdf = true;
                if room_glb {
                    let tune = ctx.room_env_for("gameplay").0;
                    let (punctual, nodes) =
                        crate::render::room_gltf_punctual::tagged_to_scene_punctual(
                            gameplay_glb::gameplay_embedded_point_lights_runtime_tagged(
                                w,
                                h,
                                env_h,
                                &tune,
                                0.0,
                                1.0,
                                ctx.flame_tuning.candle_flicker_amp,
                            ),
                        );
                    frame.scene_lighting.punctual = punctual;
                    frame.scene_lighting.punctual_gltf_nodes = nodes;
                    frame.scene_lighting.set_gltf_embedded_spot_lights(
                        gameplay_glb::gameplay_embedded_spot_lights_runtime(w, h, env_h, &tune),
                    );
                }
            }
        }

        let title_font = typography::size(typography::H20, h);
        let title_h = title_font * 1.5;
        let body_font = typography::size(typography::H36, h);
        let label_font = typography::size(typography::H28, h);
        let label_line_h = label_font * 1.2;

        let title_y = h * 0.02;
        frame.text(TextLabel {
            rect: [0.0, title_y, w, title_h],
            text: "Button AABB Lab".into(),
            color: color::CHAMPAGNE,
            align: TextAlign::Center,
            font_px: Some(title_font),
            ..Default::default()
        });

        let legend_y = title_y + title_h + h * 0.006;
        frame.text(TextLabel {
            rect: [w * 0.04, legend_y, w * 0.92, body_font * 4.2],
            text: format!(
                "Green = mesh AABB (hit target) + green cube = that bounds in 3D. \
                 Magenta = spawned model AABB; bowl/mirror are the live pick proxies.\n\
                 Room: {} (↑/↓). Click overlays to log ids. Back / Esc to exit.",
                self.room.label(),
            ),
            color: color::PARCHMENT,
            align: TextAlign::Center,
            font_px: Some(body_font),
            ..Default::default()
        });

        let layout_scale = (w.min(h)) / 600.0;
        let probes = match self.room {
            RoomPreset::Archive => archive_probes(w, h),
            RoomPreset::Gameplay => gameplay_probes(w, h, layout_scale),
        };

        let resolved = resolve_probes(self.room, w, h, &cam, env_h, layout_scale, &probes);
        let mut reference_objects: Vec<Object3d> = Vec::new();
        for (i, probe) in resolved.iter().enumerate() {
            draw_probe_overlays(&mut frame, probe, label_font, label_line_h);
            if let Some(obj) = &probe.mesh_ref {
                reference_objects.push(obj.clone());
            }
            if let Some(obj) = &probe.model_ref {
                reference_objects.push(obj.clone());
            }
            if let Some(rect) = probe.mesh {
                frame.buttons.push(ButtonDef::scene(
                    (rect[0], rect[1], rect[2], rect[3]),
                    CLICK_PROBE_BASE + i as u32,
                ));
            }
        }
        if !reference_objects.is_empty() {
            frame.object3d_batch(reference_objects);
        }

        let btn_font = typography::size(typography::H36, h);
        let btn_h = (44.0 * scale).max(32.0);
        let btn_w = (160.0 * scale).max(100.0);
        let btn_y = h * 0.94;
        let btn_x = (w - btn_w) * 0.5;
        frame.quad(GpuInstance {
            rect: [btn_x, btn_y, btn_w, btn_h],
            color: color::WALNUT_INK,
            user: 0,
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

        frame.window_title = format!("Mahjuro — Button AABB Lab ({})", self.room.label());
        push_screen_footer_hint(
            &mut frame,
            &ctx,
            back_footer_row(ctx.input_mode),
            HintStyle::standard(w, h),
        );
        frame
    }
}

fn resolve_probes(
    room: RoomPreset,
    w: f32,
    h: f32,
    cam: &CameraParams,
    env_h: f32,
    _layout_scale: f32,
    probes: &[ButtonProbe],
) -> Vec<ResolvedProbe> {
    match room {
        RoomPreset::Archive => archive_glb::with_archive_glb_cpu(|opt| {
            let Some(cpu) = opt else {
                return Vec::new();
            };
            probes
                .iter()
                .map(|p| resolve_archive_probe(w, h, cam, env_h, cpu, p))
                .collect()
        }),
        RoomPreset::Gameplay => gameplay_glb::with_gameplay_glb_cpu(|opt| {
            let Some(cpu) = opt else {
                return Vec::new();
            };
            probes
                .iter()
                .map(|p| resolve_gameplay_probe(w, h, cam, env_h, cpu, p))
                .collect()
        }),
    }
}

fn resolve_archive_probe(
    w: f32,
    h: f32,
    cam: &CameraParams,
    env_h: f32,
    cpu: &room_glb::RoomGlbCpu,
    probe: &ButtonProbe,
) -> ResolvedProbe {
    let min_rw = probe.min_rw;
    let min_rh = probe.min_rh;
    let mesh = screen_rect_for_marker_mesh_bounds(&MarkerScreenRectParams {
        win_w: w,
        win_h: h,
        cam,
        env_height_scale: env_h,
        cpu,
        node_name: probe.node,
        min_rw,
        min_rh,
    });
    let mesh_ref = mesh.and_then(|_| {
        marker_mesh_bounds_reference_object3d(w, h, env_h, cpu, probe.node, MESH_REF_TINT)
    });
    ResolvedProbe {
        label: probe.label,
        mesh,
        model: None,
        mesh_ref,
        model_ref: None,
    }
}

fn resolve_gameplay_probe(
    w: f32,
    h: f32,
    cam: &CameraParams,
    env_h: f32,
    cpu: &room_glb::RoomGlbCpu,
    probe: &ButtonProbe,
) -> ResolvedProbe {
    let min_rw = probe.min_rw;
    let min_rh = probe.min_rh;
    let mesh = screen_rect_for_marker_mesh_bounds(&MarkerScreenRectParams {
        win_w: w,
        win_h: h,
        cam,
        env_height_scale: env_h,
        cpu,
        node_name: probe.node,
        min_rw,
        min_rh,
    });
    let (model, model_ref) = if probe.show_model_aabb {
        model_aabb_and_ref_for_marker(w, h, cam, env_h, cpu, probe.node, min_rw, min_rh)
    } else {
        (None, None)
    };
    let mesh_ref = mesh.and_then(|_| {
        marker_mesh_bounds_reference_object3d(w, h, env_h, cpu, probe.node, MESH_REF_TINT)
    });
    ResolvedProbe {
        label: probe.label,
        mesh,
        model,
        mesh_ref,
        model_ref,
    }
}

fn model_aabb_and_ref_for_marker(
    w: f32,
    h: f32,
    cam: &CameraParams,
    env_h: f32,
    cpu: &room_glb::RoomGlbCpu,
    node: &str,
    min_rw: f32,
    min_rh: f32,
) -> (Option<[f32; 4]>, Option<Object3d>) {
    let fallback = match gameplay_glb::gameplay_marker_screen_rect_resolved(
        w, h, cam, env_h, cpu, node, min_rw, min_rh,
    ) {
        Ok(r) => r,
        Err(_) => return (None, None),
    };
    let obj = match node {
        DISCARD_RIVER => gameplay_glb::gameplay_pick_discard_river(w, h, env_h, cpu, fallback).ok(),
        PLAY_MIRROR => gameplay_glb::gameplay_pick_play_mirror(w, h, env_h, cpu, fallback).ok(),
        _ => None,
    };
    let Some(mut obj) = obj else {
        return (None, None);
    };
    obj.color = MODEL_REF_TINT;
    let rect = match node {
        DISCARD_RIVER => gameplay_discard_river_model_screen_rect(w, h, cam, &obj),
        PLAY_MIRROR => gameplay_play_mirror_model_screen_rect(w, h, cam, &obj),
        _ => return (None, None),
    };
    (Some(rect), Some(obj))
}

fn draw_probe_overlays(
    frame: &mut UiFrame,
    probe: &ResolvedProbe,
    label_font: f32,
    label_line_h: f32,
) {
    if let Some(rect) = probe.model {
        frame.quad(GpuInstance {
            rect,
            color: color::alpha([0.85, 0.35, 0.75, 1.0], 0.22),
            user: 0,
        });
    }
    if let Some(rect) = probe.mesh {
        frame.quad(GpuInstance {
            rect,
            color: color::alpha([0.25, 0.85, 0.45, 1.0], 0.24),
            user: 0,
        });
        let label_rect = [
            rect[0],
            rect[1] - label_line_h - 2.0,
            rect[2].max(80.0),
            label_line_h,
        ];
        frame.text(TextLabel {
            rect: label_rect,
            text: probe.label.into(),
            color: color::CHAMPAGNE,
            align: TextAlign::Center,
            font_px: Some(label_font),
            ..Default::default()
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::room_glb::SHOP_ENV_HEIGHT_SCALE;

    #[test]
    fn archive_tab_buttons_have_mesh_aabb_bounds() {
        let w = 1920.0;
        let h = 1080.0;
        let env_h = SHOP_ENV_HEIGHT_SCALE;
        let cam = archive_glb::archive_camera_base(w, h, env_h);
        archive_glb::with_archive_glb_cpu(|opt| {
            let cpu = opt.expect("archive.glb");
            for node in ARCHIVE_TAB_BUTTON_NODES {
                let rect = screen_rect_for_marker_mesh_bounds(&MarkerScreenRectParams {
                    win_w: w,
                    win_h: h,
                    cam: &cam,
                    env_height_scale: env_h,
                    cpu,
                    node_name: node,
                    min_rw: w * 0.09,
                    min_rh: h * 0.06,
                });
                assert!(
                    rect.is_some(),
                    "expected mesh AABB for archive tab `{node}`",
                );
            }
        });
    }
}
