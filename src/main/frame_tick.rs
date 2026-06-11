use super::*;

use crate::scene_transition::{PendingSceneDestination, SceneTag, transition_spec_for_edge};
use crate::sdl_shell::SdlShell;
use crate::ui::input::RumbleLabOp;

#[path = "frame_phases/mod.rs"]
mod frame_phases;

impl App {
    /// After game over, meta profile level-up waits until the main menu is
    /// fully visible (fade-in complete, no blocking modals) before the stinger
    /// and unlock showcase overlay.
    fn try_surface_pending_post_game_over_level_up(&mut self) {
        if self.pending_post_game_over_level_up.is_none() {
            return;
        }
        if self.pending_scene.is_some()
            || self.transition_alpha < 1.0
            || self.modals.is_active()
            || !self.overlay_stack.is_empty()
            || !matches!(self.scene, Scene::MainMenu(_))
        {
            return;
        }
        let Some(modal) = self.pending_post_game_over_level_up.take() else {
            return;
        };
        self.audio.play_sfx(crate::audio::SfxId::LevelUp);
        self.overlay_stack
            .push(Scene::Showcase(scenes::ShowcaseScene::new(
                scenes::ShowcasePresenter::MetaLevelUp(scenes::MetaLevelUpPresenter::new(modal)),
            )));
    }

    /// Whether scoring-cascade / hold-to-sell rumble should fire this frame:
    /// the player is on the controller and hasn't disabled gameplay rumble in
    /// settings. (The setting was originally named after shop hold-to-sell
    /// but now gates every gameplay-driven rumble, including cascade pulses.)
    fn controller_rumble_active(&self) -> bool {
        self.input.as_ref().is_some_and(|input| {
            input.mode == crate::ui::input::InputMode::Controller
                && input.hold_to_sell_rumble_enabled
        })
    }

    /// Keep fade-out transitions on black until destination room GPU data is ready.
    fn pending_destination_scene_key(&self) -> Option<&'static str> {
        if let Some(next) = self.pending_scene.as_ref() {
            return crate::scenes::active_scene_key(next);
        }
        self.pending_scene_intent
            .as_ref()
            .and_then(|intent| intent.scene_key())
    }

    fn pending_scene_room_gpu_ready(&self) -> bool {
        let Some(scene_key) = self.pending_destination_scene_key() else {
            return true;
        };
        let Some(renderer) = self.renderer.as_ref() else {
            return true;
        };
        renderer.scene_room_gpu_ready(scene_key)
    }

    fn scene_replace_in_flight(&self) -> bool {
        self.pending_scene_intent.is_some() || self.pending_scene.is_some()
    }

    /// Scene fades normally pause while a modal is up; stairway → shop after
    /// decimation (burn handoff or Descend) should never wait on one.
    pub(super) fn scene_transition_unblocked(&self) -> bool {
        if !self.modals.is_active() {
            return true;
        }
        matches!(
            (
                SceneTag::from(&self.scene),
                self.pending_scene_intent.as_ref()
            ),
            (
                SceneTag::Stairway,
                Some(crate::scenes::SceneIntent::ShopFromRun)
            )
        )
    }

    pub(super) fn begin_scene_replace(
        &mut self,
        intent: crate::scenes::SceneIntent,
        from_tag: SceneTag,
        destination: PendingSceneDestination,
    ) {
        if self.pending_scene_intent.as_ref() == Some(&intent) {
            return;
        }
        if from_tag == SceneTag::Stairway && intent == crate::scenes::SceneIntent::ShopFromRun {
            while self.modals.dismiss() {}
        }
        if intent.grants_memorial_on_start() {
            self.run.grant_pending_memorial(&mut self.progress);
            self.mark_profile_dirty();
        }
        let spec = transition_spec_for_edge(from_tag, intent.scene_tag());
        self.transition_kind = spec.kind;
        self.transition_speed = spec.speed;
        self.transition_timer = 0.0;
        self.pending_scene = None;
        let prefetch_key = intent.scene_key();
        self.pending_scene_intent = Some(intent);
        if let Some(key) = prefetch_key {
            if let Some(renderer) = self.renderer.as_mut() {
                renderer.start_room_cpu_prefetch_for_scene_key(key);
            }
        }
        self.pending_scene_destination = destination;
        self.transition_alpha = 1.0;
    }

    fn resolve_pending_scene_intent_at_black(&mut self) {
        let Some(intent) = self.pending_scene_intent.take() else {
            return;
        };
        let next = intent.resolve(crate::scenes::SceneResolveCtx {
            run: &mut self.run,
            progress: &self.progress,
        });
        self.pending_scene = Some(next);
    }

    /// Fire a one-shot rumble pulse on connected SDL gamepads.
    fn fire_rumble_pulse(
        &mut self,
        shell: &mut SdlShell,
        now: Instant,
        weak: u16,
        strong: u16,
        duration_ms: u32,
        gain: f32,
    ) {
        if let Some(input) = self.input.as_mut() {
            input.play_scoring_rumble_pulse(shell, now, weak, strong, duration_ms, gain);
        }
    }

    /// Drain rumble-lab ops onto SDL gamepads ([`crate::ui::input::InputState`]'s queue).
    fn dispatch_rumble_lab_ops(
        &mut self,
        shell: &mut SdlShell,
        now: Instant,
        ops: Vec<RumbleLabOp>,
    ) {
        if let Some(input) = self.input.as_mut() {
            input.apply_rumble_lab_ops(shell, now, ops);
        }
    }

    /// Drive the shop sell-hold rumble on SDL gamepads.
    /// Off-path callers (other scenes) should not invoke this.
    fn sync_shop_sell_hold_rumble(
        &mut self,
        shell: &mut SdlShell,
        hold: bool,
        controller: bool,
        enabled: bool,
        progress: f32,
    ) {
        if let Some(input) = self.input.as_mut() {
            input.sync_shop_sell_hold_rumble(shell, hold, controller, enabled, progress);
        }
    }

    pub(super) fn frame_tick(&mut self, shell: &mut SdlShell) {
        frame_phases::run(self, shell);
    }
}
