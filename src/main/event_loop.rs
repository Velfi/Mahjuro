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

impl App {
    pub fn run_sdl_main(mut self, shell: &mut SdlShell) -> anyhow::Result<()> {
        // Match [`draw::App::draw`]: HDR swapchain is only used when both the
        // options toggle and `EffectLayers::hdr` allow it. Baseline builds keep
        // `hdr` off, so seeding the surface from `gfx.hdr_enabled` alone forced
        // an HDR swapchain at init then an immediate SDR reconfigure on frame
        // 1 — a redundant Metal surface transition linked to intermittent black
        // startup frames on macOS.
        let renderer = WgpuRenderer::new(render::wgpu_renderer::TargetInit::Windowed {
            window: shell.window.clone(),
            hdr_enabled: self.effect_layers.hdr_enabled(&self.gfx),
        })?;
        self.renderer = Some(renderer);
        self.input = Some(InputState::new()?);
        let (w0, h0) = shell.drawable_size();
        self.last_drawable_px = PhysicalSize::new(w0.max(1), h0.max(1));
        #[cfg(debug_menu_enabled)]
        {
            self.debug.menu = Some(DebugMenuBar::new(&shell.window));
        }
        log::debug!("SDL shell: window + wgpu + input ready");

        'running: loop {
            self.steam.run_callbacks();
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
            if prev != self.last_drawable_px {
                if let Some(r) = self.renderer.as_mut() {
                    r.resize(self.last_drawable_px);
                }
            }
            if shell.window_is_foreground() {
                self.frame_tick(shell);
            } else {
                // Idle cheaply while backgrounded: no catch-up simulation tick, no GPU work.
                let now = Instant::now();
                self.last_frame = now;
                self.last_frame_dt = 1.0 / 60.0;
                std::thread::sleep(Duration::from_millis(50));
            }
        }

        if self.close_saved {
            return Ok(());
        }
        log::debug!("SDL loop exit — saving profile");
        self.progress.record_score(self.run.round_score);
        let _ = persistence::save_profile(self.active_profile, &self.progress);
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
                let _ = persistence::save_profile(self.active_profile, &self.progress);
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
                        let _ = persistence::save_profile(self.active_profile, &self.progress);
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
            } if window_id == our_win => {
                let (px, py) = shell.event_xy_to_pixels(x, y);
                self.sdl_handle_mouse_motion(shell, px, py);
            }
            Event::MouseButtonDown {
                window_id,
                mouse_btn,
                ..
            } if window_id == our_win => {
                if mouse_btn == SdlMouseButton::Left {
                    self.sdl_handle_left_button(shell, true);
                }
            }
            Event::MouseButtonUp {
                window_id,
                mouse_btn,
                ..
            } if window_id == our_win => {
                if mouse_btn == SdlMouseButton::Left {
                    self.sdl_handle_left_button(shell, false);
                }
            }
            Event::MouseWheel { window_id, y, .. } if window_id == our_win => {
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
        let shop_face = matches!(&self.scene, Scene::Shop(_))
            && self.overlay_stack.is_empty()
            && !self.scene.has_blocking_overlay();
        let collection_uses_north_for_inspect = matches!(&self.scene, Scene::Collection(_))
            && self.overlay_stack.is_empty()
            && !self.scene.has_blocking_overlay();
        let showcase_orbit_overlay = self
            .overlay_stack
            .last()
            .is_some_and(|top| matches!(top, Scene::Showcase(s) if s.wants_orbit_input()));
        crate::ui::input::GamepadPollCtx {
            shop_face_buttons: shop_face,
            collection_uses_north_for_inspect,
            item_inspect_overlay: showcase_orbit_overlay,
        }
    }

    fn sdl_handle_left_button(&mut self, _shell: &mut SdlShell, down: bool) {
        let cursor = self
            .input
            .as_ref()
            .map(|i| i.last_cursor)
            .unwrap_or((0.0, 0.0));

        if down {
            self.mouse_left_down = true;
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
                    Some(n) => log::debug!(
                        "Object hit test: {} at ({:.0}, {:.0})",
                        n,
                        cursor.0,
                        cursor.1
                    ),
                    None => log::debug!(
                        "Object hit test: (no object) at ({:.0}, {:.0})",
                        cursor.0,
                        cursor.1
                    ),
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
                    let picked = self
                        .renderer
                        .as_ref()
                        .and_then(|r| r.pick_debug_object_with_model(cursor.0, cursor.1));
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
                            log::info!("[Arrange] Click missed all pickables — no move");
                        }
                    }
                }
                self.mouse_clicked = false;
                return;
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
                            self.mouse_button_clicks.push(id);
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
        if let Some(input) = self.input.as_mut() {
            let new_cursor = (x, y);
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
            let hand_slot_count = self.run.hand().len().max(layout.hand_slots.len());
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
            // Showcase inspect (shop / collection): orbit with LMB drag — same stick channel as gamepad.
            let showcase_orbit = self
                .overlay_stack
                .last()
                .is_some_and(|top| matches!(top, Scene::Showcase(s) if s.wants_orbit_input()));
            if showcase_orbit && self.mouse_left_down {
                input.accum_item_inspect_mouse_orbit(dx, dy);
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
        if self.wants_fullscreen_shortcut(scancode, keymod, repeat) {
            let _ = self.toggle_fullscreen(shell);
            return Ok(());
        }
        if let Some(ref mut o) = self.debug.shop_env_debug_overlay {
            let ctrl = mod_ctrl(self.modifiers) || mod_gui(self.modifiers);
            if o.feed_key_event(scancode, ctrl) {
                return Ok(());
            }
        }

        // Arrange mode: Escape while waiting for a click exits the
        // mode entirely.
        if matches!(self.debug.arrange_mode, Some(None)) && scancode == Some(Scancode::Escape) {
            self.debug.arrange_mode = None;
            log::info!("[Arrange] Mode exited");
            return Ok(());
        }

        // Arrange mode: Tab / Shift+Tab cycles through the active
        // scene's placement hierarchy. Works whether an object is
        // already selected or not — picking a group applies deltas
        // to every descendant leaf on save.
        if self.debug.arrange_mode.is_some() && scancode == Some(Scancode::Tab) {
            let flat = arrange_hierarchy_flat(&self.scene);
            if flat.is_empty() {
                log::info!("[Arrange] Current scene has no hierarchy");
            } else {
                let current_name = match &self.debug.arrange_mode {
                    Some(Some(s)) => Some(s.object_name.as_str()),
                    _ => None,
                };
                let current_idx = current_name.and_then(|n| flat.iter().position(|e| e.name == n));
                let reverse = mod_shift(self.modifiers);
                let next_idx = match (current_idx, reverse) {
                    (None, false) => 0,
                    (None, true) => flat.len() - 1,
                    (Some(i), false) => (i + 1) % flat.len(),
                    (Some(i), true) => (i + flat.len() - 1) % flat.len(),
                };
                let entry = &flat[next_idx];
                self.debug.arrange_mode = Some(Some(ArrangeModeState {
                    object_name: entry.name.to_string(),
                    selected_world_origin: glam::Vec3::ZERO,
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
            }
            return Ok(());
        }

        // Arrange mode: when an object is selected, consume arrow
        // keys (move X/Y), Shift+arrows (rotate Z/X), Enter
        // (confirm+copy), and Escape (cancel selection). Normal
        // input path is skipped so gameplay doesn't also fire.
        if let Some(Some(ref mut state)) = self.debug.arrange_mode {
            let shift = mod_shift(self.modifiers);
            let step_px = state.trans_step_px; // pixels per key press
            let step_deg = state.rot_step_deg; // degrees per key press
            let mut handled = true;
            let mut nudged = false;
            let mut escape_pending = false;
            if let Some(code) = scancode {
                match code {
                    Scancode::_1 => {
                        state.trans_step_px = 1.0;
                        state.rot_step_deg = 1.0;
                        log::info!("[Arrange] Step 1 (1 px / 1°)");
                    }
                    Scancode::_2 => {
                        state.trans_step_px = 5.0;
                        state.rot_step_deg = 15.0;
                        log::info!("[Arrange] Step 2 (5 px / 15°)");
                    }
                    Scancode::_3 => {
                        state.trans_step_px = 25.0;
                        state.rot_step_deg = 45.0;
                        log::info!("[Arrange] Step 3 (25 px / 45°)");
                    }
                    Scancode::_4 => {
                        state.trans_step_px = 100.0;
                        state.rot_step_deg = 90.0;
                        log::info!("[Arrange] Step 4 (100 px / 90°)");
                    }
                    // Translation: WASD = forward/left/back/right, Q/E = down/up
                    Scancode::D if !shift => {
                        state.delta_px += step_px;
                        nudged = true;
                    }
                    Scancode::A if !shift => {
                        state.delta_px -= step_px;
                        nudged = true;
                    }
                    Scancode::S if !shift => {
                        state.delta_py += step_px;
                        nudged = true;
                    }
                    Scancode::W if !shift => {
                        state.delta_py -= step_px;
                        nudged = true;
                    }
                    Scancode::Q if !shift => {
                        state.delta_lift -= step_px;
                        nudged = true;
                    }
                    Scancode::E if !shift => {
                        state.delta_lift += step_px;
                        nudged = true;
                    }
                    // Rotation: Shift+A/D = rz, Shift+W/S = rx, Shift+Q/E = ry
                    Scancode::D if shift => {
                        state.delta_rz_deg += step_deg;
                        nudged = true;
                    }
                    Scancode::A if shift => {
                        state.delta_rz_deg -= step_deg;
                        nudged = true;
                    }
                    Scancode::W if shift => {
                        state.delta_rx_deg -= step_deg;
                        nudged = true;
                    }
                    Scancode::S if shift => {
                        state.delta_rx_deg += step_deg;
                        nudged = true;
                    }
                    Scancode::Q if shift => {
                        state.delta_ry_deg -= step_deg;
                        nudged = true;
                    }
                    Scancode::E if shift => {
                        state.delta_ry_deg += step_deg;
                        nudged = true;
                    }
                    Scancode::Return | Scancode::KpEnter => {
                        // Confirm: convert pixel deltas to proportional fractions
                        // so the output is screen-size independent.
                        let size = self.last_drawable_px;
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
                                    log::error!("[Arrange] Clipboard write failed: {e}");
                                } else {
                                    log::info!("[Arrange] Copied to clipboard:\n{text}");
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
                    Scancode::R if !shift => {
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
                    Scancode::Escape => {
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
                let size = self.last_drawable_px;
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
                return Ok(());
            }
            // Fall through for unhandled keys (e.g. fullscreen).
        }

        // Cross-platform debug shortcut: Ctrl+Shift+M opens the
        // material viewer pushdown scene. Mirrors the Debug menu
        // entry so Linux (where muda has no non-GTK menu) and any
        // other OS the menu doesn't reach still has access.
        if let Some(code) = scancode
            && code == Scancode::M
            && mod_shift(self.modifiers)
            && (mod_ctrl(self.modifiers) || mod_gui(self.modifiers))
        {
            if !self
                .overlay_stack
                .iter()
                .any(|s| matches!(s, Scene::MaterialViewer(_)))
            {
                self.overlay_stack
                    .push(Scene::MaterialViewer(MaterialViewerScene::new(true)));
                log::debug!("Opened material viewer (keyboard shortcut)");
            }
            return Ok(());
        }

        if let Some(code) = scancode
            && code == Scancode::H
            && mod_shift(self.modifiers)
            && (mod_ctrl(self.modifiers) || mod_gui(self.modifiers))
        {
            if !self
                .overlay_stack
                .iter()
                .any(|s| matches!(s, Scene::RumbleLab(_)))
            {
                self.overlay_stack
                    .push(Scene::RumbleLab(RumbleLabScene::new(true)));
                log::debug!("Opened rumble lab (keyboard shortcut)");
            }
            return Ok(());
        }

        let mut v = Vec::new();
        let shift = mod_shift(self.modifiers);
        let mode_changed = if let Some(input) = self.input.as_mut() {
            input.on_key(scancode, shift, &mut v)
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
