//! Shared pause menu overlay used by gameplay, shop, and blind-selection scenes.

use crate::game::engine::GameEngine;
use crate::game::event_bus::{EventBus, GameEvent};
use crate::game::run::RunState;
use crate::render::theme::{ButtonVariant, color, metrics, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextLabel};
use crate::sfx_id::SfxId;
use crate::ui::input::UiAction;
use crate::ui::widget_tree::{self as wt, FocusId, Tree, TreeFrame, TreeInput, TreeState};

use super::options::OptionsScene;
use super::{ButtonDef, SceneIntent, SceneTransition, UpdateCtx};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PauseAction {
    Resume,
    Restart,
    Options,
    MainMenu,
    Exit,
}

impl PauseAction {
    fn id(self) -> FocusId {
        FocusId(0x1000_0000 + self as u32)
    }
}

/// Frame input for `PauseMenu::update`: action queue, newly clicked
/// button ids, cursor position, scroll delta, the current input
/// device (kb/controller/mouse), and LMB held state (for options sliders).
pub struct PauseInput<'a> {
    pub actions: &'a [UiAction],
    pub button_clicks: &'a [u32],
    pub cursor_pos: (f32, f32),
    pub scroll_lines: f32,
    pub input_mode: crate::ui::input::InputMode,
    pub mouse_left_down: bool,
}

/// Result of processing pause menu input for the current frame.
pub enum PauseUpdate {
    /// Stay paused, no scene transition.
    StayPaused,
    /// Resume the current scene (unpause).
    Resume,
    /// Transition to another scene.
    Transition(Box<SceneTransition>),
    /// Quit the application.
    Quit,
}

/// Reusable pause menu state.
pub struct PauseMenu {
    pub paused: bool,
    tree: TreeState,
    /// When `Some`, the options menu is open as a sub-overlay on top of the
    /// pause menu — input and draw are delegated to it until the user
    /// chooses Back, at which point we drop back to the pause root.
    options_overlay: Option<OptionsScene>,
    /// One-shot flag set when the user opens Credits from the options overlay.
    credits_requested: bool,
}

impl Default for PauseMenu {
    fn default() -> Self {
        Self::new()
    }
}

impl PauseMenu {
    pub fn new() -> Self {
        Self {
            paused: false,
            tree: TreeState::new(),
            options_overlay: None,
            credits_requested: false,
        }
    }

    /// Drain the credits-open request flag. Returns `true` exactly once
    /// after the user picks "Credits" from the pause options overlay.
    pub fn take_credits_request(&mut self) -> bool {
        let v = self.credits_requested;
        self.credits_requested = false;
        v
    }

    /// Open the pause menu.
    pub fn open(&mut self) {
        self.paused = true;
        self.tree = TreeState::new();
        self.options_overlay = None;
        self.credits_requested = false;
    }

    /// True when the embedded options overlay is currently visible. Callers
    /// (e.g. main loop) use this to know when to sync `OptionsScene`-driven
    /// settings (audio, tile preset, etc.) back to the live app state, the
    /// same way they do for the standalone options scene.
    pub fn options_overlay(&self) -> Option<&OptionsScene> {
        self.options_overlay.as_ref()
    }

    fn build_tree(&self, window_w: f32, window_h: f32) -> Tree<PauseAction> {
        let scale = metrics::scene_scale(window_w, window_h);
        let btn_w = (220.0 * scale).min(window_w * 0.55);
        let btn_gap = (12.0 * scale).max(6.0);
        let count = 5_f32;
        let title_h = typography::size(typography::H20, window_h);
        let title_gap = (24.0 * scale).max(10.0);
        // Cap button height so the full menu fits on screen.
        let max_menu_h = window_h * 0.88 - title_h - title_gap;
        let btn_h = (44.0 * scale)
            .max(28.0)
            .min((max_menu_h - (count - 1.0) * btn_gap) / count);
        let total_menu_h = count * btn_h + (count - 1.0) * btn_gap;
        let block_h = title_h + title_gap + total_menu_h;
        let start_y = ((window_h - block_h) * 0.5).max(8.0);
        let menu_y = start_y + title_h + title_gap;
        let menu_x = (window_w - btn_w) * 0.5;

        let items = vec![
            wt::button_id(
                PauseAction::Resume.id(),
                "Resume",
                PauseAction::Resume,
                ButtonVariant::Primary,
            ),
            wt::button_id(
                PauseAction::Restart.id(),
                "Restart",
                PauseAction::Restart,
                ButtonVariant::Default,
            ),
            wt::button_id(
                PauseAction::Options.id(),
                "Options",
                PauseAction::Options,
                ButtonVariant::Default,
            ),
            wt::button_id(
                PauseAction::MainMenu.id(),
                "Main Menu",
                PauseAction::MainMenu,
                ButtonVariant::Default,
            ),
            wt::button_id(
                PauseAction::Exit.id(),
                "Exit",
                PauseAction::Exit,
                ButtonVariant::Danger,
            ),
        ];

        Tree::vertical_menu(items).with_anchor([menu_x, menu_y, btn_w, total_menu_h])
    }

    /// One-call frame handler for scenes that own a `PauseMenu`. Returns:
    ///
    ///   - `Some(transition)` — the pause menu consumed this frame. The
    ///     scene should immediately `return transition` from its `update()`
    ///     without running any more input or game logic. `transition` is
    ///     `Some(scene)` for menu choices that navigate (Restart, Main Menu)
    ///     and `None` for "stay in this scene" (StayPaused, Resume, Quit).
    ///   - `None` — the pause menu did not intercept this frame and the
    ///     scene should continue with its normal update logic.
    ///
    /// Two cases consume the frame:
    ///   1. The menu is currently `paused` — input is dispatched into
    ///      [`Self::update`] and the result is folded into the return value.
    ///      `Quit` also flips `ctx.quit_requested`.
    ///   2. The menu is closed but a `UiAction::Pause` was pressed this
    ///      frame — the menu opens and the frame is consumed. This means
    ///      Pause always wins over any same-frame scene action (clicks,
    ///      tree navigation), which is the desired behavior: pressing
    ///      Pause should never be eaten by a competing input.
    ///
    /// Centralizing both cases here keeps every pause-aware scene's
    /// `update()` to a single line of pause boilerplate, and means future
    /// pause-menu changes (new menu items, new transitions) only touch
    /// this file.
    pub fn handle(&mut self, ctx: &mut UpdateCtx<'_>) -> Option<SceneTransition> {
        if self.paused {
            let result = self.update(
                PauseInput {
                    actions: ctx.actions,
                    button_clicks: ctx.button_clicks,
                    cursor_pos: ctx.cursor_pos,
                    scroll_lines: ctx.scroll_lines,
                    input_mode: ctx.input_mode,
                    mouse_left_down: ctx.mouse_left_down,
                },
                ctx.progress,
                ctx.run,
                ctx.bus,
                crate::ui::layout::ViewportCtx {
                    window_w: ctx.layout.window_w,
                    window_h: ctx.layout.window_h,
                },
                ctx.active_profile,
            );
            return Some(match result {
                PauseUpdate::StayPaused | PauseUpdate::Resume => None,
                PauseUpdate::Transition(t) => *t,
                PauseUpdate::Quit => {
                    *ctx.quit_requested = true;
                    None
                }
            });
        }
        // Not currently paused — listen for the open shortcut.
        for a in ctx.actions {
            if matches!(a, UiAction::Pause) {
                self.open();
                ctx.bus.push(GameEvent::UiSound(SfxId::Pause));
                return Some(None);
            }
        }
        None
    }

    /// Process actions while paused. Returns what the caller should do.
    /// `input.cursor_pos` and `viewport` enable mouse hover-to-focus.
    pub fn update(
        &mut self,
        input: PauseInput<'_>,
        progress: &crate::core::progression::PlayerProgress,
        run: &mut RunState,
        bus: &mut EventBus,
        viewport: crate::ui::layout::ViewportCtx,
        _active_profile: usize,
    ) -> PauseUpdate {
        let PauseInput {
            actions,
            button_clicks,
            cursor_pos,
            scroll_lines,
            input_mode,
            mouse_left_down,
        } = input;
        let crate::ui::layout::ViewportCtx { window_w, window_h } = viewport;
        // If the options sub-overlay is open, all input goes to it. When it
        // signals close, drop back to the pause root rather than resuming
        // the underlying scene — the player explicitly hit Pause.
        if let Some(opts) = self.options_overlay.as_mut() {
            if opts.update_input(crate::scenes::options::OptionsInput {
                actions,
                button_clicks,
                cursor_pos,
                window_w,
                window_h,
                scroll_lines,
                input_mode,
                mouse_left_down,
            }) {
                self.options_overlay = None;
            }
            if let Some(opts) = self.options_overlay.as_mut() {
                if opts.take_focus_changed() {
                    bus.push(GameEvent::UiSound(SfxId::TilePlace));
                }
                if opts.take_confirm_requested() {
                    bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                }
                #[cfg(any(feature = "game", feature = "headless-screenshot"))]
                if opts.take_export_requested() {
                    if let Some(path) =
                        mahjuro_distribution::PlatformShell::resolve_play_stats_export_path(
                            _active_profile,
                        )
                    {
                    match crate::bot::export_play_history_html(&path, progress) {
                        Ok(()) => bus.push(GameEvent::InfoModal {
                            title: "Stats exported".into(),
                            body: format!("Saved HTML report to:\n{}", path.display()),
                        }),
                        Err(e) => bus.push(GameEvent::InfoModal {
                            title: "Export failed".into(),
                            body: format!("{e:#}"),
                        }),
                    }
                    }
                }
                #[cfg(any(feature = "game", feature = "headless-screenshot"))]
                if opts.take_open_tileset_mods_requested()
                    && let Err(e) = crate::shell_open::open_tileset_mods_folder()
                {
                    bus.push(GameEvent::InfoModal {
                        title: "Could not open folder".into(),
                        body: e,
                    });
                }
                if opts.take_credits_requested() {
                    bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                    self.options_overlay = None;
                    self.credits_requested = true;
                    self.paused = false;
                    return PauseUpdate::Resume;
                }
            }
            if self.options_overlay.is_none() {
                bus.push(GameEvent::UiSound(SfxId::UiCancel));
            }
            return PauseUpdate::StayPaused;
        }

        // Esc/Cancel resume the underlying scene.
        for a in actions {
            if matches!(a, UiAction::Pause | UiAction::Cancel) {
                bus.push(GameEvent::UiSound(SfxId::Unpause));
                self.paused = false;
                return PauseUpdate::Resume;
            }
        }

        let tree = self.build_tree(window_w, window_h);
        let action = self.tree.update(
            &tree,
            TreeInput {
                actions,
                button_clicks,
                cursor_pos,
                window: (window_w, window_h),
                input_mode,
                scroll_lines: 0.0,
            },
        );
        if self.tree.take_focus_changed() {
            bus.push(GameEvent::UiSound(SfxId::TilePlace));
        }
        match action {
            Some(PauseAction::Resume) => {
                bus.push(GameEvent::UiSound(SfxId::Unpause));
                self.paused = false;
                PauseUpdate::Resume
            }
            Some(PauseAction::Restart) => {
                bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                self.do_restart(run, progress)
            }
            Some(PauseAction::Options) => {
                bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                self.options_overlay = Some(OptionsScene::new());
                PauseUpdate::StayPaused
            }
            Some(PauseAction::MainMenu) => {
                bus.push(GameEvent::UiSound(SfxId::UiCancel));
                PauseUpdate::Transition(Box::new(Some(SceneIntent::MainMenu)))
            }
            Some(PauseAction::Exit) => {
                bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                PauseUpdate::Quit
            }
            None => PauseUpdate::StayPaused,
        }
    }

    fn do_restart(
        &mut self,
        run: &mut RunState,
        progress: &crate::core::progression::PlayerProgress,
    ) -> PauseUpdate {
        let settings = crate::persistence::load_settings();
        GameEngine::reset_to_demo(run, progress, &settings);
        PauseUpdate::Transition(Box::new(Some(SceneIntent::ShopFromRun)))
    }

    /// Append pause-overlay draw elements to the given vectors.
    pub fn draw(
        &self,
        viewport: crate::ui::layout::ViewportCtx,
        scale: f32,
        scroll_fade_backdrop: [f32; 4],
        instances: &mut Vec<GpuInstance>,
        text_labels: &mut Vec<TextLabel>,
        buttons: &mut Vec<ButtonDef>,
    ) {
        let crate::ui::layout::ViewportCtx { window_w, window_h } = viewport;
        if !self.paused {
            return;
        }

        // Dim background — theme WALNUT_INK (deepest walnut base), not pure black.
        instances.push(GpuInstance {
            rect: [0.0, 0.0, window_w, window_h],
            color: color::alpha(color::WALNUT_INK, 0.78),
            user: 0,
        });

        // If the options sub-overlay is open, draw it instead of the pause
        // menu root. The dim quad above already provides the backdrop.
        if let Some(opts) = self.options_overlay.as_ref() {
            opts.draw_overlay(
                window_w,
                window_h,
                scroll_fade_backdrop,
                instances,
                text_labels,
                buttons,
            );
            return;
        }

        // Title — gold serif, centered just above the menu block.
        let btn_gap = (12.0 * scale).max(6.0);
        let count = 5.0;
        let title_h = typography::size(typography::H20, window_h);
        let title_gap = (24.0 * scale).max(10.0);
        let max_menu_h = window_h * 0.88 - title_h - title_gap;
        let btn_h = (44.0 * scale)
            .max(28.0)
            .min((max_menu_h - (count - 1.0) * btn_gap) / count);
        let total_menu_h = count * btn_h + (count - 1.0) * btn_gap;
        let block_h = title_h + title_gap + total_menu_h;
        let title_y = ((window_h - block_h) * 0.5).max(8.0);
        text_labels.push(TextLabel {
            rect: [0.0, title_y, window_w, title_h],
            text: "PAUSED".into(),
            color: color::CHAMPAGNE,
            ..Default::default()
        });

        // Menu via the widget tree.
        let tree = self.build_tree(window_w, window_h);
        let mut frame = TreeFrame {
            instances,
            labels: text_labels,
            buttons,
        };
        self.tree.draw(&tree, &mut frame);
    }

    /// Publish pause-menu focus rects for the debug overlay (when not in options).
    pub fn stash_focus_nav_debug(
        &self,
        ctx: &mut super::DrawCtx<'_>,
        window_w: f32,
        window_h: f32,
    ) {
        if !self.paused || self.options_overlay.is_some() {
            return;
        }
        let tree = self.build_tree(window_w, window_h);
        let label = |id: FocusId| {
            [
                PauseAction::Resume,
                PauseAction::Restart,
                PauseAction::Options,
                PauseAction::MainMenu,
                PauseAction::Exit,
            ]
            .into_iter()
            .find(|a| a.id() == id)
            .map(|a| match a {
                PauseAction::Resume => "Resume",
                PauseAction::Restart => "Restart",
                PauseAction::Options => "Options",
                PauseAction::MainMenu => "Main Menu",
                PauseAction::Exit => "Exit",
            })
            .unwrap_or("?")
            .into()
        };
        ctx.stash_focus_nav_debug(self.tree.focus_nav_debug_snapshot_tree(
            &tree,
            (window_w, window_h),
            label,
        ));
    }
}
