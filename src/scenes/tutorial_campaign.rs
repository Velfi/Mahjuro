//! Scripted onboarding campaign scenes shown before the tutorial shop.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::audio::SfxId;
use crate::core::tile::{Suit, Tile};
use crate::game::engine::GameEngine;
use crate::game::event_bus::GameEvent;
use crate::render::draw_cmd::{
    CameraParams, DrawCmd, Object3d, Object3dKind, ShowcaseTilePlacement, UiFrame,
};
use crate::render::theme::{ButtonState, ButtonVariant, color, metrics, typography};
use crate::render::wgpu_renderer::{GpuInstance, PointLight, TextAlign, TextLabel};
use crate::render::world_space::LayoutAnchorPx;
use crate::ui::colored_keywords;
use crate::ui::focus_nav;
use crate::ui::widget::{self, TextStyle};
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use super::{BackgroundId, DrawCtx, Scene, SceneBehavior, SceneTransition, UpdateCtx};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TutorialNav {
    Back,
    Next,
    TryPlay,
    TryTrigger,
}

impl TutorialNav {
    fn id(self) -> FocusId {
        FocusId(0x7000 + self as u32)
    }
}

/// Layout for the Play / Trigger demo strip (matches `draw_frame` geometry).
struct TryItLayout {
    play_rect: [f32; 4],
    trigger_rect: [f32; 4],
    /// Y position for the one-line demo result (chips × mult = total).
    demo_line_y: f32,
    /// Minimum Y where glossary / callout may start (below demo line).
    content_floor_y: f32,
}

pub struct TutorialCampaignScene {
    page: usize,
    tree: TreeState,
    /// Demo rhythm: 0 = idle, 1 = banked (after Play), 2 = scored (after Trigger).
    try_it_phase: u8,
    /// Arrange-mode-tunable placements for the shop preview props and the
    /// try-it-demo Mirror/Trigger pair.
    pub positions: crate::ui::scene_layout::TutorialPositions,
}

struct TileGroup {
    label: &'static str,
    accent: [f32; 4],
    tiles: &'static [(Suit, u8)],
    rows: &'static [&'static [(Suit, u8)]],
    debuffed_visual: bool,
}

struct TutorialPage {
    title: &'static str,
    subtitle: &'static str,
    glossary: &'static [&'static str],
    callout: Option<&'static str>,
    /// Interactive Play → Trigger demo strip with fake totals (structure + boss pages).
    try_it_demo: bool,
    groups: &'static [TileGroup],
}

/// Three numbered suits (2–9 tiles each in the full wall).
const PART1_SUITS_GROUPS: &[TileGroup] = &[
    TileGroup {
        label: "Bamboos",
        accent: color::WALNUT_BRIGHT,
        tiles: &[(Suit::Bamboos, 2), (Suit::Bamboos, 5), (Suit::Bamboos, 8)],
        rows: &[],
        debuffed_visual: false,
    },
    TileGroup {
        label: "Dots",
        accent: color::CHAMPAGNE,
        tiles: &[(Suit::Dots, 3), (Suit::Dots, 5), (Suit::Dots, 7)],
        rows: &[],
        debuffed_visual: false,
    },
    TileGroup {
        label: "Characters",
        accent: color::STONE,
        tiles: &[
            (Suit::Characters, 1),
            (Suit::Characters, 5),
            (Suit::Characters, 9),
        ],
        rows: &[],
        debuffed_visual: false,
    },
];

/// Part 2 — How to Score (0-based index into `PAGES`).
const TUTORIAL_PAGE_SCORING: usize = 1;

const SCORING_DEMO_GROUPS: &[TileGroup] = &[TileGroup {
    label: "Pair",
    accent: color::CHAMPAGNE,
    tiles: &[(Suit::Dots, 5), (Suit::Dots, 5)],
    rows: &[],
    debuffed_visual: false,
}];

const PAGES: &[TutorialPage] = &[
    TutorialPage {
        title: "Part 1 — Tiles",
        subtitle: "Mahjuro uses three numbered suits: Bamboos, Dots, and Characters. Each suit has ranks 1 through 9. Most melds stay inside one suit.",
        glossary: &[
            "Bamboos = green bamboo sticks",
            "Dots = circles",
            "Characters = red kanji",
            "Matching = same suit and rank",
        ],
        callout: Some("You'll learn the rest by playing — starting with pairs."),
        try_it_demo: false,
        groups: PART1_SUITS_GROUPS,
    },
    TutorialPage {
        title: "Part 2 — How to Score",
        subtitle: "Select tiles, press Play to bank them into your structure, then Cash In to score. Your round score is chips × mult.",
        glossary: &[
            "Structure = banked melds until you cash in",
            "Play = bank selected melds",
            "Cash In = score your structure",
            "Chips × mult = round score",
        ],
        callout: Some("Try the demo below, then you'll play a short guided blind."),
        try_it_demo: true,
        groups: SCORING_DEMO_GROUPS,
    },
];

impl TutorialCampaignScene {
    pub fn new() -> Self {
        Self {
            page: 0,
            tree: TreeState::new(),
            try_it_phase: 0,
            positions: crate::ui::scene_layout::TutorialPositions::default(),
        }
    }

    fn page(&self) -> &'static TutorialPage {
        &PAGES[self.page.min(PAGES.len() - 1)]
    }

    fn try_it_demo_line(page_index: usize, phase: u8) -> Option<&'static str> {
        match (page_index, phase) {
            (TUTORIAL_PAGE_SCORING, 0) => Some("Tap Play (bank), then Cash In."),
            (TUTORIAL_PAGE_SCORING, 1) => Some("Banked — structure is locked in."),
            (TUTORIAL_PAGE_SCORING, 2) => Some("Demo: 4 chips × 3 mult = 12"),
            _ => None,
        }
    }

    fn compute_try_it_layout(panel_x: f32, panel_w: f32, label_y: f32, scale: f32) -> TryItLayout {
        let btn_w = (150.0 * scale).max(100.0);
        let btn_h = (40.0 * scale).max(28.0);
        let gap = 12.0 * scale;
        let strip_y = label_y + 36.0 * scale;
        let center = panel_x + panel_w * 0.5;
        let play_x = center - btn_w - gap * 0.5;
        let trigger_x = center + gap * 0.5;
        let demo_line_y = strip_y + btn_h + 10.0 * scale;
        let content_floor_y = demo_line_y + 24.0 * scale;
        TryItLayout {
            play_rect: [play_x, strip_y, btn_w, btn_h],
            trigger_rect: [trigger_x, strip_y, btn_w, btn_h],
            demo_line_y,
            content_floor_y,
        }
    }

    fn glossary_term_metrics(
        glossary: &[&'static str],
        term_w: f32,
        term_font: f32,
        scale: f32,
    ) -> (Vec<f32>, f32) {
        let mut heights = Vec::with_capacity(glossary.len());
        let mut total_h = 0.0;
        for term in glossary {
            let lines_n =
                colored_keywords::colored_wrapped_line_count(term, term_w, term_font, color::STONE);
            let term_h = lines_n as f32 * term_font * 1.25;
            heights.push(term_h);
            total_h += term_h;
        }
        if !glossary.is_empty() {
            total_h += (glossary.len().saturating_sub(1) as f32) * 6.0 * scale;
        }
        (heights, total_h)
    }

    /// Matches `draw_frame` subtitle + tile row metrics so Try-it rects align with visuals.
    fn page_content_metrics(
        page: &TutorialPage,
        w: f32,
        h: f32,
        _panel_x: f32,
        panel_y: f32,
        panel_w: f32,
    ) -> (f32, f32) {
        let scale = metrics::scene_scale(w, h);
        let subtitle_y = panel_y + 70.0 * scale;
        let subtitle_w = panel_w - 60.0 * scale;
        let subtitle_font = typography::size(typography::H36, h);
        let subtitle_h = {
            let subtitle_lines_n = colored_keywords::colored_wrapped_line_count(
                page.subtitle,
                subtitle_w,
                subtitle_font,
                color::PARCHMENT,
            );
            (subtitle_lines_n as f32 * subtitle_font * 1.35)
                .max(70.0 * scale)
                .min(128.0 * scale)
        };
        let tile_area_y = subtitle_y + subtitle_h + 40.0 * scale;
        let label_y = tile_area_y + 74.0;
        (tile_area_y, label_y)
    }

    fn flat_items(&self, w: f32, h: f32) -> Vec<FlatItem<TutorialNav>> {
        let scale = metrics::scene_scale(w, h);
        let btn_w = (170.0 * scale).max(120.0);
        let btn_h = (46.0 * scale).max(30.0);
        let gap = 14.0 * scale;
        let y = h - btn_h - 22.0 * scale;
        let next_x = w * 0.5 + gap * 0.5;
        let back_x = next_x - btn_w - gap;

        let page = self.page();
        let panel_x = w * 0.06;
        let panel_w = w * 0.88;
        let panel_y = h * 0.07;
        let (_, label_y) = Self::page_content_metrics(page, w, h, panel_x, panel_y, panel_w);

        let mut items = Vec::new();
        if self.page > 0 {
            items.push(FlatItem::new(
                TutorialNav::Back.id(),
                [back_x, y, btn_w, btn_h],
                TutorialNav::Back,
            ));
        }
        items.push(FlatItem::new(
            TutorialNav::Next.id(),
            [next_x, y, btn_w, btn_h],
            TutorialNav::Next,
        ));

        if page.try_it_demo {
            let t = Self::compute_try_it_layout(panel_x, panel_w, label_y, scale);
            items.push(FlatItem::new(
                TutorialNav::TryPlay.id(),
                t.play_rect,
                TutorialNav::TryPlay,
            ));
            items.push(FlatItem::new(
                TutorialNav::TryTrigger.id(),
                t.trigger_rect,
                TutorialNav::TryTrigger,
            ));
        }

        items
    }

    fn preview_tile_placements(
        _page_index: usize,
        page: &TutorialPage,
        panel_x: f32,
        panel_w: f32,
        label_y: f32,
        scale: f32,
    ) -> Vec<ShowcaseTilePlacement> {
        let group_count = page.groups.len().max(1) as f32;
        let group_w = panel_w * 0.74 / group_count;
        let start_x = panel_x + panel_w * 0.13 + group_w * 0.5;
        let mut next_id = 30_000u32;
        let mut placements = Vec::new();

        for (group_idx, group) in page.groups.iter().enumerate() {
            let center_x = start_x + group_idx as f32 * group_w;
            let rows: Vec<&[(Suit, u8)]> = if group.rows.is_empty() {
                vec![group.tiles]
            } else {
                group.rows.to_vec()
            };
            let widest_row_units = rows
                .iter()
                .map(|row| {
                    let tiles = row.len() as f32;
                    tiles + (tiles - 1.0) * 0.02
                })
                .fold(1.0, f32::max);
            let tile_size = ((group_w * 0.68) / widest_row_units).clamp(18.0 * scale, 34.0 * scale);
            let step = tile_size * 1.02;
            let row_gap = tile_size * 0.94;
            let base_row_y = label_y - tile_size * 0.80 - 8.0 * scale;
            let top_row_y = base_row_y - row_gap * (rows.len().saturating_sub(1) as f32);

            for (row_idx, row) in rows.iter().enumerate() {
                let total_w = tile_size + (row.len().saturating_sub(1) as f32) * step;
                let mut x = center_x - total_w * 0.5 + tile_size * 0.5;
                let row_y = top_row_y + row_idx as f32 * row_gap;
                for (suit, rank) in row.iter().copied() {
                    let mut tile = Tile::new(suit, rank, next_id);
                    tile.debuffed_visual = group.debuffed_visual;
                    placements.push(ShowcaseTilePlacement {
                        tile,
                        center_pos: [x, row_y, 0.0],
                        rotation: [0.0, 0.0, std::f32::consts::PI],
                        scale: 1.0,
                        size_px: tile_size,
                        brightness: 1.08,
                        selected: false,
                        hovered: false,
                        outline: false,
                        glow: false,
                        glow_color: None,
                        pick_id: None,
                        arrange_group: None,
                    });
                    next_id += 1;
                    x += step;
                }
            }
        }
        placements
    }
}

impl SceneBehavior for TutorialCampaignScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        let items = self.flat_items(ctx.layout.window_w, ctx.layout.window_h);
        let action = self.tree.update_flat(
            &items,
            TreeInput {
                actions: ctx.actions,
                button_clicks: ctx.button_clicks,
                cursor_pos: ctx.cursor_pos,
                window: (ctx.layout.window_w, ctx.layout.window_h),
                input_mode: ctx.input_mode,
                scroll_lines: 0.0,
            },
        );
        if self.tree.take_focus_changed() {
            ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
        }

        match action {
            Some(TutorialNav::Back) => {
                ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
                if self.page > 0 {
                    self.page -= 1;
                    self.try_it_phase = 0;
                    ctx.bus.push(GameEvent::UiSound(SfxId::TileClick));
                }
                None
            }
            Some(TutorialNav::Next) => {
                ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                if self.page + 1 < PAGES.len() {
                    self.page += 1;
                    self.try_it_phase = 0;
                    ctx.bus.push(GameEvent::UiSound(SfxId::PackBuy));
                    None
                } else {
                    ctx.bus.push(GameEvent::UiSound(SfxId::RelicPickup));
                    GameEngine::begin_onboarding_lessons(ctx.run);
                    Some(Scene::Gameplay(Box::new(
                        super::gameplay::GameplayScene::with_pending_blind(
                            crate::core::rules::BlindKind::Small,
                        ),
                    )))
                }
            }
            Some(TutorialNav::TryPlay) => {
                if !self.page().try_it_demo {
                    return None;
                }
                match self.try_it_phase {
                    0 => {
                        self.try_it_phase = 1;
                        ctx.bus.push(GameEvent::StructureCommitted);
                    }
                    2 => {
                        self.try_it_phase = 0;
                    }
                    _ => {
                        ctx.bus.push(GameEvent::UiSound(SfxId::InvalidAction));
                    }
                }
                None
            }
            Some(TutorialNav::TryTrigger) => {
                if !self.page().try_it_demo {
                    return None;
                }
                if self.try_it_phase == 1 {
                    self.try_it_phase = 2;
                    ctx.bus.push(GameEvent::UiSound(SfxId::ScoreReveal));
                    ctx.bus.push(GameEvent::UiSound(SfxId::ScoreFinal));
                } else {
                    ctx.bus.push(GameEvent::UiSound(SfxId::InvalidAction));
                }
                None
            }
            None => None,
        }
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let scale = metrics::scene_scale(w, h);
        let page = self.page();

        let mut bg_quads = Vec::new();
        let mut fg_quads = Vec::new();
        let mut texts = Vec::new();
        let mut wood_tablet_placements: Vec<Object3d> = Vec::new();
        let mut mirror_placement: Option<Object3d> = None;
        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);
        if ctx.effect_layers.starfield {
            frame.starfield();
        }
        if ctx.effect_layers.golden_dust {
            frame.golden_dust();
        }
        let cam_scale = h / 1600.0;
        frame.camera_override = Some(CameraParams {
            eye: [0.0, -220.0 * cam_scale, 1960.0 * cam_scale],
            target: [0.0, -40.0 * cam_scale, 0.0],
            up: [0.0, 0.0, 1.0],
            fovy_deg: 45.0,
            clip_near: None,
            clip_far: None,
        });

        let panel_x = w * 0.06;
        let panel_y = h * 0.07;
        let panel_w = w * 0.88;
        let panel_h = h * 0.84;
        bg_quads.push(GpuInstance {
            rect: [panel_x, panel_y, panel_w, panel_h],
            color: color::WALNUT_DEEP,
            user: 0,
        });
        bg_quads.push(GpuInstance {
            rect: [panel_x, panel_y, panel_w, (2.0 * scale).max(1.0)],
            color: color::BRASS,
            user: 0,
        });
        bg_quads.push(GpuInstance {
            rect: [panel_x, panel_y, (2.0 * scale).max(1.0), panel_h],
            color: color::BRASS,
            user: 0,
        });
        bg_quads.push(GpuInstance {
            rect: [
                panel_x + panel_w - (2.0 * scale).max(1.0),
                panel_y,
                (2.0 * scale).max(1.0),
                panel_h,
            ],
            color: color::BRASS,
            user: 0,
        });
        bg_quads.push(GpuInstance {
            rect: [
                panel_x,
                panel_y + panel_h - (2.0 * scale).max(1.0),
                panel_w,
                (2.0 * scale).max(1.0),
            ],
            color: color::BRASS,
            user: 0,
        });

        texts.push(TextLabel {
            rect: [
                panel_x + 24.0 * scale,
                panel_y + 18.0 * scale,
                panel_w - 48.0 * scale,
                40.0 * scale,
            ],
            text: page.title.to_string(),
            color: color::CHAMPAGNE,
            align: TextAlign::Center,
            font_px: Some(typography::size(typography::H36, h)),
            ..Default::default()
        });

        let subtitle_x = panel_x + 30.0 * scale;
        let subtitle_y = panel_y + 70.0 * scale;
        let subtitle_w = panel_w - 60.0 * scale;
        let subtitle_font = typography::size(typography::H36, h);
        let subtitle_lines_n = colored_keywords::colored_wrapped_line_count(
            page.subtitle,
            subtitle_w,
            subtitle_font,
            color::PARCHMENT,
        );
        let subtitle_h = (subtitle_lines_n as f32 * subtitle_font * 1.35)
            .max(70.0 * scale)
            .min(128.0 * scale);
        colored_keywords::push_colored_text_block(
            &mut texts,
            [subtitle_x, subtitle_y, subtitle_w, subtitle_h],
            page.subtitle,
            TextStyle {
                tier: typography::H36,
                color: color::PARCHMENT,
                padding: 0.0,
                align: TextAlign::Center,
                ..Default::default()
            },
            h,
        );

        let (_tile_area_y, label_y) =
            Self::page_content_metrics(page, w, h, panel_x, panel_y, panel_w);
        let showcase_tiles =
            Self::preview_tile_placements(self.page, page, panel_x, panel_w, label_y, scale);
        let group_w = panel_w * 0.74 / page.groups.len().max(1) as f32;
        for (idx, group) in page.groups.iter().enumerate() {
            let gx = panel_x + panel_w * 0.13 + idx as f32 * group_w;
            fg_quads.push(GpuInstance {
                rect: [gx + group_w * 0.14, label_y, group_w * 0.72, 4.0 * scale],
                color: group.accent,
                user: 0,
            });
            texts.push(TextLabel {
                rect: [gx, label_y + 10.0 * scale, group_w, 22.0 * scale],
                text: group.label.to_string(),
                color: color::PARCHMENT,
                align: TextAlign::Center,
                font_px: Some(typography::size(typography::H42, h)),
                ..Default::default()
            });
        }

        let try_it_layout = page
            .try_it_demo
            .then(|| Self::compute_try_it_layout(panel_x, panel_w, label_y, scale));
        let glossary_y = if let Some(ref t) = try_it_layout {
            t.content_floor_y
                .min(panel_y + panel_h * 0.62)
                .max(label_y + 132.0 * scale)
        } else {
            (label_y + 148.0 * scale).min(panel_y + panel_h * 0.60)
        };

        if let Some(ref layout) = try_it_layout {
            let heading_y = layout.play_rect[1] - 22.0 * scale;
            let try_it_lift = (28.0 * scale).max(20.0);
            let try_it_world_z_py_nudge = 18.0 * scale;
            let play_focused = self.tree.focused() == Some(TutorialNav::TryPlay.id());
            let play_center_x = layout.play_rect[0] + layout.play_rect[2] * 0.5;
            let play_center_y = layout.play_rect[1] + layout.play_rect[3] * 0.5;
            let trigger_center_x = layout.trigger_rect[0] + layout.trigger_rect[2] * 0.5;
            let trigger_center_y = layout.trigger_rect[1] + layout.trigger_rect[3] * 0.5;
            let wobble_t = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs_f32())
                .unwrap_or(0.0);
            let mirror_diam = layout.play_rect[2]
                .max(layout.play_rect[3] * 1.8)
                .max(72.0 * scale);
            let mp = &self.positions.try_it_mirror;
            let mirror_pos = LayoutAnchorPx {
                px: play_center_x,
                py: play_center_y + try_it_world_z_py_nudge,
                lift_z: try_it_lift,
            }
            .to_draw_cmd_triple();
            mirror_placement = Some(Object3d {
                pos: [
                    mirror_pos[0] + w * mp.nx,
                    mirror_pos[1] + h * mp.ny,
                    mirror_pos[2] + ctx.layout.mm(mp.lift_mm),
                ],
                extents: [mirror_diam, mirror_diam, mirror_diam],
                rotation: [0.0, 0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
                kind: Object3dKind::Mirror {
                    rotation_x_deg: 36.0,
                    rotation_z_deg: (wobble_t * 2.4).sin() * 7.5,
                },
                hover_target: if play_focused { 1.0 } else { 0.0 },
                anim_id: 2,
                arrange_name: Some("tutorial.try_it.mirror"),
            });
            let tp = &self.positions.try_it_trigger;
            let trigger_pos = LayoutAnchorPx {
                px: trigger_center_x,
                py: trigger_center_y + try_it_world_z_py_nudge,
                lift_z: try_it_lift,
            }
            .to_draw_cmd_triple();
            wood_tablet_placements.push(Object3d {
                pos: [
                    trigger_pos[0] + w * tp.nx,
                    trigger_pos[1] + h * tp.ny,
                    trigger_pos[2] + ctx.layout.mm(tp.lift_mm),
                ],
                extents: [
                    layout.trigger_rect[2],
                    (layout.trigger_rect[3] * 0.35).max(8.0),
                    layout.trigger_rect[3],
                ],
                rotation: [0.0, 0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
                kind: Object3dKind::WoodTablet {
                    label: std::borrow::Cow::Borrowed("Cash In"),
                    pick_id: None,
                },
                hover_target: 0.0,
                anim_id: 0,
                arrange_name: Some("tutorial.try_it.trigger"),
            });
            texts.push(TextLabel {
                rect: [
                    panel_x + 24.0 * scale,
                    heading_y,
                    panel_w - 48.0 * scale,
                    20.0 * scale,
                ],
                text: "Try it (demo)".to_string(),
                color: color::GOLD,
                align: TextAlign::Center,
                font_px: Some(typography::size(typography::H36, h)),
                ..Default::default()
            });
            let note_w = (150.0 * scale).max(120.0);
            let note_h = (54.0 * scale).max(40.0);
            let note_x = (layout.play_rect[0] - note_w - 22.0 * scale).max(panel_x + 28.0 * scale);
            let note_y = layout.play_rect[1] - 10.0 * scale;
            fg_quads.push(GpuInstance {
                rect: [note_x, note_y, note_w, note_h],
                color: color::alpha(color::CHAMPAGNE, 0.16),
                user: 0,
            });
            fg_quads.push(GpuInstance {
                rect: [note_x, note_y, 3.0 * scale, note_h],
                color: color::GOLD,
                user: 0,
            });
            widget::push_text_block(
                &mut texts,
                [
                    note_x + 10.0 * scale,
                    note_y + 8.0 * scale,
                    note_w - 18.0 * scale,
                    note_h - 12.0 * scale,
                ],
                "Note: a bronze mirror is an old round metal mirror. Here, it is the \"Commit Meld\" button.",
                TextStyle {
                    tier: typography::H42,
                    color: color::CHAMPAGNE,
                    padding: 0.0,
                    align: TextAlign::Left,
                    ..Default::default()
                },
                h,
            );
            if let Some(line) = Self::try_it_demo_line(self.page, self.try_it_phase) {
                colored_keywords::push_colored_text_block(
                    &mut texts,
                    [
                        panel_x + 24.0 * scale,
                        layout.demo_line_y,
                        panel_w - 48.0 * scale,
                        22.0 * scale,
                    ],
                    line,
                    TextStyle {
                        tier: typography::H42,
                        color: color::CHAMPAGNE,
                        padding: 0.0,
                        align: TextAlign::Center,
                        ..Default::default()
                    },
                    h,
                );
            }
        }

        texts.push(TextLabel {
            rect: [
                panel_x + 34.0 * scale,
                glossary_y,
                if page.try_it_demo {
                    panel_w * 0.42
                } else {
                    panel_w * 0.34
                },
                24.0 * scale,
            ],
            text: "Key Terms".to_string(),
            color: color::GOLD,
            align: TextAlign::Left,
            font_px: Some(typography::size(typography::H32, h)),
            ..Default::default()
        });
        let term_w = if page.try_it_demo {
            panel_w * 0.42
        } else {
            panel_w * 0.34
        };
        let glossary_floor = panel_y + panel_h - 86.0 * scale;
        let _glossary_available_h = (glossary_floor - (glossary_y + 28.0 * scale)).max(0.0);
        let term_font = typography::size(typography::H42, h);
        let (term_heights, _glossary_total_h) =
            Self::glossary_term_metrics(page.glossary, term_w, term_font, scale);
        let mut gy = glossary_y + 28.0 * scale;
        for (idx, term) in page.glossary.iter().enumerate() {
            let term_h = term_heights.get(idx).copied().unwrap_or(term_font * 1.25);
            colored_keywords::push_colored_text_block(
                &mut texts,
                [panel_x + 36.0 * scale, gy, term_w, term_h],
                term,
                TextStyle {
                    tier: typography::H42,
                    color: color::STONE,
                    padding: 0.0,
                    align: TextAlign::Left,
                    ..Default::default()
                },
                h,
            );
            gy += term_h + 6.0 * scale;
        }

        if let Some(callout) = page.callout {
            let callout_x = if page.try_it_demo {
                panel_x + panel_w * 0.54
            } else {
                panel_x + panel_w * 0.47
            };
            let callout_y = glossary_y + 6.0 * scale;
            let callout_w = if page.try_it_demo {
                panel_w * 0.36
            } else {
                panel_w * 0.45
            };
            let callout_font = typography::size(typography::H36, h);
            let callout_lines_n = colored_keywords::colored_wrapped_line_count(
                callout,
                callout_w - 32.0 * scale,
                callout_font,
                color::CHAMPAGNE,
            );
            let callout_h =
                (callout_lines_n as f32 * callout_font * 1.3 + 36.0 * scale).max(112.0 * scale);
            fg_quads.push(GpuInstance {
                rect: [callout_x, callout_y, callout_w, callout_h],
                color: color::alpha(color::WALNUT_INK, 0.85),
                user: 0,
            });
            fg_quads.push(GpuInstance {
                rect: [callout_x, callout_y, 4.0 * scale, callout_h],
                color: color::GOLD,
                user: 0,
            });
            colored_keywords::push_colored_text_block(
                &mut texts,
                [
                    callout_x + 18.0 * scale,
                    callout_y + 14.0 * scale,
                    callout_w - 32.0 * scale,
                    callout_h - 28.0 * scale,
                ],
                callout,
                TextStyle {
                    tier: typography::H36,
                    color: color::CHAMPAGNE,
                    padding: 0.0,
                    align: TextAlign::Left,
                    ..Default::default()
                },
                h,
            );
        }

        let items = self.flat_items(w, h);
        let mut buttons = Vec::new();
        for item in &items {
            if matches!(item.action, TutorialNav::TryPlay | TutorialNav::TryTrigger) {
                continue;
            }
            let (label, variant, state) = match item.action {
                TutorialNav::Next => {
                    let label = if self.page + 1 == PAGES.len() {
                        "Start Lesson"
                    } else {
                        "Next"
                    };
                    (label, ButtonVariant::Primary, ButtonState::Rest)
                }
                TutorialNav::Back => ("Back", ButtonVariant::Default, ButtonState::Rest),
                TutorialNav::TryPlay | TutorialNav::TryTrigger => continue,
            };
            widget::push_button(
                &mut fg_quads,
                &mut texts,
                &mut buttons,
                widget::ButtonSpec {
                    rect: item.rect,
                    label,
                    variant,
                    state,
                    action: crate::ui::input::UiAction::Confirm,
                },
            );
        }
        buttons.clear();
        self.tree.register_flat_buttons(&items, &mut buttons);

        if let Some(rect) = match self.tree.focused() {
            Some(id) if id == TutorialNav::TryPlay.id() => ctx.proj.mirror_rect,
            Some(id) if id == TutorialNav::TryTrigger.id() => {
                ctx.proj.wood_tablet_rects.first().copied()
            }
            _ => None,
        } {
            focus_nav::push_focus_ring(rect, scale, w, h, &mut fg_quads);
        }

        frame.quads(bg_quads);
        if !showcase_tiles.is_empty() {
            frame.cmds.push(DrawCmd::ShowcaseTileBatch(showcase_tiles));
        }
        if let Some(mirror) = mirror_placement {
            frame.object3d(mirror);
        }
        if !wood_tablet_placements.is_empty() {
            frame.object3d_batch(wood_tablet_placements);
        }
        // Broad, forgiving lighting for educational showcase objects.
        let light_y = h * 0.18;
        for &(lx, ly, intensity) in &[
            (panel_x + panel_w * 0.24, label_y - 54.0 * scale, 1.95),
            (panel_x + panel_w * 0.50, label_y - 72.0 * scale, 2.15),
            (panel_x + panel_w * 0.76, label_y - 54.0 * scale, 1.95),
            (panel_x + panel_w * 0.34, label_y + 10.0 * scale, 1.10),
            (panel_x + panel_w * 0.66, label_y + 10.0 * scale, 1.10),
        ] {
            frame.scene_lighting.push_smooth(PointLight {
                pos: [lx, ly, light_y],
                radius: h * 0.95,
                color: color::rgb(color::PARCHMENT),
                intensity,
            });
        }
        frame.quads(fg_quads);
        frame.texts(texts);
        frame.buttons = buttons;
        frame.window_title = format!("Mahjuro — {}", page.title);
        frame
    }
}
