//! Shared pause menu overlay used by gameplay, shop, and blind-selection scenes.

use crate::game::run::RunState;
use crate::render::theme::{ButtonVariant, color, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextLabel};
use crate::ui::input::UiAction;
use crate::ui::widget_tree::{
    self as wt, FocusId, Tree, TreeFrame, TreeInput, TreeState, noop_render_custom,
};

use super::options::OptionsScene;
use super::shop::ShopScene;
use super::start_screen::StartScreenScene;
use super::{ButtonDef, Scene, SceneTransition, UpdateCtx};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PauseAction {
    Resume,
    Restart,
    Glossary,
    Options,
    MainMenu,
    Exit,
}

impl PauseAction {
    fn id(self) -> FocusId {
        FocusId(0x1000_0000 + self as u32)
    }
}

/// Result of processing pause menu input for the current frame.
pub enum PauseUpdate {
    /// Stay paused, no scene transition.
    StayPaused,
    /// Resume the current scene (unpause).
    Resume,
    /// Transition to another scene.
    Transition(SceneTransition),
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
    /// One-shot flag set when the user picks "Glossary" from the menu. The
    /// owning scene drains it via `take_glossary_request()` after
    /// `handle()` returns and toggles its glossary overlay; the pause menu
    /// itself closes in the same step so the glossary can take over the
    /// screen unobstructed.
    glossary_requested: bool,
}

impl PauseMenu {
    pub fn new() -> Self {
        Self {
            paused: false,
            tree: TreeState::new(),
            options_overlay: None,
            glossary_requested: false,
        }
    }

    /// Drain the glossary-open request flag. Returns `true` exactly once
    /// after the user picks "Glossary" from the pause menu; subsequent
    /// calls return `false` until they re-open the menu and pick it again.
    pub fn take_glossary_request(&mut self) -> bool {
        let v = self.glossary_requested;
        self.glossary_requested = false;
        v
    }

    /// Open the pause menu.
    pub fn open(&mut self) {
        self.paused = true;
        self.tree = TreeState::new();
        self.options_overlay = None;
        self.glossary_requested = false;
    }

    /// True when the embedded options overlay is currently visible. Callers
    /// (e.g. main loop) use this to know when to sync `OptionsScene`-driven
    /// settings (audio, smoke, tile preset) back to the live app state, the
    /// same way they do for the standalone options scene.
    pub fn options_overlay(&self) -> Option<&OptionsScene> {
        self.options_overlay.as_ref()
    }

    fn build_tree(&self, window_w: f32, window_h: f32) -> Tree<PauseAction> {
        let scale = (window_w.min(window_h)) / 600.0;
        let btn_w = (220.0 * scale).min(window_w * 0.55);
        let btn_h = (44.0 * scale).max(28.0);
        let btn_gap = (12.0 * scale).max(6.0);
        let count = 6;
        let total_menu_h = count as f32 * btn_h + (count as f32 - 1.0) * btn_gap;
        let title_h = typography::size(typography::TITLE, window_h);
        let title_gap = (24.0 * scale).max(10.0);
        let block_h = title_h + title_gap + total_menu_h;
        let start_y = (window_h - block_h) * 0.5;
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
                PauseAction::Glossary.id(),
                "Glossary",
                PauseAction::Glossary,
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
                ctx.actions,
                ctx.button_clicks,
                ctx.run,
                ctx.cursor_pos,
                ctx.layout.window_w,
                ctx.layout.window_h,
            );
            return Some(match result {
                PauseUpdate::StayPaused | PauseUpdate::Resume => None,
                PauseUpdate::Transition(t) => t,
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
                return Some(None);
            }
        }
        None
    }

    /// Process actions while paused. Returns what the caller should do.
    /// `cursor_pos` and `(window_w, window_h)` enable mouse hover-to-focus.
    pub fn update(
        &mut self,
        actions: &[UiAction],
        button_clicks: &[u32],
        run: &mut RunState,
        cursor_pos: (f32, f32),
        window_w: f32,
        window_h: f32,
    ) -> PauseUpdate {
        // If the options sub-overlay is open, all input goes to it. When it
        // signals close, drop back to the pause root rather than resuming
        // the underlying scene — the player explicitly hit Pause.
        if let Some(opts) = self.options_overlay.as_mut() {
            if opts.update_input(actions, button_clicks, cursor_pos, window_w, window_h) {
                self.options_overlay = None;
            }
            return PauseUpdate::StayPaused;
        }

        // Esc/Cancel resume the underlying scene.
        for a in actions {
            if matches!(a, UiAction::Pause | UiAction::Cancel) {
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
            },
        );
        match action {
            Some(PauseAction::Resume) => {
                self.paused = false;
                PauseUpdate::Resume
            }
            Some(PauseAction::Restart) => self.do_restart(run),
            Some(PauseAction::Glossary) => {
                // Set the one-shot flag and close the pause menu so the
                // owning scene can take over with its glossary overlay on
                // the next frame. The scene drains the flag via
                // `take_glossary_request()` after `handle()` returns.
                self.glossary_requested = true;
                self.paused = false;
                PauseUpdate::Resume
            }
            Some(PauseAction::Options) => {
                self.options_overlay = Some(OptionsScene::new());
                PauseUpdate::StayPaused
            }
            Some(PauseAction::MainMenu) => {
                PauseUpdate::Transition(Some(Scene::StartScreen(StartScreenScene::new())))
            }
            Some(PauseAction::Exit) => PauseUpdate::Quit,
            None => PauseUpdate::StayPaused,
        }
    }

    fn do_restart(&mut self, run: &mut RunState) -> PauseUpdate {
        *run = RunState::new_demo();
        PauseUpdate::Transition(Some(Scene::Shop(ShopScene::new(
            run.run_number,
            &run.relics,
        ))))
    }

    /// Append pause-overlay draw elements to the given vectors.
    pub fn draw(
        &self,
        window_w: f32,
        window_h: f32,
        scale: f32,
        instances: &mut Vec<GpuInstance>,
        text_labels: &mut Vec<TextLabel>,
        buttons: &mut Vec<ButtonDef>,
    ) {
        if !self.paused {
            return;
        }

        // Dim background — Midnight Gold theme: cool deep indigo, not pure black.
        instances.push(GpuInstance {
            rect: [0.0, 0.0, window_w, window_h],
            color: color::alpha(color::OBSIDIAN, 0.78),
        });

        // If the options sub-overlay is open, draw it instead of the pause
        // menu root. The dim quad above already provides the backdrop.
        if let Some(opts) = self.options_overlay.as_ref() {
            opts.draw_overlay(window_w, window_h, instances, text_labels, buttons);
            return;
        }

        // Title — gold serif, centered just above the menu block.
        let btn_h = (44.0 * scale).max(28.0);
        let btn_gap = (12.0 * scale).max(6.0);
        let count = 5.0;
        let total_menu_h = count * btn_h + (count - 1.0) * btn_gap;
        let title_h = typography::size(typography::TITLE, window_h);
        let title_gap = (24.0 * scale).max(10.0);
        let block_h = title_h + title_gap + total_menu_h;
        let title_y = (window_h - block_h) * 0.5;
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
            window: (window_w, window_h),
        };
        self.tree.draw(&tree, &mut frame, &noop_render_custom);
    }
}
