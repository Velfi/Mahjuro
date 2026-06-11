//! Post-ordeal interstitial — [`staircase.glb`](../../assets/3d/staircase.glb) and optional decimation.

use std::time::Instant;

use crate::core::relic::{RelicFlavorSpan, flavor_spans_plain_text};
use crate::core::tile::Tile;
use crate::game::decimation::{
    HOUSE_PICKS, PLAYER_PICKS, apply_decimation, can_seal_decimation, decimation_eligible_tiles,
    decimation_house_pool, pick_house_tiles,
};
use crate::game::engine::GameEngine;
use crate::game::event_bus::GameEvent;
use crate::render::draw_cmd::{SceneLighting, UiFrame};
use crate::render::particles::ParticleSystem;
use crate::render::scene_keys;
use crate::render::staircase_glb;
use crate::render::theme::{color, metrics, typography};
use crate::render::vocabulary_colors::GlossaryMode;
use crate::render::wgpu_renderer::{
    GpuInstance, PointLight, TextAlign, TextBlockVerticalAlign, TextLabel,
};
use crate::sfx_id::SfxId;
use crate::ui::controller_hints::{
    HintStyle, decimation_footer_row, hint_style_with_alpha, push_screen_footer_hint,
    screen_footer_reserve, stairway_prompt_footer_row,
};
use crate::ui::focus_nav::{self, FocusDir, FocusNavState};
use crate::ui::input::{InputMode, MarqueeSelect, UiAction};
use crate::ui::inspect_plaque::{estimated_flavor_line_count, flavor_spans_layout_width};
use crate::ui::styled_text;
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use super::header_chrome::HeaderTitleLayout;
use super::pause_menu::PauseMenu;
use super::tile_picker::{
    SCROLLABLE_GRID_COLS, ScrollableTilePickerConfig, ScrollableTilePickerLayout,
    apply_pick_selection_mask, camera_params, compute_decimation_reveal_layout,
    compute_scrollable_tile_picker_layout, footer_button_rects, grid_marquee_swept_slots,
    pick_selection_mask, picker_header_chrome, picker_seal_button_rect, push_tile_picker_scrollbar,
    tile_picker_scroll_y_from_cursor, tile_picker_scrollbar,
};
use super::{
    BackgroundId, DrawCtx, OverlayRequest, Scene, SceneBehavior, SceneIntent, SceneTransition,
    UpdateCtx,
};

#[derive(Clone, Debug)]
enum StairwayPhase {
    Prompt,
    Picking {
        selected: Vec<u32>,
        display_tiles: Vec<Tile>,
    },
    Revealed {
        player: [u32; PLAYER_PICKS],
        house: [u32; HOUSE_PICKS],
        display_tiles: Vec<Tile>,
    },
    Burning {
        player: [u32; PLAYER_PICKS],
        house: [u32; HOUSE_PICKS],
        display_tiles: Vec<Tile>,
        started_at: Instant,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PromptAction {
    Decimate,
    Descend,
}

impl PromptAction {
    fn id(self) -> FocusId {
        FocusId(self as u32 + 0xC001)
    }

    fn label(self) -> &'static str {
        match self {
            Self::Decimate => "undergo the ritual of decimation",
            Self::Descend => "pass unscathed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DecimationAction {
    Seal,
    Cancel,
    Continue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DecimationFocus {
    Tile(usize),
    Seal,
    Cancel,
}

const DECIMATION_SCROLL_LINES_PX: f32 = 52.0;

struct DecimationPickingChrome {
    back: [f32; 4],
    seal: [f32; 4],
    copy_x: f32,
    title_y: f32,
    subtitle_y: f32,
    viewport: [f32; 4],
}

fn decimation_copy_inset_x(w: f32) -> f32 {
    w * 0.055
}

fn decimation_picking_chrome(w: f32, h: f32) -> DecimationPickingChrome {
    let scale = metrics::scene_scale(w, h);
    let (back, _chrome_bottom) = picker_header_chrome(w, h);
    let seal = picker_seal_button_rect(w, h);
    let title_font = typography::size(typography::H20, h);
    let body_font = typography::size(typography::H42, h);
    let jr = (w.min(h) / 720.0).clamp(1.0, 1.38);
    let title = HeaderTitleLayout::nav_row_aligned(
        back,
        decimation_copy_inset_x(w),
        (18.0 * scale).max(10.0),
        title_font,
        jr,
    );
    let panel_top = title.body_top_below_subtitle(body_font, jr);
    let margin = (14.0 * scale).max(8.0);
    let bottom = h - screen_footer_reserve(w, h) - margin;
    let viewport = [
        w * 0.025,
        panel_top,
        w * 0.95,
        (bottom - panel_top).max(1.0),
    ];
    DecimationPickingChrome {
        back,
        seal,
        copy_x: title.copy_x,
        title_y: title.title_y,
        subtitle_y: title.subtitle_y,
        viewport,
    }
}

fn decimation_picking_chrome_actions(
    chrome: &DecimationPickingChrome,
) -> [(DecimationAction, [f32; 4]); 2] {
    [
        (DecimationAction::Cancel, chrome.back),
        (DecimationAction::Seal, chrome.seal),
    ]
}

fn decimation_seal_button_label(selected_len: usize) -> String {
    let counter = format!("{selected_len}/{PLAYER_PICKS}");
    if selected_len == PLAYER_PICKS {
        format!("{counter} · Seal")
    } else {
        let remaining = PLAYER_PICKS.saturating_sub(selected_len);
        format!("{counter} · Pick {remaining}")
    }
}

fn point_in_rect(mx: f32, my: f32, r: [f32; 4]) -> bool {
    mx >= r[0] && mx <= r[0] + r[2] && my >= r[1] && my <= r[1] + r[3]
}

const DECIMATION_BURN_SECS: f32 = 1.05;
const DECIMATION_SPARK_COLOR: [f32; 4] = [1.0, 0.52, 0.12, 0.92];

pub struct StairwayScene {
    flavor: &'static [RelicFlavorSpan],
    phase: StairwayPhase,
    tree: TreeState,
    tile_scroll_y: f32,
    focus: Option<DecimationFocus>,
    focus_nav: FocusNavState<DecimationFocus>,
    hovered_tile: Option<usize>,
    marquee: Option<MarqueeSelect>,
    particles: ParticleSystem,
    last_frame: Instant,
    /// Burn finished and [`SceneIntent::ShopFromRun`] was returned once.
    burn_shop_handoff: bool,
    dragging_scrollbar: bool,
    scroll_drag_grab_y: f32,
    prev_mouse_down: bool,
    pause_menu: PauseMenu,
}

impl Default for StairwayScene {
    fn default() -> Self {
        Self::new()
    }
}

impl StairwayScene {
    pub fn new() -> Self {
        Self {
            flavor: crate::core::staircase_flavor::random_entry_flavor(),
            phase: StairwayPhase::Prompt,
            tree: TreeState::default(),
            tile_scroll_y: 0.0,
            focus: None,
            focus_nav: FocusNavState::new(),
            hovered_tile: None,
            marquee: None,
            particles: ParticleSystem::new(),
            last_frame: Instant::now(),
            burn_shop_handoff: false,
            dragging_scrollbar: false,
            scroll_drag_grab_y: 0.0,
            prev_mouse_down: false,
            pause_menu: PauseMenu::new(),
        }
    }

    /// Headless screenshot: decimation picker mid-selection (3/5 marked).
    pub fn for_decimation_screenshot(run: &crate::game::run::RunState) -> Self {
        let display_tiles = Self::eligible_display_tiles(run);
        let selected: Vec<u32> = display_tiles.iter().take(3).map(|t| t.id).collect();
        Self {
            flavor: crate::core::staircase_flavor::random_entry_flavor(),
            phase: StairwayPhase::Picking {
                selected,
                display_tiles,
            },
            tree: TreeState::default(),
            tile_scroll_y: 0.0,
            focus: Some(DecimationFocus::Tile(2)),
            focus_nav: FocusNavState::new(),
            hovered_tile: Some(2),
            marquee: None,
            particles: ParticleSystem::new(),
            last_frame: Instant::now(),
            burn_shop_handoff: false,
            dragging_scrollbar: false,
            scroll_drag_grab_y: 0.0,
            prev_mouse_down: false,
            pause_menu: PauseMenu::new(),
        }
    }

    /// Headless screenshot: decimation reveal (10 tiles, pre-burn).
    pub fn for_decimation_revealed_screenshot(run: &crate::game::run::RunState) -> Self {
        let display_tiles = Self::eligible_display_tiles(run);
        let player: [u32; PLAYER_PICKS] = display_tiles
            .iter()
            .take(PLAYER_PICKS)
            .map(|t| t.id)
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
        let pool = decimation_house_pool(run, &player);
        let house: [u32; HOUSE_PICKS] = pool
            .iter()
            .take(HOUSE_PICKS)
            .copied()
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
        Self {
            flavor: crate::core::staircase_flavor::random_entry_flavor(),
            phase: StairwayPhase::Revealed {
                player,
                house,
                display_tiles,
            },
            tree: TreeState::default(),
            tile_scroll_y: 0.0,
            focus: Some(DecimationFocus::Seal),
            focus_nav: FocusNavState::new(),
            hovered_tile: None,
            marquee: None,
            particles: ParticleSystem::new(),
            last_frame: Instant::now(),
            burn_shop_handoff: false,
            dragging_scrollbar: false,
            scroll_drag_grab_y: 0.0,
            prev_mouse_down: false,
            pause_menu: PauseMenu::new(),
        }
    }

    pub fn wants_hand_tile_pick(&self) -> bool {
        matches!(self.phase, StairwayPhase::Picking { .. })
    }

    fn eligible_display_tiles(run: &crate::game::run::RunState) -> Vec<Tile> {
        decimation_eligible_tiles(run)
            .into_iter()
            .map(|t| GameEngine::display_tile(t, run))
            .collect()
    }

    fn update_picking_scrollbar(
        &mut self,
        ctx: &UpdateCtx<'_>,
        picking_chrome: &DecimationPickingChrome,
        layout: &ScrollableTilePickerLayout<DecimationAction>,
        scale: f32,
    ) -> bool {
        let scroll = &layout.scroll;
        let Some(sb) = tile_picker_scrollbar(
            picking_chrome.viewport,
            scale,
            scroll.content_height,
            scroll.scroll_y,
            scroll.max_scroll_y,
        ) else {
            if !ctx.mouse_left_down {
                self.dragging_scrollbar = false;
            }
            self.prev_mouse_down = ctx.mouse_left_down;
            return false;
        };

        let (mx, my) = ctx.cursor_pos;
        let mouse_down = ctx.mouse_left_down;
        let mouse_click = mouse_down && !self.prev_mouse_down;
        let mut scroll_dirty = false;

        if self.dragging_scrollbar && mouse_down {
            self.tile_scroll_y = tile_picker_scroll_y_from_cursor(
                my,
                self.scroll_drag_grab_y,
                &sb,
                scroll.max_scroll_y,
            );
            scroll_dirty = true;
        } else if mouse_click {
            if point_in_rect(mx, my, sb.thumb) {
                self.dragging_scrollbar = true;
                self.scroll_drag_grab_y = my - sb.thumb[1];
                self.tile_scroll_y = tile_picker_scroll_y_from_cursor(
                    my,
                    self.scroll_drag_grab_y,
                    &sb,
                    scroll.max_scroll_y,
                );
                scroll_dirty = true;
            } else if point_in_rect(mx, my, sb.hit_track) {
                self.dragging_scrollbar = true;
                self.scroll_drag_grab_y = sb.thumb[3] * 0.5;
                self.tile_scroll_y = tile_picker_scroll_y_from_cursor(
                    my,
                    self.scroll_drag_grab_y,
                    &sb,
                    scroll.max_scroll_y,
                );
                scroll_dirty = true;
            }
        }

        if !mouse_down {
            self.dragging_scrollbar = false;
        }
        self.prev_mouse_down = mouse_down;
        scroll_dirty
    }

    fn chrome_focus_targets(
        footer: &[(DecimationAction, [f32; 4])],
    ) -> Vec<(DecimationFocus, [f32; 4])> {
        footer
            .iter()
            .filter_map(|&(action, rect)| {
                let focus = match action {
                    DecimationAction::Seal => DecimationFocus::Seal,
                    DecimationAction::Cancel => DecimationFocus::Cancel,
                    DecimationAction::Continue => return None,
                };
                Some((focus, rect))
            })
            .collect()
    }

    fn tile_focus_targets(
        layout: &ScrollableTilePickerLayout<DecimationAction>,
    ) -> Vec<(DecimationFocus, [f32; 4])> {
        layout
            .pick_tile_rects
            .iter()
            .enumerate()
            .map(|(i, rect)| (DecimationFocus::Tile(i), *rect))
            .collect()
    }

    fn focus_targets_for_layout(
        layout: &ScrollableTilePickerLayout<DecimationAction>,
        chrome: &[(DecimationAction, [f32; 4])],
    ) -> Vec<(DecimationFocus, [f32; 4])> {
        let mut targets = Self::tile_focus_targets(layout);
        targets.extend(Self::chrome_focus_targets(chrome));
        targets
    }

    fn focus_rect(
        focus: DecimationFocus,
        layout: &ScrollableTilePickerLayout<DecimationAction>,
        chrome: &[(DecimationAction, [f32; 4])],
    ) -> Option<[f32; 4]> {
        match focus {
            DecimationFocus::Tile(i) => layout.pick_tile_rects.get(i).copied(),
            DecimationFocus::Seal => chrome
                .iter()
                .find(|(a, _)| matches!(a, DecimationAction::Seal))
                .map(|(_, r)| *r),
            DecimationFocus::Cancel => chrome
                .iter()
                .find(|(a, _)| matches!(a, DecimationAction::Cancel))
                .map(|(_, r)| *r),
        }
    }

    fn clamp_focus_to_layout(
        &mut self,
        layout: &ScrollableTilePickerLayout<DecimationAction>,
        chrome: &[(DecimationAction, [f32; 4])],
    ) {
        let targets = Self::focus_targets_for_layout(layout, chrome);
        if targets.is_empty() {
            self.focus = None;
            return;
        }
        if self
            .focus
            .is_some_and(|f| Self::focus_rect(f, layout, chrome).is_some())
        {
            return;
        }
        self.focus = Some(targets[0].0);
    }

    fn ensure_tile_focus_visible(
        &mut self,
        tile_idx: usize,
        layout: &ScrollableTilePickerLayout<DecimationAction>,
    ) {
        let Some(rect) = layout.pick_tile_rects.get(tile_idx) else {
            return;
        };
        let viewport = layout.scroll.viewport;
        let pad = 6.0;
        if rect[1] < viewport[1] + pad {
            self.tile_scroll_y = (self.tile_scroll_y + rect[1] - viewport[1] - pad).max(0.0);
        } else if rect[1] + rect[3] > viewport[1] + viewport[3] - pad {
            self.tile_scroll_y = (self.tile_scroll_y + rect[1] + rect[3]
                - (viewport[1] + viewport[3] - pad))
                .min(layout.scroll.max_scroll_y);
        }
    }

    fn tile_focus_index(&self) -> Option<usize> {
        match self.focus {
            Some(DecimationFocus::Tile(i)) => Some(i),
            _ => None,
        }
    }

    fn decimation_section_index(
        layout: &ScrollableTilePickerLayout<DecimationAction>,
        focus: Option<DecimationFocus>,
        scroll_y: f32,
    ) -> usize {
        if let Some(DecimationFocus::Tile(i)) = focus {
            return layout
                .sections
                .iter()
                .rposition(|s| i >= s.first_pick_index)
                .unwrap_or(0);
        }
        layout
            .sections
            .iter()
            .rposition(|s| scroll_y + 1.0 >= s.header_content_y)
            .unwrap_or(0)
    }

    fn step_decimation_section(
        &mut self,
        layout: &ScrollableTilePickerLayout<DecimationAction>,
        forward: bool,
        bus: &mut crate::game::event_bus::EventBus,
    ) -> bool {
        let sections = &layout.sections;
        if sections.is_empty() {
            return false;
        }
        let current = Self::decimation_section_index(layout, self.focus, self.tile_scroll_y);
        let len = sections.len();
        let next = if forward {
            (current + 1) % len
        } else {
            (current + len - 1) % len
        };
        let section = sections[next];
        self.marquee = None;
        self.tile_scroll_y = section.header_content_y.min(layout.scroll.max_scroll_y);
        self.focus = Some(DecimationFocus::Tile(section.first_pick_index));
        bus.push(GameEvent::UiSound(SfxId::UiConfirm));
        true
    }

    fn apply_decimation_marquee(
        marquee: &mut MarqueeSelect,
        slot: usize,
        layout: &ScrollableTilePickerLayout<DecimationAction>,
        selected: &mut Vec<u32>,
        bus: &mut crate::game::event_bus::EventBus,
    ) {
        if slot == marquee.current_slot {
            return;
        }
        marquee.set_current_slot(slot);
        let swept = grid_marquee_swept_slots(
            marquee.start_slot,
            marquee.current_slot,
            SCROLLABLE_GRID_COLS,
            &layout.sections,
        );
        let mut mask = pick_selection_mask(selected, &layout.pick_tile_ids);
        let (added, removed) = marquee.apply_capped_swept(&mut mask, PLAYER_PICKS, &swept);
        apply_pick_selection_mask(selected, &layout.pick_tile_ids, &mask);
        if added > 0 {
            bus.push(GameEvent::UiSound(SfxId::TilePlace));
        } else if removed > 0 {
            bus.push(GameEvent::UiSound(SfxId::UiCancel));
        }
    }

    fn begin_decimation_marquee(
        slot: usize,
        pick_tile_ids: &[u32],
        selected: &mut Vec<u32>,
        bus: &mut crate::game::event_bus::EventBus,
    ) -> MarqueeSelect {
        let snapshot = pick_selection_mask(selected, pick_tile_ids);
        let m = MarqueeSelect::new(slot, snapshot.clone());
        let mut mask = snapshot;
        let (added, removed) = m.apply_capped(&mut mask, PLAYER_PICKS);
        apply_pick_selection_mask(selected, pick_tile_ids, &mask);
        if added > 0 || removed > 0 {
            bus.push(GameEvent::UiSound(if added > 0 {
                SfxId::TilePlace
            } else {
                SfxId::UiCancel
            }));
        }
        m
    }

    fn seal_decimation(
        &mut self,
        run: &mut crate::game::run::RunState,
        bus: &mut crate::game::event_bus::EventBus,
    ) -> bool {
        let StairwayPhase::Picking {
            selected,
            display_tiles,
        } = &self.phase
        else {
            return false;
        };
        if !can_seal_decimation(run, selected) {
            return false;
        }
        let player: [u32; PLAYER_PICKS] = selected.as_slice().try_into().expect("5 player picks");
        let pool = decimation_house_pool(run, selected);
        let house_vec = pick_house_tiles(&pool, &mut rand::rng());
        if house_vec.len() != HOUSE_PICKS {
            return false;
        }
        let house: [u32; HOUSE_PICKS] = house_vec.try_into().unwrap();
        apply_decimation(run, player, house, bus, false);
        self.phase = StairwayPhase::Revealed {
            player,
            house,
            display_tiles: display_tiles.clone(),
        };
        true
    }
}

fn push_staircase_environment(frame: &mut UiFrame, ctx: &DrawCtx<'_>, w: f32, h: f32) {
    if !staircase_glb::staircase_glb_loaded() {
        return;
    }
    frame.camera_override = Some(staircase_glb::staircase_camera(
        w,
        h,
        ctx.room_gltf_height_scale,
    ));
    frame.staircase_environment();
    let room_glb = staircase_glb::staircase_glb_has_embedded_lights();
    frame.scene_lighting.embedded_gltf_punctual = room_glb;
    frame.scene_lighting.room_glb_brdf = room_glb;
    let (inverse_punctual, punctual_gltf_nodes) = if room_glb {
        crate::render::room_gltf_punctual::tagged_to_scene_punctual(
            staircase_glb::staircase_embedded_point_lights_runtime_tagged(
                w,
                h,
                ctx.room_gltf_height_scale,
                &ctx.room_env_for(scene_keys::STAIRWAY).0,
            ),
        )
    } else {
        (Vec::new(), Vec::new())
    };
    frame.scene_lighting.punctual = inverse_punctual;
    frame.scene_lighting.punctual_gltf_nodes = punctual_gltf_nodes;
    if !room_glb {
        frame.scene_lighting.set_smooth_points(vec![PointLight {
            pos: [w * 0.5, h * 0.55, h * 0.35],
            radius: h * 1.4,
            color: [0.92, 0.82, 0.68],
            intensity: 1.35,
        }]);
    }
}

fn push_flavor_text(frame: &mut UiFrame, w: f32, h: f32, flavor: &'static [RelicFlavorSpan]) {
    if flavor.is_empty() {
        return;
    }
    let margin_x = w * 0.045;
    let margin_y = h * 0.11;
    let max_inner_w = (w * 0.44).min(560.0).max(w - 2.0 * margin_x);
    let body_px = typography::size(typography::H32, h);
    let line_step = styled_text::colored_row_line_step(body_px);
    let content_w = flavor_spans_layout_width(flavor, body_px, max_inner_w);
    let content_lines = estimated_flavor_line_count(flavor, content_w, body_px, 16);
    let content_h = line_step * content_lines as f32 + body_px * 0.35;
    let left = w - margin_x - content_w;
    let top = margin_y;
    let plain = flavor_spans_plain_text(flavor);
    let has_inline_style = flavor.iter().any(|s| s.bold || s.italic);

    if has_inline_style {
        frame.text(TextLabel {
            rect: [left, top, content_w, content_h],
            text: plain,
            color: color::CHAMPAGNE,
            font_px: Some(body_px),
            align: TextAlign::Center,
            block_vertical_align: TextBlockVerticalAlign::Top,
            flavor_spans: Some(flavor),
            ..Default::default()
        });
        return;
    }

    let wrapped = styled_text::wrap_colored_text_multiline(
        &plain,
        content_w,
        body_px,
        color::CHAMPAGNE,
        false,
        GlossaryMode::Prose,
    );
    let mut labels = Vec::new();
    styled_text::push_colored_rows_in_width(
        &mut labels,
        styled_text::ColoredRowsLayout {
            text_left: left,
            top_y: top,
            inner_w: content_w,
            line_h: body_px,
            fallback_plain: &plain,
            fallback_color: color::CHAMPAGNE,
            italic: false,
            glossary: GlossaryMode::Prose,
        },
        &wrapped,
        TextAlign::Center,
    );
    frame.texts(labels);
}

fn prompt_items(w: f32, h: f32) -> Vec<FlatItem<PromptAction>> {
    let scale = metrics::scene_scale(w, h);
    let btn_h = (48.0 * scale).max(40.0);
    let gap = 10.0 * scale;
    let btn_w = (w * 0.88).min(680.0 * scale).max(240.0);
    let x0 = (w - btn_w) * 0.5;
    let margin = 20.0 * scale;
    let bottom_y = h - screen_footer_reserve(w, h) - margin - btn_h;
    let y0 = bottom_y - btn_h - gap;
    vec![
        FlatItem::new(
            PromptAction::Descend.id(),
            [x0, y0, btn_w, btn_h],
            PromptAction::Descend,
        ),
        FlatItem::new(
            PromptAction::Decimate.id(),
            [x0, y0 + btn_h + gap, btn_w, btn_h],
            PromptAction::Decimate,
        ),
    ]
}

impl StairwayScene {
    fn decimation_layout<'a>(
        &'a self,
        w: f32,
        h: f32,
        face_aspect: f32,
        display_tiles: &'a [Tile],
        selected: &'a [u32],
        hovered: Option<usize>,
        chrome: &'a [(DecimationAction, [f32; 4])],
        viewport: [f32; 4],
    ) -> ScrollableTilePickerLayout<DecimationAction> {
        compute_scrollable_tile_picker_layout(
            w,
            h,
            ScrollableTilePickerConfig {
                tiles: display_tiles,
                face_aspect,
                scroll_y: self.tile_scroll_y,
                pickable: true,
                dim_unmarked: true,
                hovered_pick: hovered,
                selected_ids: selected,
                player_claim_ids: None,
                house_claim_ids: None,
                chrome_actions: chrome,
                selection_outline_sel: Some(2.0),
                grid_cols: SCROLLABLE_GRID_COLS,
                viewport: Some(viewport),
                grouped_rows: true,
                show_scrollbar: true,
            },
        )
    }

    fn update_prompt(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let items = prompt_items(w, h);
        let action = self.tree.update_flat(
            &items,
            TreeInput {
                actions: ctx.actions,
                button_clicks: ctx.button_clicks,
                cursor_pos: ctx.cursor_pos,
                window: (w, h),
                input_mode: ctx.input_mode,
                scroll_lines: 0.0,
            },
        );
        if self.tree.take_focus_changed() {
            ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
        }
        match action {
            Some(PromptAction::Descend) => Some(SceneIntent::ShopFromRun),
            Some(PromptAction::Decimate) => {
                ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                let display_tiles = Self::eligible_display_tiles(ctx.run);
                self.phase = StairwayPhase::Picking {
                    selected: Vec::new(),
                    display_tiles,
                };
                self.tile_scroll_y = 0.0;
                self.focus = Some(DecimationFocus::Tile(0));
                self.marquee = None;
                None
            }
            None => None,
        }
    }

    fn update_picking(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let face_aspect = crate::persistence::load_settings()
            .tile_preset
            .face_long_ratio();

        let (mut selected, display_tiles) = match &self.phase {
            StairwayPhase::Picking {
                selected,
                display_tiles,
            } => (selected.clone(), display_tiles.clone()),
            _ => return None,
        };

        let picking_chrome = decimation_picking_chrome(w, h);
        let chrome = decimation_picking_chrome_actions(&picking_chrome);
        let can_seal = can_seal_decimation(ctx.run, &selected);

        let manual_nav = matches!(ctx.input_mode, InputMode::Controller | InputMode::Keyboard);
        let hover_for_layout = if manual_nav {
            self.tile_focus_index()
        } else if ctx.input_mode == InputMode::Cursor {
            ctx.picked_hand_tile
        } else {
            None
        };

        if ctx.scroll_lines.abs() > f32::EPSILON && !self.dragging_scrollbar {
            self.tile_scroll_y =
                (self.tile_scroll_y + ctx.scroll_lines * DECIMATION_SCROLL_LINES_PX).max(0.0);
        }

        let scale = metrics::scene_scale(w, h);
        let mut layout = self.decimation_layout(
            w,
            h,
            face_aspect,
            &display_tiles,
            &selected,
            hover_for_layout,
            &chrome,
            picking_chrome.viewport,
        );
        if self.update_picking_scrollbar(&ctx, &picking_chrome, &layout, scale) {
            layout = self.decimation_layout(
                w,
                h,
                face_aspect,
                &display_tiles,
                &selected,
                hover_for_layout,
                &chrome,
                picking_chrome.viewport,
            );
        }
        self.tile_scroll_y = layout.scroll.scroll_y;
        let mut scroll_dirty = false;
        self.clamp_focus_to_layout(&layout, &chrome);

        let focus_targets = Self::focus_targets_for_layout(&layout, &chrome);

        let mut back_to_prompt = false;
        let mut seal_now = false;

        if ctx.input_mode == InputMode::Cursor && !self.dragging_scrollbar {
            let (cx, cy) = ctx.cursor_pos;
            let scrollbar_blocks = tile_picker_scrollbar(
                picking_chrome.viewport,
                scale,
                layout.scroll.content_height,
                layout.scroll.scroll_y,
                layout.scroll.max_scroll_y,
            )
            .is_some_and(|sb| point_in_rect(cx, cy, sb.hit_track));
            if !scrollbar_blocks {
                if let Some(idx) = ctx.picked_hand_tile {
                    self.focus = Some(DecimationFocus::Tile(idx));
                } else if let Some(target) =
                    focus_nav::focus_target_at_cursor(&focus_targets, cx, cy)
                {
                    self.focus = Some(target);
                }
            }
        }

        let marquee_slot = if ctx.input_mode == InputMode::Cursor {
            ctx.picked_hand_tile.or_else(|| self.tile_focus_index())
        } else {
            self.tile_focus_index()
        };
        if let (Some(m), Some(idx)) = (self.marquee.as_mut(), marquee_slot)
            && idx != m.current_slot
        {
            Self::apply_decimation_marquee(m, idx, &layout, &mut selected, ctx.bus);
        }

        let mut tree_actions: Vec<UiAction> = Vec::new();
        for &a in ctx.actions {
            match a {
                UiAction::Pause => {
                    self.marquee = None;
                    ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
                    back_to_prompt = true;
                    break;
                }
                UiAction::Cancel => {
                    self.marquee = None;
                    ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
                    if !selected.is_empty() {
                        selected.clear();
                    } else {
                        self.focus = Some(DecimationFocus::Cancel);
                    }
                    break;
                }
                UiAction::InvertSelection if manual_nav => {
                    self.marquee = None;
                    self.focus = Some(DecimationFocus::Seal);
                }
                UiAction::TabNext | UiAction::NavigateHudNext => {
                    if self.step_decimation_section(&layout, true, ctx.bus) {
                        scroll_dirty = true;
                    }
                }
                UiAction::TabPrev | UiAction::NavigateHudPrev => {
                    if self.step_decimation_section(&layout, false, ctx.bus) {
                        scroll_dirty = true;
                    }
                }
                UiAction::PageNext => {
                    self.tile_scroll_y += layout.scroll.viewport[3] * 0.85;
                    scroll_dirty = true;
                }
                UiAction::PagePrev => {
                    self.tile_scroll_y =
                        (self.tile_scroll_y - layout.scroll.viewport[3] * 0.85).max(0.0);
                    scroll_dirty = true;
                }
                UiAction::FocusUp => {
                    if manual_nav {
                        self.step_spatial_focus(FocusDir::Up, &layout, &chrome, &focus_targets);
                    } else {
                        tree_actions.push(a);
                    }
                }
                UiAction::FocusDown => {
                    if manual_nav {
                        self.step_spatial_focus(FocusDir::Down, &layout, &chrome, &focus_targets);
                    } else {
                        tree_actions.push(a);
                    }
                }
                UiAction::FocusPrev => {
                    if manual_nav {
                        self.step_spatial_focus(FocusDir::Left, &layout, &chrome, &focus_targets);
                    } else {
                        tree_actions.push(a);
                    }
                }
                UiAction::FocusNext => {
                    if manual_nav {
                        self.step_spatial_focus(FocusDir::Right, &layout, &chrome, &focus_targets);
                    } else {
                        tree_actions.push(a);
                    }
                }
                UiAction::Confirm | UiAction::CommitDiscard => match self.focus {
                    Some(DecimationFocus::Tile(i)) => {
                        self.marquee = Some(Self::begin_decimation_marquee(
                            i,
                            &layout.pick_tile_ids,
                            &mut selected,
                            ctx.bus,
                        ));
                    }
                    Some(DecimationFocus::Seal) if can_seal => {
                        ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                        self.marquee = None;
                        seal_now = true;
                    }
                    Some(DecimationFocus::Seal) => {
                        ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
                    }
                    Some(DecimationFocus::Cancel) => {
                        ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
                        self.marquee = None;
                        back_to_prompt = true;
                    }
                    _ if ctx.input_mode == InputMode::Cursor && ctx.picked_hand_tile.is_some() => {
                        if let Some(idx) = ctx.picked_hand_tile {
                            self.marquee = Some(Self::begin_decimation_marquee(
                                idx,
                                &layout.pick_tile_ids,
                                &mut selected,
                                ctx.bus,
                            ));
                        }
                    }
                    _ if manual_nav => {}
                    _ => tree_actions.push(a),
                },
                UiAction::ConfirmRelease => {
                    self.marquee = None;
                }
                other => tree_actions.push(other),
            }
        }

        if manual_nav {
            self.hovered_tile = self.tile_focus_index();
        } else if ctx.input_mode == InputMode::Cursor {
            self.hovered_tile = ctx.picked_hand_tile;
        } else {
            self.hovered_tile = None;
        }

        let marquee_slot = if ctx.input_mode == InputMode::Cursor {
            ctx.picked_hand_tile.or_else(|| self.tile_focus_index())
        } else {
            self.tile_focus_index()
        };
        if let (Some(m), Some(idx)) = (self.marquee.as_mut(), marquee_slot)
            && idx != m.current_slot
        {
            Self::apply_decimation_marquee(m, idx, &layout, &mut selected, ctx.bus);
        }

        if let Some(DecimationFocus::Tile(idx)) = self.focus {
            let before = self.tile_scroll_y;
            self.ensure_tile_focus_visible(idx, &layout);
            if (self.tile_scroll_y - before).abs() > 0.5 {
                scroll_dirty = true;
            }
        }

        if scroll_dirty {
            layout = self.decimation_layout(
                w,
                h,
                face_aspect,
                &display_tiles,
                &selected,
                self.tile_focus_index(),
                &chrome,
                picking_chrome.viewport,
            );
            self.tile_scroll_y = layout.scroll.scroll_y;
        }

        if let StairwayPhase::Picking {
            selected: phase_sel,
            ..
        } = &mut self.phase
        {
            *phase_sel = selected.clone();
        }

        if back_to_prompt {
            self.phase = StairwayPhase::Prompt;
            return None;
        }
        if seal_now {
            self.seal_decimation(ctx.run, ctx.bus);
            return None;
        }

        let can_seal_after = can_seal_decimation(
            ctx.run,
            match &self.phase {
                StairwayPhase::Picking { selected, .. } => selected.as_slice(),
                _ => &[],
            },
        );

        let input = TreeInput {
            actions: &tree_actions,
            button_clicks: ctx.button_clicks,
            cursor_pos: ctx.cursor_pos,
            window: (w, h),
            input_mode: ctx.input_mode,
            scroll_lines: ctx.scroll_lines,
        };
        let fired = self.tree.update_flat(&layout.flat_items, input);
        if self.tree.take_focus_changed() {
            ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
        }
        let can_seal = can_seal_after;
        match fired {
            Some(DecimationAction::Cancel) if ctx.input_mode == InputMode::Cursor => {
                ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
                self.marquee = None;
                self.phase = StairwayPhase::Prompt;
            }
            Some(DecimationAction::Seal) if can_seal && ctx.input_mode == InputMode::Cursor => {
                ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                self.marquee = None;
                self.seal_decimation(ctx.run, ctx.bus);
            }
            Some(DecimationAction::Seal) if ctx.input_mode == InputMode::Cursor => {
                ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
            }
            _ => {}
        }
        None
    }

    fn step_spatial_focus(
        &mut self,
        dir: FocusDir,
        layout: &ScrollableTilePickerLayout<DecimationAction>,
        chrome: &[(DecimationAction, [f32; 4])],
        focus_targets: &[(DecimationFocus, [f32; 4])],
    ) {
        let Some(current) = self
            .focus
            .or_else(|| focus_targets.first().map(|(target, _)| *target))
        else {
            return;
        };
        self.focus_nav.load_candidates(focus_targets, &[]);
        if let Some(next) = self.focus_nav.pick(current, dir) {
            self.focus = Some(next);
        } else if self.focus.is_none() {
            self.clamp_focus_to_layout(layout, chrome);
        }
    }

    fn begin_burn(
        &mut self,
        player: [u32; PLAYER_PICKS],
        house: [u32; HOUSE_PICKS],
        display_tiles: Vec<Tile>,
        bus: &mut crate::game::event_bus::EventBus,
        w: f32,
        h: f32,
        face_aspect: f32,
    ) {
        let layout = compute_decimation_reveal_layout::<DecimationAction>(
            w,
            h,
            face_aspect,
            &display_tiles,
            &player,
            &house,
            0.0,
            &[],
        );
        for anchor in &layout.spark_anchors {
            self.particles
                .emit(anchor[0], anchor[1], 10, DECIMATION_SPARK_COLOR, 0.72);
            self.particles
                .emit(anchor[0], anchor[1], 5, [1.0, 0.88, 0.35, 0.85], 0.45);
        }
        bus.push(GameEvent::TilesDestroyed);
        self.phase = StairwayPhase::Burning {
            player,
            house,
            display_tiles,
            started_at: Instant::now(),
        };
    }

    fn update_revealed(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let face_aspect = crate::persistence::load_settings()
            .tile_preset
            .face_long_ratio();

        let (player, house, display_tiles) = match &self.phase {
            StairwayPhase::Revealed {
                player,
                house,
                display_tiles,
            } => (*player, *house, display_tiles.clone()),
            _ => return None,
        };

        let rects = footer_button_rects(w, h, 1);
        let footer = [(DecimationAction::Continue, rects[0])];
        let layout = compute_decimation_reveal_layout(
            w,
            h,
            face_aspect,
            &display_tiles,
            &player,
            &house,
            0.0,
            &footer,
        );

        let input = TreeInput {
            actions: ctx.actions,
            button_clicks: ctx.button_clicks,
            cursor_pos: ctx.cursor_pos,
            window: (w, h),
            input_mode: ctx.input_mode,
            scroll_lines: ctx.scroll_lines,
        };
        let fired = self.tree.update_flat(&layout.flat_items, input);
        if matches!(fired, Some(DecimationAction::Continue)) {
            self.begin_burn(player, house, display_tiles, ctx.bus, w, h, face_aspect);
        }
        None
    }

    fn update_burning(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        let now = Instant::now();
        let dt = now.saturating_duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;

        let (started_at, display_tiles, player, house, w, h) = match &self.phase {
            StairwayPhase::Burning {
                started_at,
                display_tiles,
                player,
                house,
            } => (
                *started_at,
                display_tiles.clone(),
                *player,
                *house,
                ctx.layout.window_w,
                ctx.layout.window_h,
            ),
            _ => return None,
        };

        let elapsed = now.saturating_duration_since(started_at).as_secs_f32();
        self.particles.update(dt, None);

        let face_aspect = crate::persistence::load_settings()
            .tile_preset
            .face_long_ratio();
        if elapsed < DECIMATION_BURN_SECS * 0.55 {
            let layout = compute_decimation_reveal_layout::<DecimationAction>(
                w,
                h,
                face_aspect,
                &display_tiles,
                &player,
                &house,
                (elapsed / DECIMATION_BURN_SECS).min(1.0),
                &[],
            );
            let spark_frame = (elapsed * 18.0) as u32;
            for (i, anchor) in layout.spark_anchors.iter().enumerate() {
                if (spark_frame + i as u32) % 3 == 0 {
                    self.particles
                        .emit(anchor[0], anchor[1], 3, DECIMATION_SPARK_COLOR, 0.35);
                }
            }
        }

        if elapsed >= DECIMATION_BURN_SECS {
            if self.burn_shop_handoff {
                return None;
            }
            self.burn_shop_handoff = true;
            return Some(SceneIntent::ShopFromRun);
        }
        None
    }
}

impl SceneBehavior for StairwayScene {
    fn pause_options_overlay(&self) -> Option<&super::options::OptionsScene> {
        self.pause_menu.options_overlay()
    }

    fn has_blocking_overlay(&self) -> bool {
        self.pause_menu.paused
    }

    fn update(&mut self, mut ctx: UpdateCtx<'_>) -> SceneTransition {
        if let Some(t) = self.pause_menu.handle(&mut ctx) {
            if self.pause_menu.take_credits_request() {
                *ctx.overlay_request = Some(OverlayRequest::Push(Box::new(Scene::Credits(
                    crate::scenes::CreditsScene::overlay(),
                ))));
                return None;
            }
            return t;
        }
        match &self.phase {
            StairwayPhase::Prompt => self.update_prompt(ctx),
            StairwayPhase::Picking { .. } => self.update_picking(ctx),
            StairwayPhase::Revealed { .. } => self.update_revealed(ctx),
            StairwayPhase::Burning { .. } => self.update_burning(ctx),
        }
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let mut frame = match &self.phase {
            StairwayPhase::Prompt => self.draw_prompt(ctx),
            StairwayPhase::Picking {
                selected,
                display_tiles,
            } => self.draw_decimation_picking(ctx, display_tiles, selected),
            StairwayPhase::Revealed {
                player,
                house,
                display_tiles,
            } => self.draw_decimation_reveal(ctx, display_tiles, player, house, 0.0, true),
            StairwayPhase::Burning {
                player,
                house,
                display_tiles,
                started_at,
            } => {
                let elapsed = Instant::now()
                    .saturating_duration_since(*started_at)
                    .as_secs_f32();
                let burn_t = (elapsed / DECIMATION_BURN_SECS).min(1.0);
                self.draw_decimation_reveal(ctx, display_tiles, player, house, burn_t, false)
            }
        };

        let scale = metrics::scene_scale(w, h);
        let mut pause_quads: Vec<GpuInstance> = Vec::new();
        let mut pause_text: Vec<TextLabel> = Vec::new();
        if self.pause_menu.paused {
            frame.buttons.clear();
        }
        self.pause_menu.draw(
            crate::ui::layout::ViewportCtx {
                window_w: w,
                window_h: h,
            },
            scale,
            crate::scenes::options::options_scroll_fade_backdrop(true),
            &mut pause_quads,
            &mut pause_text,
            &mut frame.buttons,
        );
        if !pause_quads.is_empty() {
            frame.quads(pause_quads);
        }
        if !pause_text.is_empty() {
            frame.texts(pause_text);
        }
        if self.pause_menu.paused {
            frame
                .buttons
                .push(super::ButtonDef::scene((0.0, 0.0, w, h), u32::MAX));
        }
        frame
    }
}

impl StairwayScene {
    fn draw_prompt(&self, mut ctx: DrawCtx<'_>) -> UiFrame {
        let flavor = self.flavor;
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);
        push_staircase_environment(&mut frame, &ctx, w, h);

        push_flavor_text(&mut frame, w, h, flavor);

        let items = prompt_items(w, h);
        let focused = self.tree.focused();
        let btn_font = typography::size(typography::H20, h);
        for item in &items {
            let is_focused = focused == Some(item.id);
            frame.quad(GpuInstance {
                rect: item.rect,
                color: if is_focused {
                    color::alpha(color::BRASS, 0.92)
                } else {
                    color::alpha(color::WALNUT_INK, 0.94)
                },
                user: 0,
            });
            frame.text(TextLabel {
                rect: item.rect,
                text: item.action.label().into(),
                color: if is_focused {
                    color::WALNUT_DEEP
                } else {
                    color::CHAMPAGNE
                },
                align: TextAlign::Center,
                font_px: Some(btn_font),
                ..Default::default()
            });
        }
        self.tree.register_flat_buttons(&items, &mut frame.buttons);

        let hint_style = hint_style_with_alpha(HintStyle::standard(w, h), 0.92);
        push_screen_footer_hint(
            &mut frame,
            &ctx,
            stairway_prompt_footer_row(ctx.input_mode),
            hint_style,
        );

        ctx.stash_focus_nav_tree_flat(&self.tree, &items, |a| a.label().into());
        frame
    }

    fn draw_decimation_picking(
        &self,
        mut ctx: DrawCtx<'_>,
        display_tiles: &[Tile],
        selected: &[u32],
    ) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let scale = metrics::scene_scale(w, h);
        let face_aspect = ctx.tile_preset.face_long_ratio();

        let picking_chrome = decimation_picking_chrome(w, h);
        let chrome = decimation_picking_chrome_actions(&picking_chrome);

        let layout = self.decimation_layout(
            w,
            h,
            face_aspect,
            display_tiles,
            selected,
            self.hovered_tile,
            &chrome,
            picking_chrome.viewport,
        );
        let viewport = layout.scroll.viewport;

        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);
        push_staircase_environment(&mut frame, &ctx, w, h);
        frame.clear_scene_depth();
        frame.camera_override_after_depth_clear = Some(camera_params(h));
        frame.scene_lighting_after_depth_clear = Some(SceneLighting::showcase_tile_picker(w, h));

        let title_font = typography::size(typography::H20, h);
        let body_font = typography::size(typography::H42, h);
        let header_font = (body_font * 0.94).max(12.0);
        let suit_label_font = if layout.grouped_label_font_px > 0.0 {
            layout.grouped_label_font_px
        } else {
            header_font
        };

        let tile_clip = layout.scroll.viewport;
        let focus_ring_rect = matches!(ctx.input_mode, InputMode::Controller | InputMode::Keyboard)
            .then_some(self.focus)
            .flatten()
            .and_then(|f| match f {
                DecimationFocus::Tile(_) => Self::focus_rect(f, &layout, &chrome),
                _ => None,
            });

        let focus_targets = Self::focus_targets_for_layout(&layout, &chrome);

        if !layout.placements.is_empty() {
            frame.showcase_tile_batch_clipped(layout.placements, Some(tile_clip));
        }

        for header in &layout.section_headers {
            frame.text(TextLabel {
                rect: header.rect,
                text: header.drawer.label().into(),
                color: header.drawer.accent(),
                align: TextAlign::Left,
                font_px: Some(suit_label_font),
                ..Default::default()
            });
        }

        if let Some(rect) = focus_ring_rect {
            let mut rings = Vec::new();
            focus_nav::push_focus_ring(rect, scale, w, h, &mut rings);
            frame.quads(rings);
        }

        if layout.scroll.max_scroll_y > 0.0 {
            let fade_h = (12.0 * scale).max(8.0);
            if layout.scroll.scroll_y > 1.0 {
                frame.quad(GpuInstance {
                    rect: [viewport[0], viewport[1], viewport[2], fade_h],
                    color: color::alpha(color::WALNUT_DEEP, 0.55),
                    user: 0,
                });
            }
            if layout.scroll.scroll_y + 1.0 < layout.scroll.max_scroll_y {
                frame.quad(GpuInstance {
                    rect: [
                        viewport[0],
                        viewport[1] + viewport[3] - fade_h,
                        viewport[2],
                        fade_h,
                    ],
                    color: color::alpha(color::WALNUT_DEEP, 0.55),
                    user: 0,
                });
            }
        }

        if let Some(sb) = tile_picker_scrollbar(
            picking_chrome.viewport,
            scale,
            layout.scroll.content_height,
            layout.scroll.scroll_y,
            layout.scroll.max_scroll_y,
        ) {
            push_tile_picker_scrollbar(&mut frame, &sb, self.dragging_scrollbar);
        }

        let back_focused = matches!(self.focus, Some(DecimationFocus::Cancel))
            || layout
                .flat_items
                .iter()
                .find(|it| matches!(it.action, DecimationAction::Cancel))
                .is_some_and(|it| self.tree.focused() == Some(it.id));
        let seal_focused = matches!(self.focus, Some(DecimationFocus::Seal))
            || layout
                .flat_items
                .iter()
                .find(|it| matches!(it.action, DecimationAction::Seal))
                .is_some_and(|it| self.tree.focused() == Some(it.id));
        let can_seal = selected.len() == PLAYER_PICKS;
        let seal_label = decimation_seal_button_label(selected.len());
        for it in layout
            .flat_items
            .iter()
            .filter(|it| matches!(it.action, DecimationAction::Seal | DecimationAction::Cancel))
        {
            let is_focused = match it.action {
                DecimationAction::Cancel => back_focused,
                DecimationAction::Seal => seal_focused,
                DecimationAction::Continue => false,
            };
            let label = match it.action {
                DecimationAction::Cancel => "Back",
                DecimationAction::Seal => seal_label.as_str(),
                DecimationAction::Continue => continue,
            };
            let bg = match it.action {
                DecimationAction::Seal if can_seal => {
                    if is_focused {
                        color::alpha(color::BRASS, 0.96)
                    } else {
                        color::alpha(color::BRASS, 0.82)
                    }
                }
                DecimationAction::Seal => {
                    if is_focused {
                        color::alpha(color::WALNUT_INK, 0.58)
                    } else {
                        color::alpha(color::WALNUT_INK, 0.42)
                    }
                }
                _ if is_focused => color::alpha(color::BRASS, 0.92),
                _ => color::alpha(color::WALNUT_INK, 0.94),
            };
            frame.quad(GpuInstance {
                rect: it.rect,
                color: bg,
                user: 0,
            });
            frame.text(TextLabel {
                rect: it.rect,
                text: label.into(),
                color: if matches!(it.action, DecimationAction::Seal) && can_seal || is_focused {
                    color::WALNUT_DEEP
                } else {
                    color::CHAMPAGNE
                },
                align: TextAlign::Center,
                font_px: Some(header_font),
                ..Default::default()
            });
            if is_focused {
                let mut rings = Vec::new();
                focus_nav::push_focus_ring(it.rect, scale, w, h, &mut rings);
                frame.quads(rings);
            }
        }

        frame.text(TextLabel {
            rect: [
                picking_chrome.copy_x,
                picking_chrome.title_y,
                w * 0.55,
                title_font * 1.15,
            ],
            text: "Decimation".into(),
            color: color::CHAMPAGNE,
            align: TextAlign::Left,
            font_px: Some(title_font),
            bold: true,
            ..Default::default()
        });
        let mut subtitle_labels = Vec::new();
        styled_text::push_colored_line_left(
            &mut subtitle_labels,
            picking_chrome.copy_x,
            picking_chrome.subtitle_y,
            w * 0.72,
            body_font,
            &format!(
                "Choose {PLAYER_PICKS} tiles to destroy. The House claims {HOUSE_PICKS} more."
            ),
            color::PARCHMENT,
            GlossaryMode::Prose,
        );
        frame.texts(subtitle_labels);

        self.tree
            .register_flat_buttons(&layout.flat_items, &mut frame.buttons);

        push_screen_footer_hint(
            &mut frame,
            &ctx,
            decimation_footer_row(ctx.input_mode),
            HintStyle::standard(w, h),
        );

        ctx.stash_focus_nav_graph(
            &focus_targets,
            &[],
            self.focus,
            self.focus_nav.memory(),
            |f| format!("{f:?}"),
        );
        frame
    }

    fn draw_decimation_reveal(
        &self,
        mut ctx: DrawCtx<'_>,
        display_tiles: &[Tile],
        player: &[u32; PLAYER_PICKS],
        house: &[u32; HOUSE_PICKS],
        burn_t: f32,
        show_footer: bool,
    ) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let scale = metrics::scene_scale(w, h);
        let margin = (14.0 * scale).max(8.0);
        let panel_pad_x = (22.0 * scale).max(14.0);
        let panel_pad_y = (14.0 * scale).max(10.0);
        let face_aspect = ctx.tile_preset.face_long_ratio();
        let title_font = typography::size(typography::H20, h);
        let body_font = typography::size(typography::H42, h);
        let label_font = (body_font * 0.92).max(11.0);
        let header_font = label_font;

        let footer: Vec<(DecimationAction, [f32; 4])> = if show_footer {
            let mut rect = footer_button_rects(w, h, 1)[0];
            let available_w = (w - margin * 2.0).max(1.0);
            let mut button_w = (available_w * 0.42).max(220.0 * scale);
            button_w = button_w.min(460.0 * scale).min(available_w);
            rect[0] = (w - button_w) * 0.5;
            rect[2] = button_w;
            vec![(DecimationAction::Continue, rect)]
        } else {
            Vec::new()
        };

        let layout = compute_decimation_reveal_layout(
            w,
            h,
            face_aspect,
            display_tiles,
            player,
            house,
            burn_t,
            &footer,
        );

        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);
        push_staircase_environment(&mut frame, &ctx, w, h);
        frame.clear_scene_depth();
        frame.camera_override_after_depth_clear = Some(camera_params(h));
        frame.scene_lighting_after_depth_clear = Some(SceneLighting::showcase_tile_picker(w, h));

        let tiles_bottom = layout
            .placements
            .iter()
            .map(|p| p.center_pos[1] + p.size_px * face_aspect * 0.5)
            .fold(0.0, f32::max);
        let labels_top = (layout.yours_label_y - label_font).min(layout.house_label_y - label_font);
        let panel_top = (labels_top - panel_pad_y).max(margin);
        let mut panel_bottom = (tiles_bottom + panel_pad_y).min(h - margin);
        if let Some((_, rect)) = footer.first() {
            panel_bottom = panel_bottom.min(rect[1] - margin * 0.35);
        }
        let panel_left = (layout.group_x - panel_pad_x).max(margin);
        let panel_right = (layout.group_x + layout.group_w + panel_pad_x).min(w - margin);
        let panel_rect = [
            panel_left,
            panel_top,
            (panel_right - panel_left).max(1.0),
            (panel_bottom - panel_top).max(1.0),
        ];

        if !layout.placements.is_empty() {
            frame.showcase_tile_batch(layout.placements);
        }

        for (rect, color) in self.particles.instances() {
            frame.quad(GpuInstance {
                rect,
                color,
                user: 0,
            });
        }

        frame.text(TextLabel {
            rect: [panel_rect[0], margin, panel_rect[2], title_font * 1.5],
            text: "Decimation".into(),
            color: color::CHAMPAGNE,
            align: TextAlign::Left,
            font_px: Some(title_font),
            ..Default::default()
        });

        let mut subtitle_labels = Vec::new();
        let subtitle = if burn_t > 0.0 {
            "The wall remembers what was taken."
        } else {
            "Ten tiles leave the wall forever."
        };
        styled_text::push_colored_line_left(
            &mut subtitle_labels,
            panel_rect[0],
            margin + title_font * 1.35,
            panel_rect[2],
            body_font,
            subtitle,
            color::PARCHMENT,
            GlossaryMode::Prose,
        );
        frame.texts(subtitle_labels);

        frame.text(TextLabel {
            rect: [
                layout.group_x,
                layout.yours_label_y - label_font,
                layout.group_w,
                label_font * 1.2,
            ],
            text: "Your five".into(),
            color: color::alpha([0.86, 0.18, 0.14, 1.0], 0.95),
            align: TextAlign::Left,
            font_px: Some(label_font),
            ..Default::default()
        });
        let mut house_labels = Vec::new();
        styled_text::push_colored_line_left(
            &mut house_labels,
            layout.group_x,
            layout.house_label_y - label_font,
            layout.group_w,
            label_font,
            "The House's five",
            color::CHAMPAGNE,
            GlossaryMode::Prose,
        );
        frame.texts(house_labels);

        if show_footer {
            let focused = self.tree.focused();
            for it in layout.flat_items.iter() {
                let is_focused = focused == Some(it.id);
                frame.quad(GpuInstance {
                    rect: it.rect,
                    color: if is_focused {
                        color::alpha(color::BRASS, 0.92)
                    } else {
                        color::alpha(color::WALNUT_INK, 0.94)
                    },
                    user: 0,
                });
                frame.text(TextLabel {
                    rect: it.rect,
                    text: "Continue to shop".into(),
                    color: if is_focused {
                        color::WALNUT_DEEP
                    } else {
                        color::CHAMPAGNE
                    },
                    align: TextAlign::Center,
                    font_px: Some(header_font),
                    ..Default::default()
                });
            }
            self.tree
                .register_flat_buttons(&layout.flat_items, &mut frame.buttons);
            ctx.stash_focus_nav_tree_flat(&self.tree, &layout.flat_items, |_| {
                "Continue to shop".into()
            });
        }

        frame
    }
}
