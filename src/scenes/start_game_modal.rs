//! "Choose Your Tiles" scene shown between the start screen and the first
//! shop. Prev/next arrows cycle tile materials; each material displays its
//! name and gameplay bonus. Play starts the run.

use crate::core::season::Season;
use crate::core::tile::{Suit, Tile};
use crate::game::engine::GameEngine;
use crate::game::event_bus::GameEvent;
use crate::game::run::RunState;
use crate::persistence::TileMaterial;
use crate::render::theme::{ButtonState, ButtonVariant, button_colors, color, metrics, typography};
use crate::render::wgpu_renderer::{GpuInstance, PointLight, TextAlign, TextLabel};
use crate::sfx_id::SfxId;
use crate::ui::controller_hints::{HintStyle, menu_footer_row, push_screen_footer_hint};
use crate::ui::focus_nav::{self, FocusDir};
use crate::ui::input::{InputMode, UiAction};
use crate::ui::widget::{self, TextStyle};
use crate::ui::widget_tree::FocusId;

use super::main_menu::MainMenuScene;
use super::shop::ShopScene;
use crate::render::draw_cmd::UiFrame;
use crate::render::world_space::{LayoutAnchorPx, layout_px_py_from_norm};

use super::{BackgroundId, ButtonDef, DrawCtx, Scene, SceneBehavior, SceneTransition, UpdateCtx};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModalAction {
    MaterialPrev,
    MaterialNext,
    Play,
    SkipTutorial,
    Back,
    SeasonSelect(Season),
}

impl ModalAction {
    fn id(self) -> FocusId {
        let variant = match self {
            ModalAction::MaterialPrev => 0,
            ModalAction::MaterialNext => 1,
            ModalAction::Play => 2,
            ModalAction::SkipTutorial => 3,
            ModalAction::Back => 4,
            ModalAction::SeasonSelect(s) => {
                5 + Season::ALL.iter().position(|k| *k == s).unwrap_or(0) as u32
            }
        };
        FocusId(0x2000_0000 + variant)
    }
}

/// Shared left-column geometry for copy, material picker, and button stack.
struct LeftPanelLayout {
    x: f32,
    w: f32,
    scale: f32,
    gap_sm: f32,
    gap_lg: f32,
    title_h: f32,
    bonus_h: f32,
    season_desc_h: f32,
    hint_h: f32,
    material_row_h: f32,
    menu_gap: f32,
    menu_y: f32,
}

impl LeftPanelLayout {
    fn compute(
        w: f32,
        h: f32,
        positions: &crate::ui::scene_layout::TileSelectPositions,
        tutorial_mode: bool,
    ) -> Self {
        let scale = metrics::scene_scale(w, h);
        let panel_w = w * 0.38;
        let gap_sm = (16.0 * scale).max(8.0);
        let gap_lg = (28.0 * scale).max(14.0);

        let title_px = typography::size(typography::H16, h);
        let bonus_px = typography::size(typography::H28, h);
        let season_desc_px = typography::size(typography::H28, h);
        let hint_px = typography::size(typography::H42, h);

        let title_h = crate::ui::colored_keywords::colored_row_line_step(title_px);
        let bonus_h = crate::ui::colored_keywords::colored_row_line_step(bonus_px);
        let season_desc_h = crate::ui::colored_keywords::colored_row_line_step(season_desc_px);
        let hint_h = crate::ui::colored_keywords::colored_row_line_step(hint_px);

        let x = positions.left_panel.nx * w;
        let content_w = panel_w * 0.90;
        let material_row_h = (44.0 * scale).max(36.0);
        let menu_gap = if tutorial_mode {
            (10.0 * scale).max(6.0)
        } else {
            (12.0 * scale).max(6.0)
        };

        let mut content_y = positions.left_panel.ny * h;
        if tutorial_mode {
            content_y += 0.12 * h;
        }
        content_y += title_h + gap_lg;

        let menu_y = if tutorial_mode {
            positions.button_menu.ny * h + 0.02 * h
        } else {
            content_y += material_row_h + gap_sm * 0.5;
            content_y += bonus_h + gap_lg;
            content_y += hint_h + gap_sm * 0.25;
            content_y += season_desc_h + gap_lg;
            content_y
        };

        Self {
            x,
            w: content_w,
            scale,
            gap_sm,
            gap_lg,
            title_h,
            bonus_h,
            season_desc_h,
            hint_h,
            material_row_h,
            menu_gap,
            menu_y,
        }
    }

    fn material_row_y(
        &self,
        h: f32,
        positions: &crate::ui::scene_layout::TileSelectPositions,
    ) -> f32 {
        positions.left_panel.ny * h + self.title_h + self.gap_lg
    }

    fn material_row(
        &self,
        h: f32,
        positions: &crate::ui::scene_layout::TileSelectPositions,
    ) -> MaterialRowLayout {
        let row_y = self.material_row_y(h, positions);
        let row_h = self.material_row_h;
        let gap = (4.0 * self.scale).max(2.0);
        MaterialRowLayout {
            prev: [self.x, row_y, row_h, row_h],
            next: [self.x + self.w - row_h, row_y, row_h, row_h],
            name: [
                self.x + row_h + gap,
                row_y,
                (self.w - 2.0 * (row_h + gap)).max(0.0),
                row_h,
            ],
        }
    }

    fn menu_item_h(&self) -> f32 {
        (38.0 * self.scale).max(24.0)
    }

    /// Season token, Play, and Back rects for spatial nav + draw.
    fn menu_button_rects(&self) -> (Vec<(Season, [f32; 4])>, [f32; 4], [f32; 4]) {
        let row_h = self.menu_item_h();
        let row_y = self.menu_y;
        let gap = (8.0 * self.scale).max(4.0);
        let n = Season::ALL.len() as f32;
        let total_gap = gap * (n - 1.0);
        let cw = ((self.w - total_gap) / n).max(0.0);
        let mut cx = self.x;
        let mut seasons = Vec::with_capacity(Season::ALL.len());
        for season in Season::ALL {
            seasons.push((season, [cx, row_y, cw, row_h]));
            cx += cw + gap;
        }
        let play_y = row_y + row_h + self.menu_gap;
        let play = [self.x, play_y, self.w, row_h];
        let back_y = play_y + row_h + self.menu_gap;
        let back = [self.x, back_y, self.w, row_h];
        (seasons, play, back)
    }
}

struct MaterialRowLayout {
    name: [f32; 4],
    prev: [f32; 4],
    next: [f32; 4],
}

fn point_in_rect((x, y): (f32, f32), rect: [f32; 4]) -> bool {
    let [rx, ry, rw, rh] = rect;
    x >= rx && x <= rx + rw && y >= ry && y <= ry + rh
}

fn push_material_arrow(
    instances: &mut Vec<GpuInstance>,
    text_labels: &mut Vec<TextLabel>,
    buttons: &mut Vec<ButtonDef>,
    rect: [f32; 4],
    label: &str,
    click_id: u32,
    hovered: bool,
) {
    let state = if hovered {
        ButtonState::Hover
    } else {
        ButtonState::Rest
    };
    let colors = button_colors(ButtonVariant::Default, state);
    instances.push(GpuInstance {
        rect,
        color: colors.bg,
        user: 0,
    });
    text_labels.push(TextLabel {
        rect,
        text: label.into(),
        color: colors.text,
        align: TextAlign::Center,
        ..Default::default()
    });
    buttons.push(ButtonDef::scene(
        (rect[0], rect[1], rect[2], rect[3]),
        click_id,
    ));
}

fn push_modal_button(
    instances: &mut Vec<GpuInstance>,
    text_labels: &mut Vec<TextLabel>,
    buttons: &mut Vec<ButtonDef>,
    rect: [f32; 4],
    label: &str,
    click_id: u32,
    variant: ButtonVariant,
    enabled: bool,
    focused: bool,
    hovered: bool,
    scale: f32,
    window_w: f32,
    window_h: f32,
) {
    let state = if !enabled {
        ButtonState::Disabled
    } else if focused || hovered {
        ButtonState::Hover
    } else {
        ButtonState::Rest
    };
    if focused {
        focus_nav::push_focus_ring(rect, scale, window_w, window_h, instances);
    }
    let colors = button_colors(variant, state);
    instances.push(GpuInstance {
        rect,
        color: colors.bg,
        user: 0,
    });
    text_labels.push(TextLabel {
        rect,
        text: label.into(),
        color: colors.text,
        align: TextAlign::Center,
        ..Default::default()
    });
    if enabled {
        buttons.push(ButtonDef::scene(
            (rect[0], rect[1], rect[2], rect[3]),
            click_id,
        ));
    }
}

/// Season emoji for the season switcher tokens. Matches the seasonal naming
/// scheme (Spring → Summer → Autumn → Winter).
fn season_glyph(season: Season) -> &'static str {
    match season {
        Season::Spring => "\u{1F331}",        // 🌱
        Season::Summer => "\u{2600}\u{FE0F}", // ☀️
        Season::Autumn => "\u{1F342}",        // 🍂
        Season::Winter => "\u{2744}\u{FE0F}", // ❄️
    }
}

pub struct TileSelectScene {
    focus: Option<ModalAction>,
    pub positions: crate::ui::scene_layout::TileSelectPositions,
    material: TileMaterial,
    /// Currently-selected difficulty season. Cycled by SeasonPrev/SeasonNext
    /// buttons on the modal; gated by `PlayerProgress::season_unlocked_for`
    /// per-material so a player can't pick Winter on Bamboo unless they've
    /// cleared Autumn on Bamboo.
    season: Season,
    /// If true, the next run starts in tutorial mode instead of standard.
    tutorial_mode: bool,
}

impl Default for TileSelectScene {
    fn default() -> Self {
        Self::new()
    }
}

impl TileSelectScene {
    pub fn new() -> Self {
        Self {
            focus: None,
            positions: crate::ui::scene_layout::TileSelectPositions::default(),
            material: TileMaterial::default(),
            season: Season::default(),
            tutorial_mode: false,
        }
    }

    /// Create a tile-select scene that will start a tutorial run.
    pub fn new_tutorial() -> Self {
        Self {
            focus: None,
            positions: crate::ui::scene_layout::TileSelectPositions::default(),
            material: TileMaterial::Bamboo,
            season: Season::Spring,
            tutorial_mode: true,
        }
    }

    /// Clamp the currently-selected season back into the unlocked range for
    /// the current material. Called after material cycles so a player who
    /// had Autumn selected on Bamboo doesn't carry it to Tortoiseshell when
    /// Tortoiseshell only has Summer unlocked.
    fn clamp_season_to_unlocks(&mut self, progress: &crate::core::progression::PlayerProgress) {
        while !progress.season_unlocked_for(self.material, self.season) {
            self.season = match self.season.previous() {
                Some(prev) => prev,
                None => {
                    // Spring is always unlocked; if we land here something
                    // else has gone wrong, but Spring is the safe floor.
                    break;
                }
            };
        }
    }

    fn clamp_material_to_unlocks(&mut self, progress: &crate::core::progression::PlayerProgress) {
        if progress.material_unlocked(self.material) {
            return;
        }
        self.material = TileMaterial::default();
        self.clamp_season_to_unlocks(progress);
    }

    /// Spatial focus graph: `(action, screen rect)` for every navigable control.
    fn focus_targets(
        &self,
        window_w: f32,
        window_h: f32,
        progress: &crate::core::progression::PlayerProgress,
        positions: &crate::ui::scene_layout::TileSelectPositions,
    ) -> Vec<(ModalAction, [f32; 4])> {
        let panel = LeftPanelLayout::compute(window_w, window_h, positions, self.tutorial_mode);
        let mut targets = Vec::new();

        if !self.tutorial_mode {
            let material_row = panel.material_row(window_h, positions);
            targets.push((ModalAction::MaterialPrev, material_row.prev));
            targets.push((ModalAction::MaterialNext, material_row.next));

            let (season_rects, play_rect, back_rect) = panel.menu_button_rects();
            for (season, rect) in season_rects {
                if progress.season_unlocked_for(self.material, season) {
                    targets.push((ModalAction::SeasonSelect(season), rect));
                }
            }
            targets.push((ModalAction::Play, play_rect));
            targets.push((ModalAction::Back, back_rect));
        } else {
            let row_h = panel.menu_item_h();
            let mut y = panel.menu_y;
            for action in [
                ModalAction::Play,
                ModalAction::SkipTutorial,
                ModalAction::Back,
            ] {
                targets.push((action, [panel.x, y, panel.w, row_h]));
                y += row_h + panel.menu_gap;
            }
        }

        targets
    }

    fn default_focus(&self, targets: &[(ModalAction, [f32; 4])]) -> Option<ModalAction> {
        targets
            .iter()
            .find(|(action, _)| matches!(action, ModalAction::Play))
            .map(|(action, _)| *action)
            .or_else(|| targets.first().map(|(action, _)| *action))
    }

    fn clamp_focus(&mut self, targets: &[(ModalAction, [f32; 4])]) {
        let valid = |focus: ModalAction| targets.iter().any(|(action, _)| *action == focus);
        if self.focus.is_none_or(|focus| !valid(focus)) {
            self.focus = self.default_focus(targets);
        }
    }

    fn current_focus_rect(
        focus: Option<ModalAction>,
        targets: &[(ModalAction, [f32; 4])],
    ) -> Option<[f32; 4]> {
        let focus = focus?;
        targets
            .iter()
            .find(|(action, _)| *action == focus)
            .map(|(_, rect)| *rect)
    }

    fn activate_focus(&mut self, action: ModalAction, ctx: &mut UpdateCtx<'_>) -> SceneTransition {
        match action {
            ModalAction::MaterialPrev => {
                self.material = ctx.progress.prev_unlocked_material(self.material);
                self.clamp_season_to_unlocks(ctx.progress);
                ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
                None
            }
            ModalAction::MaterialNext => {
                self.material = ctx.progress.next_unlocked_material(self.material);
                self.clamp_season_to_unlocks(ctx.progress);
                ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
                None
            }
            ModalAction::Play => {
                if !ctx.loading_done {
                    ctx.bus.push(GameEvent::UiSound(SfxId::InvalidAction));
                    return None;
                }
                ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                self.start_game(ctx.run, ctx.progress)
            }
            ModalAction::SkipTutorial => {
                if !ctx.loading_done {
                    ctx.bus.push(GameEvent::UiSound(SfxId::InvalidAction));
                    return None;
                }
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
            ModalAction::Back => {
                ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
                Some(Scene::MainMenu(MainMenuScene::new()))
            }
            ModalAction::SeasonSelect(season) => {
                if ctx.progress.season_unlocked_for(self.material, season) {
                    if self.season != season {
                        self.season = season;
                        ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
                    }
                } else {
                    ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
                }
                None
            }
        }
    }

    fn draw_menu_buttons(
        &self,
        panel: &LeftPanelLayout,
        progress: &crate::core::progression::PlayerProgress,
        cursor_pos: (f32, f32),
        input_mode: InputMode,
        window_w: f32,
        window_h: f32,
        instances: &mut Vec<GpuInstance>,
        text_labels: &mut Vec<TextLabel>,
        buttons: &mut Vec<ButtonDef>,
    ) {
        let scale = panel.scale;
        let focused = self.focus;
        let cursor_hover =
            |rect: [f32; 4]| input_mode == InputMode::Cursor && point_in_rect(cursor_pos, rect);

        if self.tutorial_mode {
            let row_h = panel.menu_item_h();
            let mut y = panel.menu_y;
            for (action, label, variant) in [
                (ModalAction::Play, "Play Tutorial", ButtonVariant::Primary),
                (
                    ModalAction::SkipTutorial,
                    "Skip Tutorial",
                    ButtonVariant::Default,
                ),
                (ModalAction::Back, "Back", ButtonVariant::Default),
            ] {
                let rect = [panel.x, y, panel.w, row_h];
                let is_focused = focused == Some(action);
                push_modal_button(
                    instances,
                    text_labels,
                    buttons,
                    rect,
                    label,
                    action.id().0,
                    variant,
                    true,
                    is_focused,
                    cursor_hover(rect),
                    scale,
                    window_w,
                    window_h,
                );
                y += row_h + panel.menu_gap;
            }
            return;
        }

        let (season_rects, play_rect, back_rect) = panel.menu_button_rects();
        for (season, rect) in season_rects {
            let action = ModalAction::SeasonSelect(season);
            let unlocked = progress.season_unlocked_for(self.material, season);
            let variant = if self.season == season {
                ButtonVariant::Primary
            } else {
                ButtonVariant::Subtle
            };
            push_modal_button(
                instances,
                text_labels,
                buttons,
                rect,
                season_glyph(season),
                action.id().0,
                variant,
                unlocked,
                focused == Some(action),
                cursor_hover(rect),
                scale,
                window_w,
                window_h,
            );
        }

        push_modal_button(
            instances,
            text_labels,
            buttons,
            play_rect,
            "Play",
            ModalAction::Play.id().0,
            ButtonVariant::Primary,
            true,
            focused == Some(ModalAction::Play),
            cursor_hover(play_rect),
            scale,
            window_w,
            window_h,
        );
        push_modal_button(
            instances,
            text_labels,
            buttons,
            back_rect,
            "Back",
            ModalAction::Back.id().0,
            ButtonVariant::Default,
            true,
            focused == Some(ModalAction::Back),
            cursor_hover(back_rect),
            scale,
            window_w,
            window_h,
        );
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
            GameEngine::start_run_with_material_and_season(
                run,
                self.material,
                self.season,
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
    for suit in [Suit::Manzu, Suit::Souzu, Suit::Pinzu] {
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
    (0, 9),  // Manzu 1–9
    (9, 9),  // Souzu 1–9
    (18, 9), // Pinzu 1–9
    (27, 7), // Winds 1–4 + Dragons 1–3
    (34, 4), // Flowers 1–4
];

/// Key light height above the felt uses a fixed fraction of window height (screen-space cue).
const TILE_PREVIEW_KEY_LIGHT_LIFT_FRAC_OF_H: f32 = 0.80;

/// Compute 38 top-down pixel `(x, y, w, h)` slot rects (`y` increases downward; fed to
/// [`crate::render::world_space::layout_anchor_to_world`] via showcase draws).
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
    fn update(&mut self, mut ctx: UpdateCtx<'_>) -> SceneTransition {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;

        if !self.tutorial_mode {
            self.clamp_material_to_unlocks(ctx.progress);
        }

        let targets = self.focus_targets(w, h, ctx.progress, &self.positions);
        self.clamp_focus(&targets);
        let prev_focus = self.focus;

        for &a in ctx.actions {
            if matches!(a, UiAction::Cancel | UiAction::Pause) {
                ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
                return Some(Scene::MainMenu(MainMenuScene::new()));
            }
        }

        let pointer_pick = if ctx.input_mode == InputMode::Cursor && !targets.is_empty() {
            focus_nav::focus_target_at_cursor(&targets, ctx.cursor_pos.0, ctx.cursor_pos.1)
        } else {
            None
        };

        if ctx.input_mode == InputMode::Cursor
            && let Some(action) = pointer_pick
        {
            self.focus = Some(action);
        }

        for &cid in ctx.button_clicks {
            if let Some(action) = targets
                .iter()
                .find(|(target, _)| target.id().0 == cid)
                .map(|(target, _)| *target)
            {
                self.focus = Some(action);
                let transition = self.activate_focus(action, &mut ctx);
                let targets = self.focus_targets(w, h, ctx.progress, &self.positions);
                self.clamp_focus(&targets);
                return transition;
            }
        }

        let current_rect = Self::current_focus_rect(self.focus, &targets);
        let mut activated = false;
        for &a in ctx.actions {
            let dir = match a {
                UiAction::FocusUp => Some(FocusDir::Up),
                UiAction::FocusDown => Some(FocusDir::Down),
                UiAction::FocusPrev => Some(FocusDir::Left),
                UiAction::FocusNext => Some(FocusDir::Right),
                UiAction::Confirm | UiAction::CommitDiscard => {
                    activated = true;
                    None
                }
                _ => None,
            };
            if let Some(dir) = dir
                && let Some(rect) = current_rect
                && let Some(next) = focus_nav::pick_neighbor(rect, dir, &targets)
            {
                self.focus = Some(next);
            }
        }

        if ctx.input_mode == InputMode::Cursor
            && !ctx.button_clicks.is_empty()
            && pointer_pick.is_some()
        {
            activated = true;
        }

        if self.focus != prev_focus {
            ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
        }

        if activated {
            let Some(action) = self.focus else {
                return None;
            };
            let transition = self.activate_focus(action, &mut ctx);
            let targets = self.focus_targets(w, h, ctx.progress, &self.positions);
            self.clamp_focus(&targets);
            return transition;
        }

        None
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;

        let mut instances: Vec<GpuInstance> = Vec::new();
        let mut text_labels: Vec<TextLabel> = Vec::new();
        let mut buttons: Vec<ButtonDef> = Vec::new();

        let panel = LeftPanelLayout::compute(w, h, &self.positions, self.tutorial_mode);
        let title_px = typography::size(typography::H16, h);
        let name_px = typography::size(typography::H20, h);
        let bonus_px = typography::size(typography::H28, h);
        let season_desc_px = typography::size(typography::H28, h);
        let hint_px = typography::size(typography::H42, h);

        let mut cursor_y =
            self.positions.left_panel.ny * h + if self.tutorial_mode { 0.12 * h } else { 0.0 };

        let title_text = if self.tutorial_mode {
            "First-Time Tutorial"
        } else {
            "Choose Your Tiles"
        };
        text_labels.push(TextLabel {
            rect: [panel.x, cursor_y, panel.w, panel.title_h],
            text: title_text.into(),
            color: color::CHAMPAGNE,
            font_px: Some(title_px),
            ..Default::default()
        });
        cursor_y += panel.title_h + panel.gap_lg;

        if self.tutorial_mode {
            let intro_h = 90.0 * panel.scale;
            widget::push_text_block(
                &mut text_labels,
                [panel.x, cursor_y, panel.w, intro_h],
                "A short guided campaign teaches melds, structure scoring, relics, bosses, and the shop before one final practice fight.",
                TextStyle {
                    tier: typography::H28,
                    color: color::STONE,
                    padding: 0.0,
                    align: TextAlign::Left,
                    ..Default::default()
                },
                h,
            );
            cursor_y += intro_h + 10.0 * panel.scale;
            let skip_h = 50.0 * panel.scale;
            widget::push_text_block(
                &mut text_labels,
                [panel.x, cursor_y, panel.w, skip_h],
                "Skip marks the tutorial complete for this profile and starts a normal run immediately.",
                TextStyle {
                    tier: typography::H42,
                    color: color::PARCHMENT,
                    padding: 0.0,
                    align: TextAlign::Left,
                    ..Default::default()
                },
                h,
            );
        } else {
            let material_row = panel.material_row(h, &self.positions);

            text_labels.push(TextLabel {
                rect: material_row.name,
                text: self.material.label().into(),
                color: color::CHAMPAGNE,
                font_px: Some(name_px),
                align: TextAlign::Center,
                ..Default::default()
            });
            let cursor_pos = ctx.cursor_pos;
            let focused = self.focus;
            let hover_prev = focused == Some(ModalAction::MaterialPrev)
                || (ctx.input_mode == InputMode::Cursor
                    && point_in_rect(cursor_pos, material_row.prev));
            let hover_next = focused == Some(ModalAction::MaterialNext)
                || (ctx.input_mode == InputMode::Cursor
                    && point_in_rect(cursor_pos, material_row.next));
            push_material_arrow(
                &mut instances,
                &mut text_labels,
                &mut buttons,
                material_row.prev,
                "\u{25C0}",
                ModalAction::MaterialPrev.id().0,
                hover_prev,
            );
            push_material_arrow(
                &mut instances,
                &mut text_labels,
                &mut buttons,
                material_row.next,
                "\u{25B6}",
                ModalAction::MaterialNext.id().0,
                hover_next,
            );
            cursor_y += panel.material_row_h + panel.gap_sm * 0.5;

            text_labels.push(TextLabel {
                rect: [panel.x, cursor_y, panel.w, panel.bonus_h],
                text: format!("\u{2022}  {}", self.material.bonus_description()),
                color: color::BRASS,
                font_px: Some(bonus_px),
                ..Default::default()
            });
            cursor_y += panel.bonus_h + panel.gap_lg;

            text_labels.push(TextLabel {
                rect: [panel.x, cursor_y, panel.w, panel.hint_h],
                text: "SEASON".into(),
                color: color::UMBER,
                font_px: Some(hint_px),
                ..Default::default()
            });
            cursor_y += panel.hint_h + panel.gap_sm * 0.25;

            text_labels.push(TextLabel {
                rect: [panel.x, cursor_y, panel.w, panel.season_desc_h],
                text: format!(
                    "{} \u{2014} {}",
                    self.season.label(),
                    self.season.description()
                ),
                color: color::STONE,
                font_px: Some(season_desc_px),
                ..Default::default()
            });
        }

        self.draw_menu_buttons(
            &panel,
            ctx.progress,
            ctx.cursor_pos,
            ctx.input_mode,
            w,
            h,
            &mut instances,
            &mut text_labels,
            &mut buttons,
        );

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
                        overlay_rect_group: None,
                    }
                })
                .collect()
        };

        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);
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
        push_screen_footer_hint(
            &mut frame,
            &ctx,
            menu_footer_row(ctx.input_mode),
            HintStyle::standard(h),
        );
        frame.window_title = if self.tutorial_mode {
            "Mahjuro — Tutorial Prompt".into()
        } else {
            "Mahjuro — Choose Tiles".into()
        };
        frame
    }
}
