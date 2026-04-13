//! Collection scene — paginated grids of relics, yaku, rules, talismans, and
//! zodiacs. Locked items show a placeholder card with a clue hint. Unlocked
//! talismans and zodiacs render their 3D model tilted and slowly rotating
//! (Resident-Evil-style item viewer).

use std::time::Instant;

use crate::audio::SfxId;
use crate::core::boss::{ALL_BOSSES, FINAL_BOSSES};
use crate::core::relic::{Rarity, RelicId, all_relic_defs};
use crate::core::rules::RuleModifier;
use crate::core::talisman::TalismanKind;
use crate::core::yaku::YakuKind;
use crate::core::zodiac::ZodiacKind;
use crate::game::event_bus::GameEvent;
use crate::render::draw_cmd::{CameraParams, TalismanPlacement, UiFrame, ZodiacRibbonPlacement};
use crate::render::theme::{color, typography};
use crate::render::wgpu_renderer::TextAlign;
use crate::render::wgpu_renderer::{GpuInstance, PointLight, RelicIcon, TextLabel};
use crate::ui::input::UiAction;
use crate::ui::widget::{self, TextStyle};
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use super::start_screen::StartScreenScene;
use super::{DrawCtx, Scene, SceneBehavior, SceneTransition, UpdateCtx};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CollectionAction {
    SelectTab(Tab),
    PrevPage,
    NextPage,
    Back,
}

impl CollectionAction {
    fn id(self) -> FocusId {
        match self {
            CollectionAction::SelectTab(Tab::Relics) => FocusId(1),
            CollectionAction::SelectTab(Tab::Yaku) => FocusId(2),
            CollectionAction::SelectTab(Tab::Rules) => FocusId(3),
            CollectionAction::SelectTab(Tab::Bosses) => FocusId(4),
            CollectionAction::SelectTab(Tab::Talismans) => FocusId(5),
            CollectionAction::SelectTab(Tab::Zodiacs) => FocusId(6),
            CollectionAction::PrevPage => FocusId(10),
            CollectionAction::NextPage => FocusId(11),
            CollectionAction::Back => FocusId(20),
        }
    }
}

// ── Tab enum ────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    Relics,
    Yaku,
    Rules,
    Bosses,
    Talismans,
    Zodiacs,
}

const TABS: [Tab; 6] = [
    Tab::Relics,
    Tab::Yaku,
    Tab::Rules,
    Tab::Bosses,
    Tab::Talismans,
    Tab::Zodiacs,
];

// ── Grid card data ──────────────────────────────────────────────────

struct GridCard {
    name: String,
    subtitle: String,
    clue: String,
    unlocked: bool,
    relic_id: Option<RelicId>,
    rarity_color: [f32; 4],
    /// Set on Talismans tab cards so the 3D viewer knows which mesh to show.
    talisman_kind: Option<TalismanKind>,
    /// Set on Zodiacs tab cards so the 3D viewer knows which ribbon to show.
    zodiac_kind: Option<ZodiacKind>,
}

// ── Scene state ─────────────────────────────────────────────────────

pub struct CollectionScene {
    tab: Tab,
    page: usize,
    tree: TreeState,
    /// Wall-clock instant when the scene was created — drives the slow
    /// turntable rotation of 3D models on the Talismans / Zodiacs tabs.
    created_at: Instant,
}

impl CollectionScene {
    pub fn new() -> Self {
        Self {
            tab: Tab::Relics,
            page: 0,
            tree: TreeState::new(),
            created_at: Instant::now(),
        }
    }

    /// Whether the active tab renders 3D models that need continuous redraws.
    pub fn has_3d_tab(&self) -> bool {
        matches!(self.tab, Tab::Talismans | Tab::Zodiacs)
    }

    fn page_count(&self, entries: usize, per_page: usize) -> usize {
        if per_page == 0 {
            return 1;
        }
        (entries + per_page - 1) / per_page
    }

    /// Single source of truth for tab/footer/back rects. Returns the flat
    /// item list shared by update() (hit-test) and draw() (button registration).
    fn flat_items(&self, w: f32, h: f32, ui_scale: f32) -> Vec<FlatItem<CollectionAction>> {
        let scale = (w.min(h)) / 600.0 * ui_scale;
        let title_font = (24.0 * scale).max(14.0);
        let title_h = text_rect_h(title_font);
        let title_y = h * 0.02;

        let tab_font = (13.0 * scale).max(9.0);
        let tab_y = title_y + title_h + h * 0.015;
        let tab_h = text_rect_h(tab_font);
        let tab_w = (95.0 * scale).min(w * 0.22);
        let tab_gap = (6.0 * scale).max(3.0);
        let tab_count = TABS.len() as f32;
        let tab_total_w = tab_w * tab_count + tab_gap * (tab_count - 1.0);
        let tab_start_x = (w - tab_total_w) * 0.5;

        let mut items = Vec::with_capacity(TABS.len() + 3);
        for (i, &t) in TABS.iter().enumerate() {
            let tx = tab_start_x + i as f32 * (tab_w + tab_gap);
            items.push(FlatItem::new(
                CollectionAction::SelectTab(t).id(),
                [tx, tab_y, tab_w, tab_h],
                CollectionAction::SelectTab(t),
            ));
        }

        // Footer arrows (always pushed; the click handler clamps page).
        let footer_font = (11.0 * scale).max(8.0);
        let footer_h = text_rect_h(footer_font);
        let grid_bottom = h - footer_h - h * 0.02;
        let footer_y = grid_bottom + h * 0.005;
        let arrow_w = (30.0 * scale).max(20.0);
        let center_x = w * 0.5;
        let left_x = center_x - arrow_w - (60.0 * scale);
        let right_x = center_x + (60.0 * scale);
        items.push(FlatItem::new(
            CollectionAction::PrevPage.id(),
            [left_x, footer_y, arrow_w, footer_h],
            CollectionAction::PrevPage,
        ));
        items.push(FlatItem::new(
            CollectionAction::NextPage.id(),
            [right_x, footer_y, arrow_w, footer_h],
            CollectionAction::NextPage,
        ));

        // Back button (top-left).
        let margin_x = w * 0.04;
        let back_w = (70.0 * scale).max(48.0);
        let back_h = (24.0 * scale).max(18.0);
        items.push(FlatItem::new(
            CollectionAction::Back.id(),
            [margin_x, title_y, back_w, back_h],
            CollectionAction::Back,
        ));

        items
    }
}

impl SceneBehavior for CollectionScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        let items = self.flat_items(ctx.layout.window_w, ctx.layout.window_h, ctx.ui_scale);
        let action = self.tree.update_flat(
            &items,
            TreeInput {
                actions: ctx.actions,
                button_clicks: ctx.button_clicks,
                cursor_pos: ctx.cursor_pos,
                window: (ctx.layout.window_w, ctx.layout.window_h),
                ui_scale: ctx.ui_scale,
                input_mode: ctx.input_mode,
                scroll_lines: 0.0,
            },
        );
        if self.tree.take_focus_changed() {
            ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
        }

        // Keyboard tab cycling stays separate from the flat-tree's linear nav
        // (which would otherwise fight with page-flip arrows).
        for a in ctx.actions {
            match a {
                UiAction::NavigateHudNext => {
                    let idx = TABS.iter().position(|t| *t == self.tab).unwrap_or(0);
                    self.tab = TABS[(idx + 1) % TABS.len()];
                    self.page = 0;
                }
                UiAction::NavigateHudPrev => {
                    let idx = TABS.iter().position(|t| *t == self.tab).unwrap_or(0);
                    self.tab = TABS[(idx + TABS.len() - 1) % TABS.len()];
                    self.page = 0;
                }
                UiAction::FocusNext | UiAction::FocusDown => {
                    self.page = self.page.saturating_add(1);
                }
                UiAction::FocusPrev | UiAction::FocusUp => {
                    self.page = self.page.saturating_sub(1);
                }
                UiAction::Cancel | UiAction::Pause | UiAction::CommitDiscard => {
                    ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
                    return Some(Scene::StartScreen(StartScreenScene::new()));
                }
                _ => {}
            }
        }

        match action {
            Some(CollectionAction::SelectTab(t)) => {
                ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                self.tab = t;
                self.page = 0;
            }
            Some(CollectionAction::PrevPage) => {
                ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                self.page = self.page.saturating_sub(1);
            }
            Some(CollectionAction::NextPage) => {
                ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                self.page = self.page.saturating_add(1);
            }
            Some(CollectionAction::Back) => {
                ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
                return Some(Scene::StartScreen(StartScreenScene::new()));
            }
            None => {}
        }
        None
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let ui_scale = ctx.ui_scale;
        let scale = (w.min(h)) / 600.0 * ui_scale;
        let progress = ctx.progress;

        let mut frame = UiFrame::new();

        let mut instances = vec![GpuInstance {
            rect: [0.0, 0.0, w, h],
            color: color::OBSIDIAN,
        }];
        let mut text_labels = Vec::new();
        let mut relic_icons = Vec::new();

        // ── Title ───────────────────────────────────────────────────
        let title_font = (24.0 * scale).max(14.0);
        let title_h = text_rect_h(title_font);
        let title_y = h * 0.02;
        text_labels.push(TextLabel {
            rect: [0.0, title_y, w, title_h],
            text: "Collection".into(),
            color: color::CHAMPAGNE,
            ..Default::default()
        });

        // ── Tab bar ─────────────────────────────────────────────────
        let tab_font = (13.0 * scale).max(9.0);
        let tab_y = title_y + title_h + h * 0.015;
        let tab_h = text_rect_h(tab_font);
        let tab_w = (80.0 * scale).min(w * 0.15);
        let tab_gap = (6.0 * scale).max(3.0);
        let tab_count = TABS.len() as f32;
        let tab_total_w = tab_w * tab_count + tab_gap * (tab_count - 1.0);
        let tab_start_x = (w - tab_total_w) * 0.5;

        let tab_names = ["Relics", "Yaku", "Rules", "Bosses", "Talismans", "Zodiacs"];
        for (i, name) in tab_names.iter().enumerate() {
            let tx = tab_start_x + i as f32 * (tab_w + tab_gap);
            let is_active = TABS[i] == self.tab;
            instances.push(GpuInstance {
                rect: [tx, tab_y, tab_w, tab_h],
                color: if is_active {
                    [0.22, 0.38, 0.58, 0.95]
                } else {
                    [0.10, 0.12, 0.20, 0.85]
                },
            });
            text_labels.push(TextLabel {
                rect: [tx, tab_y, tab_w, tab_h],
                text: name.to_string(),
                color: if is_active {
                    [1.0, 1.0, 1.0, 1.0]
                } else {
                    [0.45, 0.45, 0.55, 0.9]
                },
                ..Default::default()
            });
        }

        // ── Grid area ───────────────────────────────────────────────
        let grid_top = tab_y + tab_h + h * 0.025;
        let footer_font = (11.0 * scale).max(8.0);
        let footer_h = text_rect_h(footer_font);
        let grid_bottom = h - footer_h - h * 0.02;
        let grid_h = grid_bottom - grid_top;

        let margin_x = w * 0.04;
        let grid_w = w - margin_x * 2.0;

        // Card sizing: aim for ~4 columns on relics, ~3 on most, ~2 on
        // 3D tabs so the models have room to breathe.
        let target_cols: usize = match self.tab {
            Tab::Relics => 4,
            Tab::Talismans => 2,
            Tab::Zodiacs => 3,
            _ => 3,
        };
        let card_gap = (8.0 * scale).max(4.0);
        let card_w =
            ((grid_w - card_gap * (target_cols as f32 - 1.0)) / target_cols as f32).max(80.0);
        let cols = ((grid_w + card_gap) / (card_w + card_gap)).floor().max(1.0) as usize;
        let card_aspect = match self.tab {
            Tab::Relics => 1.35,
            Tab::Talismans | Tab::Zodiacs => 1.5,
            _ => 1.1,
        };
        let card_h = (card_w * card_aspect).min(grid_h * 0.48);
        let rows = ((grid_h + card_gap) / (card_h + card_gap)).floor().max(1.0) as usize;
        let per_page = cols * rows;

        // Build entries.
        let cards = match self.tab {
            Tab::Relics => build_relic_cards(progress),
            Tab::Yaku => build_yaku_cards(progress),
            Tab::Rules => build_rule_cards(progress),
            Tab::Bosses => build_boss_cards(),
            Tab::Talismans => build_talisman_cards(),
            Tab::Zodiacs => build_zodiac_cards(progress),
        };

        let total_pages = self.page_count(cards.len(), per_page);
        let page = self.page.min(total_pages.saturating_sub(1));
        let page_start = page * per_page;

        // Center the grid horizontally.
        let actual_grid_w = cols as f32 * card_w + (cols as f32 - 1.0) * card_gap;
        let grid_x = margin_x + (grid_w - actual_grid_w) * 0.5;

        // Center the grid vertically within the available space.
        let actual_grid_h = rows as f32 * card_h + (rows as f32 - 1.0) * card_gap;
        let grid_y = grid_top + (grid_h - actual_grid_h).max(0.0) * 0.5;

        // ── 3D model placements (Talismans / Zodiacs tabs) ──────────
        // Slow turntable rotation: ~30°/s yaw, tilted forward ~20° on X.
        let elapsed = Instant::now()
            .saturating_duration_since(self.created_at)
            .as_secs_f32();
        let turntable_y_deg = (elapsed * 30.0) % 360.0;
        let tilt_x_deg: f32 = -20.0;

        let mut talisman_placements: Vec<TalismanPlacement> = Vec::new();
        let mut ribbon_placements: Vec<ZodiacRibbonPlacement> = Vec::new();

        for (i, card) in cards.iter().skip(page_start).take(per_page).enumerate() {
            let col = i % cols;
            let row = i / cols;
            let cx = grid_x + col as f32 * (card_w + card_gap);
            let cy = grid_y + row as f32 * (card_h + card_gap);

            let has_3d = card.unlocked && matches!(self.tab, Tab::Talismans | Tab::Zodiacs);

            if has_3d {
                draw_model_card(
                    cx,
                    cy,
                    card_w,
                    card_h,
                    scale,
                    card,
                    w,
                    &mut instances,
                    &mut text_labels,
                );
                // Push 3D placement centered in upper card region.
                // pixel_to_world maps (px, py) → world (px-w/2, world_y, py-h/2).
                // The top-down camera with 20° FOV sees roughly ±0.53*h in
                // each axis, so extents of ~0.10*h fill a card nicely.
                let model_center_x = cx + card_w * 0.5;
                let model_center_y = cy + card_h * 0.32;
                let base = h * 0.10;
                let lift = base * 0.5;

                match self.tab {
                    Tab::Talismans => {
                        if let Some(tk) = card.talisman_kind {
                            talisman_placements.push(TalismanPlacement {
                                center_pos: [model_center_x, model_center_y, lift],
                                extents: [base * 1.2, base * 1.7, base * 0.25],
                                rotation_y_deg: 0.0,
                                rotation_x_deg: tilt_x_deg,
                                rotation_z_deg: turntable_y_deg,
                                color: talisman_tint(tk),
                                kind: tk,
                            });
                        }
                    }
                    Tab::Zodiacs => {
                        if let Some(zk) = card.zodiac_kind {
                            ribbon_placements.push(ZodiacRibbonPlacement {
                                anchor_pos: [model_center_x, model_center_y, lift + base * 0.8],
                                length: base * 2.0,
                                width: base * 0.9,
                                rotation_y_deg: 0.0,
                                rotation_x_deg: tilt_x_deg,
                                rotation_z_deg: turntable_y_deg,
                                color: [1.0, 1.0, 1.0, 1.0],
                                kind: Some(zk),
                            });
                        }
                    }
                    _ => {}
                }
            } else if card.unlocked {
                draw_unlocked_card(
                    cx,
                    cy,
                    card_w,
                    card_h,
                    scale,
                    card,
                    w,
                    h,
                    &mut instances,
                    &mut text_labels,
                    &mut relic_icons,
                    ui_scale,
                );
            } else {
                draw_locked_card(
                    cx,
                    cy,
                    card_w,
                    card_h,
                    scale,
                    card,
                    w,
                    &mut instances,
                    &mut text_labels,
                );
            }
        }

        // ── Page indicator / nav ────────────────────────────────────
        let footer_y = grid_bottom + h * 0.005;
        if total_pages > 1 {
            // Left arrow button.
            let arrow_w = (30.0 * scale).max(20.0);
            let center_x = w * 0.5;
            let left_x = center_x - arrow_w - (60.0 * scale);
            instances.push(GpuInstance {
                rect: [left_x, footer_y, arrow_w, footer_h],
                color: if page > 0 {
                    [0.18, 0.25, 0.40, 0.9]
                } else {
                    [0.10, 0.10, 0.15, 0.5]
                },
            });
            text_labels.push(TextLabel {
                rect: [left_x, footer_y, arrow_w, footer_h],
                text: "<".into(),
                color: [1.0, 1.0, 1.0, 0.9],
                ..Default::default()
            });

            // Page number.
            let page_text = format!("{} / {}", page + 1, total_pages);
            let page_w = 120.0 * scale;
            text_labels.push(TextLabel {
                rect: [center_x - page_w * 0.5, footer_y, page_w, footer_h],
                text: page_text,
                color: [0.7, 0.7, 0.8, 0.9],
                ..Default::default()
            });

            // Right arrow button.
            let right_x = center_x + (60.0 * scale);
            instances.push(GpuInstance {
                rect: [right_x, footer_y, arrow_w, footer_h],
                color: if page + 1 < total_pages {
                    [0.18, 0.25, 0.40, 0.9]
                } else {
                    [0.10, 0.10, 0.15, 0.5]
                },
            });
            text_labels.push(TextLabel {
                rect: [right_x, footer_y, arrow_w, footer_h],
                text: ">".into(),
                color: [1.0, 1.0, 1.0, 0.9],
                ..Default::default()
            });
        }

        // ── Unlock counter ──────────────────────────────────────────
        let unlocked = cards.iter().filter(|c| c.unlocked).count();
        let total = cards.len();
        let counter_w = (200.0 * scale).min(w * 0.4);
        text_labels.push(TextLabel {
            rect: [w - counter_w - margin_x, footer_y, counter_w, footer_h],
            text: format!("{} / {} unlocked", unlocked, total),
            color: [0.5, 0.5, 0.6, 0.8],
            ..Default::default()
        });

        // ── Hint ────────────────────────────────────────────────────
        let hint_w = (300.0 * scale).min(w * 0.5);
        text_labels.push(TextLabel {
            rect: [margin_x, footer_y, hint_w, footer_h],
            text: "L/R tabs  |  Arrows page  |  Esc back".into(),
            color: [0.35, 0.35, 0.45, 0.7],
            ..Default::default()
        });

        // ── Back button ─────────────────────────────────────────────
        let back_w = (70.0 * scale).max(48.0);
        let back_h = (24.0 * scale).max(18.0);
        let back_x = margin_x;
        let back_y = title_y;
        instances.push(GpuInstance {
            rect: [back_x, back_y, back_w, back_h],
            color: [0.18, 0.20, 0.30, 0.92],
        });
        text_labels.push(TextLabel {
            rect: [back_x, back_y, back_w, back_h],
            text: "< Back".into(),
            color: [0.85, 0.85, 0.95, 1.0],
            ..Default::default()
        });

        // ── Assemble frame ──────────────────────────────────────────
        let has_3d_models = !talisman_placements.is_empty() || !ribbon_placements.is_empty();
        if has_3d_models {
            // Near-orthographic top-down camera so pixel_to_world positions
            // map cleanly onto screen space without heavy perspective warp.
            // Narrow FOV + high eye keeps the 3D models sitting right where
            // the 2D cards are.
            frame.camera_override = Some(CameraParams {
                eye: [0.0, h * 3.0, h * 0.01],
                target: [0.0, 0.0, h * 0.01],
                up: [0.0, 0.0, -1.0],
                fovy_deg: 20.0,
            });
            // Point lights illuminate the 3D models like museum spotlights.
            frame.point_lights = vec![
                PointLight {
                    pos: [w * 0.5, h * 0.3, 180.0],
                    radius: w.max(h) * 1.5,
                    color: [1.0, 0.95, 0.85],
                    intensity: 1.6,
                },
                // Fill light from below-left so the underside isn't black.
                PointLight {
                    pos: [w * 0.3, h * 0.6, 60.0],
                    radius: w.max(h),
                    color: [0.6, 0.65, 0.8],
                    intensity: 0.7,
                },
            ];
        }
        // Card-background quads first, then 3D models on top.
        frame.quads(instances);
        if !talisman_placements.is_empty() {
            frame.talisman_batch(talisman_placements);
        }
        if !ribbon_placements.is_empty() {
            frame.zodiac_batch(ribbon_placements);
        }
        frame.texts(text_labels);
        frame.relic_icons(relic_icons);

        // Single hit-target list shared with update().
        let items = self.flat_items(w, h, ui_scale);
        self.tree.register_flat_buttons(&items, &mut frame.buttons);

        frame.window_title = format!("Mahjuro — Collection ({}/{})", unlocked, total);
        frame
    }
}

// ── Card drawing ────────────────────────────────────────────────────

/// Compute a text-rect height from a desired screen-relative font size in px.
/// The rasteriser picks `min(rect_h * 0.55, rect_w * 1.5 / char_count)`.
/// We want the *height* term to be the binding constraint so the font stays
/// at a consistent readable size regardless of card width.
/// Given target font px `f`, we need `rect_h >= f / 0.55`.
/// We also need `rect_w >= f * char_count / 1.5` — if the text is too long
/// for the available width, the width term takes over and shrinks the font.
/// To avoid that, callers should keep text short or widen the rect.
fn text_rect_h(target_font_px: f32) -> f32 {
    (target_font_px / 0.55).ceil()
}

fn draw_unlocked_card(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    scale: f32,
    card: &GridCard,
    win_w: f32,
    win_h: f32,
    instances: &mut Vec<GpuInstance>,
    labels: &mut Vec<TextLabel>,
    icons: &mut Vec<RelicIcon>,
    ui_scale: f32,
) {
    // Card background.
    instances.push(GpuInstance {
        rect: [x, y, w, h],
        color: [0.12, 0.17, 0.28, 0.92],
    });
    // Thin rarity accent bar at top.
    let accent_h = (3.0 * scale).max(2.0);
    instances.push(GpuInstance {
        rect: [x, y, w, accent_h],
        color: card.rarity_color,
    });

    let pad = (6.0 * scale).max(3.0);

    // Screen-relative font sizes (in pixels) — these don't depend on card size.
    let name_font = (13.0 * scale).max(10.0);
    let sub_font = (10.0 * scale).max(8.0);

    // Derived rect heights from target font size.
    let name_rect_h = text_rect_h(name_font);
    let sub_rect_h = text_rect_h(sub_font);

    if let Some(relic_id) = card.relic_id {
        // Relic icon centered in upper portion.
        let icon_size = (w * 0.45).min(h * 0.35).max(24.0);
        let icon_x = x + (w - icon_size) * 0.5;
        let icon_y = y + accent_h + pad;
        icons.push(RelicIcon {
            rect: [icon_x, icon_y, icon_size, icon_size],
            relic_id,
        });

        // Name below icon — use screen-wide rect for readability.
        let name_y = icon_y + icon_size + pad * 0.5;
        let (nx, nw) = readable_text_rect(x, w, pad, &card.name, name_font, win_w);
        labels.push(TextLabel {
            rect: [nx, name_y, nw, name_rect_h],
            text: card.name.clone(),
            color: [0.95, 0.9, 0.65, 1.0],
            ..Default::default()
        });

        // Subtitle (description) — wrapped into the remaining card space.
        let sub_y = name_y + name_rect_h + pad * 0.3;
        let sub_h = (y + h - pad - sub_y).max(sub_rect_h);
        widget::push_text_block(
            labels,
            [x + pad, sub_y, w - pad * 2.0, sub_h],
            &card.subtitle,
            TextStyle {
                tier: typography::CAPTION,
                color: [0.6, 0.6, 0.7, 0.9],
                padding: 0.0,
                align: TextAlign::Left,
            },
            win_h,
            ui_scale,
        );
    } else {
        // Non-relic card (yaku / rule): no icon, text layout only.
        let name_y = y + accent_h + h * 0.15;
        let (nx, nw) = readable_text_rect(x, w, pad, &card.name, name_font, win_w);
        labels.push(TextLabel {
            rect: [nx, name_y, nw, name_rect_h],
            text: card.name.clone(),
            color: [0.95, 0.9, 0.65, 1.0],
            ..Default::default()
        });

        let sub_y = name_y + name_rect_h + pad;
        let sub_h = (y + h - pad - sub_y).max(sub_rect_h);
        widget::push_text_block(
            labels,
            [x + pad, sub_y, w - pad * 2.0, sub_h],
            &card.subtitle,
            TextStyle {
                tier: typography::CAPTION,
                color: [0.65, 0.65, 0.75, 0.9],
                padding: 0.0,
                align: TextAlign::Left,
            },
            win_h,
            ui_scale,
        );
    }
}

fn draw_locked_card(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    scale: f32,
    card: &GridCard,
    win_w: f32,
    instances: &mut Vec<GpuInstance>,
    labels: &mut Vec<TextLabel>,
) {
    // Dark locked background.
    instances.push(GpuInstance {
        rect: [x, y, w, h],
        color: [0.07, 0.07, 0.11, 0.85],
    });
    // Dim border inset.
    let b = (2.0 * scale).max(1.0);
    instances.push(GpuInstance {
        rect: [x + b, y + b, w - b * 2.0, h - b * 2.0],
        color: [0.09, 0.09, 0.14, 0.90],
    });

    let pad = (6.0 * scale).max(3.0);

    // "?" symbol — short text, the card width is plenty.
    let lock_font = (22.0 * scale).max(14.0);
    let lock_rect_h = text_rect_h(lock_font);
    let lock_y = y + h * 0.15;
    labels.push(TextLabel {
        rect: [x, lock_y, w, lock_rect_h],
        text: "?".into(),
        color: [0.25, 0.25, 0.35, 0.7],
        ..Default::default()
    });

    // Clue text — use screen-relative sizing.
    let clue_font = (9.0 * scale).max(8.0);
    let clue_rect_h = text_rect_h(clue_font);
    let clue_y = lock_y + lock_rect_h + pad;
    let (cx, cw) = readable_text_rect(x, w, pad, &card.clue, clue_font, win_w);
    labels.push(TextLabel {
        rect: [cx, clue_y, cw, clue_rect_h],
        text: card.clue.clone(),
        color: [0.30, 0.30, 0.40, 0.7],
        ..Default::default()
    });
}

/// Compute a text rect (x, width) that ensures the rasteriser's width term
/// `(width * 1.5 / char_count)` stays >= the target font size.
///
/// If the card's inner width is enough, returns the card-padded rect.
/// Otherwise, widens the rect (centered on the card) up to `win_w`, so that
/// the font doesn't shrink below the intended size.
fn readable_text_rect(
    card_x: f32,
    card_w: f32,
    pad: f32,
    text: &str,
    target_font: f32,
    win_w: f32,
) -> (f32, f32) {
    let inner_x = card_x + pad;
    let inner_w = card_w - pad * 2.0;
    let char_count = text.chars().count().max(1) as f32;

    // Minimum width so the width-based term >= target_font:
    // target_font <= width * 1.5 / char_count  →  width >= target_font * char_count / 1.5
    let min_w = target_font * char_count / 1.5;

    if inner_w >= min_w {
        // Card width is sufficient.
        (inner_x, inner_w)
    } else {
        // Widen, centered on the card, clamped to window.
        let needed = min_w.min(win_w);
        let center = card_x + card_w * 0.5;
        let rx = (center - needed * 0.5).max(0.0);
        let rw = needed.min(win_w - rx);
        (rx, rw)
    }
}

// ── Card builders ───────────────────────────────────────────────────

fn rarity_to_color(r: Rarity) -> [f32; 4] {
    // Centralized in `theme::color::rarity` so the shop and collection don't drift.
    match r {
        Rarity::Common => color::rarity(0),
        Rarity::Uncommon => color::rarity(1),
        Rarity::Rare => color::rarity(2),
        Rarity::Legendary => color::rarity(3),
    }
}

fn build_relic_cards(progress: &crate::core::progression::PlayerProgress) -> Vec<GridCard> {
    let defs = all_relic_defs();
    let available = progress.available_relics();
    defs.iter()
        .map(|d| {
            let unlocked = available.contains(&d.id);
            GridCard {
                name: if unlocked {
                    d.name.to_string()
                } else {
                    "???".into()
                },
                subtitle: if unlocked {
                    d.description.to_string()
                } else {
                    String::new()
                },
                clue: if unlocked {
                    String::new()
                } else {
                    relic_clue(d.id)
                },
                unlocked,
                relic_id: if unlocked { Some(d.id) } else { None },
                rarity_color: rarity_to_color(d.rarity),
                talisman_kind: None,
                zodiac_kind: None,
            }
        })
        .collect()
}

fn build_yaku_cards(progress: &crate::core::progression::PlayerProgress) -> Vec<GridCard> {
    let all = YakuKind::all();
    let available = progress.available_yaku();
    all.iter()
        .map(|&yk| {
            let unlocked = available.contains(&yk);
            GridCard {
                name: if unlocked {
                    yk.name().to_string()
                } else {
                    "???".into()
                },
                subtitle: if unlocked {
                    format!("{} (+{} mult)", yaku_description(yk), yk.mult_bonus())
                } else {
                    String::new()
                },
                clue: if unlocked {
                    String::new()
                } else {
                    yaku_clue(yk)
                },
                unlocked,
                relic_id: None,
                rarity_color: color::TWILIGHT, // indigo accent for yaku
                talisman_kind: None,
                zodiac_kind: None,
            }
        })
        .collect()
}

/// All bosses, always unlocked. The collection's "Bosses" tab is reference
/// material — players want to read what each boss does outside of a fight,
/// not earn them like relics.
fn build_boss_cards() -> Vec<GridCard> {
    ALL_BOSSES
        .iter()
        .chain(FINAL_BOSSES.iter())
        .map(|def| GridCard {
            name: def.name.to_string(),
            subtitle: format!("[{}]  {}", def.tier.label(), def.description),
            clue: String::new(),
            unlocked: true,
            relic_id: None,
            rarity_color: def.tier.halo_color(),
            talisman_kind: None,
            zodiac_kind: None,
        })
        .collect()
}

fn build_rule_cards(progress: &crate::core::progression::PlayerProgress) -> Vec<GridCard> {
    let all = [
        RuleModifier::PairDoubleScore,
        RuleModifier::SequenceWrap,
        RuleModifier::NoSequenceBonus,
        RuleModifier::NoSequences,
        RuleModifier::ReducedPlays,
        RuleModifier::HonorTripleScore,
    ];
    let available = progress.available_rules();
    all.iter()
        .map(|&rm| {
            let unlocked = available.contains(&rm);
            GridCard {
                name: if unlocked {
                    rm.name().to_string()
                } else {
                    "???".into()
                },
                subtitle: if unlocked {
                    rm.description().to_string()
                } else {
                    String::new()
                },
                clue: if unlocked {
                    String::new()
                } else {
                    rule_clue(rm)
                },
                unlocked,
                relic_id: None,
                rarity_color: color::AMBER, // amber accent for rules
                talisman_kind: None,
                zodiac_kind: None,
            }
        })
        .collect()
}

// ── Clue text ───────────────────────────────────────────────────────

fn relic_clue(id: RelicId) -> String {
    match id {
        RelicId::TripletBoost => "Rewards grouping three of a kind.".into(),
        RelicId::SequenceSurge => "Power in consecutive tiles.".into(),
        RelicId::PairPower => "Two of a kind, stronger than they appear.".into(),
        RelicId::MultiplierMaster => "Compound strength from collecting.".into(),
        RelicId::GreenLuck => "Gold in simplicity.".into(),
        RelicId::QuickDraw => "The swift hand gains advantage.".into(),
        RelicId::ChainReaction => "Momentum builds turn after turn.".into(),
        RelicId::SetMagnet => "A triplet calls to its kin.".into(),
        RelicId::HonorFury => "Ancient fury when honor tiles group.".into(),
        RelicId::WhiteSilence => "A quiet bonus to pairs.".into(),
        RelicId::Overflow => "The wall grows thick.".into(),
        RelicId::JokerTile => "Becomes anything, once per round.".into(),
        RelicId::WildWinds => "Winds bend to fill gaps.".into(),
        RelicId::DragonEcho => "Resonates with adjacent sets.".into(),
        RelicId::RedDragonRage => "Fury of the red dragon.".into(),
        // ── Patch C new relics ──
        RelicId::ShantenShove => "One last push toward tenpai.".into(),
        RelicId::KanDrum => "Beats louder for every kong.".into(),
        RelicId::DoraCrown => "Crown of indicators.".into(),
        RelicId::RiichiStick => "A bet declared with confidence.".into(),
        RelicId::TenpaiTalisman => "Doubles the first complete hand bonus.".into(),
        RelicId::RiverEraser => "Wipes the river clean.".into(),
        RelicId::FuritenWard => "Wards against your own discards.".into(),
        RelicId::RoundCompass => "Honors the round wind.".into(),
        RelicId::ZodiacPouch => "Holds an extra Zodiac.".into(),
        RelicId::LunarAlmanac => "Doubles every third Zodiac use.".into(),
        RelicId::EightTreasures => "A complete hand pulls a Zodiac from the air.".into(),
        RelicId::KongsBlessing => "Kongs blessed with a pair's power.".into(),
        RelicId::CodexCompass => "A compass from an older ruleset.".into(),
        // ── Flower-synergy relics ──
        RelicId::GardenKeeper => "Tends the garden with double care.".into(),
        RelicId::Ikebana => "Art in arranging many flowers.".into(),
        RelicId::Hanami => "Gold blooms under the blossoms.".into(),
        // ── New relics ──
        RelicId::JadeSerpent => "The serpent favors bamboo.".into(),
        RelicId::InkBrush => "Strokes of power on character tiles.".into(),
        RelicId::PearlDiver => "Dives deep among the dots.".into(),
        RelicId::LowTide => "Strength in small numbers.".into(),
        RelicId::MerchantsEye => "Sees value others miss.".into(),
        RelicId::EdgeRunner => "Terminals carry extra weight.".into(),
        RelicId::LuckySeven => "Fortune favors the seventh.".into(),
        RelicId::Momentum => "Each play builds on the last.".into(),
        RelicId::Minimalist => "Less is more.".into(),
        RelicId::TurtleShell => "Protection when power is low.".into(),
        RelicId::ClosedGate => "Only edges and honors pass.".into(),
        RelicId::GoldFurnace => "Wealth fuels the flame.".into(),
        RelicId::Snowball => "Grows larger as it rolls.".into(),
        RelicId::SecondWind => "One more chance each round.".into(),
        RelicId::GlassCannon => "Devastating power, fragile frame.".into(),
        // ── Patch F relics ──
        RelicId::LastBreath => "A dying gasp that echoes every tile.".into(),
        RelicId::TilePolisher => "Each tile remembers being scored.".into(),
        RelicId::PaperLantern => "Burns bright, burns fast.".into(),
        RelicId::IronLantern => "Forged from paper ash.".into(),
        RelicId::MirrorTile => "Reflects the relic beside it.".into(),
        RelicId::WayOfPurity => "One suit to rule them all.".into(),
        // ── Patch G relics ──
        RelicId::LeadingTile => "The first tile leads by example.".into(),
        RelicId::LowEcho => "Small numbers resonate twice.".into(),
        RelicId::TeaCeremony => "A brief, exquisite ritual.".into(),
        RelicId::GhostHand => "Even the unplayed tiles contribute.".into(),
        RelicId::CleanStreak => "Purity builds momentum.".into(),
        RelicId::Obsession => "Variety is rewarded.".into(),
        RelicId::Bonfire => "Sacrifice fuels the flame.".into(),
        RelicId::RiverRunner => "Sequences carve deeper channels.".into(),
        RelicId::MeltingIce => "Strong but fleeting.".into(),
        RelicId::SilkThread => "Delicate power, handle with care.".into(),
        RelicId::ShadowHand => "Echoes the leader's technique.".into(),
        RelicId::EmptyFrame => "Absence is its own power.".into(),
        RelicId::GoldIdol => "Steady income, no strings attached.".into(),
        RelicId::JadeAbacus => "Compound interest, jade-style.".into(),
        RelicId::NestEgg => "Grows more valuable with patience.".into(),
        RelicId::Patience => "Restraint has its rewards.".into(),
        RelicId::WayOfPairs => "Seven pairs, twice the power.".into(),
        RelicId::WayOfTriplets => "Three of a kind, in every set.".into(),
        RelicId::WayOfSequences => "Runs upon runs upon runs.".into(),
        RelicId::FortunesFavor => "Lady luck smiles wider.".into(),
        RelicId::CrackedTile => "Chaotic energy spills out.".into(),
        RelicId::StarTile => "A rare chance at enlightenment.".into(),
        RelicId::SmokeBomb => "A coward's escape — or a strategist's.".into(),
        RelicId::PhantomRelic => "Not quite real, not quite gone.".into(),
        RelicId::RitualBlade => "Power demands sacrifice.".into(),
    }
}

fn yaku_clue(yk: YakuKind) -> String {
    match yk {
        YakuKind::FullHand => "Play all 14 tiles perfectly.".into(),
        YakuKind::Yakuhai => "Available from the start.".into(),
        YakuKind::Toitoi => "Reach Level 2.".into(),
        YakuKind::Tanyao => "Reach Level 2.".into(),
        YakuKind::Iipeikou => "Reach Level 3.".into(),
        YakuKind::Honitsu => "Reach Level 3.".into(),
        YakuKind::Chinitsu => "Reach Level 4.".into(),
        YakuKind::Chiitoitsu => "Reach Level 4.".into(),
        YakuKind::SanshokuDoujun => "Reach Level 5.".into(),
        YakuKind::Honroutou => "Reach Level 5.".into(),
        YakuKind::Junchan => "Reach Level 6.".into(),
        YakuKind::Ittsu => "Reach Level 6.".into(),
        YakuKind::ChickenHand => "Available from the start.".into(),
    }
}

fn yaku_description(yk: YakuKind) -> &'static str {
    yk.description()
}

fn rule_clue(rm: RuleModifier) -> String {
    match rm {
        RuleModifier::PairDoubleScore => "Available from the start.".into(),
        RuleModifier::SequenceWrap => "Reach Level 3.".into(),
        RuleModifier::NoSequenceBonus => "Reach Level 4.".into(),
        RuleModifier::NoSequences => "Reach Level 6.".into(),
        RuleModifier::ReducedPlays => "Reach Level 6.".into(),
        RuleModifier::HonorTripleScore => "Reach Level 5.".into(),
        // Boss-only rule modifiers — applied by specific boss blinds, not
        // earned via progression. Surface them as boss flavor in the
        // collection so the player can read what each does outside a fight.
        RuleModifier::PairsScoreZero => "The Hermit boss.".into(),
        RuleModifier::SequencesHalved => "The Forest boss.".into(),
        RuleModifier::MiddleTilesZero => "The Drunkard boss.".into(),
        RuleModifier::MustPlayFive => "The Bureaucrat boss.".into(),
        RuleModifier::RequireHonor => "The Dragon final boss.".into(),
        RuleModifier::CensorRepeats => "The Censor boss.".into(),
    }
}

// ── Talisman & Zodiac card builders ────────────────────────────────

/// Talismans are always available — they're consumable reference material,
/// like the Bosses tab.
fn build_talisman_cards() -> Vec<GridCard> {
    TalismanKind::all()
        .iter()
        .map(|&tk| GridCard {
            name: tk.name().to_string(),
            subtitle: tk.description().to_string(),
            clue: String::new(),
            unlocked: true,
            relic_id: None,
            rarity_color: talisman_accent(tk),
            talisman_kind: Some(tk),
            zodiac_kind: None,
        })
        .collect()
}

/// Zodiacs are gated by yaku progression — each zodiac unlocks when its
/// paired yaku becomes available.
fn build_zodiac_cards(progress: &crate::core::progression::PlayerProgress) -> Vec<GridCard> {
    let available_yaku = progress.available_yaku();
    ZodiacKind::all()
        .iter()
        .map(|&zk| {
            let unlocked = available_yaku.contains(&zk.yaku());
            GridCard {
                name: if unlocked {
                    format!("{} ({})", zk.name(), zk.yaku().name())
                } else {
                    "???".into()
                },
                subtitle: if unlocked {
                    format!(
                        "Levels up {}. +0.5 mult, +20 chips per level.",
                        zk.yaku().name(),
                    )
                } else {
                    String::new()
                },
                clue: if unlocked {
                    String::new()
                } else {
                    zodiac_clue(zk)
                },
                unlocked,
                relic_id: None,
                rarity_color: color::TWILIGHT,
                talisman_kind: None,
                zodiac_kind: if unlocked { Some(zk) } else { None },
            }
        })
        .collect()
}

fn zodiac_clue(zk: ZodiacKind) -> String {
    // The zodiac unlocks when its paired yaku does — reuse the yaku clue.
    yaku_clue(zk.yaku())
}

/// Thin rarity-bar accent colour per talisman kind — mirrors the gemstone.
fn talisman_accent(tk: TalismanKind) -> [f32; 4] {
    match tk {
        TalismanKind::Jade => [0.30, 0.78, 0.50, 1.0],
        TalismanKind::Pearl => [0.85, 0.88, 0.95, 1.0],
        TalismanKind::Gilded => [0.95, 0.78, 0.30, 1.0],
        TalismanKind::Polychrome => [0.80, 0.40, 0.90, 1.0],
        TalismanKind::Kiln => [0.85, 0.35, 0.18, 1.0],
    }
}

/// Base colour tint for the 3D talisman model in the collection viewer.
fn talisman_tint(tk: TalismanKind) -> [f32; 4] {
    match tk {
        TalismanKind::Jade => [0.42, 0.82, 0.55, 1.0],
        TalismanKind::Pearl => [0.94, 0.95, 0.98, 1.0],
        TalismanKind::Gilded => [0.96, 0.78, 0.30, 1.0],
        TalismanKind::Polychrome => [0.82, 0.55, 0.95, 1.0],
        TalismanKind::Kiln => [0.85, 0.35, 0.18, 1.0],
    }
}

// ── 3D model card drawing ──────────────────────────────────────────

/// Draw a card that hosts a 3D model in its upper region and text below.
/// The 3D placement itself is pushed separately by draw_frame; this
/// function only draws the 2D card background and text labels.
fn draw_model_card(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    scale: f32,
    card: &GridCard,
    win_w: f32,
    instances: &mut Vec<GpuInstance>,
    labels: &mut Vec<TextLabel>,
) {
    // Card background — slightly darker to make the lit 3D model pop.
    instances.push(GpuInstance {
        rect: [x, y, w, h],
        color: [0.08, 0.10, 0.20, 0.92],
    });
    // Thin accent bar at top.
    let accent_h = (3.0 * scale).max(2.0);
    instances.push(GpuInstance {
        rect: [x, y, w, accent_h],
        color: card.rarity_color,
    });

    let pad = (6.0 * scale).max(3.0);
    let name_font = (13.0 * scale).max(10.0);
    let sub_font = (10.0 * scale).max(8.0);
    let name_rect_h = text_rect_h(name_font);
    let sub_rect_h = text_rect_h(sub_font);

    // Name sits below the model region (~65% of card height).
    let name_y = y + h * 0.65;
    let (nx, nw) = readable_text_rect(x, w, pad, &card.name, name_font, win_w);
    labels.push(TextLabel {
        rect: [nx, name_y, nw, name_rect_h],
        text: card.name.clone(),
        color: [0.95, 0.9, 0.65, 1.0],
        ..Default::default()
    });

    // Description below name.
    let sub_y = name_y + name_rect_h + pad * 0.3;
    let sub_h = (y + h - pad - sub_y).max(sub_rect_h);
    labels.push(TextLabel {
        rect: [x + pad, sub_y, w - pad * 2.0, sub_h],
        text: card.subtitle.clone(),
        color: [0.6, 0.6, 0.7, 0.9],
        align: TextAlign::Left,
        ..Default::default()
    });
}
