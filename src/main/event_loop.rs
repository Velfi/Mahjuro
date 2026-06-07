use super::*;

use std::time::{Duration, Instant};

use crate::physical_size::PhysicalSize;
use crate::sdl_shell::SdlShell;
use sdl3::event::Event;
use sdl3::keyboard::{Mod, Scancode};
use sdl3::mouse::MouseButton as SdlMouseButton;

fn mod_shift(m: Mod) -> bool {
    m.contains(Mod::LSHIFTMOD | Mod::RSHIFTMOD)
}

fn mod_ctrl(m: Mod) -> bool {
    m.contains(Mod::LCTRLMOD | Mod::RCTRLMOD)
}

fn mod_gui(m: Mod) -> bool {
    m.contains(Mod::LGUIMOD | Mod::RGUIMOD)
}

use sdl3::gamepad::Button as GpButton;

fn poll_loading_logo_skip(shell: &mut SdlShell) {
    for event in shell.pump.poll_iter() {
        match event {
            Event::KeyDown {
                scancode: Some(sc),
                repeat: false,
                ..
            } if matches!(
                sc,
                Scancode::Space | Scancode::Return | Scancode::KpEnter | Scancode::Backspace
            ) =>
            {
                crate::render::wgpu_renderer::loading_screen::request_skip();
            }
            Event::ControllerButtonDown {
                button: GpButton::South | GpButton::East,
                ..
            } => {
                crate::render::wgpu_renderer::loading_screen::request_skip();
            }
            Event::MouseButtonDown {
                mouse_btn: SdlMouseButton::Left,
                ..
            } => {
                crate::render::wgpu_renderer::loading_screen::request_skip();
            }
            _ => {}
        }
    }
}

fn poll_loading_during_init(shell: &mut SdlShell, audio: &mut crate::audio::AudioManager) {
    poll_loading_logo_skip(shell);
    if crate::render::wgpu_renderer::loading_screen::take_production_logo_stinger() {
        audio.play_sfx(crate::audio::SfxId::ProductionLogo);
    }
}

impl App {
    pub(crate) fn try_play_production_logo_stinger(&mut self) {
        if crate::render::wgpu_renderer::loading_screen::take_production_logo_stinger() {
            self.audio.play_sfx(crate::audio::SfxId::ProductionLogo);
        }
    }

    pub fn run_sdl_main(mut self, shell: &mut SdlShell) -> anyhow::Result<()> {
        // Explicit pre-wgpu startup scope so wall-time gaps are attributable in
        // startup_profile output (window policy + CPU prefetch kickoff + init config).
        let (window, hdr_enabled, show_window_during_init) = {
            let _pre_wgpu = crate::startup_profile::scope("startup.pre_wgpu");
            // macOS: hide during renderer init to avoid Shown-without-focus races. Elsewhere
            // (Steam Deck / gamescope) keep the window visible and present a boot clear frame
            // as soon as the swapchain exists so launch is not a blank screen for ~10s+.
            #[cfg(target_os = "macos")]
            {
                let _scope = crate::startup_profile::scope("startup.pre_wgpu.window_visibility");
                shell.window.hide();
            }
            #[cfg(not(target_os = "macos"))]
            {
                let _scope = crate::startup_profile::scope("startup.pre_wgpu.window_visibility");
                shell.window.show();
            }

            // Mount the `rooms` pack off-thread so pre-wgpu boot can overlap
            // this I/O/index work with renderer init.
            {
                let _scope = crate::startup_profile::scope("startup.pre_wgpu.submit_pack_mount");
                crate::render::loader_pool::submit_pack_mount(|| {
                    mahjuro_assets::asset_path::prefetch_rooms_pack_once();
                });
            }
            {
                let _scope = crate::startup_profile::scope("startup.pre_wgpu.main_menu_prefetch");
                crate::render::room_preload::start_main_menu_cpu_prefetch();
            }

            // Match [`draw::App::draw`]: HDR swapchain is only used when both the
            // options toggle and `EffectLayers::hdr` allow it. Baseline builds keep
            // `hdr` off, so seeding the surface from `gfx.hdr_enabled` alone forced
            // an HDR swapchain at init then an immediate SDR reconfigure on frame
            // 1 — a redundant Metal surface transition linked to intermittent black
            // startup frames on macOS.
            {
                let _scope = crate::startup_profile::scope("startup.pre_wgpu.window_clone_and_hdr");
                (
                    shell.window.clone(),
                    self.effect_layers.hdr_enabled(&self.gfx),
                    !cfg!(target_os = "macos"),
                )
            }
        };
        let renderer = {
            let _wgpu = crate::startup_profile::scope("wgpu.renderer_new");
            let mut poll_boot = || poll_loading_during_init(shell, &mut self.audio);
            WgpuRenderer::new_windowed(
                window,
                hdr_enabled,
                show_window_during_init,
                Some(&mut poll_boot),
            )?
        };
        self.renderer = Some(renderer);
        let continue_warmup = self.continue_room_warmup();
        let run_in_progress = self.run.is_in_progress();
        let resume_scene = self.resume_scene;
        if let Some(renderer) = self.renderer.as_mut() {
            let settings = crate::persistence::load_settings();
            if !settings.graphics_mode_user_set {
                self.gfx.graphics_mode = renderer.suggested_graphics_mode();
            }
            renderer.set_graphics_budget(self.gfx.graphics_mode);
            use crate::persistence::ResumeScene;
            use crate::render::room_preload::RoomSceneChain;
            // Begin hub GLB GPU upload as soon as the renderer exists (splash plate).
            renderer.tick_splash_hub_boot();
            // Eager CPU decode for all hub/run rooms (respects concurrent decode cap).
            crate::render::room_preload::kick_eager_all_room_cpu_prefetches();
            // Shop is always first in the hub chain; start CPU decode early.
            renderer.prefetch_room_chain_next(RoomSceneChain::Shop);
            if run_in_progress {
                mahjuro_assets::asset_path::prefetch_gameplay_bulk_pack_once();
                crate::render::room_preload::kick_continue_run_cpu_prefetches(continue_warmup);
                match resume_scene {
                    ResumeScene::Gameplay => {
                        renderer.prefetch_room_chain_next(RoomSceneChain::Hallway);
                        renderer.prefetch_room_chain_next(RoomSceneChain::Gameplay);
                    }
                    ResumeScene::Hallway => {
                        renderer.prefetch_room_chain_next(RoomSceneChain::Hallway);
                        renderer.prefetch_room_chain_next(RoomSceneChain::Gameplay);
                    }
                    ResumeScene::Shop => {
                        renderer.prefetch_room_chain_next(RoomSceneChain::Shop);
                    }
                }
            }
        }
        self.input = {
            let _input = crate::startup_profile::scope("input.new");
            Some(InputState::new()?)
        };
        let (w0, h0) = shell.drawable_size();
        self.last_drawable_px = PhysicalSize::new(w0.max(1), h0.max(1));
        #[cfg(debug_menu_enabled)]
        {
            self.debug.menu = Some(DebugMenuBar::new(&shell.window));
        }
        shell.window.show();
        shell.raise_to_foreground();
        crate::startup_profile::report_sync_boot();
        if self.renderer.as_ref().is_some_and(|r| !r.is_loading()) {
            crate::startup_profile::note_async_boot_complete();
        }
        log::debug!("SDL shell: window + wgpu + input ready");

        'running: loop {
            self.dist.tick();
            if self.quit_requested {
                break 'running;
            }
            let events: Vec<Event> = shell.pump.poll_iter().collect();
            for event in events {
                if self.dispatch_sdl_event(shell, event)? {
                    break 'running;
                }
            }
            if self.quit_requested {
                break 'running;
            }
            if std::env::var_os("SteamTenfoot").is_some() && !shell.desktop_fullscreen_on() {
                let _ = shell.set_desktop_fullscreen(true);
            }
            let prev = self.last_drawable_px;
            let (dw, dh) = shell.drawable_size();
            self.last_drawable_px = PhysicalSize::new(dw.max(1), dh.max(1));
            if prev != self.last_drawable_px
                && let Some(r) = self.renderer.as_mut()
            {
                r.resize(self.last_drawable_px);
            }
            // Splash dismissal runs in `frame_tick` (`SplashScene::update` + scene switch).
            // If we only pre-baked the showcase atlas while backgrounded, `loading_done`
            // becomes true but the splash never advances — keep ticking through splash even
            // when the window has no input/mouse focus (and no gamepad).
            //
            // Also keep ticking while a scene fade is in flight. Without this, a launch that
            // starts unfocused can swap Splash -> MainMenu at `transition_alpha == 0` and then
            // stop ticking, leaving the menu covered by a frozen black fade until refocus.
            let transition_in_flight = self.pending_scene_intent.is_some()
                || self.pending_scene.is_some()
                || self.transition_alpha < 1.0;
            let window_foreground = shell.window_is_foreground();
            if window_foreground || matches!(self.scene, Scene::Splash(_)) || transition_in_flight {
                self.frame_tick(shell);
            } else {
                // Idle cheaply while backgrounded: no catch-up simulation tick, no draw.
                // Still drain async relic/background uploads and run the splash decal
                // atlas pre-bake so boot loading does not stall until the window refocuses.
                let mut did_loader_work = false;
                let continue_warmup = self.continue_room_warmup();
                let scene_transition_unblocked = self.scene_transition_unblocked();
                if let Some(renderer) = self.renderer.as_mut() {
                    if renderer.is_loading() {
                        renderer.poll_pending_texture_uploads();
                        did_loader_work = true;
                    }
                    if matches!(self.scene, Scene::Splash(_)) {
                        let tileset = self.gfx.tileset_name.clone();
                        if !renderer.splash_hub_boot_ready(&tileset) {
                            renderer.ensure_active_showcase_decal_atlas(&tileset);
                            did_loader_work = true;
                        }
                    }
                    if matches!(self.scene, Scene::Splash(_)) {
                        renderer.tick_splash_hub_boot();
                        did_loader_work = true;
                    }
                    if matches!(
                        self.scene,
                        Scene::MainMenu(_) | Scene::Shop(_) | Scene::Hallway(_)
                    ) {
                        let pending_scene_key = self
                            .pending_scene
                            .as_ref()
                            .and_then(crate::scenes::active_scene_key)
                            .or_else(|| {
                                self.pending_scene_intent
                                    .as_ref()
                                    .and_then(|intent| intent.scene_key())
                            });
                        let pending_transition_at_black = (self.pending_scene_intent.is_some()
                            || self.pending_scene.is_some())
                            && self.transition_alpha <= 0.0
                            && scene_transition_unblocked;
                        renderer.poll_room_prefetch_gpu_uploads(
                            crate::scenes::active_scene_key(&self.scene),
                            self.last_frame_dt * 1000.0,
                            continue_warmup,
                            pending_scene_key,
                            pending_transition_at_black,
                        );
                        did_loader_work = true;
                    }
                }
                let now = Instant::now();
                self.last_frame = now;
                self.last_frame_dt = 1.0 / 60.0;
                if did_loader_work {
                    std::thread::sleep(Duration::from_millis(16));
                } else {
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        }

        if self.close_saved {
            return Ok(());
        }
        log::debug!("SDL loop exit — saving profile");
        self.progress.record_score(self.run.round_score);
        self.save_profile_sync_for_exit();
        self.persist_run_if_in_progress();
        Ok(())
    }

    /// Returns `true` when the outer loop should exit.
    fn dispatch_sdl_event(&mut self, shell: &mut SdlShell, event: Event) -> anyhow::Result<bool> {
        let our_win = shell.window.id();
        match event {
            Event::Quit { .. } => {
                log::debug!("Quit event — saving and exiting");
                self.progress.record_score(self.run.round_score);
                self.save_profile_sync_for_exit();
                self.persist_run_if_in_progress();
                self.close_saved = true;
                return Ok(true);
            }
            Event::Window {
                window_id,
                win_event,
                ..
            } if window_id == our_win => {
                use sdl3::event::WindowEvent;
                match win_event {
                    WindowEvent::CloseRequested => {
                        if self.close_saved {
                            log::debug!("CloseRequested again — exiting immediately");
                            return Ok(true);
                        }
                        log::debug!("CloseRequested — saving profile and exiting");
                        self.progress.record_score(self.run.round_score);
                        self.save_profile_sync_for_exit();
                        self.persist_run_if_in_progress();
                        self.close_saved = true;
                        return Ok(true);
                    }
                    WindowEvent::Resized(_, _) => {
                        let (w, h) = shell.drawable_size();
                        let sz = PhysicalSize::new(w.max(1), h.max(1));
                        self.last_drawable_px = sz;
                        if let Some(r) = self.renderer.as_mut() {
                            r.resize(sz);
                        }
                    }
                    _ => {}
                }
            }
            Event::MouseMotion {
                window_id, x, y, ..
            } if window_id == our_win || window_id == 0 => {
                let (px, py) = shell.event_xy_to_pixels(x, y);
                self.sdl_handle_mouse_motion(shell, px, py);
            }
            Event::MouseButtonDown {
                window_id,
                mouse_btn,
                ..
            } if window_id == our_win || window_id == 0 => {
                if mouse_btn == SdlMouseButton::Left {
                    self.sdl_handle_left_button(shell, true);
                } else if mouse_btn == SdlMouseButton::Right {
                    self.sdl_handle_right_button(shell, true);
                }
            }
            Event::MouseButtonUp {
                window_id,
                mouse_btn,
                ..
            } if window_id == our_win || window_id == 0 => {
                if mouse_btn == SdlMouseButton::Left {
                    self.sdl_handle_left_button(shell, false);
                }
            }
            Event::MouseWheel { window_id, y, .. } if window_id == our_win || window_id == 0 => {
                self.scroll_delta += y;
            }
            Event::KeyDown {
                window_id,
                scancode,
                keymod,
                repeat,
                ..
            } if window_id == our_win || window_id == 0 => {
                self.modifiers = keymod;
                self.sdl_key_down(shell, scancode, keymod, repeat)?;
            }
            Event::KeyUp {
                window_id,
                scancode,
                keymod,
                ..
            } if window_id == our_win || window_id == 0 => {
                self.modifiers = keymod;
                self.sdl_key_up(shell, scancode);
            }
            event => {
                if matches!(
                    event,
                    Event::ControllerButtonDown { .. }
                        | Event::ControllerButtonUp { .. }
                        | Event::ControllerAxisMotion { .. }
                        | Event::ControllerDeviceAdded { .. }
                        | Event::ControllerDeviceRemoved { .. }
                        | Event::ControllerDeviceRemapped { .. }
                ) {
                    self.dispatch_controller_event(shell, event);
                }
            }
        }
        Ok(false)
    }

    fn dispatch_controller_event(&mut self, shell: &mut SdlShell, event: Event) {
        let gp_ctx = self.gamepad_poll_ctx();
        if let Some(input) = self.input.as_mut() {
            let _ = input.handle_controller_event(shell, event, gp_ctx, &mut self.mouse_actions);
        }
    }

    fn gamepad_poll_ctx(&self) -> crate::ui::input::GamepadPollCtx {
        let showcase_orbit_overlay = self
            .overlay_stack
            .last()
            .is_some_and(|top| matches!(top, Scene::Showcase(s) if s.wants_orbit_input()));
        crate::ui::input::GamepadPollCtx {
            face_bindings: self.active_face_bindings(),
            item_inspect_overlay: showcase_orbit_overlay,
            shop_storeroom_orbit: self.shop_storeroom_face_active(),
        }
    }

    fn sdl_handle_right_button(&mut self, _shell: &mut SdlShell, down: bool) {
        if down && self.shop_storeroom_face_active() {
            self.mouse_right_clicked = true;
            if let Some(input) = self.input.as_mut() {
                input.mode = InputMode::Cursor;
            }
        }
    }

    fn sdl_handle_left_button(&mut self, _shell: &mut SdlShell, down: bool) {
        let cursor = self
            .input
            .as_ref()
            .map(|i| i.last_cursor)
            .unwrap_or((0.0, 0.0));
        let defer_shop_click = self.shop_defer_lmb_clicks();

        if down {
            self.mouse_left_down = true;
            self.mouse_clicked = true;
            if let Some(input) = self.input.as_mut() {
                input.mode = InputMode::Cursor;
            }
            if defer_shop_click {
                self.mouse_left_press_cursor = Some(cursor);
                self.deferred_lmb_button_click = None;
            }

            // Check if click hit any button.
            let mut hit = false;
            for btn in &self.active_buttons {
                let (bx, by, bw, bh) = btn.rect;
                if cursor.0 >= bx && cursor.0 <= bx + bw && cursor.1 >= by && cursor.1 <= by + bh {
                    self.audio.play_sfx(audio::SfxId::TileClick);
                    match btn.action {
                        ButtonAction::Ui(a) => self.mouse_actions.push(a),
                        ButtonAction::Scene(id) => {
                            if defer_shop_click {
                                self.deferred_lmb_button_click = Some(id);
                            } else {
                                self.mouse_button_clicks.push(id);
                            }
                        }
                    }
                    hit = true;
                    break;
                }
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
        } else {
            self.mouse_left_down = false;
            if self.shop_defer_lmb_clicks() {
                let click_ok = self
                    .mouse_left_press_cursor
                    .map(|(x0, y0)| {
                        let dx = cursor.0 - x0;
                        let dy = cursor.1 - y0;
                        dx * dx + dy * dy < 100.0
                    })
                    .unwrap_or(true);
                if click_ok && let Some(id) = self.deferred_lmb_button_click.take() {
                    self.mouse_button_clicks.push(id);
                }
                self.mouse_left_press_cursor = None;
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
                                self.run.relics.swap_relics(drag.from_slot, target_slot);
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
    }

    fn sdl_handle_mouse_motion(&mut self, shell: &mut SdlShell, x: f32, y: f32) {
        let new_cursor = (x, y);
        let showcase_orbit = self
            .overlay_stack
            .last()
            .is_some_and(|top| matches!(top, Scene::Showcase(s) if s.wants_orbit_input()));
        let shop_storeroom_orbit = self.shop_storeroom_face_active() && self.mouse_left_down;
        if let Some((x0, y0)) = self.mouse_left_press_cursor {
            let ddx = new_cursor.0 - x0;
            let ddy = new_cursor.1 - y0;
            if ddx * ddx + ddy * ddy > 100.0 {
                self.deferred_lmb_button_click = None;
            }
        }
        if let Some(input) = self.input.as_mut() {
            let prev_mode = input.mode;
            let dx = new_cursor.0 - input.last_cursor.0;
            let dy = new_cursor.1 - input.last_cursor.1;
            let dist_sq = dx * dx + dy * dy;
            // Keyboard → cursor: small movement restores mouse modality (same as before).
            const CURSOR_MODE_MOUSE_MOVE_SQ: f32 = 4.0;
            // Controller → cursor: require a larger delta so stick / driver jitter
            // does not exit controller mode; intentional mouse motion still restores hover.
            const CONTROLLER_TO_CURSOR_MOUSE_MOVE_SQ: f32 = 100.0;
            let switch_to_cursor = match input.mode {
                InputMode::Controller => dist_sq > CONTROLLER_TO_CURSOR_MOUSE_MOVE_SQ,
                InputMode::Keyboard => dist_sq > CURSOR_MODE_MOUSE_MOVE_SQ,
                InputMode::Cursor => false,
            };
            if switch_to_cursor {
                input.mode = InputMode::Cursor;
            }
            let was_hidden = switch_to_cursor && prev_mode != InputMode::Cursor;
            input.last_cursor = new_cursor;
            let layout = self.layout_engine.solve(
                self.last_drawable_px.width as f32,
                self.last_drawable_px.height as f32,
            );
            // Same raycast-based pick as the per-frame update path.
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
            // Showcase item inspect: LMB drag orbits the subject.
            if showcase_orbit && self.mouse_left_down {
                input.accum_item_inspect_mouse_orbit(-dx, dy);
            }
            // Storeroom shop (pre-inspect): LMB drag orbits the room camera.
            if shop_storeroom_orbit {
                input.accum_shop_storeroom_mouse_orbit(-dx, dy);
            }
            // Update drag position if dragging.
            if let Some(ref mut drag) = input.drag {
                drag.current_pos = input.last_cursor;
            }
            if was_hidden {
                shell.show_cursor(true);
            }
        }
    }

    fn sdl_key_down(
        &mut self,
        shell: &mut SdlShell,
        scancode: Option<Scancode>,
        keymod: Mod,
        repeat: bool,
    ) -> anyhow::Result<()> {
        self.modifiers = keymod;
        #[cfg(debug_menu_enabled)]
        if let Some(action) =
            crate::debug_menu::debug_action_for_keyboard_shortcut(scancode, keymod, repeat)
        {
            self.handle_debug_action(action);
            return Ok(());
        }
        if let Some(ref mut o) = self.debug.rain_debug_overlay
            && o.hide_all_ui
            && o.feed_key_event(scancode, false)
        {
            return Ok(());
        }
        if self.wants_fullscreen_shortcut(scancode, keymod, repeat) {
            let _ = self.toggle_fullscreen(shell);
            return Ok(());
        }
        if let Some(ref mut o) = self.debug.hallway_distortion_debug_overlay {
            let ctrl = mod_ctrl(self.modifiers) || mod_gui(self.modifiers);
            if o.feed_key_event(scancode, ctrl) {
                return Ok(());
            }
        }
        if let Some(ref mut o) = self.debug.scene_look_debug_overlay {
            let ctrl = mod_ctrl(self.modifiers) || mod_gui(self.modifiers);
            if o.feed_key_event(scancode, ctrl) {
                return Ok(());
            }
        }
        if let Some(ref mut o) = self.debug.rain_debug_overlay {
            let ctrl = mod_ctrl(self.modifiers) || mod_gui(self.modifiers);
            if o.feed_key_event(scancode, ctrl) {
                return Ok(());
            }
        }
        if let Some(ref mut o) = self.debug.flame_debug_overlay {
            let ctrl = mod_ctrl(self.modifiers) || mod_gui(self.modifiers);
            if o.feed_key_event(scancode, ctrl) {
                return Ok(());
            }
        }
        if let Some(ref mut o) = self.debug.victory_moon_debug_overlay {
            let ctrl = mod_ctrl(self.modifiers) || mod_gui(self.modifiers);
            if o.feed_key_event(scancode, ctrl) {
                return Ok(());
            }
        }
        if let Some(crate::scenes::Scene::CascadeLab(lab)) = self.overlay_stack.last_mut() {
            let shift = mod_shift(self.modifiers);
            if lab.feed_structure_key(scancode, shift) {
                return Ok(());
            }
        }

        let mut v = Vec::new();
        let shift = mod_shift(self.modifiers);
        let orbit_overlay_active = self
            .overlay_stack
            .last()
            .is_some_and(|top| matches!(top, Scene::Showcase(s) if s.wants_orbit_input()));
        // While orbit inspect is active, arrow keys are reserved for orbit
        // rotation (sampled in `gamepad_frame_tick`) rather than focus actions.
        let scancode_for_actions = if orbit_overlay_active
            && matches!(
                scancode,
                Some(Scancode::Left | Scancode::Right | Scancode::Up | Scancode::Down)
            ) {
            None
        } else {
            scancode
        };
        let mode_changed = if let Some(input) = self.input.as_mut() {
            input.on_key(scancode_for_actions, shift, &mut v)
        } else {
            false
        };
        self.mouse_actions.extend(v);
        if mode_changed {
            shell.show_cursor(false);
        }
        Ok(())
    }

    fn sdl_key_up(&mut self, _shell: &mut SdlShell, scancode: Option<Scancode>) {
        let mut v = Vec::new();
        if let Some(input) = self.input.as_mut() {
            input.on_key_release(scancode, &mut v);
        }
        self.mouse_actions.extend(v);
    }
}
