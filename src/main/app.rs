use std::time::Instant;

use crate::game::event_bus::EventBus;
use crate::game::run::RunState;
use crate::game::scene_look_tuning::SceneLookTuningSet;
use crate::scenes::{Scene, SceneBehavior};
use sdl3::keyboard::{Mod, Scancode};

impl crate::App {
    /// Prefetch / upload `gameplay.glb` during the hub only when Continue will
    /// land in an in-progress run saved mid-gameplay (~600 MiB VRAM).
    pub(crate) fn warm_gameplay_gpu_for_resume(&self) -> bool {
        self.run.is_in_progress()
            && matches!(
                self.resume_scene,
                crate::persistence::ResumeScene::Gameplay
            )
    }

    pub(crate) fn saved_resume_scene_for(scene: &Scene) -> Option<crate::persistence::ResumeScene> {
        match scene {
            Scene::Shop(_) => Some(crate::persistence::ResumeScene::Shop),
            Scene::Hallway(_) => Some(crate::persistence::ResumeScene::Hallway),
            Scene::Gameplay(_) => Some(crate::persistence::ResumeScene::Gameplay),
            _ => None,
        }
    }

    /// Face-button semantics for the scene currently receiving controller input.
    pub(crate) fn active_face_bindings(&self) -> crate::ui::input::FaceButtonBindings {
        if self.overlay_stack.last().is_some_and(|top| {
            matches!(
                top,
                Scene::Showcase(s) if s.wants_orbit_input()
            )
        }) {
            return crate::ui::input::FaceButtonBindings {
                north_press: Some(crate::ui::input::UiAction::NorthFacePress),
                ..Default::default()
            };
        }
        if !self.overlay_stack.is_empty() || self.scene.has_blocking_overlay() {
            return crate::ui::input::FaceButtonBindings::default();
        }
        let xy_quick_action = self
            .input
            .as_ref()
            .map(|i| i.xy_quick_action)
            .unwrap_or(true);
        self.scene
            .face_button_bindings(crate::ui::input::FaceBindingCtx { xy_quick_action })
    }

    /// Single source of truth for "is anything modal-like up right now?"
    ///
    /// **The modal-blocking pattern.** Any overlay that should block input
    /// and hover for elements below it is reported here, by ORing together:
    ///   - The app-owned [`ModalQueue`] (toast modals).
    ///   - App-owned debug overlays (`tuning_overlay`, `sfx_test_overlay`).
    ///   - The active scene's own internal overlays, via
    ///     [`Scene::has_blocking_overlay`].
    ///
    /// The main loop also consults this for the **click safety wipe**:
    /// right after the scene populates `active_buttons`, those buttons are
    /// cleared if any modal is up, so scene buttons can never be clicked
    /// through. Overlays that *want* their own clickable surface (e.g.
    /// `ModalQueue`'s full-screen dismiss) write to `active_buttons`
    /// *after* the wipe in their own draw step.
    ///
    /// To make a new overlay modal-blocking by default:
    ///   - If it's app-owned: add it to this OR-chain.
    ///   - If it's scene-owned: report it from the scene's
    ///     `has_blocking_overlay()` method.
    ///
    /// No per-call-site changes are needed — the gates pick it up
    /// automatically.
    pub(crate) fn modal_overlay_active(&self) -> bool {
        self.modals.is_active()
            || self.debug.any_overlay_active()
            || self.scene.has_blocking_overlay()
            || self.overlay_stack.iter().any(|s| s.has_blocking_overlay())
            || !self.overlay_stack.is_empty()
    }

    /// Storeroom shop is the active face (no showcase inspect overlay, not paused).
    pub(crate) fn shop_storeroom_face_active(&self) -> bool {
        self.overlay_stack.is_empty()
            && matches!(self.scene, Scene::Shop(_))
            && !self.scene.has_blocking_overlay()
    }

    /// Shop or item-inspect showcase: LMB drag orbits instead of firing clicks on press.
    pub(crate) fn shop_defer_lmb_clicks(&self) -> bool {
        self.shop_storeroom_face_active()
            || self
                .overlay_stack
                .last()
                .is_some_and(|s| matches!(s, Scene::Showcase(s) if s.wants_orbit_input()))
    }

    /// Standard storeroom visit (not tutorial/paused/transitioning away).
    pub(crate) fn shop_storeroom_dwell_active(&self) -> bool {
        if self.pending_scene.is_some() {
            return false;
        }
        match &self.scene {
            Scene::Shop(shop) => shop.counts_storeroom_dwell_time(),
            _ => false,
        }
    }

    pub(crate) fn gameplay_relic_slot_at_cursor(&self, cursor: (f32, f32)) -> Option<usize> {
        if self.modal_overlay_active() || !matches!(self.scene, Scene::Gameplay(_)) {
            return None;
        }
        let renderer = self.renderer.as_ref()?;
        renderer
            .projections()
            .relic_rects
            .iter()
            .enumerate()
            .find_map(|(i, rect)| {
                let [x, y, w, h] = *rect;
                (w > 1.0
                    && h > 1.0
                    && x.is_finite()
                    && y.is_finite()
                    && cursor.0 >= x
                    && cursor.0 <= x + w
                    && cursor.1 >= y
                    && cursor.1 <= y + h)
                    .then_some(i)
            })
    }

    pub(crate) fn new(steam: crate::steam::SteamClient) -> Self {
        let settings = crate::persistence::load_settings();
        let active_profile = settings.active_profile;
        let progress = crate::persistence::load_profile(active_profile);
        // Prefer a saved-on-quit run for this profile (resume). If none
        // exists or it was written by a previous build version, fall back
        // to a fresh demo run. `load_run` deletes stale/corrupt saves.
        let loaded_run = crate::persistence::load_run(active_profile);
        let resume_scene = loaded_run
            .as_ref()
            .map(|saved| saved.scene)
            .unwrap_or(crate::persistence::ResumeScene::Gameplay);
        let mut run = loaded_run
            .map(|saved| saved.run)
            .unwrap_or_else(RunState::new_demo);
        run.set_auto_cash_in_on_full_structure(settings.auto_cash_in_on_full_structure);
        run.apply_progression(&progress);
        steam.sync_profile_stats(&progress);
        let mut audio = crate::audio::AudioManager::new();
        audio.set_master_volume(settings.master_volume);
        audio.set_sfx_volume(settings.sfx_volume);
        audio.set_music_volume(settings.music_volume);
        if !settings.sfx_enabled {
            audio.set_enabled(false);
        }
        let scene_look = SceneLookTuningSet::load();
        Self {
            last_drawable_px: crate::physical_size::PhysicalSize::new(1920, 1080),
            renderer: None,
            layout_engine: crate::ui::layout::UiLayout::new(),
            input: None,
            run,
            bus: EventBus::default(),
            anim: crate::render::animation::AnimationController::new(),
            last_frame: Instant::now(),
            last_frame_dt: 1.0 / 60.0,
            mouse_actions: Vec::new(),
            mouse_button_clicks: Vec::new(),
            mouse_clicked: false,
            mouse_left_down: false,
            mouse_right_clicked: false,
            deferred_lmb_button_click: None,
            mouse_left_press_cursor: None,
            scroll_delta: 0.0,
            active_buttons: Vec::new(),
            scene: Scene::Splash(crate::scenes::splash::SplashScene::new()),
            resume_scene,
            progress,
            active_profile,
            audio,
            transition_alpha: 1.0,
            transition_speed: crate::scene_transition::DEFAULT_QUICK_SPEC.speed,
            transition_timer: 0.0,
            transition_kind: crate::scene_transition::TransitionKind::Quick,
            pending_scene: None,
            pending_scene_destination: crate::scene_transition::PendingSceneDestination::default(),
            overlay_stack: Vec::new(),
            prev_controller_present: false,
            quit_requested: false,
            close_saved: false,
            modals: crate::ui::modal::ModalQueue::default(),
            pending_post_game_over_level_up: None,
            deferred_round_end: None,
            gfx: crate::main_render_settings::RenderSettings {
                effects_quality: settings.effects_quality,
                tile_preset: settings.tile_preset,
                tile_material: settings.tile_material,
                tileset_name: settings.tileset_name.clone(),
                gamma: settings.gamma,
                graphics_mode: settings.graphics_mode,
                hdr_enabled: settings.hdr_enabled,
                vhs_enabled: settings.vhs_enabled,
            },
            // Default: cheap baseline; see `effect_layers.rs`. Use `FULL` or flip
            // flags to restore shadows, SSR, particles, transition FX, HDR, etc.
            effect_layers: crate::effect_layers::EffectLayers::BASELINE,
            debug: crate::main_debug_state::DebugState::new(),
            cascade_tuning: crate::game::cascade::CascadeTuning::default(),
            scene_look,
            modifiers: Mod::NOMOD,
            steam,
            archive_last_seen_run_len: settings.archive_last_seen_run_len,
            cpu_profiler: crate::render::cpu_profiler::CpuProfiler::new(),
            profile_saver: crate::persistence::ProfileSaver::spawn(),
            profile_dirty: false,
            last_window_title: String::new(),
            room_gltf_brownout: crate::main_room_gltf_brownout::RoomGltfBrownout::new(),
            frame_picks: crate::FramePicks::default(),
            perf_watchdog: crate::main_perf_watchdog::FramePerfWatchdog::new(),
        }
    }

    /// Flag `progress` for a background save at frame end. Cheap — the
    /// actual JSON serialize + write happens off-thread via
    /// [`persistence::ProfileSaver`]. The cache is updated when the
    /// flush enqueues a snapshot, which is fine because nothing
    /// `load_profile`s mid-frame between event handlers.
    pub(crate) fn mark_profile_dirty(&mut self) {
        self.profile_dirty = true;
    }

    /// Frame-end flush: hand a snapshot of `progress` to the saver
    /// thread iff something marked it dirty. Resets the flag.
    pub(crate) fn flush_dirty_profile(&mut self) {
        if self.profile_dirty {
            self.profile_saver
                .enqueue(self.active_profile, &self.progress);
            self.profile_dirty = false;
        }
    }

    /// Quit / window-close hand-off: synchronously persist `progress`
    /// after stopping the background saver. The saver is shut down
    /// first so any pending older snapshot in its channel can't land
    /// on disk after the synchronous write.
    pub(crate) fn save_profile_sync_for_exit(&mut self) {
        self.profile_saver.shutdown();
        if let Err(e) = crate::persistence::save_profile(self.active_profile, &self.progress) {
            log::warn!("save_profile (exit) failed: {e}");
        }
        self.profile_dirty = false;
    }

    pub(crate) fn toggle_fullscreen(
        &mut self,
        shell: &mut crate::sdl_shell::SdlShell,
    ) -> anyhow::Result<()> {
        let on = shell.desktop_fullscreen_on();
        shell.set_desktop_fullscreen(!on)?;
        let mut settings = crate::persistence::load_settings();
        settings.borderless_fullscreen = shell.desktop_fullscreen_on();
        let _ = crate::persistence::save_settings(&settings);
        Ok(())
    }

    pub(crate) fn wants_fullscreen_shortcut(
        &self,
        scancode: Option<Scancode>,
        keymod: Mod,
        repeat: bool,
    ) -> bool {
        if repeat {
            return false;
        }
        let Some(code) = scancode else {
            return false;
        };

        #[cfg(target_os = "windows")]
        {
            let no_extra_modifiers = !(keymod.contains(Mod::LCTRLMOD | Mod::RCTRLMOD)
                || keymod.contains(Mod::LSHIFTMOD | Mod::RSHIFTMOD)
                || keymod.contains(Mod::LGUIMOD | Mod::RGUIMOD));
            keymod.contains(Mod::LALTMOD | Mod::RALTMOD)
                && no_extra_modifiers
                && matches!(code, Scancode::Return | Scancode::KpEnter)
        }

        #[cfg(target_os = "macos")]
        {
            if code != Scancode::F {
                return false;
            }
            let disallowed_mod = keymod.contains(Mod::LCTRLMOD)
                || keymod.contains(Mod::RCTRLMOD)
                || keymod.contains(Mod::LALTMOD)
                || keymod.contains(Mod::RALTMOD)
                || keymod.contains(Mod::LGUIMOD)
                || keymod.contains(Mod::RGUIMOD)
                || keymod.contains(Mod::LSHIFTMOD)
                || keymod.contains(Mod::RSHIFTMOD);
            if disallowed_mod {
                return false;
            }
            crate::macos_fullscreen_shortcut::fn_modifier_held()
        }

        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            let _ = code;
            let _ = keymod;
            false
        }
    }

    /// Switch to a different profile, reloading progress.
    pub(crate) fn switch_profile(&mut self, new_index: usize) {
        // Save current profile + any in-progress run before swapping out.
        let _ = crate::persistence::save_profile(self.active_profile, &self.progress);
        self.persist_run_if_in_progress();
        self.active_profile = new_index;
        crate::persistence::clear_profile_cache_slot(new_index);
        self.progress = crate::persistence::load_profile(new_index);
        // Resume the new profile's saved run if it has one — otherwise a
        // fresh demo run, exactly like first-launch behavior.
        let loaded_run = crate::persistence::load_run(new_index);
        self.resume_scene = loaded_run
            .as_ref()
            .map(|saved| saved.scene)
            .unwrap_or(crate::persistence::ResumeScene::Gameplay);
        self.run = loaded_run
            .map(|saved| saved.run)
            .unwrap_or_else(RunState::new_demo);
        let mut settings = crate::persistence::load_settings();
        self.run
            .set_auto_cash_in_on_full_structure(settings.auto_cash_in_on_full_structure);
        self.run.apply_progression(&self.progress);
        self.steam.sync_profile_stats(&self.progress);
        // Persist the active profile choice.
        settings.active_profile = new_index;
        let _ = crate::persistence::save_settings(&settings);
        self.archive_last_seen_run_len = settings.archive_last_seen_run_len;
    }

    /// Persist `self.run` for resume on next launch. Called from every quit
    /// path so the player can resume regardless of how the game was closed.
    /// If the run is fresh (default starting state — e.g. the player started
    /// a new game then quit immediately), the saved-run file is deleted
    /// instead of overwritten. Otherwise the existing save would still
    /// linger and we'd resume into a stale run on next launch.
    pub(crate) fn persist_run_if_in_progress(&self) {
        if self.run.is_in_progress() {
            let scene = Self::saved_resume_scene_for(&self.scene).unwrap_or(self.resume_scene);
            if let Err(e) = crate::persistence::save_run(self.active_profile, &self.run, scene) {
                log::warn!("save_run failed: {e}");
            }
        } else {
            crate::persistence::delete_saved_run(self.active_profile);
        }
    }
}
