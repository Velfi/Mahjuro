//! "Choose Your Tiles" scene shown between the start screen and the first
//! shop. Left/right arrows cycle through tile materials; each material
//! displays its name and gameplay bonus. Play starts the run.

use crate::audio::SfxId;
use crate::core::tile::{Suit, Tile};
use crate::game::engine::GameEngine;
use crate::game::event_bus::GameEvent;
use crate::game::run::RunState;
use crate::persistence::TileMaterial;
use crate::render::theme::{ButtonVariant, color, metrics, typography};
use crate::render::wgpu_renderer::{GpuInstance, PointLight, TextAlign, TextLabel};
use crate::ui::input::UiAction;
use crate::ui::widget::{self, TextStyle};
use crate::ui::widget_tree::{
    self as wt, FocusId, Tree, TreeFrame, TreeInput, TreeState, noop_render_custom,
};

use super::shop::ShopScene;
use super::start_screen::StartScreenScene;
use crate::render::draw_cmd::UiFrame;

use super::{BackgroundId, ButtonDef, DrawCtx, Scene, SceneBehavior, SceneTransition, UpdateCtx};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModalAction {
    Play,
    SkipTutorial,
    Back,
}

impl ModalAction {
    fn id(self) -> FocusId {
        FocusId(0x2000_0000 + self as u32)
    }
}

pub struct TileSelectScene {
    tree: TreeState,
    material: TileMaterial,
    /// If true, the next run starts in tutorial mode instead of standard.
    tutorial_mode: bool,
}

impl TileSelectScene {
    pub fn new() -> Self {
        Self {
            tree: TreeState::new(),
            material: TileMaterial::default(),
            tutorial_mode: false,
        }
    }

    /// Create a tile-select scene that will start a tutorial run.
    pub fn new_tutorial() -> Self {
        Self {
            tree: TreeState::new(),
            material: TileMaterial::Bamboo,
            tutorial_mode: true,
        }
    }

    /// Build the button-only widget tree. Text labels are emitted separately
    /// in `draw()` because `draw_decoration_top` doesn't support column layout.
    fn build_tree(&self, window_w: f32, window_h: f32, ui_scale: f32) -> Tree<ModalAction> {
        let scale = metrics::scene_scale(window_w, window_h, ui_scale);
        let panel_w = window_w * 0.38;
        let btn_w = if self.tutorial_mode {
            (220.0 * scale).min(panel_w * 0.78)
        } else {
            (240.0 * scale).min(panel_w * 0.85)
        };

        let btn_h = if self.tutorial_mode {
            (38.0 * scale).max(26.0)
        } else {
            (44.0 * scale).max(28.0)
        };
        let btn_gap = if self.tutorial_mode {
            (10.0 * scale).max(6.0)
        } else {
            (12.0 * scale).max(6.0)
        };

        let start_y = if self.tutorial_mode {
            window_h * 0.62
        } else {
            window_h * 0.55
        };
        let menu_x = (panel_w - btn_w) * 0.5;

        let items = if self.tutorial_mode {
            vec![
                wt::button_id(
                    ModalAction::Play.id(),
                    "Play Tutorial",
                    ModalAction::Play,
                    ButtonVariant::Primary,
                ),
                wt::button_id(
                    ModalAction::SkipTutorial.id(),
                    "Skip Tutorial",
                    ModalAction::SkipTutorial,
                    ButtonVariant::Default,
                ),
                wt::button_id(
                    ModalAction::Back.id(),
                    "Back",
                    ModalAction::Back,
                    ButtonVariant::Default,
                ),
            ]
        } else {
            vec![
                wt::button_id(
                    ModalAction::Play.id(),
                    "Play",
                    ModalAction::Play,
                    ButtonVariant::Primary,
                ),
                wt::button_id(
                    ModalAction::Back.id(),
                    "Back",
                    ModalAction::Back,
                    ButtonVariant::Default,
                ),
            ]
        };
        let block_h = items.len() as f32 * btn_h + (items.len().saturating_sub(1) as f32) * btn_gap;
        Tree::vertical_menu(items).with_anchor([menu_x, start_y, btn_w, block_h])
    }

    fn start_game(
        &self,
        run: &mut RunState,
        progress: &crate::core::progression::PlayerProgress,
    ) -> SceneTransition {
        let settings = crate::persistence::load_settings();
        if self.tutorial_mode {
            GameEngine::start_onboarding_run(run, progress, &settings);
            Some(Scene::TutorialCampaign(
                super::tutorial_campaign::TutorialCampaignScene::new(),
            ))
        } else {
            GameEngine::start_run_with_material(run, self.material, progress, &settings);
            Some(Scene::Shop(ShopScene::new(
                GameEngine::current_run_number(run),
                run,
            )))
        }
    }
}

/// The 38 unique tile faces in the standard set, ordered by suit for grid display.
/// Uses stable IDs starting at 50_000 so the renderer doesn't re-rasterize each frame.
fn preview_tiles() -> Vec<Tile> {
    let mut tiles = Vec::with_capacity(38);
    let mut id = 50_000u32;
    for suit in [Suit::Characters, Suit::Bamboos, Suit::Circles] {
        for rank in 1..=9 {
            tiles.push(Tile::new(suit, rank, id));
            id += 1;
        }
    }
    for rank in 1..=4 {
        tiles.push(Tile::new(Suit::Wind, rank, id));
        id += 1;
    }
    for rank in 1..=3 {
        tiles.push(Tile::new(Suit::Dragon, rank, id));
        id += 1;
    }
    for rank in 1..=4 {
        tiles.push(Tile::new(Suit::Flower, rank, id));
        id += 1;
    }
    tiles
}

/// Row definitions: (start_index, count) for each row.
const GRID_ROWS: [(usize, usize); 5] = [
    (0, 9),  // Characters 1–9
    (9, 9),  // Bamboos 1–9
    (18, 9), // Circles 1–9
    (27, 7), // Winds 1–4 + Dragons 1–3
    (34, 4), // Flowers 1–4
];

/// Compute 38 screen-space `(x, y, w, h)` slot rects for the tile preview grid.
fn grid_slots(grid_x: f32, grid_y: f32, grid_w: f32, grid_h: f32) -> Vec<(f32, f32, f32, f32)> {
    let cols = 9.0_f32;
    let rows = GRID_ROWS.len() as f32;
    let slot_w = grid_w / cols;
    // Face aspect ~1.36 (long axis / short axis from the tile mesh).
    let slot_h = slot_w * 1.36;
    let total_h = rows * slot_h;
    // Vertical gap between rows, distributed evenly.
    let row_gap = if rows > 1.0 {
        ((grid_h - total_h) / (rows - 1.0)).max(0.0)
    } else {
        0.0
    };

    let mut slots = Vec::with_capacity(38);
    for (row_idx, &(_start, count)) in GRID_ROWS.iter().enumerate() {
        let row_y = grid_y + row_idx as f32 * (slot_h + row_gap);
        // Center shorter rows within the 9-column width.
        let row_offset = (cols - count as f32) * slot_w * 0.5;
        for col in 0..count {
            let x = grid_x + row_offset + col as f32 * slot_w;
            slots.push((x, row_y, slot_w, slot_h));
        }
    }
    slots
}

impl SceneBehavior for TileSelectScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;

        // Left/right cycle materials; filter them so the tree doesn't
        // consume them as focus movement.
        let mut filtered: Vec<UiAction> = Vec::new();
        for &a in ctx.actions {
            match a {
                UiAction::FocusNext => self.material = self.material.next(),
                UiAction::FocusPrev => self.material = self.material.prev(),
                UiAction::Cancel | UiAction::Pause => {
                    ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
                    return Some(Scene::StartScreen(StartScreenScene::new()));
                }
                other => filtered.push(other),
            }
        }

        let tree = self.build_tree(w, h, ctx.ui_scale);
        let action = self.tree.update(
            &tree,
            TreeInput {
                actions: &filtered,
                button_clicks: ctx.button_clicks,
                cursor_pos: ctx.cursor_pos,
                window: (w, h),
                ui_scale: ctx.ui_scale,
                input_mode: ctx.input_mode,
                scroll_lines: 0.0,
            },
        );
        if self.tree.take_focus_changed() {
            ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
        }

        match action {
            Some(ModalAction::Play) => {
                ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                self.start_game(ctx.run, ctx.progress)
            }
            Some(ModalAction::SkipTutorial) => {
                ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                *ctx.complete_onboarding = true;
                let settings = crate::persistence::load_settings();
                GameEngine::start_run_with_material(
                    ctx.run,
                    TileMaterial::default(),
                    ctx.progress,
                    &settings,
                );
                Some(Scene::Shop(ShopScene::new(
                    GameEngine::current_run_number(ctx.run),
                    ctx.run,
                )))
            }
            Some(ModalAction::Back) => {
                ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
                Some(Scene::StartScreen(StartScreenScene::new()))
            }
            None => None,
        }
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let ui_scale = ctx.ui_scale;

        let mut instances: Vec<GpuInstance> = Vec::new();
        let mut text_labels: Vec<TextLabel> = Vec::new();
        let mut buttons: Vec<ButtonDef> = Vec::new();

        // ── Left panel text labels (manually laid out) ─────────────
        let panel_w = w * 0.38;
        let scale = metrics::scene_scale(w, h, ui_scale);
        let gap_sm = (16.0 * scale).max(8.0);
        let gap_lg = (28.0 * scale).max(14.0);

        let title_px = typography::size(2.25, h, ui_scale);
        let name_px = typography::size(typography::TITLE, h, ui_scale);
        let bonus_px = typography::size(typography::HEADING, h, ui_scale);
        let hint_px = typography::size(typography::CAPTION, h, ui_scale);

        // Rect heights need room above the font size for line padding.
        let title_h = title_px * 1.4;
        let name_h = name_px * 1.4;
        let bonus_h = bonus_px * 1.4;
        let hint_h = hint_px * 1.4;

        let text_block_h = title_h + gap_sm + name_h + gap_sm + bonus_h;
        let mut cursor_y = if self.tutorial_mode {
            h * 0.22
        } else {
            (h * 0.5 - text_block_h) * 0.5 + h * 0.05
        };
        let text_x = panel_w * 0.05;
        let text_w = panel_w * 0.90;

        let title_text = if self.tutorial_mode {
            "First-Time Tutorial"
        } else {
            "Choose Your Tiles"
        };
        text_labels.push(TextLabel {
            rect: [text_x, cursor_y, text_w, title_h],
            text: title_text.into(),
            color: color::CHAMPAGNE,
            font_px: Some(title_px),
            ..Default::default()
        });
        cursor_y += title_h + gap_lg;

        if self.tutorial_mode {
            let intro_h = 90.0 * scale;
            widget::push_text_block(
                &mut text_labels,
                [text_x, cursor_y, text_w, intro_h],
                "A short guided campaign teaches melds, structure scoring, relics, bosses, and the shop before one final practice fight.",
                TextStyle {
                    tier: typography::HEADING,
                    color: color::MIST,
                    padding: 0.0,
                    align: TextAlign::Left,
                },
                h,
                ui_scale,
            );
            cursor_y += intro_h + 10.0 * scale;
            let skip_h = 50.0 * scale;
            widget::push_text_block(
                &mut text_labels,
                [text_x, cursor_y, text_w, skip_h],
                "Skip marks the tutorial complete for this profile and starts a normal run immediately.",
                TextStyle {
                    tier: typography::CAPTION,
                    color: color::PARCHMENT,
                    padding: 0.0,
                    align: TextAlign::Left,
                },
                h,
                ui_scale,
            );
        } else {
            text_labels.push(TextLabel {
                rect: [text_x, cursor_y, text_w, name_h],
                text: self.material.label().into(),
                color: color::CHAMPAGNE,
                font_px: Some(name_px),
                ..Default::default()
            });
            cursor_y += name_h + gap_sm;

            text_labels.push(TextLabel {
                rect: [text_x, cursor_y, text_w, bonus_h],
                text: self.material.bonus_description().into(),
                color: color::MIST,
                font_px: Some(bonus_px),
                ..Default::default()
            });
        }

        // Hint at the bottom of the panel.
        let hint_y = h - hint_h - (12.0 * scale);
        text_labels.push(TextLabel {
            rect: [text_x, hint_y, text_w, hint_h],
            text: if self.tutorial_mode {
                "Enter to confirm the focused option".into()
            } else {
                "\u{25C0}  \u{25B6}  Change tiles   |   Enter to play".into()
            },
            color: color::SLATE,
            font_px: Some(hint_px),
            ..Default::default()
        });

        // ── Buttons (via widget tree) ──────────────────────────────
        let tree = self.build_tree(w, h, ui_scale);
        let mut tree_frame = TreeFrame {
            instances: &mut instances,
            labels: &mut text_labels,
            buttons: &mut buttons,
            window: (w, h),
        };
        self.tree.draw(&tree, &mut tree_frame, &noop_render_custom);

        // ── Tile preview grid on the right ─────────────────────────
        let grid_margin = w * 0.02;
        let grid_x = w * 0.40 + grid_margin;
        let grid_y = h * 0.10;
        let grid_w = w * 0.58 - grid_margin * 2.0;
        let grid_h = h * 0.80;
        let hand_tiles = preview_tiles();
        let hand_slots = grid_slots(grid_x, grid_y, grid_w, grid_h);

        // Build tile preview placements for the showcase pipeline.
        let preview_placements: Vec<crate::render::draw_cmd::ShowcaseTilePlacement> = {
            let tiles: Vec<Tile> = if self.tutorial_mode {
                hand_tiles
                    .into_iter()
                    .filter(|t| !matches!(t.suit, Suit::Flower))
                    .collect()
            } else {
                hand_tiles
            };
            let slots: Vec<(f32, f32, f32, f32)> = if self.tutorial_mode {
                hand_slots.into_iter().take(34).collect()
            } else {
                hand_slots
            };
            tiles
                .into_iter()
                .zip(slots)
                .map(|(tile, (sx, sy, sw, sh))| {
                    let cx = sx + sw * 0.5;
                    let cy = sy + sh * 0.5;
                    crate::render::draw_cmd::ShowcaseTilePlacement {
                        tile,
                        center_pos: [cx, cy, 0.0],
                        rotation: [0.0, 0.0, std::f32::consts::PI],
                        scale: 1.0,
                        size_px: sw,
                        brightness: 1.0,
                        selected: false,
                        hovered: false,
                        outline: false,
                        glow: false,
                        glow_color: None,
                        pick_id: None,
                    }
                })
                .collect()
        };

        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);
        frame.table();
        if !preview_placements.is_empty() {
            frame.showcase_tile_batch(preview_placements);
        }
        frame.quads(instances);
        frame.texts(text_labels);
        frame.tile_material_override = Some(self.material);
        frame.point_lights = vec![PointLight {
            pos: [w * 0.5, h * 0.05, h * 0.55],
            radius: h * 1.50,
            color: [1.00, 0.86, 0.55],
            intensity: 1.40,
        }];
        frame.fluid_smoke();
        frame.buttons = buttons;
        frame.window_title = if self.tutorial_mode {
            "Mahjuro — Tutorial Prompt".into()
        } else {
            "Mahjuro — Choose Tiles".into()
        };
        frame
    }
}
