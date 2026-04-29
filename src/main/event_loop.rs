use super::*;

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let t_resumed = Instant::now();

        let mut attrs = Window::default_attributes();
        attrs.title = "Mahjuro".to_string();
        if let Some(ref shot) = self.headless_screenshot {
            attrs.inner_size = Some(PhysicalSize::new(shot.width, shot.height).into());
            // Note: leaving the window visible during screenshot capture.
            // macOS appears to mark fully-hidden windows as Occluded which
            // makes wgpu's swapchain skip presenting frames, so the
            // capture path never runs. Visible window costs nothing for
            // a one-shot CLI run.
        } else {
            attrs.inner_size = Some(PhysicalSize::new(1920, 1080).into());
        }

        let t0 = Instant::now();
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        self.window = Some(window.clone());
        log::info!("window created in {:?}", t0.elapsed());

        let renderer = WgpuRenderer::new(render::wgpu_renderer::TargetInit::Windowed {
            window: window.clone(),
            hdr_enabled: self.gfx.hdr_enabled,
        })
        .expect("wgpu");
        self.renderer = Some(renderer);

        let t0 = Instant::now();
        self.input = Some(InputState::new().expect("input"));
        // Keep the debug menu wrapper alive for the full app lifetime.
        // Dropping it invalidates the installed native menubar on macOS.
        // Release builds ship without the debug menu — the `Option` stays
        // `None` so the poll site below is a no-op. Set `MAHJURO_DEBUG_MENU=1`
        // at build time to compile the menu into a release build (for
        // perf metrics).
        #[cfg(debug_menu_enabled)]
        {
            self.debug.menu = Some(DebugMenuBar::new(&window));
        }
        log::info!("input + debug menu init in {:?}", t0.elapsed());

        log::info!("App::resumed() total: {:?}", t_resumed.elapsed());
        window.request_redraw();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                if self.close_saved {
                    log::info!("CloseRequested received again — exiting immediately");
                    event_loop.exit();
                } else {
                    log::info!("CloseRequested — saving profile and exiting");
                    self.progress.record_score(self.run.round_score);
                    let _ = persistence::save_profile(self.active_profile, &self.progress);
                    self.persist_run_if_in_progress();
                    self.close_saved = true;
                    event_loop.exit();
                }
            }
            WindowEvent::Resized(size) => {
                if let Some(r) = self.renderer.as_mut() {
                    r.resize(size);
                }
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                // Advance animation clock once per presented frame. Doing this
                // at the top of `window_event` instead would tick animations on
                // every input event (CursorMoved fires faster than vsync), so
                // the game would effectively run faster than the monitor can
                // render. RedrawRequested is gated by the Fifo presenter, which
                // blocks at vsync, so this caps the tick to refresh rate.
                let now = Instant::now();
                self.last_frame_dt = now
                    .saturating_duration_since(self.last_frame)
                    .as_secs_f32()
                    .max(0.0001);
                self.last_frame = now;
                self.anim.update(now);
                self.audio.tick(now);

                // 1. Drain event bus — bus events can trigger scene transitions.
                // Track yaku stinger offsets so multiple yaku scored in the
                // same frame roll out as a staggered sequence rather than
                // stacking on the same tick.
                let mut yaku_stinger_index: u32 = 0;
                for ev in self.bus.drain() {
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
                        }
                        GameEvent::ScoreCascadeFinal => {
                            // Crescendo: brassy hit jingle layered over the
                            // existing confirmation sting so the closing
                            // beat lands with weight.
                            self.audio.play_sfx(audio::SfxId::ScoreFinal);
                            self.audio.play_sfx(audio::SfxId::ScoreCrescendo);
                            self.steam
                                .unlock_achievement(crate::steam::Achievement::FirstHand);
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
                        GameEvent::TutorialMilestone(milestone) => {
                            use crate::game::tutorial::TutorialMilestone;
                            let (title, body) = match milestone {
                                TutorialMilestone::FirstPair => (
                                    "First Pair!",
                                    "Two matching tiles \u{2014} the foundation of every hand.",
                                ),
                                TutorialMilestone::FirstTriplet => (
                                    "First Triplet!",
                                    "Three of a kind scores big chips. Keep building!",
                                ),
                                TutorialMilestone::FirstSequence => (
                                    "First Sequence!",
                                    "Three in a row, same suit. Smooth and versatile.",
                                ),
                                TutorialMilestone::FirstDiscard => {
                                    ("First Discard!", "Out with the old, in with the new.")
                                }
                                TutorialMilestone::FirstFullHand => (
                                    "First Full Hand!",
                                    "4 melds + 1 pair \u{2014} the ultimate yaku. Huge multiplier!",
                                ),
                                TutorialMilestone::FirstYakuhai => (
                                    "First Yakuhai!",
                                    "A wind or dragon triplet fired a yaku bonus \u{2014} honor tiles reward bigger mult than suit melds.",
                                ),
                                TutorialMilestone::FirstShopBuy => (
                                    "First Relic!",
                                    "Relics power up your scoring for the rest of the run.",
                                ),
                                TutorialMilestone::FirstBossCleared => (
                                    "Boss Defeated!",
                                    "You cleared your first boss blind. Antes rise from here \u{2014} each one introduces tougher bosses and bigger targets.",
                                ),
                            };
                            let win_size = self
                                .window
                                .as_ref()
                                .map(|w| w.inner_size())
                                .unwrap_or(PhysicalSize::new(800, 600));
                            let ww = win_size.width as f32;
                            let wh = win_size.height as f32;
                            let modal = crate::ui::modal::Modal::new(
                                title,
                                body,
                                crate::ui::modal::ModalTheme::Success,
                            )
                            .with_fireworks(
                                ww * 0.5,
                                wh * 0.8,
                                ww * 0.5,
                                3,
                            );
                            self.modals.push(modal);
                            self.audio.play_sfx(audio::SfxId::ScoreFinal);
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
                            let _ = persistence::save_profile(self.active_profile, &self.progress);
                        }
                        GameEvent::BossEncountered(bk) => {
                            self.audio.play_sfx(audio::SfxId::BossEncountered);
                            *self.progress.boss_times_encountered.entry(bk).or_insert(0) += 1;
                            let _ = persistence::save_profile(self.active_profile, &self.progress);
                            // "All bosses seen" — the regular (non-final)
                            // pool is the breadth-of-content signal we want.
                            // Beating Dragon is implied by `FirstRunCompleted`,
                            // so we don't gate this on it.
                            let pool = crate::core::boss::regular_pool();
                            if pool
                                .iter()
                                .all(|kind| self.progress.boss_times_encountered.contains_key(kind))
                            {
                                self.steam
                                    .unlock_achievement(crate::steam::Achievement::AllBossesSeen);
                            }
                        }
                        GameEvent::BossDefeated(bk) => {
                            self.audio.play_sfx(audio::SfxId::BossDefeated);
                            *self.progress.boss_times_defeated.entry(bk).or_insert(0) += 1;
                            let _ = persistence::save_profile(self.active_profile, &self.progress);
                            self.steam
                                .unlock_achievement(crate::steam::Achievement::FirstBossDefeated);
                        }
                        GameEvent::TalismanPurchased(tk) => {
                            self.audio.play_sfx(audio::SfxId::TalismanPurchased);
                            *self
                                .progress
                                .talisman_times_purchased
                                .entry(tk)
                                .or_insert(0) += 1;
                            let _ = persistence::save_profile(self.active_profile, &self.progress);
                        }
                        GameEvent::TalismanUsed(tk) => {
                            self.audio.play_sfx(audio::SfxId::TalismanUsed);
                            *self.progress.talisman_times_used.entry(tk).or_insert(0) += 1;
                            let _ = persistence::save_profile(self.active_profile, &self.progress);
                        }
                        GameEvent::YakuScored(yk) => {
                            *self.progress.yaku_times_scored.entry(yk).or_insert(0) += 1;
                            let _ = persistence::save_profile(self.active_profile, &self.progress);
                            const YAKU_STINGER_SPACING_MS: u64 = 200;
                            let offset = std::time::Duration::from_millis(
                                (yaku_stinger_index as u64) * YAKU_STINGER_SPACING_MS,
                            );
                            self.audio
                                .schedule_sfx(audio::SfxId::for_yaku(yk), now + offset);
                            yaku_stinger_index += 1;
                        }
                        GameEvent::AchievementUnlocked(ach) => {
                            self.steam.unlock_achievement(ach);
                        }
                    }
                }

                // 1a. Poll background update pipeline. Skipped on macOS when
                // Sparkle is driving updates — Sparkle owns the entire UX
                // (appcast polling, prompts, download, atomic bundle swap)
                // because Gatekeeper blocks any in-process self-replace inside
                // `/Applications/Mahjuro.app`. On dev `cargo run` builds the
                // framework isn't embedded, `self.sparkle` is `None`, and the
                // legacy in-game path takes over.
                #[cfg(target_os = "macos")]
                let skip_legacy_update_poll = self.sparkle.is_some();
                #[cfg(not(target_os = "macos"))]
                let skip_legacy_update_poll = false;
                if !skip_legacy_update_poll
                    && let Some(checker) = self.update_checker.as_mut()
                    && let Some(result) = checker.poll()
                {
                    let modal = match result {
                        update_check::UpdateResult::UpdateAvailable { new_version } => {
                            let current = env!("CARGO_PKG_VERSION");
                            let body = format!(
                                "v{new_version} is available (you have v{current}).\n\nDownload and install now?\n\nPress Enter to install, Esc to skip."
                            );
                            self.pending_update_prompt = Some(new_version);
                            Modal::new("Update Available", body, ModalTheme::Info)
                        }
                        update_check::UpdateResult::Updated { new_version } => Modal::new(
                            "Updated!",
                            format!("v{new_version} installed.\nRestart to use the new version."),
                            ModalTheme::Info,
                        ),
                        update_check::UpdateResult::UpdateFailed {
                            new_version,
                            release_url,
                            error,
                        } => {
                            log::warn!("auto-update to v{new_version} failed: {error}");
                            Modal::new(
                                "Update Failed",
                                format!(
                                    "v{new_version} download/install failed.\n\nGet it manually:\n{release_url}"
                                ),
                                ModalTheme::Info,
                            )
                        }
                    };
                    self.modals.push(modal);
                }

                // 1b. Poll debug menu actions.
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
                if let Some(input) = self.input.as_mut() {
                    if input.poll_gamepads(&mut actions) {
                        hide_cursor = true;
                    }
                    actions.append(&mut self.mouse_actions);

                    let size = self
                        .window
                        .as_ref()
                        .map(|w| w.inner_size())
                        .unwrap_or(PhysicalSize::new(800, 600));
                    let layout = self
                        .layout_engine
                        .solve(size.width as f32, size.height as f32);
                    // Hit-test by raycasting from the camera through the
                    // cursor against each tile's OBB (last-frame snapshot).
                    // We feed `update_pointer_hover` synthetic slots so only
                    // the picked tile contains the cursor — the rest are
                    // collapsed off-screen so they can't compete.
                    let hand_slot_count = self.run.hand.len().max(layout.hand_slots.len());
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
                                input.wrap_focus_slot(*a, self.run.hand.len());
                            }
                            _ => {}
                        }
                    }
                }

                if hide_cursor && let Some(w) = self.window.as_ref() {
                    w.set_cursor_visible(false);
                }

                // 3b. If the tuning overlay is open, intercept input.
                if let Some(ref mut overlay) = self.debug.tuning_overlay {
                    match overlay.update(&actions) {
                        TuningResult::Stay => {
                            // Apply live tuning changes.
                            self.cascade_tuning = overlay.tuning.clone();
                        }
                        TuningResult::Close => {
                            // Apply final tuning and close.
                            self.cascade_tuning = overlay.tuning.clone();
                            self.debug.tuning_overlay = None;
                            log::info!("[Debug] Closed cascade tuning overlay");
                        }
                        TuningResult::Export => {
                            let json = serde_json::to_string_pretty(&overlay.tuning)
                                .unwrap_or_else(|e| format!("{{\"error\": \"{e}\"}}"));
                            let path = "cascade_tuning.json";
                            match std::fs::write(path, &json) {
                                Ok(()) => log::info!("[Debug] Exported tuning to {path}"),
                                Err(e) => log::error!("[Debug] Failed to export tuning: {e}"),
                            }
                        }
                    }
                    actions.clear();
                    button_clicks.clear();
                }

                // 3b'. If the SFX test overlay is open, intercept input.
                if let Some(mut overlay) = self.debug.sfx_test_overlay.take() {
                    let mouse = self.input.as_ref().map(|i| {
                        let (mx, my) = i.last_cursor;
                        (mx, my, self.mouse_clicked)
                    });
                    let close = overlay.update(&actions, &mut self.audio, mouse);
                    self.mouse_clicked = false;
                    if !close {
                        self.debug.sfx_test_overlay = Some(overlay);
                    } else {
                        log::info!("[Debug] Closed SFX test overlay");
                    }
                    actions.clear();
                    button_clicks.clear();
                }

                // 3b'''. If the camera debug overlay is open, intercept input.
                if let Some(mut overlay) = self.debug.camera_debug_overlay.take() {
                    let wh = self
                        .window
                        .as_ref()
                        .map(|w| w.inner_size().height as f32)
                        .unwrap_or(800.0);
                    let close = overlay.update(&actions, wh);
                    if !close {
                        self.debug.camera_debug_overlay = Some(overlay);
                    } else {
                        log::info!("[Debug] Closed camera debug overlay");
                    }
                    actions.clear();
                    button_clicks.clear();
                }

                // 3b''. If the debug visibility overlay is open, intercept
                // input. Mirror the toggle state back to App fields each
                // frame so the gameplay scene + retain filter pick up live
                // changes immediately.
                if let Some(mut overlay) = self.debug.visibility_overlay.take() {
                    let result = overlay.update(&actions);
                    self.debug.hide_tiles = overlay.hide_tiles;
                    self.debug.hide_candles = overlay.hide_candles;
                    self.debug.hide_blind_plaque = overlay.hide_blind_plaque;
                    self.debug.hide_scoring_placard = overlay.hide_scoring_placard;
                    self.debug.hide_inventory = overlay.hide_inventory;
                    if result == DebugVisResult::Stay {
                        self.debug.visibility_overlay = Some(overlay);
                    } else {
                        log::info!("[Debug] Closed debug visibility overlay");
                    }
                    actions.clear();
                    button_clicks.clear();
                }

                // 3b''''. If the shop smoke debug overlay is open, intercept
                // input. Live-copy the overlay's tuning into the App each
                // frame so the shop scene's next draw picks up edits.
                if let Some(ref mut overlay) = self.debug.smoke_debug_overlay {
                    match overlay.update(&actions) {
                        SmokeDebugResult::Stay => {
                            self.shop_smoke_tuning = overlay.tuning.clone();
                        }
                        SmokeDebugResult::Reset => {
                            overlay.tuning = ShopSmokeTuning::default();
                            self.shop_smoke_tuning = overlay.tuning.clone();
                            // Clearing the override means the code default
                            // takes over on next launch. Logs go to stderr
                            // so tuning sessions leave a paper trail.
                            match persistence::clear_tuning_override("ShopSmokeTuning") {
                                Ok(()) => log::info!("[Debug] Cleared ShopSmokeTuning override"),
                                Err(e) => log::warn!(
                                    "[Debug] Failed to clear ShopSmokeTuning override: {e}"
                                ),
                            }
                        }
                        SmokeDebugResult::SaveAsDefault => {
                            self.shop_smoke_tuning = overlay.tuning.clone();
                            match persistence::save_tuning_override(
                                "ShopSmokeTuning",
                                &overlay.tuning,
                            ) {
                                Ok(()) => log::info!("[Debug] Saved ShopSmokeTuning override"),
                                Err(e) => log::warn!(
                                    "[Debug] Failed to save ShopSmokeTuning override: {e}"
                                ),
                            }
                        }
                        SmokeDebugResult::Close => {
                            self.shop_smoke_tuning = overlay.tuning.clone();
                            self.debug.smoke_debug_overlay = None;
                            log::info!("[Debug] Closed shop smoke debug overlay");
                        }
                    }
                    actions.clear();
                    button_clicks.clear();
                }

                // 3b'''''. Volumetric tuning overlay — same pattern as smoke.
                // Live-copy so `renderer.set_dust_strength` picks up edits
                // on the next frame.
                if let Some(ref mut overlay) = self.debug.volumetric_debug_overlay {
                    match overlay.update(&actions) {
                        VolumetricDebugResult::Stay => {
                            self.volumetric_tuning = overlay.tuning;
                        }
                        VolumetricDebugResult::Reset => {
                            overlay.tuning = VolumetricTuning::default();
                            self.volumetric_tuning = overlay.tuning;
                            match persistence::clear_tuning_override("VolumetricTuning") {
                                Ok(()) => log::info!("[Debug] Cleared VolumetricTuning override"),
                                Err(e) => log::warn!(
                                    "[Debug] Failed to clear VolumetricTuning override: {e}"
                                ),
                            }
                        }
                        VolumetricDebugResult::SaveAsDefault => {
                            self.volumetric_tuning = overlay.tuning;
                            match persistence::save_tuning_override(
                                "VolumetricTuning",
                                &overlay.tuning,
                            ) {
                                Ok(()) => log::info!("[Debug] Saved VolumetricTuning override"),
                                Err(e) => log::warn!(
                                    "[Debug] Failed to save VolumetricTuning override: {e}"
                                ),
                            }
                        }
                        VolumetricDebugResult::Close => {
                            self.volumetric_tuning = overlay.tuning;
                            self.debug.volumetric_debug_overlay = None;
                            log::info!("[Debug] Closed volumetric debug overlay");
                        }
                    }
                    actions.clear();
                    button_clicks.clear();
                }

                // 3c. If a modal is active, intercept input.
                if self.modals.is_active() {
                    for a in &actions {
                        match a {
                            UiAction::Confirm => {
                                if let Some(version) = self.pending_update_prompt.take()
                                    && let Some(checker) = self.update_checker.as_mut()
                                {
                                    checker.start_install(version);
                                }
                                self.modals.advance_page();
                                break;
                            }
                            UiAction::Cancel => {
                                if self.pending_update_prompt.take().is_some() {
                                    log::info!("user skipped update");
                                }
                                self.modals.dismiss();
                                break;
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
                }

                // Clear one-shot mouse click flag so it doesn't bleed into
                // the next frame if no overlay consumed it.
                self.mouse_clicked = false;

                // 4. Delegate actions to the active scene.
                let focus = self.input.as_ref().map(|i| i.focused_index()).unwrap_or(0);
                let win_size = self
                    .window
                    .as_ref()
                    .map(|w| w.inner_size())
                    .unwrap_or(PhysicalSize::new(800, 600));
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
                let loading_done = self.renderer.as_ref().is_none_or(|r| !r.is_loading());
                let picked_shop_object = self
                    .renderer
                    .as_ref()
                    .and_then(|r| r.pick_shop_object(cursor_pos.0, cursor_pos.1));
                let picked_gameplay_object = self
                    .renderer
                    .as_ref()
                    .and_then(|r| r.pick_gameplay_object(cursor_pos.0, cursor_pos.1));
                let picked_collection_object = self
                    .renderer
                    .as_ref()
                    .and_then(|r| r.pick_collection_object(cursor_pos.0, cursor_pos.1));
                let picked_hand_tile_for_update = self
                    .renderer
                    .as_ref()
                    .and_then(|r| r.pick_hand_tile(cursor_pos.0, cursor_pos.1));
                let scroll_lines = std::mem::take(&mut self.scroll_delta);
                let mut overlay_request: Option<scenes::OverlayRequest> = None;
                let ctx = UpdateCtx {
                    actions: &actions,
                    button_clicks: &button_clicks,
                    progress: &self.progress,
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
                    loading_done,
                    cascade_tuning: &self.cascade_tuning,
                    picked_shop_object,
                    picked_gameplay_object,
                    picked_collection_object,
                    input_mode: self
                        .input
                        .as_ref()
                        .map(|i| i.mode)
                        .unwrap_or(crate::ui::input::InputMode::Cursor),
                    picked_hand_tile: picked_hand_tile_for_update,
                    scroll_lines,
                    ui_scale: self.gfx.ui_scale,
                    tutorial_eligible: self.progress.runs_completed == 0
                        && !self.progress.tutorial_completed,
                    multiple_materials: self.progress.plastic_unlocked(),
                    resume_scene: self.resume_scene,
                    transitioning: self.pending_scene.is_some(),
                    overlay_request: &mut overlay_request,
                    headless: false,
                };
                let updated_overlay = !self.overlay_stack.is_empty();
                let update_result = if let Some(top) = self.overlay_stack.last_mut() {
                    top.update(ctx)
                } else {
                    self.scene.update(ctx)
                };
                // Apply overlay push/pop before a SceneTransition (Replace).
                // Push/Pop operate on the overlay stack; they never fade.
                match overlay_request {
                    Some(scenes::OverlayRequest::Push(s)) => {
                        self.overlay_stack.push(*s);
                    }
                    Some(scenes::OverlayRequest::Pop) => {
                        let _ = self.overlay_stack.pop();
                    }
                    None => {}
                }
                if let Some(next_scene) = update_result {
                    // Choose transition style: dramatic cascade for
                    // new-game flows, quick fade for everything else.
                    let use_cascade = matches!(
                        (&self.scene, &next_scene),
                        (Scene::StartScreen(_), Scene::TileSelect(_))
                            | (Scene::StartScreen(_), Scene::Shop(_))
                            | (Scene::TileSelect(_), Scene::Shop(_))
                            | (Scene::TileSelect(_), Scene::TutorialCampaign(_))
                    );
                    let use_tile_teeth = matches!(
                        (&self.scene, &next_scene),
                        (Scene::StartScreen(_), Scene::Collection(_))
                            | (Scene::Collection(_), Scene::StartScreen(_))
                    );
                    let use_galaxy = matches!(
                        (&self.scene, &next_scene),
                        (Scene::Collection(_), Scene::YakuJournal(_))
                            | (Scene::YakuJournal(_), Scene::Collection(_))
                    );
                    let use_maelstrom = matches!(
                        (&self.scene, &next_scene),
                        (Scene::StartScreen(_), Scene::Options(_))
                            | (Scene::Options(_), Scene::StartScreen(_))
                    );
                    let use_waterfall = matches!(
                        (&self.scene, &next_scene),
                        (Scene::StartScreen(_), Scene::TileLiteracy(_))
                            | (Scene::TileLiteracy(_), Scene::StartScreen(_))
                    );
                    let use_shuffling_fan = matches!(
                        (&self.scene, &next_scene),
                        (Scene::StartScreen(_), Scene::ProfileSelect(_))
                            | (Scene::ProfileSelect(_), Scene::StartScreen(_))
                    );
                    // Restart from the pause menu is the only path from
                    // Gameplay straight back to Shop; give it a deliberate
                    // fade-to-black instead of the snappy default.
                    let slow_fade = matches!(
                        (&self.scene, &next_scene),
                        (Scene::Gameplay(_), Scene::Shop(_))
                    );
                    if use_cascade {
                        self.transition_kind = TransitionKind::ShootingStarCascade;
                        self.transition_speed = 0.012;
                        self.audio.play_sfx(audio::SfxId::StarShimmer);
                    } else if use_tile_teeth {
                        self.transition_kind = TransitionKind::ForestOfTiles;
                        self.transition_speed = 0.035;
                    } else if use_galaxy {
                        self.transition_kind = TransitionKind::GalaxyOfTiles;
                        self.transition_speed = 0.032;
                    } else if use_maelstrom {
                        self.transition_kind = TransitionKind::Maelstrom;
                        self.transition_speed = 0.032;
                    } else if use_waterfall {
                        self.transition_kind = TransitionKind::TileWaterfall;
                        self.transition_speed = 0.034;
                    } else if use_shuffling_fan {
                        self.transition_kind = TransitionKind::ShufflingFan;
                        self.transition_speed = 0.035;
                    } else if slow_fade {
                        self.transition_kind = TransitionKind::Quick;
                        self.transition_speed = 0.025;
                    } else {
                        self.transition_kind = TransitionKind::Quick;
                        self.transition_speed = 0.08;
                    }
                    self.transition_timer = 0.0;
                    // Start fade-out transition.
                    self.pending_scene = Some(next_scene);
                    self.pending_scene_targets_overlay = updated_overlay;
                    self.transition_alpha = 1.0;
                }

                if complete_onboarding {
                    self.progress.tutorial_completed = true;
                    let _ = persistence::save_profile(self.active_profile, &self.progress);
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
                    self.gfx.smoke_quality = opts.smoke_quality;
                    self.gfx.smoke_amount = opts.smoke_amount;
                    self.gfx.effects_quality = opts.effects_quality;
                    self.gfx.tile_preset = opts.tile_preset;
                    self.gfx.tileset_name = opts.tileset_name.clone();
                    self.gfx.surface_kind = opts.surface_kind;
                    self.gfx.gamma = opts.gamma;
                    self.gfx.shadows_enabled = opts.shadows_enabled;
                    self.gfx.ssr_enabled = opts.ssr_enabled;
                    self.gfx.hdr_enabled = opts.hdr_enabled;
                    self.gfx.ui_scale = opts.ui_scale;
                    self.run
                        .set_auto_cash_in_on_full_structure(opts.auto_cash_in_on_full_structure);
                    self.run.set_hints_enabled(opts.hints_enabled);
                    if let Some(ref mut input) = self.input {
                        input.swap_ab = opts.swap_ab;
                        input.xy_quick_action = opts.xy_quick_action;
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
                    }
                }

                // If we deferred a round-end event so the player could watch
                // the scoring cascade play out, fire it now that the gameplay
                // scene has gone idle.
                if self.deferred_round_end.is_some() {
                    let cascade_done = match &self.scene {
                        Scene::Gameplay(g) => !g.is_animating(),
                        _ => true,
                    };
                    if cascade_done && let Some(ev) = self.deferred_round_end.take() {
                        self.handle_round_end_event(ev);
                    }
                }

                // Advance transition animation using the animation controller.
                // Pause the transition while a modal is active so the player
                // must dismiss milestone / celebration modals before the scene
                // change proceeds (e.g. "First Pair!" before the recap screen).
                if self.pending_scene.is_some() && !self.modals.is_active() {
                    self.transition_alpha -= self.transition_speed;
                    // Map alpha 1→0 onto timer 0→0.5 (first half of transition).
                    self.transition_timer =
                        (1.0 - self.transition_alpha.max(0.0)).clamp(0.0, 1.0) * 0.5;
                    if self.transition_alpha <= 0.0 {
                        self.transition_alpha = 0.0;
                        if let Some(next) = self.pending_scene.take() {
                            // If we're transitioning out of the GameOver scene,
                            // surface any deferred celebration modals now.
                            if matches!(self.scene, Scene::GameOver(_))
                                && !self.pending_post_game_over_modals.is_empty()
                            {
                                for modal in self.pending_post_game_over_modals.drain(..) {
                                    self.modals.push(modal);
                                }
                            }
                            // Clear residual smoke when entering the shop
                            // or the shrine-select screen so the new scene
                            // starts with a clean atmosphere.
                            let clear_smoke = matches!(
                                (&self.scene, &next),
                                (Scene::TileSelect(_), Scene::Shop(_))
                                    | (Scene::TutorialCampaign(_), Scene::Shop(_))
                                    | (Scene::Shop(_), Scene::PickBlind(_))
                            );
                            // Route the new scene to the target recorded
                            // when the transition started, not whatever is
                            // on top now — overlays may have been pushed
                            // mid-fade (e.g. a zodiac celebration after a
                            // skip) and must not clobber them.
                            let entering_main_menu = matches!(next, Scene::StartScreen(_));
                            if self.pending_scene_targets_overlay {
                                if let Some(top) = self.overlay_stack.last_mut() {
                                    *top = next;
                                } else {
                                    self.scene = next;
                                }
                            } else {
                                self.scene = next;
                            }
                            self.pending_scene_targets_overlay = false;
                            if entering_main_menu {
                                self.audio.play_sfx(audio::SfxId::MainMenuEnter);
                            }
                            if let Some(scene) = Self::saved_resume_scene_for(&self.scene) {
                                self.resume_scene = scene;
                            }
                            if clear_smoke && let Some(r) = self.renderer.as_mut() {
                                r.clear_smoke();
                            }
                            if let Some(input) = self.input.as_mut() {
                                input.focus_slot = 0;
                            }
                            // Fade score panel in for the new scene.
                            self.anim
                                .fade(render::animation::ENTITY_SCORE_PANEL, 0.0, 1.0, 300);
                            // Slide hand strip up from below.
                            self.anim.slide_to(
                                render::animation::ENTITY_HAND_STRIP,
                                0.0,
                                -20.0,
                                400,
                            );
                        }
                    }
                } else if self.transition_alpha < 1.0 {
                    self.transition_alpha =
                        (self.transition_alpha + self.transition_speed).min(1.0);
                    // Map alpha 0→1 onto timer 0.5→1.0 (second half).
                    self.transition_timer = 0.5 + (self.transition_alpha.clamp(0.0, 1.0)) * 0.5;
                    // Reset transition kind once fully faded in.
                    if self.transition_alpha >= 1.0 {
                        self.transition_timer = 0.0;
                        self.transition_kind = TransitionKind::Quick;
                        self.transition_speed = 0.08;
                    }
                }

                // Handle quit request from scene.
                if quit_requested {
                    self.quit_requested = true;
                }

                // Headless screenshot tick. We render `warmup_frames + 1`
                // total frames: warmup frames let async loaders settle, then
                // the final draw is the one captured. The renderer writes
                // the PNG synchronously during that draw (between submit
                // and present). After it returns, the file is on disk.
                let mut should_capture_this_frame = false;
                if let Some(shot) = self.headless_screenshot.as_ref()
                    && shot.frames_remaining == 0
                {
                    should_capture_this_frame = true;
                    let path = shot.output.clone();
                    if let Some(r) = self.renderer.as_ref() {
                        r.queue_screenshot(path);
                    }
                }

                // Cursor → smoke impulses are now injected by the renderer
                // itself (it has the gameplay camera matrices required to
                // unproject the cursor onto the table plane).
                self.draw();

                if let Some(shot) = self.headless_screenshot.as_mut() {
                    if should_capture_this_frame {
                        // Verify the renderer actually consumed the
                        // queued screenshot — when the swapchain returns
                        // Outdated/Lost the draw early-returns and the
                        // queued path is left untouched. In that case,
                        // tick to the next frame instead of exiting
                        // with no file written. Bounded by `retries` so
                        // a permanently-broken swapchain doesn't loop.
                        let still_pending = self
                            .renderer
                            .as_ref()
                            .map(|r| r.screenshot_pending())
                            .unwrap_or(false);
                        if still_pending && shot.retries < 30 {
                            shot.retries += 1;
                            log::warn!("screenshot: capture frame dropped, retry {}", shot.retries);
                            if let Some(w) = self.window.as_ref() {
                                w.request_redraw();
                            }
                        } else {
                            if still_pending {
                                log::error!(
                                    "screenshot: still pending after {} retries, exiting anyway",
                                    shot.retries
                                );
                            } else {
                                log::info!("screenshot saved → {}", shot.output.display());
                            }
                            self.headless_screenshot = None;
                            event_loop.exit();
                        }
                    } else {
                        shot.frames_remaining = shot.frames_remaining.saturating_sub(1);
                        if let Some(w) = self.window.as_ref() {
                            w.request_redraw();
                        }
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if button == MouseButton::Left {
                    let cursor = self
                        .input
                        .as_ref()
                        .map(|i| i.last_cursor)
                        .unwrap_or((0.0, 0.0));

                    if state == ElementState::Pressed {
                        self.mouse_clicked = true;
                        if let Some(input) = self.input.as_mut() {
                            input.mode = InputMode::Cursor;
                        }

                        // Debug "Object Hit Test" one-shot picker. If armed,
                        // consume this click: hit-test the cursor against
                        // every known scene object and log the match. Skip
                        // all the normal click dispatch (buttons, tiles,
                        // drag) so the click can't accidentally fire a
                        // gameplay action while we're just probing.
                        if self.debug.object_hit_test_armed {
                            self.debug.object_hit_test_armed = false;
                            let name = self
                                .renderer
                                .as_ref()
                                .and_then(|r| r.pick_debug_object(cursor.0, cursor.1));
                            match name {
                                Some(n) => log::info!(
                                    "[Debug] Object hit test: {n} at ({:.0}, {:.0})",
                                    cursor.0,
                                    cursor.1
                                ),
                                None => log::info!(
                                    "[Debug] Object hit test: (no object) at ({:.0}, {:.0})",
                                    cursor.0,
                                    cursor.1
                                ),
                            }
                            if let Some(w) = self.window.as_ref() {
                                w.request_redraw();
                            }
                            return;
                        }

                        // Arrange mode: consume all clicks for 3D object
                        // picking — buttons fire their scene actions (restock,
                        // leave, etc.) which is never what you want while
                        // arranging, so suppress them too.
                        if self.debug.arrange_mode.is_some() {
                            // Only try to select an object when nothing is
                            // selected yet (inner = None).
                            if matches!(self.debug.arrange_mode, Some(None)) {
                                let picked = self.renderer.as_ref().and_then(|r| {
                                    r.pick_debug_object_with_model(cursor.0, cursor.1)
                                });
                                match picked {
                                    Some((name, Some(model))) => {
                                        // Start with zero deltas — the override
                                        // is additive on top of the scene's own
                                        // placement, so no decomposition needed.
                                        let origin = model.transform_point3(glam::Vec3::ZERO);
                                        self.debug.arrange_mode = Some(Some(ArrangeModeState {
                                            object_name: name.to_string(),
                                            selected_world_origin: origin,
                                            delta_px: 0.0,
                                            delta_py: 0.0,
                                            delta_lift: 0.0,
                                            delta_rz_deg: 0.0,
                                            delta_rx_deg: 0.0,
                                            delta_ry_deg: 0.0,
                                            trans_step_px: 1.0,
                                            rot_step_deg: 1.0,
                                        }));
                                        log::info!(
                                            "[Arrange] Selected '{}' — all deltas zero, ready to nudge",
                                            name,
                                        );
                                        log::info!(
                                            "[Arrange] Arrow keys: move X/Y | Shift+Arrow: rotate Z/X | Enter: confirm+copy | Esc: cancel"
                                        );
                                    }
                                    Some((name, None)) => {
                                        // Hand tile or object without a model — just log
                                        log::info!(
                                            "[Arrange] Hit '{}' — no placement matrix available (hand tile?), cannot arrange",
                                            name
                                        );
                                    }
                                    None => {
                                        log::info!(
                                            "[Arrange] No object under cursor — click on an object to select it"
                                        );
                                    }
                                }
                            } else if let Some(Some(ref mut st)) = self.debug.arrange_mode {
                                // Object already selected — click teleports it to
                                // the cursor's world-space hit point. Preserves
                                // lift (Z) so dragging across the felt behaves
                                // like a top-down nudge. Selection is locked —
                                // Tab or Escape to change it.
                                let hit = self
                                    .renderer
                                    .as_ref()
                                    .and_then(|r| r.pick_debug_world_point(cursor.0, cursor.1));
                                match hit {
                                    Some(h) => {
                                        // world_x = px - w/2 (linear). Delta in
                                        // world X == delta in px; world_y inverts
                                        // sign vs py.
                                        st.delta_px = h.x - st.selected_world_origin.x;
                                        st.delta_py = -(h.y - st.selected_world_origin.y);
                                        log::info!(
                                            "[Arrange] Click-move '{}' → world ({:.1}, {:.1}) | Δpx={:+.1} Δpy={:+.1}",
                                            st.object_name,
                                            h.x,
                                            h.y,
                                            st.delta_px,
                                            st.delta_py,
                                        );
                                    }
                                    None => {
                                        log::info!(
                                            "[Arrange] Click missed all pickables — no move"
                                        );
                                    }
                                }
                            }
                            self.mouse_clicked = false;
                            if let Some(w) = self.window.as_ref() {
                                w.request_redraw();
                            }
                            return;
                        }

                        // Check if click hit any button.
                        let mut hit = false;
                        let mut hit_shop_3d = false;
                        for btn in &self.active_buttons {
                            let (bx, by, bw, bh) = btn.rect;
                            if cursor.0 >= bx
                                && cursor.0 <= bx + bw
                                && cursor.1 >= by
                                && cursor.1 <= by + bh
                            {
                                self.audio.play_sfx(audio::SfxId::TileClick);
                                match btn.action {
                                    ButtonAction::Ui(a) => self.mouse_actions.push(a),
                                    ButtonAction::Scene(id) => {
                                        if id == scenes::shop::SHOP_3D_HIT_ID {
                                            hit_shop_3d = true;
                                        }
                                        self.mouse_button_clicks.push(id);
                                    }
                                }
                                hit = true;
                                break;
                            }
                        }
                        // Shop drag-to-sell: on mouse-down over a 3D shop object, record
                        // which item was under the cursor so that a drag onto the sell
                        // tray (detected on mouse-up) can sell the right item.
                        if hit_shop_3d {
                            if let Some(renderer) = self.renderer.as_ref() {
                                let picked = renderer.pick_shop_object(cursor.0, cursor.1);
                                self.shop_drag_start = match picked {
                                    Some(ShopHit::Relic(_))
                                    | Some(ShopHit::Ribbon(_))
                                    | Some(ShopHit::Talisman(_)) => picked.map(|h| (h, cursor)),
                                    _ => None,
                                };
                            }
                        } else {
                            self.shop_drag_start = None;
                        }
                        if !hit {
                            // Check if we're clicking on a hand tile to start drag.
                            let clicked_relic_slot = self.gameplay_relic_slot_at_cursor(cursor);
                            if let Some(input) = self.input.as_mut() {
                                if input.pointer_slot.is_some() {
                                    // Hand tile click: gameplay scene's
                                    // marquee handler picks this up. No
                                    // drag-to-swap state is recorded — the
                                    // gesture is now hold-to-multi-select,
                                    // not click-and-drag-to-reorder.
                                    self.audio.play_sfx(audio::SfxId::TileClick);
                                    self.mouse_actions.push(UiAction::Confirm);
                                } else if let Some(slot) = clicked_relic_slot {
                                    input.drag = Some(ui::input::DragState {
                                        subject: ui::input::DragSubject::Relic,
                                        from_slot: slot,
                                        start_pos: cursor,
                                        current_pos: cursor,
                                    });
                                }
                            }
                        }
                    } else if state == ElementState::Released {
                        // Shop drag-to-sell: if a drag started on an owned item and the
                        // cursor moved far enough and is now over the sell tray, inject
                        // a drop event so the shop can complete the sale.
                        if let Some((_, start)) = self.shop_drag_start.take()
                            && matches!(&self.scene, Scene::Shop(_))
                        {
                            let dx = cursor.0 - start.0;
                            let dy = cursor.1 - start.1;
                            let dist = (dx * dx + dy * dy).sqrt();
                            if dist > 10.0
                                && let Some(renderer) = self.renderer.as_ref()
                            {
                                const SELL_TRAY_PICK: u32 = 8; // PICK_SELL_TRAY
                                let over_sell_tray = matches!(
                                    renderer.pick_shop_object(cursor.0, cursor.1),
                                    Some(ShopHit::Dish(id)) if id == SELL_TRAY_PICK
                                );
                                if over_sell_tray {
                                    self.mouse_button_clicks.push(SHOP_DRAG_DROP_ID);
                                }
                            }
                        }
                        // End drag — swap relics if dropped on a different slot.
                        // Require minimum drag distance to avoid accidental swaps.
                        let dropped_relic_slot = self.gameplay_relic_slot_at_cursor(cursor);
                        if let Some(input) = self.input.as_mut()
                            && let Some(drag) = input.drag.take()
                        {
                            let dx = cursor.0 - drag.start_pos.0;
                            let dy = cursor.1 - drag.start_pos.1;
                            let dist = (dx * dx + dy * dy).sqrt();
                            if dist > 10.0 {
                                match drag.subject {
                                    ui::input::DragSubject::Relic => {
                                        if let Some(target_slot) = dropped_relic_slot
                                            && target_slot != drag.from_slot
                                        {
                                            self.run
                                                .relics
                                                .swap_relics(drag.from_slot, target_slot);
                                        }
                                    }
                                }
                            }
                        }
                        // LMB release ends a marquee multi-select gesture.
                        // Always emit; the gameplay scene clears its marquee
                        // state on ConfirmRelease and other scenes ignore it.
                        self.mouse_actions.push(UiAction::ConfirmRelease);
                    }
                    if let Some(w) = self.window.as_ref() {
                        w.request_redraw();
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let (Some(input), Some(win)) = (self.input.as_mut(), self.window.as_ref()) {
                    let new_cursor = (position.x as f32, position.y as f32);
                    // Only flip back to Cursor mode if the cursor actually
                    // moved meaningfully. Skip while in Controller mode —
                    // micro-movements still fight stick navigation; switching
                    // to Cursor uses LMB instead (see MouseInput).
                    let dx = new_cursor.0 - input.last_cursor.0;
                    let dy = new_cursor.1 - input.last_cursor.1;
                    let moved = (dx * dx + dy * dy) > 4.0;
                    let was_hidden = moved && input.mode != InputMode::Cursor;
                    if moved && input.mode != InputMode::Controller {
                        // Pointer jitter / OS drift used to flip Controller→Cursor
                        // and break stick navigation; explicit mouse use is LMB
                        // (see MouseInput).
                        input.mode = InputMode::Cursor;
                    }
                    input.last_cursor = new_cursor;
                    let size = win.inner_size();
                    let layout = self
                        .layout_engine
                        .solve(size.width as f32, size.height as f32);
                    // Same raycast-based pick as the per-frame update path.
                    let hand_slot_count = self.run.hand.len().max(layout.hand_slots.len());
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
                    // Update drag position if dragging.
                    if let Some(ref mut drag) = input.drag {
                        drag.current_pos = input.last_cursor;
                    }
                    if was_hidden {
                        win.set_cursor_visible(true);
                    }
                    win.request_redraw();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => y,
                    winit::event::MouseScrollDelta::PixelDelta(pos) => {
                        // Convert pixel delta to approximate line units.
                        (pos.y as f32) / 40.0
                    }
                };
                self.scroll_delta += lines;
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if self.wants_fullscreen_shortcut(&event) {
                    self.toggle_fullscreen();
                    if let Some(w) = self.window.as_ref() {
                        w.request_redraw();
                    }
                } else if event.state == ElementState::Pressed {
                    // Arrange mode: Escape while waiting for a click exits the
                    // mode entirely.
                    if matches!(self.debug.arrange_mode, Some(None))
                        && event.physical_key == PhysicalKey::Code(KeyCode::Escape)
                    {
                        self.debug.arrange_mode = None;
                        log::info!("[Arrange] Mode exited");
                        if let Some(w) = self.window.as_ref() {
                            w.request_redraw();
                        }
                        return;
                    }

                    // Arrange mode: Tab / Shift+Tab cycles through the active
                    // scene's placement hierarchy. Works whether an object is
                    // already selected or not — picking a group applies deltas
                    // to every descendant leaf on save.
                    if self.debug.arrange_mode.is_some()
                        && event.physical_key == PhysicalKey::Code(KeyCode::Tab)
                    {
                        let flat = arrange_hierarchy_flat(&self.scene);
                        if flat.is_empty() {
                            log::info!("[Arrange] Current scene has no hierarchy");
                        } else {
                            let current_name = match &self.debug.arrange_mode {
                                Some(Some(s)) => Some(s.object_name.as_str()),
                                _ => None,
                            };
                            let current_idx =
                                current_name.and_then(|n| flat.iter().position(|e| e.name == n));
                            let reverse = self.modifiers.shift_key();
                            let next_idx = match (current_idx, reverse) {
                                (None, false) => 0,
                                (None, true) => flat.len() - 1,
                                (Some(i), false) => (i + 1) % flat.len(),
                                (Some(i), true) => (i + flat.len() - 1) % flat.len(),
                            };
                            let entry = &flat[next_idx];
                            let origin = self
                                .renderer
                                .as_ref()
                                .and_then(|r| r.debug_object_origin(entry.name))
                                .unwrap_or(glam::Vec3::ZERO);
                            self.debug.arrange_mode = Some(Some(ArrangeModeState {
                                object_name: entry.name.to_string(),
                                selected_world_origin: origin,
                                delta_px: 0.0,
                                delta_py: 0.0,
                                delta_lift: 0.0,
                                delta_rz_deg: 0.0,
                                delta_rx_deg: 0.0,
                                delta_ry_deg: 0.0,
                                trans_step_px: 1.0,
                                rot_step_deg: 1.0,
                            }));
                            let indent = "  ".repeat(entry.depth);
                            let marker = if entry.is_group { "▸" } else { "•" };
                            log::info!(
                                "[Arrange] {}{} {} ({}) — {}/{} in hierarchy",
                                indent,
                                marker,
                                entry.label,
                                entry.name,
                                next_idx + 1,
                                flat.len(),
                            );
                            if let Some(w) = self.window.as_ref() {
                                w.request_redraw();
                            }
                        }
                        return;
                    }

                    // Arrange mode: when an object is selected, consume arrow
                    // keys (move X/Y), Shift+arrows (rotate Z/X), Enter
                    // (confirm+copy), and Escape (cancel selection). Normal
                    // input path is skipped so gameplay doesn't also fire.
                    if let Some(Some(ref mut state)) = self.debug.arrange_mode {
                        let shift = self.modifiers.shift_key();
                        let step_px = state.trans_step_px; // pixels per key press
                        let step_deg = state.rot_step_deg; // degrees per key press
                        let mut handled = true;
                        let mut nudged = false;
                        let mut escape_pending = false;
                        if let PhysicalKey::Code(code) = event.physical_key {
                            match code {
                                KeyCode::Digit1 => {
                                    state.trans_step_px = 1.0;
                                    state.rot_step_deg = 1.0;
                                    log::info!("[Arrange] Step 1 (1 px / 1°)");
                                }
                                KeyCode::Digit2 => {
                                    state.trans_step_px = 5.0;
                                    state.rot_step_deg = 15.0;
                                    log::info!("[Arrange] Step 2 (5 px / 15°)");
                                }
                                KeyCode::Digit3 => {
                                    state.trans_step_px = 25.0;
                                    state.rot_step_deg = 45.0;
                                    log::info!("[Arrange] Step 3 (25 px / 45°)");
                                }
                                KeyCode::Digit4 => {
                                    state.trans_step_px = 100.0;
                                    state.rot_step_deg = 90.0;
                                    log::info!("[Arrange] Step 4 (100 px / 90°)");
                                }
                                // Translation: WASD = forward/left/back/right, Q/E = down/up
                                KeyCode::KeyD if !shift => {
                                    state.delta_px += step_px;
                                    nudged = true;
                                }
                                KeyCode::KeyA if !shift => {
                                    state.delta_px -= step_px;
                                    nudged = true;
                                }
                                KeyCode::KeyS if !shift => {
                                    state.delta_py += step_px;
                                    nudged = true;
                                }
                                KeyCode::KeyW if !shift => {
                                    state.delta_py -= step_px;
                                    nudged = true;
                                }
                                KeyCode::KeyQ if !shift => {
                                    state.delta_lift -= step_px;
                                    nudged = true;
                                }
                                KeyCode::KeyE if !shift => {
                                    state.delta_lift += step_px;
                                    nudged = true;
                                }
                                // Rotation: Shift+A/D = rz, Shift+W/S = rx, Shift+Q/E = ry
                                KeyCode::KeyD if shift => {
                                    state.delta_rz_deg += step_deg;
                                    nudged = true;
                                }
                                KeyCode::KeyA if shift => {
                                    state.delta_rz_deg -= step_deg;
                                    nudged = true;
                                }
                                KeyCode::KeyW if shift => {
                                    state.delta_rx_deg -= step_deg;
                                    nudged = true;
                                }
                                KeyCode::KeyS if shift => {
                                    state.delta_rx_deg += step_deg;
                                    nudged = true;
                                }
                                KeyCode::KeyQ if shift => {
                                    state.delta_ry_deg -= step_deg;
                                    nudged = true;
                                }
                                KeyCode::KeyE if shift => {
                                    state.delta_ry_deg += step_deg;
                                    nudged = true;
                                }
                                KeyCode::Enter | KeyCode::NumpadEnter => {
                                    // Confirm: convert pixel deltas to proportional fractions
                                    // so the output is screen-size independent.
                                    let size = self
                                        .window
                                        .as_ref()
                                        .map(|w| w.inner_size())
                                        .unwrap_or(winit::dpi::PhysicalSize::new(1280, 720));
                                    let ww = size.width as f32;
                                    let wh = size.height as f32;
                                    let dnx = state.delta_px / ww;
                                    let dny = state.delta_py / wh;
                                    let text = format!(
                                        "// [Arrange] object: {}\nnx += {:.6};\nny += {:.6};\nlift_z += {:.3};\nrotation_z_deg += {:.2};\nrotation_x_deg += {:.2};\nrotation_y_deg += {:.2};",
                                        state.object_name,
                                        dnx,
                                        dny,
                                        state.delta_lift,
                                        state.delta_rz_deg,
                                        state.delta_rx_deg,
                                        state.delta_ry_deg,
                                    );
                                    match arboard::Clipboard::new() {
                                        Ok(mut cb) => {
                                            if let Err(e) = cb.set_text(&text) {
                                                log::error!(
                                                    "[Arrange] Clipboard write failed: {e}"
                                                );
                                            } else {
                                                log::info!(
                                                    "[Arrange] Copied to clipboard:\n{text}"
                                                );
                                            }
                                        }
                                        Err(e) => {
                                            log::error!("[Arrange] Could not open clipboard: {e}")
                                        }
                                    }
                                    // Apply deltas to the scene's positions struct and save to JSON.
                                    apply_arrange_to_layout(
                                        &state.object_name,
                                        ArrangeInput {
                                            delta_px: state.delta_px,
                                            delta_py: state.delta_py,
                                            delta_lift: state.delta_lift,
                                            delta_rx_deg: state.delta_rx_deg,
                                            delta_ry_deg: state.delta_ry_deg,
                                            delta_rz_deg: state.delta_rz_deg,
                                        },
                                        ww,
                                        wh,
                                        &mut self.scene,
                                    );
                                    // apply_arrange_to_layout already mutated the
                                    // scene's positions struct in-place, so no reload
                                    // is needed — reloading from disk risks returning
                                    // defaults if the save failed or the file is absent.
                                    log::info!(
                                        "[Arrange] Confirmed '{}': Δnx={:.6} Δny={:.6} Δlift={:.3} Δrz={:.2}° Δrx={:.2}° Δry={:.2}°",
                                        state.object_name,
                                        dnx,
                                        dny,
                                        state.delta_lift,
                                        state.delta_rz_deg,
                                        state.delta_rx_deg,
                                        state.delta_ry_deg,
                                    );
                                    state.delta_px = 0.0;
                                    state.delta_py = 0.0;
                                    state.delta_lift = 0.0;
                                    state.delta_rz_deg = 0.0;
                                    state.delta_rx_deg = 0.0;
                                    state.delta_ry_deg = 0.0;
                                }
                                KeyCode::KeyR if !shift => {
                                    // Reset: restore compiled-in defaults for the
                                    // selected placement (or every descendant of a
                                    // selected group) and drop any accumulated
                                    // deltas so the on-screen preview matches disk.
                                    reset_arrange_to_default(&state.object_name, &mut self.scene);
                                    state.delta_px = 0.0;
                                    state.delta_py = 0.0;
                                    state.delta_lift = 0.0;
                                    state.delta_rz_deg = 0.0;
                                    state.delta_rx_deg = 0.0;
                                    state.delta_ry_deg = 0.0;
                                }
                                KeyCode::Escape => {
                                    // Cancel selection, go back to waiting for click.
                                    // Deferred so the borrow of `state` (above) ends
                                    // cleanly before we overwrite the enum.
                                    escape_pending = true;
                                }
                                _ => {
                                    handled = false;
                                }
                            }
                        } else {
                            handled = false;
                        }
                        if nudged {
                            // Log the resolved placement (on-disk + staged delta)
                            // so both HUD and log agree on what Enter will commit.
                            let size = self
                                .window
                                .as_ref()
                                .map(|w| w.inner_size())
                                .unwrap_or(winit::dpi::PhysicalSize::new(1280, 720));
                            let ww = size.width as f32;
                            let wh = size.height as f32;
                            let name = state.object_name.clone();
                            let dpx = state.delta_px;
                            let dpy = state.delta_py;
                            let dlift = state.delta_lift;
                            let drx = state.delta_rx_deg;
                            let dry = state.delta_ry_deg;
                            let drz = state.delta_rz_deg;
                            if let Some(p) = sample_arrange_placement(&name, &self.scene) {
                                let dnx = dpx / ww;
                                let dny = dpy / wh;
                                let d_lift_mm = dlift * crate::ui::scene_layout::HFRAC_TO_MM
                                    / crate::ui::scene_layout::CANONICAL_WINDOW_W;
                                log::info!(
                                    "[Arrange] {} nx={:.4} ny={:.4} lift={:.2}mm rx={:+.1}° ry={:+.1}° rz={:+.1}°",
                                    name,
                                    p.nx + dnx,
                                    p.ny + dny,
                                    p.lift_mm + d_lift_mm,
                                    p.rx_deg + drx,
                                    p.ry_deg + dry,
                                    p.rz_deg + drz,
                                );
                            } else {
                                log::info!(
                                    "[Arrange] {} (group) Δpx={:+.1} Δpy={:+.1} Δlift={:+.1} Δrx={:+.1}° Δry={:+.1}° Δrz={:+.1}°",
                                    name,
                                    dpx,
                                    dpy,
                                    dlift,
                                    drx,
                                    dry,
                                    drz,
                                );
                            }
                        }
                        if escape_pending {
                            log::info!(
                                "[Arrange] Selection cancelled — click another object or use Debug > Arrange Mode to exit"
                            );
                            self.debug.arrange_mode = Some(None);
                        }
                        if handled {
                            if let Some(w) = self.window.as_ref() {
                                w.request_redraw();
                            }
                            return;
                        }
                        // Fall through for unhandled keys (e.g. fullscreen).
                    }

                    // Cross-platform debug shortcut: Ctrl+Shift+M opens the
                    // material viewer pushdown scene. Mirrors the Debug menu
                    // entry so Linux (where muda has no non-GTK menu) and any
                    // other OS the menu doesn't reach still has access.
                    if let PhysicalKey::Code(code) = event.physical_key
                        && code == KeyCode::KeyM
                        && self.modifiers.shift_key()
                        && (self.modifiers.control_key() || self.modifiers.super_key())
                    {
                        if !self
                            .overlay_stack
                            .iter()
                            .any(|s| matches!(s, Scene::MaterialViewer(_)))
                        {
                            self.overlay_stack
                                .push(Scene::MaterialViewer(MaterialViewerScene::new(true)));
                            log::info!("[Debug] Opened material viewer (keyboard shortcut)");
                            if let Some(w) = self.window.as_ref() {
                                w.request_redraw();
                            }
                        }
                        return;
                    }

                    let mut v = Vec::new();
                    let shift = self.modifiers.shift_key();
                    let mode_changed = if let Some(input) = self.input.as_mut() {
                        input.on_key(event.physical_key, shift, &mut v)
                    } else {
                        false
                    };
                    self.mouse_actions.extend(v);
                    if mode_changed && let Some(w) = self.window.as_ref() {
                        w.set_cursor_visible(false);
                    }
                    if let Some(w) = self.window.as_ref() {
                        w.request_redraw();
                    }
                } else if event.state == ElementState::Released {
                    // Confirm-key release drives marquee multi-select commit.
                    let mut v = Vec::new();
                    if let Some(input) = self.input.as_mut() {
                        input.on_key_release(event.physical_key, &mut v);
                    }
                    self.mouse_actions.extend(v);
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // Drain Steam callbacks once per loop iteration so achievement
        // toasts, overlay activation, and any future async results land
        // promptly. No-op when Steam is disabled.
        self.steam.run_callbacks();

        if self.quit_requested {
            let _ = persistence::save_profile(self.active_profile, &self.progress);
            let _ = persistence::save_settings(&persistence::load_settings());
            self.persist_run_if_in_progress();
            _event_loop.exit();
            return;
        }
        // Continuous redraw: input (including gilrs) is processed from
        // `RedrawRequested`; presenting stays vsync-gated in the GPU path.
        // Trade CPU work for predictable latency — [`UiLayout::solve`] and the
        // renderer cache cheap wins when window size is unchanged.
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }
}
