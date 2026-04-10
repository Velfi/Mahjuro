//! "Choose Your Tiles" scene shown between the start screen and the first
//! shop. Left/right arrows cycle through tile materials; each material
//! displays its name and gameplay bonus. Play starts the run.

use crate::core::tile::{Suit, Tile};
use crate::game::run::RunState;
use crate::persistence::TileMaterial;
use crate::render::theme::{ButtonVariant, color, metrics, typography};
use crate::render::wgpu_renderer::{GpuInstance, PointLight, TextLabel};
use crate::ui::input::UiAction;
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
        let btn_w = (240.0 * scale).min(panel_w * 0.85);

        let btn_h = (44.0 * scale).max(28.0);
        let btn_gap = (12.0 * scale).max(6.0);

        let block_h = 2.0 * btn_h + btn_gap;
        let start_y = window_h * 0.55;
        let menu_x = (panel_w - btn_w) * 0.5;

        let items = vec![
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
        ];

        Tree::vertical_menu(items).with_anchor([menu_x, start_y, btn_w, block_h])
    }

    fn start_game(&self, run: &mut RunState) -> SceneTransition {
        if self.tutorial_mode {
            *run = RunState::new_tutorial();
            // Tutorial skips the initial shop — go straight to gameplay.
            Some(Scene::Gameplay(super::gameplay::GameplayScene::new()))
        } else {
            *run = RunState::new_with_material(self.material);
            Some(Scene::Shop(ShopScene::new(run.run_number, run)))
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

        match action {
            Some(ModalAction::Play) => self.start_game(ctx.run),
            Some(ModalAction::Back) => Some(Scene::StartScreen(StartScreenScene::new())),
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

        let title_h = typography::size(2.25, h, ui_scale);
        let name_h = typography::size(typography::TITLE, h, ui_scale);
        let bonus_h = typography::size(typography::HEADING, h, ui_scale);
        let hint_h = typography::size(typography::CAPTION, h, ui_scale);

        let text_block_h = title_h + gap_sm + name_h + gap_sm + bonus_h;
        let mut cursor_y = (h * 0.5 - text_block_h) * 0.5 + h * 0.05;
        let text_x = panel_w * 0.05;
        let text_w = panel_w * 0.90;

        text_labels.push(TextLabel {
            rect: [text_x, cursor_y, text_w, title_h],
            text: "Choose Your Tiles".into(),
            color: color::CHAMPAGNE,
            ..Default::default()
        });
        cursor_y += title_h + gap_lg;

        text_labels.push(TextLabel {
            rect: [text_x, cursor_y, text_w, name_h],
            text: self.material.label().into(),
            color: color::CHAMPAGNE,
            ..Default::default()
        });
        cursor_y += name_h + gap_sm;

        text_labels.push(TextLabel {
            rect: [text_x, cursor_y, text_w, bonus_h],
            text: self.material.bonus_description().into(),
            color: color::MIST,
            ..Default::default()
        });

        // Hint at the bottom of the panel.
        let hint_y = h - hint_h - (12.0 * scale);
        text_labels.push(TextLabel {
            rect: [text_x, hint_y, text_w, hint_h],
            text: "\u{25C0}  \u{25B6}  Change tiles   |   Enter to play".into(),
            color: color::SLATE,
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

        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);
        frame.table();
        frame.hand_tile_backdrop();
        frame.quads(instances);
        frame.hand_tile_faces();
        frame.texts(text_labels);
        frame.hand_tiles = hand_tiles;
        frame.hand_slots = hand_slots;
        frame.focus = usize::MAX;
        frame.tile_material_override = Some(self.material);
        frame.point_lights = vec![PointLight {
            pos: [w * 0.5, h * 0.05, h * 0.55],
            radius: h * 1.50,
            color: [1.00, 0.86, 0.55],
            intensity: 1.40,
        }];
        frame.fluid_smoke();
        frame.buttons = buttons;
        frame.window_title = "Mahjuro — Choose Tiles".into();
        frame
    }
}
