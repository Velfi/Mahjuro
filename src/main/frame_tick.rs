use super::*;

use crate::render::scene_keys;
use crate::scene_transition::{
    DEFAULT_QUICK_SPEC, PendingSceneDestination, PostSceneTransitionCtx, SceneTag,
    apply_post_scene_transition_effects, sync_music_for_scene, transition_spec_for_edge,
};
use crate::sdl_shell::SdlShell;
use crate::ui::input::RumbleLabOp;

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
            (SceneTag::from(&self.scene), self.pending_scene_intent.as_ref()),
            (SceneTag::Stairway, Some(crate::scenes::SceneIntent::ShopFromRun))
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
        let now = Instant::now();
        self.last_frame_dt = now
            .saturating_duration_since(self.last_frame)
            .as_secs_f32()
            .max(0.0001);
        self.last_frame = now;
        // Pause the watchdog during scene fades (`transition_alpha < 1.0` /
        // `pending_scene` set): those frames legitimately stall on shader /
        // texture loads and would otherwise false-fire on first launch.
        let transitioning = self.scene_replace_in_flight() || self.transition_alpha < 1.0;
        self.perf_watchdog
            .tick(self.last_frame_dt * 1000.0, transitioning, now);
        self.anim.update(now);
        self.audio.tick(now);
        if self
            .debug
            .trailer_mode
            .as_ref()
            .is_some_and(|tm| tm.finished_at(now))
        {
            self.debug.trailer_mode = None;
            log::debug!("Trailer mode finished");
        }
        self.try_play_production_logo_stinger();

        let drawn = self.overlay_stack.last().unwrap_or(&self.scene);
        let brownout_room = crate::main_room_gltf_brownout::RoomGltfBrownout::scene_eligible(drawn);
        let brownout_freeze = self.debug.scene_look_debug_overlay.is_some()
            || self.debug.rain_debug_overlay.is_some()
            || self.scene.has_blocking_overlay()
            || self.overlay_stack.iter().any(|s| s.has_blocking_overlay());
        let room_ambience =
            self.room_gltf_brownout
                .tick(self.last_frame_dt, brownout_room, brownout_freeze);
        if room_ambience.brownout_started {
            self.audio.play_sfx(audio::SfxId::BrownoutFlicker);
        }
        if room_ambience.play_creak {
            self.audio.play_sfx(audio::SfxId::RoomCreak);
        }

        // Refresh opened gamepads before any rumble this frame. `tick_scoring_rumble_keepalive`
        // and bus handlers run before `gamepad_frame_tick`; without this, `shell.pads` can
        // still be empty on the first frames after connect or if ordering ever regresses.
        shell.prepare_gamepad_frame();

        // 1. Drain event bus — bus events can trigger scene transitions.
        if let Some(input) = self.input.as_mut() {
            input.tick_scoring_rumble_keepalive(shell, now);
        }
        // Drain into a Vec so the bus borrow ends before the loop body, which
        // calls back into `&mut self` (rumble helpers, scene transitions, …).
        let drained: Vec<GameEvent> = self.bus.drain().collect();
        for ev in drained {
            match ev {
                GameEvent::TileDrawn => {
                    self.audio.play_sfx(audio::SfxId::TilePlace);
                }
                GameEvent::TileDiscarded => {
                    self.audio.play_sfx(audio::SfxId::TileDiscard);
                }
                GameEvent::ScoreUpdated => {
                    self.audio.play_sfx(audio::SfxId::ScoreReveal);
                }
                GameEvent::ScoreStepRevealed { index } => {
                    // Climb eight semitones across the reveal so the
                    // cascade audibly rises, then wrap. Layer the
                    // existing ScoreStep "rollover" sound on top to
                    // keep the soft confirmation that's already
                    // wired into the game.
                    self.audio.play_score_tick(index);
                    self.audio.play_sfx(audio::SfxId::ScoreStep);
                    if self.controller_rumble_active() {
                        let (weak, strong, duration_ms, gain) =
                            InputState::cascade_step_rumble_params();
                        self.fire_rumble_pulse(shell, now, weak, strong, duration_ms, gain);
                    }
                }
                GameEvent::ScoreCascadeFinal { earned } => {
                    // Crescendo: brassy hit jingle layered over the
                    // existing confirmation sting so the closing
                    // beat lands with weight.
                    self.audio.play_sfx(audio::SfxId::ScoreFinal);
                    self.audio.play_sfx(audio::SfxId::ScoreCrescendo);
                    self.dist
                        .unlock_achievement(crate::steam::Achievement::FirstStructure);
                    if self.controller_rumble_active() {
                        let (weak, strong, duration_ms, gain) =
                            InputState::cascade_final_rumble_params(earned);
                        self.fire_rumble_pulse(shell, now, weak, strong, duration_ms, gain);
                    }
                }
                GameEvent::GoldChanged { .. } => {
                    self.audio.play_sfx(audio::SfxId::CoinDrop);
                }
                ev @ GameEvent::RoundComplete { .. } => {
                    // Hold the win sting + scene transition until the
                    // scoring cascade has finished playing out — the
                    // player should get to watch the winning hand pop.
                    self.deferred_round_end = Some(ev);
                }
                ev @ GameEvent::GameOver { .. } => {
                    // Same as RoundComplete: hold until the final
                    // cascade has finished animating.
                    self.deferred_round_end = Some(ev);
                }
                GameEvent::PackBought => {
                    self.audio.play_sfx(audio::SfxId::PackBuy);
                }
                GameEvent::PackOpened => {
                    self.audio.play_sfx(audio::SfxId::PackOpen);
                }
                GameEvent::PackTileRevealed => {
                    self.audio.play_sfx(audio::SfxId::PackTileReveal);
                }
                GameEvent::ZodiacReveal => {
                    self.audio.play_sfx(audio::SfxId::ZodiacReveal);
                }
                GameEvent::ZodiacLevelUp => {
                    self.audio.play_sfx(audio::SfxId::ZodiacLevelUp);
                }
                GameEvent::CandleFlare => {
                    self.audio.play_sfx(audio::SfxId::CandleFlareWhoosh);
                    self.audio.play_sfx(audio::SfxId::CandleFlareImpact);
                }
                GameEvent::StructureCommitted => {
                    self.audio.play_sfx(audio::SfxId::StructureCommit);
                }
                GameEvent::TilesDestroyed => {
                    self.audio.play_sfx(audio::SfxId::TilesDestroyed);
                }
                GameEvent::InvalidAction => {
                    self.audio.play_sfx(audio::SfxId::InvalidAction);
                }
                GameEvent::UiSound(id) => {
                    self.audio.play_sfx(id);
                }
                GameEvent::HoldWindupStart => {
                    self.audio.play_hold_windup();
                }
                GameEvent::HoldWindupStop => {
                    self.audio.stop_hold_windup();
                }
                GameEvent::PlayRelicStinger(rid) => {
                    self.audio.play_relic_trigger(rid);
                }
                GameEvent::RelicActivated(rid) => {
                    // Visual feedback (glow + wiggle) is handled by the
                    // active scene; audio is the per-relic stinger,
                    // falling back to a soft chime when the relic has
                    // no bespoke audio file.
                    self.audio.play_relic_trigger(rid);
                    *self.progress.relic_times_activated.entry(rid).or_insert(0) += 1;
                    self.mark_profile_dirty();
                }
                GameEvent::OrdealEncountered(bk) => {
                    self.audio.play_sfx(audio::SfxId::OrdealEncountered);
                    *self
                        .progress
                        .ordeal_times_encountered
                        .entry(bk)
                        .or_insert(0) += 1;
                    self.mark_profile_dirty();
                    // Full Roster — non-final ordeals only. Final-tier
                    // Dragon/House are excluded; beating either is covered
                    // by `FirstRunCompleted` / `HouseDefeated`.
                    let pool = crate::core::ordeal::regular_pool();
                    if pool
                        .iter()
                        .all(|kind| self.progress.ordeal_times_encountered.contains_key(kind))
                    {
                        self.dist
                            .unlock_achievement(crate::steam::Achievement::AllBossesSeen);
                    }
                }
                GameEvent::OrdealDefeated(bk) => {
                    if !self.run.onboarding_active() {
                        self.audio.play_sfx(audio::SfxId::OrdealDefeated);
                        *self.progress.ordeal_times_defeated.entry(bk).or_insert(0) += 1;
                        self.mark_profile_dirty();
                        self.dist
                            .unlock_achievement(crate::steam::Achievement::FirstOrdealDefeated);
                        if bk == crate::core::ordeal::OrdealKind::House {
                            self.dist
                                .unlock_achievement(crate::steam::Achievement::HouseDefeated);
                        }
                    }
                }
                GameEvent::TalismanPurchased(tk) => {
                    self.audio.play_sfx(audio::SfxId::TalismanPurchased);
                    *self
                        .progress
                        .talisman_times_purchased
                        .entry(tk)
                        .or_insert(0) += 1;
                    self.mark_profile_dirty();
                }
                GameEvent::TalismanUsed(tk) => {
                    self.audio.play_sfx(audio::SfxId::TalismanUsed);
                    *self.progress.talisman_times_used.entry(tk).or_insert(0) += 1;
                    self.mark_profile_dirty();
                }
                GameEvent::MemorialTalismanUsed(_) => {
                    self.audio.play_sfx(audio::SfxId::TalismanUsed);
                }
                GameEvent::YakuScored(yk) => {
                    *self.progress.yaku_times_scored.entry(yk).or_insert(0) += 1;
                    self.mark_profile_dirty();
                }
                GameEvent::AchievementUnlocked(ach) => {
                    self.dist.unlock_achievement(ach);
                }
                GameEvent::TransformationSuccessorDiscovered(rid) => {
                    let _ = self.progress.note_transformation_successor_discovered(rid);
                    self.mark_profile_dirty();
                }
                GameEvent::ArchiveItemSeen(mark) => {
                    if self.progress.mark_archive_seen(mark) {
                        self.mark_profile_dirty();
                    }
                }
                GameEvent::ArchiveSeedSeenIfNeeded => {
                    crate::core::archive_seen::archive_seen_migration_seed(&mut self.progress);
                    self.mark_profile_dirty();
                }
                GameEvent::InfoModal { title, body } => {
                    self.modals.push(Modal::new(title, body, ModalTheme::Info));
                }
                GameEvent::OpenControllerMappingHelp => {
                    self.modals.push(Modal::new(
                        "Controller mapping".to_string(),
                        "Mahjuro reads your gamepad through SDL3 using the standard PC \
                         layout (south = confirm by default). Use Options to swap \
                         South/East or West/North if your printed labels differ. For \
                         OS-wide or per-game remaps, use Windows / macOS / Linux \
                         settings, Steam's controller configuration, or your device's \
                         companion app."
                            .to_string(),
                        ModalTheme::Info,
                    ));
                }
                GameEvent::RoomGltfBrownout => {
                    self.room_gltf_brownout.trigger();
                    self.audio.play_sfx(audio::SfxId::BrownoutFlicker);
                }
            }
        }

        // 1a. Poll debug menu actions.
        #[cfg(debug_menu_enabled)]
        if let Some(ref debug_menu) = self.debug.menu {
            for action in debug_menu.poll() {
                self.handle_debug_action(action);
            }
        }

        // 2. Collect input actions.
        let mut actions = Vec::new();
        let mut button_clicks: Vec<u32> = Vec::new();
        button_clicks.append(&mut self.mouse_button_clicks);
        let mut hide_cursor = false;
        let showcase_orbit_overlay = self
            .overlay_stack
            .last()
            .is_some_and(|top| matches!(top, Scene::Showcase(s) if s.wants_orbit_input()));
        let gp_ctx = crate::ui::input::GamepadPollCtx {
            face_bindings: self.active_face_bindings(),
            item_inspect_overlay: showcase_orbit_overlay,
            shop_storeroom_orbit: self.shop_storeroom_face_active(),
        };
        actions.append(&mut self.mouse_actions);
        if self.mouse_right_clicked {
            self.mouse_right_clicked = false;
            if self.shop_storeroom_face_active() {
                actions.push(crate::ui::input::UiAction::NorthFacePress);
            }
        }
        if let Some(input) = self.input.as_mut() {
            input.item_inspect_orbit_stick = (0.0, 0.0);
            input.item_inspect_zoom_triggers = 0.0;
            if input.gamepad_frame_tick(shell, gp_ctx, &mut actions) {
                hide_cursor = true;
            }

            // Detect the falling edge — last controller present last frame,
            // none this frame — while the player was on a pad. Inject Pause
            // so gameplay's pause path opens naturally.
            let now_controller_present = shell
                .gamepad
                .gamepads()
                .map(|v| !v.is_empty())
                .unwrap_or(false);
            if self.prev_controller_present
                && !now_controller_present
                && input.mode == crate::ui::input::InputMode::Controller
            {
                log::info!("controller disconnected — auto-pausing");
                actions.push(crate::ui::input::UiAction::Pause);
                input.mode = crate::ui::input::InputMode::Cursor;
                shell.show_cursor(true);
            }
            self.prev_controller_present = now_controller_present;

            let size = self.last_drawable_px;
            let layout = self
                .layout_engine
                .solve(size.width as f32, size.height as f32);
            // Hit-test by raycasting from the camera through the
            // cursor against each tile's OBB (last-frame snapshot).
            // We feed `update_pointer_hover` synthetic slots so only
            // the picked tile contains the cursor — the rest are
            // collapsed off-screen so they can't compete.
            let hand_slot_count = self.run.hand().len().max(layout.hand_slot_count);
            let mut slots: Vec<(f32, f32, f32, f32)> =
                vec![(-9999.0, -9999.0, 0.0, 0.0); hand_slot_count];
            let picked = self
                .renderer
                .as_ref()
                .and_then(|r| r.pick_hand_tile(input.last_cursor.0, input.last_cursor.1));
            if let Some(idx) = picked {
                if idx >= slots.len() {
                    slots.resize(idx + 1, (-9999.0, -9999.0, 0.0, 0.0));
                }
                if let Some(s) = slots.get_mut(idx) {
                    *s = (
                        input.last_cursor.0 - 1.0,
                        input.last_cursor.1 - 1.0,
                        2.0,
                        2.0,
                    );
                }
            }
            input.update_pointer_hover(input.last_cursor, &slots);

            // 3. Update focus slot (App-level, shared across scenes).
            for a in &actions {
                match a {
                    UiAction::FocusNext | UiAction::FocusPrev => {
                        input.wrap_focus_slot(*a, self.run.hand().len());
                    }
                    _ => {}
                }
            }
        }

        if hide_cursor {
            shell.show_cursor(false);
        }

        // 3b. If the tuning overlay is open, intercept input.
        if let Some(ref mut overlay) = self.debug.tuning_overlay {
            let (ww, wh) = (
                self.last_drawable_px.width as f32,
                self.last_drawable_px.height as f32,
            );
            let mouse = self.input.as_ref().map(|i| {
                (
                    i.last_cursor.0,
                    i.last_cursor.1,
                    self.mouse_clicked,
                    self.mouse_left_down,
                )
            });
            match overlay.update(&actions, mouse, ww, wh) {
                TuningResult::Stay => {
                    // Apply live tuning changes.
                    self.cascade_tuning = overlay.tuning.clone();
                }
                TuningResult::Close => {
                    // Apply final tuning and close.
                    self.cascade_tuning = overlay.tuning.clone();
                    self.debug.tuning_overlay = None;
                    log::debug!("Closed cascade tuning overlay");
                }
                TuningResult::Export => {
                    let json = serde_json::to_string_pretty(&overlay.tuning)
                        .unwrap_or_else(|e| format!("{{\"error\": \"{e}\"}}"));
                    let path = "cascade_tuning.json";
                    match std::fs::write(path, &json) {
                        Ok(()) => log::debug!("Exported tuning to {path}"),
                        Err(e) => log::error!("Failed to export tuning: {e}"),
                    }
                }
            }
            self.mouse_clicked = false;
            actions.clear();
            button_clicks.clear();
        }

        // 3b'. If the SFX test overlay is open, intercept input.
        if let Some(mut overlay) = self.debug.sfx_test_overlay.take() {
            let mouse = self.input.as_ref().map(|i| {
                let (mx, my) = i.last_cursor;
                (mx, my, self.mouse_clicked, self.mouse_left_down)
            });
            let close = overlay.update(&actions, &mut self.audio, mouse);
            self.mouse_clicked = false;
            if !close {
                self.debug.sfx_test_overlay = Some(overlay);
            } else {
                log::debug!("Closed SFX test overlay");
            }
            actions.clear();
            button_clicks.clear();
        }

        // 3b'''. If the camera debug overlay is open, intercept input.
        if let Some(mut overlay) = self.debug.camera_debug_overlay.take() {
            let (ww, wh) = (
                self.last_drawable_px.width as f32,
                self.last_drawable_px.height as f32,
            );
            let mouse = self.input.as_ref().map(|i| {
                (
                    i.last_cursor.0,
                    i.last_cursor.1,
                    self.mouse_clicked,
                    self.mouse_left_down,
                )
            });
            let close = overlay.update(&actions, mouse, ww, wh);
            if !close {
                self.debug.camera_debug_overlay = Some(overlay);
            } else {
                log::debug!("Closed camera debug overlay");
            }
            actions.clear();
            button_clicks.clear();
        }

        // Pick-blind hallway hall FX overlay (sliders; drawn above shop env panel).
        if let Some(mut overlay) = self.debug.hallway_distortion_debug_overlay.take() {
            let (ww, wh) = (
                self.last_drawable_px.width as f32,
                self.last_drawable_px.height as f32,
            );
            let mouse = self.input.as_ref().map(|i| {
                (
                    i.last_cursor.0,
                    i.last_cursor.1,
                    self.mouse_clicked,
                    self.mouse_left_down,
                )
            });
            let close = overlay.update(&actions, mouse, ww, wh);
            self.mouse_clicked = false;
            if !close {
                self.debug.hallway_distortion_debug_overlay = Some(overlay);
            } else {
                log::debug!("Closed hallway vertex warp debug overlay");
            }
            actions.clear();
            button_clicks.clear();
        }

        // Scene look overlay (tonemap + room GLB sliders, per-scene save).
        if let Some(mut overlay) = self.debug.scene_look_debug_overlay.take() {
            use crate::debug_overlays::SceneLookDebugResult;
            use crate::game::scene_look_tuning::{
                SceneLookTuning, clear_scene_look, save_scene_look,
            };
            let (ww, wh) = (
                self.last_drawable_px.width as f32,
                self.last_drawable_px.height as f32,
            );
            let mouse = self.input.as_ref().map(|i| {
                (
                    i.last_cursor.0,
                    i.last_cursor.1,
                    self.mouse_clicked,
                    self.mouse_left_down,
                )
            });
            let scene_key_lookup = overlay.scene_key().map(str::to_string);
            let persist_key = overlay.scene_key_persist().to_string();
            let mut close = false;
            match overlay.update(&actions, mouse, ww, wh, &self.scene_look) {
                SceneLookDebugResult::Stay => {
                    self.scene_look
                        .set(scene_key_lookup.as_deref(), overlay.look);
                }
                SceneLookDebugResult::Reset => {
                    overlay.look = SceneLookTuning::default();
                    self.scene_look.clear(scene_key_lookup.as_deref());
                    match clear_scene_look(&persist_key) {
                        Ok(()) => log::debug!(
                            "Cleared SceneLookTuning override for scene '{persist_key}'"
                        ),
                        Err(e) => log::warn!(
                            "Failed to clear SceneLookTuning override for '{persist_key}': {e}"
                        ),
                    }
                }
                SceneLookDebugResult::Save => {
                    self.scene_look
                        .set(scene_key_lookup.as_deref(), overlay.look);
                    match save_scene_look(&persist_key, &overlay.look) {
                        Ok(()) => {
                            log::debug!("Saved SceneLookTuning override for scene '{persist_key}'")
                        }
                        Err(e) => log::warn!(
                            "Failed to save SceneLookTuning override for '{persist_key}': {e}"
                        ),
                    }
                }
                SceneLookDebugResult::Close => {
                    self.scene_look
                        .set(scene_key_lookup.as_deref(), overlay.look);
                    close = true;
                }
            }
            self.mouse_clicked = false;
            if !close {
                self.debug.scene_look_debug_overlay = Some(overlay);
            } else {
                log::debug!("Closed scene look debug overlay");
            }
            actions.clear();
            button_clicks.clear();
        }

        if let Some(mut overlay) = self.debug.rain_debug_overlay.take() {
            use crate::render::main_menu_effects_debug_overlay::MainMenuEffectsDebugResult;
            use crate::render::main_menu_effects_tuning::MainMenuEffectsTuning;
            let (ww, wh) = (
                self.last_drawable_px.width as f32,
                self.last_drawable_px.height as f32,
            );
            let mouse = self.input.as_ref().map(|i| {
                (
                    i.last_cursor.0,
                    i.last_cursor.1,
                    self.mouse_clicked,
                    self.mouse_left_down,
                )
            });
            let mut close = false;
            match overlay.update(&actions, mouse, ww, wh) {
                MainMenuEffectsDebugResult::Stay => {}
                MainMenuEffectsDebugResult::Reset => {
                    overlay.tuning = MainMenuEffectsTuning::shipping_default();
                    if let Err(e) = MainMenuEffectsTuning::clear_saved() {
                        log::warn!("Failed to clear MainMenuEffectsTuning override: {e}");
                    } else {
                        log::debug!("Cleared MainMenuEffectsTuning override");
                    }
                }
                MainMenuEffectsDebugResult::Save => {
                    if let Err(e) = overlay.tuning.save() {
                        log::warn!("Failed to save MainMenuEffectsTuning override: {e}");
                    } else {
                        log::debug!("Saved MainMenuEffectsTuning override");
                    }
                }
                MainMenuEffectsDebugResult::Close => close = true,
            }
            if let Some(renderer) = self.renderer.as_mut() {
                renderer.main_menu_effects = overlay.tuning;
                renderer.main_menu_pride_rainbow_debug = overlay.pride_rainbow_debug;
                renderer.main_menu_moon_phase_debug = overlay.moon_phase_debug;
            }
            self.debug.main_menu_pride_rainbow_debug = overlay.pride_rainbow_debug;
            self.debug.main_menu_moon_phase_debug = overlay.moon_phase_debug;
            self.mouse_clicked = false;
            if !close {
                self.debug.rain_debug_overlay = Some(overlay);
            } else {
                log::debug!("Closed main menu effects debug overlay");
            }
            actions.clear();
            button_clicks.clear();
        }

        if let Some(mut overlay) = self.debug.flame_debug_overlay.take() {
            use crate::render::flame_debug_overlay::FlameDebugResult;
            use crate::render::flame_tuning::FlameTuning;
            let (ww, wh) = (
                self.last_drawable_px.width as f32,
                self.last_drawable_px.height as f32,
            );
            let mouse = self.input.as_ref().map(|i| {
                (
                    i.last_cursor.0,
                    i.last_cursor.1,
                    self.mouse_clicked,
                    self.mouse_left_down,
                )
            });
            let mut close = false;
            match overlay.update(&actions, mouse, ww, wh) {
                FlameDebugResult::Stay => {}
                FlameDebugResult::Reset => {
                    overlay.tuning = FlameTuning::shipping_default();
                    if let Err(e) = FlameTuning::clear_saved() {
                        log::warn!("Failed to clear FlameTuning override: {e}");
                    } else {
                        log::debug!("Cleared FlameTuning override");
                    }
                }
                FlameDebugResult::Save => {
                    if let Err(e) = overlay.tuning.save() {
                        log::warn!("Failed to save FlameTuning override: {e}");
                    } else {
                        log::debug!("Saved FlameTuning override");
                    }
                }
                FlameDebugResult::Close => close = true,
            }
            if let Some(renderer) = self.renderer.as_mut() {
                renderer.flame_tuning = overlay.tuning;
            }
            self.mouse_clicked = false;
            if !close {
                self.debug.flame_debug_overlay = Some(overlay);
            } else {
                log::debug!("Closed flame debug overlay");
            }
            actions.clear();
            button_clicks.clear();
        }

        if let Some(mut overlay) = self.debug.victory_moon_debug_overlay.take() {
            use crate::render::victory_moon_debug_overlay::VictoryMoonDebugResult;
            use crate::render::victory_moon_tuning::VictoryMoonDebug;
            let (ww, wh) = (
                self.last_drawable_px.width as f32,
                self.last_drawable_px.height as f32,
            );
            let mouse = self.input.as_ref().map(|i| {
                (
                    i.last_cursor.0,
                    i.last_cursor.1,
                    self.mouse_clicked,
                    self.mouse_left_down,
                )
            });
            let mut close = false;
            match overlay.update(&actions, mouse, ww, wh) {
                VictoryMoonDebugResult::Stay => {}
                VictoryMoonDebugResult::Reset => {
                    overlay.debug = VictoryMoonDebug::shipping_default();
                    log::debug!("Reset victory moon debug to defaults");
                }
                VictoryMoonDebugResult::Close => close = true,
            }
            self.debug.victory_moon_debug = overlay.debug;
            self.debug.main_menu_moon_phase_debug = overlay.debug.moon_phase;
            if let Some(renderer) = self.renderer.as_mut() {
                renderer.main_menu_moon_phase_debug = overlay.debug.moon_phase;
            }
            self.mouse_clicked = false;
            if !close {
                self.debug.victory_moon_debug_overlay = Some(overlay);
            } else {
                log::debug!("Closed victory moon debug overlay");
            }
            actions.clear();
            button_clicks.clear();
        }

        // 3b''. If the debug visibility overlay is open, intercept
        // input. Mirror the toggle state back to App fields each
        // frame so the gameplay scene + retain filter pick up live
        // changes immediately.
        if let Some(mut overlay) = self.debug.visibility_overlay.take() {
            let (ww, wh) = (
                self.last_drawable_px.width as f32,
                self.last_drawable_px.height as f32,
            );
            let mouse = self.input.as_ref().map(|i| {
                (
                    i.last_cursor.0,
                    i.last_cursor.1,
                    self.mouse_clicked,
                    self.mouse_left_down,
                )
            });
            let result = overlay.update(&actions, mouse, ww, wh);
            self.mouse_clicked = false;
            self.debug.visibility = overlay.vis;
            if result == DebugVisResult::Stay {
                self.debug.visibility_overlay = Some(overlay);
            } else {
                log::debug!("Closed debug visibility overlay");
            }
            actions.clear();
            button_clicks.clear();
        }

        // 3c. If a modal is active, intercept input. Cancel / Pause press
        // does a "skim" gesture on paginated modals: tap = advance one page,
        // hold = auto-advance through remaining pages at a fast cadence.
        // The hold timer is driven inside `ModalQueue::update`; here we just
        // forward the press and release edges.
        if self.modals.is_active() {
            for a in &actions {
                match a {
                    UiAction::Confirm => {
                        self.modals.advance_page();
                        break;
                    }
                    UiAction::Cancel | UiAction::Pause => {
                        self.modals.cancel_pressed();
                        break;
                    }
                    UiAction::CancelRelease => {
                        self.modals.cancel_released();
                    }
                    UiAction::FocusNext => {
                        self.modals.navigate(1);
                        break;
                    }
                    UiAction::FocusPrev => {
                        self.modals.navigate(-1);
                        break;
                    }
                    _ => {}
                }
            }
            // Block all actions from reaching the scene.
            actions.clear();
            button_clicks.clear();
        } else {
            // Modal not active; make sure a leftover skim timer doesn't
            // tick into the next paginated modal that pops up.
            self.modals.cancel_released();
        }

        // Block scene input while a replace transition is fading or held at black.
        if self.scene_replace_in_flight() {
            actions.clear();
            button_clicks.clear();
        }

        // Splash: LMB dismisses the production logo (same as Confirm/Cancel).
        if matches!(self.scene, Scene::Splash(_))
            && self.mouse_clicked
            && !self.modals.is_active()
        {
            crate::render::wgpu_renderer::loading_screen::request_skip();
        }

        // Clear one-shot mouse click flag so it doesn't bleed into
        // the next frame if no overlay consumed it.
        self.mouse_clicked = false;

        // 4. Delegate actions to the active scene.
        let focus = self.input.as_ref().map(|i| i.focused_index()).unwrap_or(0);
        let win_size = self.last_drawable_px;
        let update_layout = self
            .layout_engine
            .solve(win_size.width as f32, win_size.height as f32);
        let mut quit_requested = false;
        let mut switch_profile_req: Option<usize> = None;
        let mut delete_profile_req: Option<usize> = None;
        let mut complete_onboarding = false;
        let cursor_pos = self
            .input
            .as_ref()
            .map(|i| i.last_cursor)
            .unwrap_or((0.0, 0.0));
        let continue_warmup = self.continue_room_warmup();
        if matches!(&self.scene, Scene::Splash(_)) {
            if let Some(r) = self.renderer.as_mut() {
                r.tick_splash_hub_boot();
                r.poll_pending_texture_uploads();
                let tileset = self.gfx.tileset_name.clone();
                r.ensure_active_showcase_decal_atlas(&tileset);
                r.poll_pending_texture_uploads();
            }
        }
        let active_tileset = self.gfx.tileset_name.clone();
        let loading_done = match &self.scene {
            // Splash stays up until showcase atlases and main_menu.glb are on the GPU.
            Scene::Splash(_) => self.renderer.as_ref().is_some_and(|r| {
                r.splash_hub_boot_ready(&active_tileset)
            }),
            _ => self.renderer.as_ref().is_none_or(|r| !r.is_loading()),
        };
        let tutorial_eligible =
            self.progress.runs_completed == 0 && !self.progress.tutorial_completed;
        let hub_loading = self.hub_menu_loading(
            loading_done,
            tutorial_eligible,
            self.progress.plastic_unlocked(),
        );
        self.hub_loading = hub_loading;
        // Compute every scene pick once per frame. The same four results
        // are consumed below for `update` and again later by `draw` (via
        // `App::frame_picks`). Without this caching, each gameplay frame
        // pays for two full walks of the per-class matrix lists for
        // shop/gameplay objects in particular.
        let scene_key = match &self.scene {
            Scene::Splash(_) => Some(scene_keys::MAIN_MENU),
            _ => crate::scenes::active_scene_key(&self.scene),
        };
        let pending_scene_key = self.pending_destination_scene_key();
        let pending_transition_at_black = self.scene_replace_in_flight()
            && self.transition_alpha <= 0.0
            && self.scene_transition_unblocked();
        if pending_scene_key.is_some_and(|k| {
            matches!(
                k,
                scene_keys::GAMEPLAY | scene_keys::VICTORY | scene_keys::DEFEAT | "tutorial"
            )
        }) {
            self.audio.prefetch_gameplay_music();
        }
        let tile_stress_lab_open = self
            .overlay_stack
            .last()
            .is_some_and(|s| matches!(s, Scene::TileStressLab(_)));
        let stairway_tile_pick = matches!(&self.scene, Scene::Stairway(s) if s.wants_hand_tile_pick());
        self.frame_picks = if let Some(r) = self.renderer.as_mut() {
            r.poll_room_prefetch_gpu_uploads(
                scene_key,
                self.last_frame_dt * 1000.0,
                continue_warmup,
                pending_scene_key,
                pending_transition_at_black,
            );
            r.ensure_rooms_for_scene_key(scene_key);
            FramePicks {
                hand: if matches!(scene_key, Some("gameplay") | Some("tutorial"))
                    || tile_stress_lab_open
                    || stairway_tile_pick
                {
                    r.pick_hand_tile(cursor_pos.0, cursor_pos.1)
                } else {
                    None
                },
                shop: if matches!(
                    scene_key,
                    Some(scene_keys::SHOP) | Some("showcase") | Some(scene_keys::HALLWAY)
                ) {
                    r.pick_shop_object(cursor_pos.0, cursor_pos.1)
                } else {
                    None
                },
                gameplay: if matches!(scene_key, Some("gameplay") | Some("tutorial")) {
                    r.pick_gameplay_object(cursor_pos.0, cursor_pos.1)
                } else {
                    None
                },
            }
        } else {
            FramePicks::default()
        };
        let picked_shop_object = self.frame_picks.shop;
        let cascade_lab_open = self
            .overlay_stack
            .last()
            .is_some_and(|s| matches!(s, Scene::CascadeLab(_)));
        let picked_gameplay_object = if cascade_lab_open {
            match self.frame_picks.gameplay {
                Some(crate::render::wgpu_renderer::GameplayPick::CashInButton) => {
                    Some(crate::render::wgpu_renderer::GameplayPick::CashInButton)
                }
                _ => None,
            }
        } else {
            self.frame_picks.gameplay
        };
        let picked_hand_tile_for_update = if cascade_lab_open {
            None
        } else {
            self.frame_picks.hand
        };
        let mut scroll_lines = std::mem::take(&mut self.scroll_delta);
        let active_scene = self.overlay_stack.last().unwrap_or(&self.scene);
        // Stick vertical scroll is opt-in by scene. Yaku Journal, Chronicle,
        // and Credits use the right stick; defeat/victory run summaries accept
        // both sticks. Guide Tanuki tips maps right-stick X into scroll_lines
        // for horizontal panning. Other scenes keep sticks free for gameplay / orbit.
        let stick_scroll_axis = {
            let input = self.input.as_ref();
            let right = input.map(|i| i.right_stick_scroll_axis).unwrap_or(0.0);
            match active_scene {
                Scene::YakuJournal(_) | Scene::Credits(_) => right,
                Scene::Archive(scene) if scene.is_chronicle_tab() => right,
                Scene::Defeat(_) | Scene::Victory(_) => {
                    let left = input.map(|i| i.left_stick_scroll_axis).unwrap_or(0.0);
                    right + left
                }
                _ => 0.0,
            }
        };
        if stick_scroll_axis.abs() > 0.0 {
            const STICK_SCROLL_LINES_PER_SEC: f32 = 24.0;
            scroll_lines += stick_scroll_axis * self.last_frame_dt * STICK_SCROLL_LINES_PER_SEC;
        }
        if matches!(active_scene, Scene::Guide(g) if g.is_tanuki_tips_page()) {
            let right_x = self
                .input
                .as_ref()
                .map(|i| i.right_stick_scroll_axis_x)
                .unwrap_or(0.0);
            if right_x.abs() > 0.0 {
                const STICK_SCROLL_LINES_PER_SEC: f32 = 24.0;
                scroll_lines += right_x * self.last_frame_dt * STICK_SCROLL_LINES_PER_SEC;
            }
        }
        let mut overlay_request: Option<scenes::OverlayRequest> = None;
        let mut rumble_lab_ops: Vec<crate::ui::input::RumbleLabOp> = Vec::new();
        let mut bump_archive_chronicle_seen: Option<u32> = None;
        let mut seed_archive_seen = false;
        let p = self.active_profile.min(2);
        let settings_for_archive = persistence::load_settings();
        let archive_chronicle_last_seen = settings_for_archive.archive_last_seen_run_len[p];
        let room_gltf_height_for_update = self.resolved_scene_look().room_gltf_height_scale;
        let updated_overlay = !self.overlay_stack.is_empty();
        let shop_storeroom_orbit_drag_px = self
            .input
            .as_mut()
            .map(|i| i.take_shop_storeroom_mouse_orbit_px())
            .unwrap_or((0.0, 0.0));
        self.cpu_profiler
            .begin(crate::render::cpu_profiler::CpuStage::Update);
        if let Some(scenes::Scene::CascadeLab(lab)) = self.overlay_stack.last_mut() {
            self.cascade_tuning = lab.tuning.clone();
        }
        let scene_transitioning = self.scene_replace_in_flight();
        let update_result = if self.overlay_stack.is_empty() {
            self.scene.update(UpdateCtx {
                actions: &actions,
                button_clicks: &button_clicks,
                progress: &self.progress,
                active_profile: self.active_profile,
                run: &mut self.run,
                bus: &mut self.bus,
                anim: &mut self.anim,
                layout: &update_layout,
                focus_tile_index: focus,
                quit_requested: &mut quit_requested,
                switch_profile: &mut switch_profile_req,
                delete_profile: &mut delete_profile_req,
                complete_onboarding: &mut complete_onboarding,
                cursor_pos,
                mouse_left_down: self.mouse_left_down,
                loading_done,
                hub_loading,
                cascade_tuning: &self.cascade_tuning,
                picked_shop_object,
                picked_gameplay_object,
                input_mode: self
                    .input
                    .as_ref()
                    .map(|i| i.mode)
                    .unwrap_or(crate::ui::input::InputMode::Cursor),
                picked_hand_tile: picked_hand_tile_for_update,
                scroll_lines,
                tutorial_eligible: self.progress.runs_completed == 0
                    && !self.progress.tutorial_completed,
                multiple_materials: self.progress.plastic_unlocked(),
                resume_scene: self.resume_scene,
                transitioning: scene_transitioning,
                overlay_request: &mut overlay_request,
                headless: false,
                effect_layers: self.effect_layers,
                item_inspect_orbit_stick: self
                    .input
                    .as_ref()
                    .map(|i| i.item_inspect_orbit_stick)
                    .unwrap_or((0.0, 0.0)),
                item_inspect_zoom_triggers: self
                    .input
                    .as_ref()
                    .map(|i| i.item_inspect_zoom_triggers)
                    .unwrap_or(0.0),
                shop_storeroom_orbit_drag_px,
                shop_storeroom_orbit_stick: self
                    .input
                    .as_ref()
                    .map(|i| i.shop_storeroom_orbit_stick)
                    .unwrap_or((0.0, 0.0)),
                rumble_lab_ops: &mut rumble_lab_ops,
                suspended_shop: None,
                suspended_collection: None,
                room_gltf_height_scale: room_gltf_height_for_update,
                bump_archive_chronicle_seen: &mut bump_archive_chronicle_seen,
                seed_archive_seen: &mut seed_archive_seen,
                archive_chronicle_last_seen,
                main_menu_effects: self
                    .renderer
                    .as_ref()
                    .map(|r| r.main_menu_effects)
                    .unwrap_or_else(
                        crate::render::main_menu_effects_tuning::MainMenuEffectsTuning::load,
                    ),
                flame_tuning: self
                    .renderer
                    .as_ref()
                    .map(|r| r.flame_tuning)
                    .unwrap_or_else(crate::render::flame_tuning::FlameTuning::load),
                audio: Some(&mut self.audio),
            })
        } else {
            let showcase_shop_inspect = self.overlay_stack.last().is_some_and(|top| {
                matches!(
                    top,
                    Scene::Showcase(s)
                        if matches!(s.presenter, scenes::ShowcasePresenter::ShopInspect(_))
                )
            });
            let showcase_archive_inspect = self.overlay_stack.last().is_some_and(|top| {
                matches!(
                    top,
                    Scene::Showcase(s)
                        if matches!(s.presenter, scenes::ShowcasePresenter::ArchiveInspect(_))
                )
            });
            let (suspended_shop, suspended_collection) = match &mut self.scene {
                Scene::Shop(shop) if showcase_shop_inspect => {
                    shop.tick_suspended_animation_clock();
                    (Some(shop), None)
                }
                Scene::Archive(collection) if showcase_archive_inspect => (None, Some(collection)),
                _ => (None, None),
            };
            self.overlay_stack
                .last_mut()
                .expect("overlay stack non-empty")
                .update(UpdateCtx {
                    actions: &actions,
                    button_clicks: &button_clicks,
                    progress: &self.progress,
                    active_profile: self.active_profile,
                    run: &mut self.run,
                    bus: &mut self.bus,
                    anim: &mut self.anim,
                    layout: &update_layout,
                    focus_tile_index: focus,
                    quit_requested: &mut quit_requested,
                    switch_profile: &mut switch_profile_req,
                    delete_profile: &mut delete_profile_req,
                    complete_onboarding: &mut complete_onboarding,
                    cursor_pos,
                    mouse_left_down: self.mouse_left_down,
                    loading_done,
                    hub_loading,
                    cascade_tuning: &self.cascade_tuning,
                    picked_shop_object,
                    picked_gameplay_object,
                    input_mode: self
                        .input
                        .as_ref()
                        .map(|i| i.mode)
                        .unwrap_or(crate::ui::input::InputMode::Cursor),
                    picked_hand_tile: picked_hand_tile_for_update,
                    scroll_lines,
                    tutorial_eligible: self.progress.runs_completed == 0
                        && !self.progress.tutorial_completed,
                    multiple_materials: self.progress.plastic_unlocked(),
                    resume_scene: self.resume_scene,
                    transitioning: scene_transitioning,
                    overlay_request: &mut overlay_request,
                    headless: false,
                    effect_layers: self.effect_layers,
                    item_inspect_orbit_stick: self
                        .input
                        .as_ref()
                        .map(|i| i.item_inspect_orbit_stick)
                        .unwrap_or((0.0, 0.0)),
                    item_inspect_zoom_triggers: self
                        .input
                        .as_ref()
                        .map(|i| i.item_inspect_zoom_triggers)
                        .unwrap_or(0.0),
                    shop_storeroom_orbit_drag_px,
                    shop_storeroom_orbit_stick: (0.0, 0.0),
                    rumble_lab_ops: &mut rumble_lab_ops,
                    suspended_shop,
                    suspended_collection,
                    room_gltf_height_scale: room_gltf_height_for_update,
                    bump_archive_chronicle_seen: &mut bump_archive_chronicle_seen,
                    seed_archive_seen: &mut seed_archive_seen,
                    archive_chronicle_last_seen,
                    main_menu_effects: self
                        .renderer
                        .as_ref()
                        .map(|r| r.main_menu_effects)
                        .unwrap_or_else(
                            crate::render::main_menu_effects_tuning::MainMenuEffectsTuning::load,
                        ),
                    flame_tuning: self
                        .renderer
                        .as_ref()
                        .map(|r| r.flame_tuning)
                        .unwrap_or_else(crate::render::flame_tuning::FlameTuning::load),
                    audio: Some(&mut self.audio),
                })
        };
        if matches!(&self.scene, crate::scenes::Scene::MainMenu(_)) {
            self.effect_layers.rain = true;
            self.effect_layers.starfield = true;
        }
        self.cpu_profiler
            .end(crate::render::cpu_profiler::CpuStage::Update);
        if self.shop_storeroom_dwell_active() {
            let milestones = self
                .progress
                .accumulate_shop_storeroom_seconds(self.last_frame_dt);
            if milestones > 0 {
                self.mark_profile_dirty();
                if let Scene::Shop(shop) = &mut self.scene {
                    for _ in 0..milestones {
                        shop.play_eyeball_travel_milestone();
                    }
                }
            }
        }
        if seed_archive_seen {
            crate::core::archive_seen::archive_seen_migration_seed(&mut self.progress);
            self.mark_profile_dirty();
        }
        if let Some(n) = bump_archive_chronicle_seen {
            let p = self.active_profile.min(2);
            let mut s = persistence::load_settings();
            if s.archive_last_seen_run_len[p] != n {
                s.archive_last_seen_run_len[p] = n;
                self.archive_last_seen_run_len[p] = n;
                let _ = persistence::save_settings(&s);
            }
        }
        // Apply overlay push/pop before a SceneTransition (Replace).
        // Push/Pop operate on the overlay stack; they never fade.
        match overlay_request {
            Some(scenes::OverlayRequest::Push(s)) => {
                self.overlay_stack.push(*s);
                if matches!(self.overlay_stack.last(), Some(Scene::Credits(_))) {
                    self.audio.set_music_track(crate::audio::MusicId::Credits);
                }
            }
            Some(scenes::OverlayRequest::Pop) => {
                let was_credits = self
                    .overlay_stack
                    .last()
                    .is_some_and(|s| matches!(s, Scene::Credits(_)));
                let _ = self.overlay_stack.pop();
                if was_credits {
                    let tag = SceneTag::from(&self.scene);
                    let gameplay_ordeal_chamber = tag == SceneTag::Gameplay
                        && matches!(
                            &self.scene,
                            Scene::Gameplay(g) if g.music_chamber_kind(self.run.chamber)
                                == crate::core::rules::ChamberKind::Ordeal
                        );
                    sync_music_for_scene(&mut self.audio, tag, gameplay_ordeal_chamber, None);
                }
            }
            None => {}
        }
        self.dispatch_rumble_lab_ops(shell, now, rumble_lab_ops);
        let shop_ready = matches!(&self.scene, Scene::Shop(_))
            && self.overlay_stack.is_empty()
            && !self.scene.has_blocking_overlay();
        // Only drive shop-hold rumble on the unobstructed shop face. When `hold`
        // is false, sync stops motors — if we ran that every frame globally it
        // would cancel rumble lab / scoring pulses the same tick they fire.
        if shop_ready && let Some(input) = self.input.as_ref() {
            let hold = matches!(
                &self.scene,
                Scene::Shop(s) if s.sell_hold_in_progress() || s.buy_hold_in_progress()
            );
            let progress = match &self.scene {
                Scene::Shop(s) if hold => {
                    let shop = crate::game::engine::GameEngine::read_shop(&self.run);
                    s.sell_hold_progress(now, &shop)
                        .or_else(|| s.buy_hold_progress(now, &self.run, &shop))
                        .unwrap_or(0.0)
                }
                _ => 0.0,
            };
            let controller = input.mode == crate::ui::input::InputMode::Controller;
            let enabled = input.hold_to_sell_rumble_enabled;
            self.sync_shop_sell_hold_rumble(shell, hold, controller, enabled, progress);
        }
        // Gameplay hold-to-cash-in rumble. Only driven while actively charging:
        // we never call sync with `hold = false` here, so the motors expire on
        // their own and we don't clobber scoring-cascade pulses the frame the
        // cash-in completes.
        let gameplay_cash_in_hold = matches!(&self.scene, Scene::Gameplay(g) if g.cash_in_hold_in_progress())
            && self.overlay_stack.is_empty()
            && !self.scene.has_blocking_overlay();
        if gameplay_cash_in_hold && let Some(input) = self.input.as_ref() {
            let progress = match &self.scene {
                Scene::Gameplay(g) => {
                    let trigger_enabled =
                        crate::game::engine::GameEngine::read(&self.run).trigger_enabled;
                    g.cash_in_hold_progress(now, trigger_enabled)
                        .unwrap_or(0.0)
                }
                _ => 0.0,
            };
            let controller = input.mode == crate::ui::input::InputMode::Controller;
            let enabled = input.hold_to_sell_rumble_enabled;
            self.sync_shop_sell_hold_rumble(shell, true, controller, enabled, progress);
        }
        if let Some(intent) = update_result {
            self.begin_scene_replace(
                intent,
                SceneTag::from(&self.scene),
                if updated_overlay {
                    PendingSceneDestination::OverlayTop
                } else {
                    PendingSceneDestination::Base
                },
            );
        }

        if complete_onboarding {
            self.progress.tutorial_completed = true;
            self.mark_profile_dirty();
            self.dist
                .unlock_achievement(crate::steam::Achievement::TutorialComplete);
        }

        // Sync live audio/graphics settings whenever the player has
        // an options menu open — either the standalone Options scene
        // (from the start screen) or the embedded options overlay
        // inside the in-game pause menu.
        let active_options_overlay = match &self.scene {
            // Standalone Options scene IS the options screen, so its
            // own state is what we sync. Every other scene defers to
            // its `SceneBehavior::pause_options_overlay()` (default
            // `None` for scenes without an embedded pause menu).
            Scene::Options(opts) => Some(opts),
            other => other.pause_options_overlay(),
        };
        if let Some(opts) = active_options_overlay {
            self.audio.set_master_volume(opts.master_volume);
            self.audio.set_sfx_volume(opts.sfx_volume);
            self.audio.set_music_volume(opts.music_volume);
            self.audio.set_enabled(opts.sfx_enabled);
            self.gfx.effects_quality = opts.effects_quality;
            self.gfx.tile_preset = opts.tile_preset;
            self.gfx.tileset_name = opts.tileset_name.clone();
            self.gfx.gamma = opts.gamma;
            self.gfx.graphics_mode = opts.graphics_mode;
            self.gfx.hdr_enabled = opts.hdr_enabled;
            if opts.take_borderless_fullscreen_apply_armed()
                && opts.borderless_fullscreen != shell.desktop_fullscreen_on()
            {
                let _ = shell.set_desktop_fullscreen(opts.borderless_fullscreen);
            }
            self.run
                .set_auto_cash_in_on_full_structure(opts.auto_cash_in_on_full_structure);
            if let Some(ref mut input) = self.input {
                input.swap_ab = opts.swap_ab;
                input.swap_xy = opts.swap_xy;
                input.xy_quick_action = opts.xy_quick_action;
                input.hold_to_sell_rumble_enabled = opts.hold_to_sell_rumble;
            }
        }

        // Handle profile switch request.
        if let Some(idx) = switch_profile_req {
            let new_idx = if idx == usize::MAX {
                // Previous profile (wrapping), from start screen arrows.
                (self.active_profile + 3 - 1) % 3
            } else if idx == usize::MAX - 1 {
                // Next profile (wrapping), from start screen arrows.
                (self.active_profile + 1) % 3
            } else {
                // Absolute index, from profile select scene.
                idx.min(2)
            };
            if new_idx != self.active_profile {
                self.switch_profile(new_idx);
            }
        }

        // Handle profile delete request.
        if let Some(idx) = delete_profile_req {
            let idx = idx.min(2);
            persistence::delete_profile(idx);
            // If we just deleted the active profile, reload it (now
            // returns a fresh default since the file is gone).
            if idx == self.active_profile {
                self.progress = persistence::load_profile(idx);
                let loaded_run = persistence::load_run(idx);
                self.resume_scene = loaded_run
                    .as_ref()
                    .map(|saved| saved.scene)
                    .unwrap_or(persistence::ResumeScene::Gameplay);
                self.run = loaded_run
                    .map(|saved| saved.run)
                    .unwrap_or_else(crate::game::run::RunState::new_demo);
                self.run.apply_progression(&self.progress);
                self.dist.sync_profile_stats(&self.progress);
            }
        }

        // If we deferred a round-end event so the player could watch the
        // scoring cascade, reel, and popups finish, fire it once that
        // presentation is done (not every unrelated gameplay tween).
        if self.deferred_round_end.is_some() {
            let cascade_lab = self
                .overlay_stack
                .last()
                .is_some_and(|s| matches!(s, Scene::CascadeLab(_)));
            if self.run.suppress_chamber_resolution || cascade_lab {
                self.deferred_round_end = None;
            } else {
                let scoring_done = match &self.scene {
                    Scene::Gameplay(g) => g.ready_for_round_end(now),
                    _ => true,
                };
                if scoring_done && let Some(ev) = self.deferred_round_end.take() {
                    self.handle_round_end_event(ev);
                }
            }
        }

        // Advance transition animation using the animation controller.
        // Pause the transition while a modal is active so the player
        // must dismiss milestone / celebration modals before the scene
        // change proceeds (e.g. "First Pair!" before the recap screen).
        // Stairway → shop after decimation is exempt — burn already finished.
        if self.scene_replace_in_flight() && self.scene_transition_unblocked() {
            self.transition_alpha -= self.transition_speed;
            // Map alpha 1→0 onto timer 0→0.5 (first half of transition).
            self.transition_timer = (1.0 - self.transition_alpha.max(0.0)).clamp(0.0, 1.0) * 0.5;
            if self.transition_alpha <= 0.0 {
                self.transition_alpha = 0.0;
                if self.pending_scene.is_none() {
                    self.resolve_pending_scene_intent_at_black();
                }
                if !self.pending_scene_room_gpu_ready() {
                    // Hold at full black until pending scene room uploads complete.
                    self.transition_timer = 0.5;
                } else if let Some(next) = self.pending_scene.take() {
                    if matches!(&next, Scene::Shop(_)) {
                        self.run.grant_pending_memorial(&mut self.progress);
                        self.mark_profile_dirty();
                    }
                    let from_tag = SceneTag::from(&self.scene);
                    let to_tag = SceneTag::from(&next);
                    let gameplay_ordeal_chamber = to_tag == SceneTag::Gameplay
                        && matches!(
                            &next,
                            Scene::Gameplay(g) if g.music_chamber_kind(self.run.chamber)
                                == crate::core::rules::ChamberKind::Ordeal
                        );
                    // Route the new scene to the target recorded
                    // when the transition started, not whatever is
                    // on top now — overlays may have been pushed
                    // mid-fade (e.g. a zodiac celebration after a
                    // skip) and must not clobber them.
                    match self.pending_scene_destination {
                        PendingSceneDestination::OverlayTop => {
                            if let Some(top) = self.overlay_stack.last_mut() {
                                *top = next;
                            } else {
                                self.scene = next;
                            }
                        }
                        PendingSceneDestination::Base => {
                            self.scene = next;
                        }
                    }
                    self.pending_scene_destination = PendingSceneDestination::default();
                    if let Some(scene) = Self::saved_resume_scene_for(&self.scene) {
                        self.resume_scene = scene;
                    }
                    apply_post_scene_transition_effects(PostSceneTransitionCtx {
                        from: from_tag,
                        to: to_tag,
                        gameplay_ordeal_chamber,
                        anim: &mut self.anim,
                        renderer: self.renderer.as_mut(),
                        input: self.input.as_mut(),
                        audio: &mut self.audio,
                    });
                }
            }
        } else if self.transition_alpha < 1.0 {
            self.transition_alpha = (self.transition_alpha + self.transition_speed).min(1.0);
            // Map alpha 0→1 onto timer 0.5→1.0 (second half).
            self.transition_timer = 0.5 + (self.transition_alpha.clamp(0.0, 1.0)) * 0.5;
            // Reset transition kind once fully faded in.
            if self.transition_alpha >= 1.0 {
                self.transition_timer = 0.0;
                self.transition_kind = DEFAULT_QUICK_SPEC.kind;
                self.transition_speed = DEFAULT_QUICK_SPEC.speed;
            }
        }

        self.try_surface_pending_post_game_over_level_up();

        // Handle quit request from scene.
        if quit_requested {
            self.quit_requested = true;
        }

        self.draw(shell);

        // Hand any progress mutations from this frame's event handlers to
        // the background saver. Cheap (one clone). The cache is updated
        // synchronously inside `enqueue` so subsequent loads see fresh data.
        self.flush_dirty_profile();

        // Profile capture completion chime: each profiler latches a one-shot
        // flag the frame its report is logged. Both latches are polled here
        // so a CPU-only run, a GPU-only run, or back-to-back captures each
        // ring once.
        let cpu_done = self.cpu_profiler.take_just_completed();
        let gpu_done = self
            .renderer
            .as_mut()
            .is_some_and(|r| r.take_gpu_profile_just_completed());
        if cpu_done || gpu_done {
            self.audio.play_sfx(audio::SfxId::UiConfirm);
        }
    }
}
