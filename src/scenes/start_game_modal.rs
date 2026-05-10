//! "Choose Your Tiles" scene shown between the start screen and the first
//! shop. Left/right arrows cycle through tile materials; each material
//! displays its name and gameplay bonus. Play starts the run.

use crate::audio::SfxId;
use crate::core::stake::Stake;
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

use super::main_menu_exterior::MainMenuExteriorScene;
use super::shop::ShopScene;
use crate::render::draw_cmd::UiFrame;
use crate::render::world_space::{LayoutAnchorPx, layout_px_py_from_norm};

use super::{BackgroundId, ButtonDef, DrawCtx, Scene, SceneBehavior, SceneTransition, UpdateCtx};
use std::borrow::Cow;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModalAction {
    Play,
    SkipTutorial,
    Back,
    StakeSelect(Stake),
}

impl ModalAction {
    fn id(self) -> FocusId {
        let variant = match self {
            ModalAction::Play => 0,
            ModalAction::SkipTutorial => 1,
            ModalAction::Back => 2,
            ModalAction::StakeSelect(s) => {
                3 + Stake::ALL.iter().position(|k| *k == s).unwrap_or(0) as u32
            }
        };
        FocusId(0x2000_0000 + variant)
    }
}

/// Season emoji for the stake switcher tokens. Matches the seasonal naming
/// scheme (Spring → Summer → Autumn → Winter).
fn stake_glyph(stake: Stake) -> &'static str {
    match stake {
        Stake::Spring => "\u{1F331}",        // 🌱
        Stake::Summer => "\u{2600}\u{FE0F}", // ☀️
        Stake::Autumn => "\u{1F342}",        // 🍂
        Stake::Winter => "\u{2744}\u{FE0F}", // ❄️
    }
}

pub struct TileSelectScene {
    tree: TreeState,
    pub positions: crate::ui::scene_layout::TileSelectPositions,
    material: TileMaterial,
    /// Currently-selected difficulty stake. Cycled by StakePrev/StakeNext
    /// buttons on the modal; gated by `PlayerProgress::stake_unlocked_for`
    /// per-material so a player can't pick Winter on Bamboo unless they've
    /// cleared Autumn on Bamboo.
    stake: Stake,
    /// If true, the next run starts in tutorial mode instead of standard.
    tutorial_mode: bool,
}

impl TileSelectScene {
    pub fn new() -> Self {
        Self {
            tree: TreeState::new(),
            positions: crate::ui::scene_layout::load_tile_select_positions(),
            material: TileMaterial::default(),
            stake: Stake::default(),
            tutorial_mode: false,
        }
    }

    /// Create a tile-select scene that will start a tutorial run.
    pub fn new_tutorial() -> Self {
        Self {
            tree: TreeState::new(),
            positions: crate::ui::scene_layout::load_tile_select_positions(),
            material: TileMaterial::Bamboo,
            stake: Stake::Spring,
            tutorial_mode: true,
        }
    }

    /// Clamp the currently-selected stake back into the unlocked range for
    /// the current material. Called after material cycles so a player who
    /// had Autumn selected on Bamboo doesn't carry it to Tortoiseshell when
    /// Tortoiseshell only has Summer unlocked.
    fn clamp_stake_to_unlocks(&mut self, progress: &crate::core::progression::PlayerProgress) {
        while !progress.stake_unlocked_for(self.material, self.stake) {
            self.stake = match self.stake.previous() {
                Some(prev) => prev,
                None => {
                    // Spring is always unlocked; if we land here something
                    // else has gone wrong, but Spring is the safe floor.
                    break;
                }
            };
        }
    }

    /// Build the button-only widget tree. Text labels are emitted separately
    /// in `draw()` because `draw_decoration_top` doesn't support column layout.
    fn build_tree(
        &self,
        window_w: f32,
        window_h: f32,
        ui_scale: f32,
        progress: &crate::core::progression::PlayerProgress,
        positions: &crate::ui::scene_layout::TileSelectPositions,
    ) -> Tree<ModalAction> {
        let scale = metrics::scene_scale(window_w, window_h, ui_scale);
        let panel_w = window_w * 0.38;
        let btn_w = if self.tutorial_mode {
            (220.0 * scale).min(panel_w * 0.78)
        } else {
            (260.0 * scale).min(panel_w * 0.85)
        };

        let btn_h = if self.tutorial_mode {
            (38.0 * scale).max(26.0)
        } else {
            (46.0 * scale).max(30.0)
        };
        let btn_gap = if self.tutorial_mode {
            (10.0 * scale).max(6.0)
        } else {
            (12.0 * scale).max(6.0)
        };

        // Vertical offset matches the old tutorial vs standard split.
        let start_y = positions.button_menu.ny * window_h
            + if self.tutorial_mode {
                0.02 * window_h
            } else {
                0.0
            };
        let menu_x = positions.button_menu.nx * window_w;

        let (root, block_h) = if self.tutorial_mode {
            let items = vec![
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
            ];
            let h = items.len() as f32 * btn_h + (items.len().saturating_sub(1) as f32) * btn_gap;
            (
                wt::Node::Column {
                    gap: btn_gap,
                    align: wt::HAlign::Stretch,
                    children: items,
                },
                h,
            )
        } else {
            // Stake token row: one focusable button per season. Selected
            // season gets Primary (gold) for the "select box" look; locked
            // seasons are disabled Subtle so focus traversal skips them.
            let token_children: Vec<_> = Stake::ALL
                .iter()
                .map(|&s| {
                    let unlocked = progress.stake_unlocked_for(self.material, s);
                    let variant = if s == self.stake {
                        ButtonVariant::Primary
                    } else {
                        ButtonVariant::Subtle
                    };
                    // Locked seasons rely on the disabled button-state
                    // desaturation (handled by the theme) — no padlock glyph,
                    // the dimmed treatment is enough to read "locked".
                    let label = stake_glyph(s).to_string();
                    let tooltip = Some(Cow::Owned(format!("{} — {}", s.label(), s.description())));
                    wt::Node::Item(wt::Item {
                        id: ModalAction::StakeSelect(s).id(),
                        size: wt::Size::Auto,
                        enabled: unlocked,
                        tooltip,
                        kind: wt::ItemKind::Button {
                            label,
                            variant,
                            on_activate: ModalAction::StakeSelect(s),
                        },
                    })
                })
                .collect();
            let token_row = wt::Node::Row {
                gap: (8.0 * scale).max(4.0),
                align: wt::VAlign::Center,
                children: token_children,
            };
            // Play is the dominant CTA; Back sits below.
            let children = vec![
                token_row,
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
            let h = 3.0 * btn_h + 2.0 * btn_gap;
            (
                wt::Node::Column {
                    gap: btn_gap,
                    align: wt::HAlign::Stretch,
                    children,
                },
                h,
            )
        };

        Tree {
            root,
            anchor: Some([menu_x, start_y, btn_w, block_h]),
        }
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
            GameEngine::start_run_with_material_and_stake(
                run,
                self.material,
                self.stake,
                progress,
                &settings,
            );
            Some(Scene::Shop(ShopScene::new(run, progress)))
        }
    }

    /// Top-left `(x, y)` and `(width, height)` in layout pixels for the showcase grid.
    fn preview_grid_rect(&self, w: f32, h: f32) -> (f32, f32, f32, f32) {
        let tl = &self.positions.preview_corner_tl;
        let br = &self.positions.preview_corner_br;
        let mut x0 = tl.nx * w;
        let mut y0 = tl.ny * h;
        let mut x1 = br.nx * w;
        let mut y1 = br.ny * h;
        if x1 < x0 {
            std::mem::swap(&mut x0, &mut x1);
        }
        if y1 < y0 {
            std::mem::swap(&mut y0, &mut y1);
        }
        (x0, y0, (x1 - x0).max(1.0), (y1 - y0).max(1.0))
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

/// Key light height above the felt uses a fixed fraction of window height (screen-space cue).
const TILE_PREVIEW_KEY_LIGHT_LIFT_FRAC_OF_H: f32 = 0.80;

/// Compute 38 top-down pixel `(x, y, w, h)` slot rects (`y` increases downward; fed to
/// [`crate::render::world_space::pixel_to_world`] via showcase draws).
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
        // consume them as focus movement. Material changes re-clamp the stake
        // so the player can't carry an unlocked-on-one-material stake to
        // another where they haven't earned it.
        let mut filtered: Vec<UiAction> = Vec::new();
        for &a in ctx.actions {
            match a {
                UiAction::FocusNext => {
                    self.material = self.material.next();
                    self.clamp_stake_to_unlocks(ctx.progress);
                }
                UiAction::FocusPrev => {
                    self.material = self.material.prev();
                    self.clamp_stake_to_unlocks(ctx.progress);
                }
                UiAction::Cancel | UiAction::Pause => {
                    ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
                    return Some(Scene::MainMenuExterior(MainMenuExteriorScene::new()));
                }
                other => filtered.push(other),
            }
        }

        let tree = self.build_tree(w, h, ctx.ui_scale, ctx.progress, &self.positions);
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
                Some(Scene::Shop(ShopScene::new(ctx.run, ctx.progress)))
            }
            Some(ModalAction::Back) => {
                ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
                Some(Scene::MainMenuExterior(MainMenuExteriorScene::new()))
            }
            Some(ModalAction::StakeSelect(s)) => {
                // Locked stakes are non-focusable + disabled, so activation
                // here should only ever arrive for unlocked seasons; guard
                // anyway so a mouse click on a visible-but-locked token
                // can't slip through.
                if ctx.progress.stake_unlocked_for(self.material, s) {
                    if self.stake != s {
                        self.stake = s;
                        ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
                    }
                } else {
                    ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
                }
                None
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
        let body_px = typography::size(typography::BODY, h, ui_scale);
        let hint_px = typography::size(typography::CAPTION, h, ui_scale);

        // Rect heights need room above the font size for line padding.
        let title_h = title_px * 1.4;
        let name_h = name_px * 1.4;
        let bonus_h = bonus_px * 1.4;
        let body_h = body_px * 1.4;
        let hint_h = hint_px * 1.4;

        let text_x = self.positions.left_panel.nx * w;
        let mut cursor_y = self.positions.left_panel.ny * h
            + if self.tutorial_mode {
                0.12 * h
            } else {
                0.0
            };
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
                    color: color::STONE,
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
            // Material block: name + bonus as a "stat" row (gold glyph + label).
            text_labels.push(TextLabel {
                rect: [text_x, cursor_y, text_w, name_h],
                text: self.material.label().into(),
                color: color::CHAMPAGNE,
                font_px: Some(name_px),
                ..Default::default()
            });
            cursor_y += name_h + gap_sm * 0.5;

            text_labels.push(TextLabel {
                rect: [text_x, cursor_y, text_w, bonus_h],
                text: format!("\u{2022}  {}", self.material.bonus_description()).into(),
                color: color::BRASS,
                font_px: Some(bonus_px),
                ..Default::default()
            });
            cursor_y += bonus_h + gap_lg;

            // Stake description sits just above the season-token row so the
            // player sees at a glance *what* their current pick does. The
            // selected token itself names the season (via its gold highlight),
            // so we don't repeat "Spring / Summer / …" here.
            text_labels.push(TextLabel {
                rect: [text_x, cursor_y, text_w, hint_h],
                text: "STAKE".into(),
                color: color::UMBER,
                font_px: Some(hint_px),
                ..Default::default()
            });
            cursor_y += hint_h + gap_sm * 0.25;

            text_labels.push(TextLabel {
                rect: [text_x, cursor_y, text_w, body_h],
                text: format!(
                    "{} \u{2014} {}",
                    self.stake.label(),
                    self.stake.description()
                )
                .into(),
                color: color::STONE,
                font_px: Some(body_px),
                ..Default::default()
            });
        }

        // Hint at the bottom of the panel — only surface info the buttons
        // don't already teach. Left/right already have a visible chevron row;
        // Esc does not, so that's what the hint says.
        let hint_panel_x = self.positions.bottom_hint.nx * w;
        let hint_panel_y = self.positions.bottom_hint.ny * h;
        text_labels.push(TextLabel {
            rect: [hint_panel_x, hint_panel_y, text_w, hint_h],
            text: if self.tutorial_mode {
                "Enter to confirm the focused option".into()
            } else {
                "Esc to go back".into()
            },
            color: color::UMBER,
            font_px: Some(hint_px),
            ..Default::default()
        });

        // ── Buttons (via widget tree) ──────────────────────────────
        let tree = self.build_tree(w, h, ui_scale, ctx.progress, &self.positions);
        let mut tree_frame = TreeFrame {
            instances: &mut instances,
            labels: &mut text_labels,
            buttons: &mut buttons,
            window: (w, h),
        };
        self.tree.draw(&tree, &mut tree_frame, &noop_render_custom);

        // ── Tile preview grid on the right ─────────────────────────
        let (grid_x, grid_y, grid_w, grid_h) = self.preview_grid_rect(w, h);
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
        // Key light positioned above the tile cluster so the warm specular
        // falls on the tiles themselves rather than puddling on the wood
        // floor in front. Intensity dialed back so the hero art reads clean.
        let kl = &self.positions.key_light;
        let (key_px, key_py) = layout_px_py_from_norm(w, h, kl.nx, kl.ny);
        let key_light = LayoutAnchorPx {
            px: key_px,
            py: key_py,
            lift_z: TILE_PREVIEW_KEY_LIGHT_LIFT_FRAC_OF_H * h,
        };
        frame.scene_lighting.set_smooth_points(vec![PointLight {
            pos: key_light.to_draw_cmd_triple(),
            radius: h * 1.80,
            color: [1.00, 0.88, 0.62],
            intensity: 1.05,
        }]);
        frame.buttons = buttons;
        frame.window_title = if self.tutorial_mode {
            "Mahjuro — Tutorial Prompt".into()
        } else {
            "Mahjuro — Choose Tiles".into()
        };
        frame
    }
}
