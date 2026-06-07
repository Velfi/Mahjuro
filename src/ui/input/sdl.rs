//! SDL gamepad / rumble paths (interactive builds only).

use std::time::{Duration, Instant};

use sdl3::event::Event;
use sdl3::gamepad::{Axis as GpAxis, Button as GpButton};
use sdl3::joystick::JoystickId;
use sdl3::keyboard::Scancode;

use crate::sdl_shell::SdlShell;

use super::{GamepadPollCtx, GamepadStyle, InputMode, InputState, RumbleLabOp, UiAction};

struct RumbleEnvelopeParams {
    gain: f32,
    weak: u16,
    strong: u16,
    duration_ms: u32,
    attack_ms: u32,
    fade_ms: u32,
}

const NAV_REPEAT_INITIAL_DELAY: Duration = Duration::from_millis(400);
const NAV_REPEAT_INTERVAL: Duration = Duration::from_millis(90);

fn joystick_id(raw: u32) -> JoystickId {
    JoystickId::new(raw)
}

fn axis_norm(v: i16) -> f32 {
    (v as f32 / 32767.0).clamp(-1.0, 1.0)
}

fn trigger_norm(v: i16) -> f32 {
    (v.max(0) as f32 / 32767.0).clamp(0.0, 1.0)
}

impl InputState {
    /// Run scheduled SDL rumble pulses (composite / staggered lab patterns).
    pub fn tick_scoring_rumble_keepalive(&mut self, shell: &mut SdlShell, now: Instant) {
        let mut fired: Vec<(u16, u16, u32, f32)> = Vec::new();
        self.scoring_rumble_schedule.retain(|(at, w, s, d, g)| {
            if *at <= now {
                fired.push((*w, *s, *d, *g));
                false
            } else {
                true
            }
        });
        for (w, s, d, g) in fired {
            Self::fire_sdl_rumble(shell, w, s, d, g);
        }
    }

    fn fire_sdl_rumble(shell: &mut SdlShell, weak: u16, strong: u16, duration_ms: u32, gain: f32) {
        if duration_ms == 0 {
            return;
        }
        let g = gain.clamp(0.0, 1.0);
        // SDL: `low_frequency_rumble` = heavy motor, `high_frequency` = light (typical Xbox layout).
        let low = ((strong as f32) * g).min(65535.0) as u16;
        let high = ((weak as f32) * g).min(65535.0) as u16;
        if low == 0 && high == 0 {
            return;
        }
        if shell.pads.is_empty() {
            log::warn!(
                "gamepad rumble skipped: no opened SDL gamepads (device may lack mapping or open failed)"
            );
            return;
        }
        shell.set_gamepad_rumble(low, high, duration_ms);
    }

    fn stop_sdl_rumble(shell: &mut SdlShell) {
        shell.stop_gamepad_rumble();
    }

    /// Drain rumble patterns queued by the rumble lab debug scene.
    pub fn apply_rumble_lab_ops(
        &mut self,
        shell: &mut SdlShell,
        now: Instant,
        ops: Vec<RumbleLabOp>,
    ) {
        for op in ops {
            match op {
                RumbleLabOp::Pulse {
                    weak,
                    strong,
                    duration_ms,
                    gain,
                } => self.play_scoring_rumble_pulse(shell, now, weak, strong, duration_ms, gain),
                RumbleLabOp::Composite { gain, segments } => {
                    self.play_rumble_composite(shell, now, gain, &segments);
                }
                RumbleLabOp::Envelope {
                    gain,
                    weak,
                    strong,
                    duration_ms,
                    attack_ms,
                    fade_ms,
                } => self.play_rumble_envelope(
                    shell,
                    now,
                    RumbleEnvelopeParams {
                        gain,
                        weak,
                        strong,
                        duration_ms,
                        attack_ms,
                        fade_ms,
                    },
                ),
            }
        }
    }

    fn play_rumble_composite(
        &mut self,
        shell: &mut SdlShell,
        now: Instant,
        gain: f32,
        segments: &[(u32, u16, u16, u32)],
    ) {
        if shell.pads.is_empty() || segments.is_empty() {
            return;
        }
        let g = gain.clamp(0.0, 1.0);
        for &(delay, weak, strong, dur) in segments {
            let at = now + Duration::from_millis(u64::from(delay));
            self.scoring_rumble_schedule
                .push((at, weak, strong, dur.max(1), g));
        }
    }

    fn play_rumble_envelope(
        &mut self,
        shell: &mut SdlShell,
        now: Instant,
        params: RumbleEnvelopeParams,
    ) {
        let RumbleEnvelopeParams {
            gain,
            weak,
            strong,
            duration_ms,
            attack_ms,
            fade_ms,
        } = params;
        let min_gap_ticks = 3u32;
        let dur_tick_u32 = duration_ms.max(60).div_ceil(50).max(2);
        let atk_tick_u32 = attack_ms.div_ceil(50);
        let fade_tick_u32 = fade_ms.div_ceil(50);
        if atk_tick_u32 + fade_tick_u32 + min_gap_ticks >= dur_tick_u32 {
            self.play_scoring_rumble_pulse(shell, now, weak, strong, duration_ms.max(60), gain);
            return;
        }
        // SDL rumble has no attack/fade envelope — single pulse is the closest match.
        self.play_scoring_rumble_pulse(shell, now, weak, strong, duration_ms.max(60), gain);
    }

    /// Fire-and-forget scoring cascade pulse on connected gamepads.
    pub fn play_scoring_rumble_pulse(
        &mut self,
        shell: &mut SdlShell,
        _now: Instant,
        weak: u16,
        strong: u16,
        duration_ms: u32,
        gain: f32,
    ) {
        Self::fire_sdl_rumble(shell, weak, strong, duration_ms, gain);
    }

    /// Drive shop hold-to-sell rumble (same master toggle as scoring-cascade rumble).
    /// Call once per frame after scene update, only while the unobstructed shop face is active.
    /// When `active` is false this stops motors — do not call from other scenes or overlays
    /// or you will cancel unrelated rumble. `hold_progress` is ignored unless `active`.
    pub fn sync_shop_sell_hold_rumble(
        &mut self,
        shell: &mut SdlShell,
        active: bool,
        controller: bool,
        rumble_enabled: bool,
        hold_progress: f32,
    ) {
        if !active || !controller || !rumble_enabled {
            Self::stop_sdl_rumble(shell);
            return;
        }

        if shell.pads.is_empty() {
            return;
        }

        let (weak, strong, hold_refresh_ms, gain) =
            Self::shop_sell_hold_rumble_params(hold_progress);
        let low = ((strong as f32) * gain).min(65535.0) as u16;
        let high = ((weak as f32) * gain).min(65535.0) as u16;
        shell.set_gamepad_rumble(low, high, hold_refresh_ms);
    }
    /// Handle one SDL controller event from the shared [`SdlShell`] pump.
    /// Returns true when focus mode switches to [`InputMode::Controller`].
    pub fn handle_controller_event(
        &mut self,
        shell: &mut SdlShell,
        event: Event,
        poll_ctx: GamepadPollCtx,
        actions: &mut Vec<UiAction>,
    ) -> bool {
        let before = actions.len();

        const STICK_DEADZONE: f32 = 0.65;
        const TRIG_PRESS: f32 = 0.65;

        match event {
            Event::ControllerDeviceAdded { .. }
            | Event::ControllerDeviceRemoved { .. }
            | Event::ControllerDeviceRemapped { .. } => {
                shell.refresh_gamepads();
            }
            Event::ControllerButtonDown { button, .. } => match button {
                GpButton::South => actions.push(if self.swap_ab {
                    UiAction::Cancel
                } else {
                    UiAction::Confirm
                }),
                GpButton::East => actions.push(if self.swap_ab {
                    UiAction::Confirm
                } else {
                    UiAction::Cancel
                }),
                GpButton::West => {
                    if let Some(action) = poll_ctx
                        .face_bindings
                        .face_press(GpButton::West, self.swap_xy)
                    {
                        actions.push(action);
                    }
                }
                GpButton::North => {
                    if let Some(action) = poll_ctx
                        .face_bindings
                        .face_press(GpButton::North, self.swap_xy)
                    {
                        actions.push(action);
                    }
                }
                GpButton::DPadRight => {
                    actions.push(UiAction::FocusNext);
                    self.dpad_repeat = Some((
                        UiAction::FocusNext,
                        Instant::now() + NAV_REPEAT_INITIAL_DELAY,
                    ));
                }
                GpButton::DPadLeft => {
                    actions.push(UiAction::FocusPrev);
                    self.dpad_repeat = Some((
                        UiAction::FocusPrev,
                        Instant::now() + NAV_REPEAT_INITIAL_DELAY,
                    ));
                }
                GpButton::DPadDown => {
                    actions.push(UiAction::FocusDown);
                    self.dpad_repeat = Some((
                        UiAction::FocusDown,
                        Instant::now() + NAV_REPEAT_INITIAL_DELAY,
                    ));
                }
                GpButton::DPadUp => {
                    actions.push(UiAction::FocusUp);
                    self.dpad_repeat =
                        Some((UiAction::FocusUp, Instant::now() + NAV_REPEAT_INITIAL_DELAY));
                }
                GpButton::Start => actions.push(UiAction::Pause),
                // Select / View / Share / − (platform-dependent label).
                GpButton::Back | GpButton::Touchpad | GpButton::Misc1 => {
                    actions.push(UiAction::Help);
                }
                GpButton::LeftStick => actions.push(UiAction::InvertSelection),
                GpButton::LeftShoulder => {
                    actions.push(UiAction::NavigateHudPrev);
                    actions.push(UiAction::TabPrev);
                }
                GpButton::RightShoulder => {
                    actions.push(UiAction::NavigateHudNext);
                    actions.push(UiAction::TabNext);
                }
                _ => {}
            },
            Event::ControllerButtonUp { button, .. } => match button {
                GpButton::South => {
                    if self.swap_ab {
                        actions.push(UiAction::CancelRelease);
                    } else {
                        actions.push(UiAction::ConfirmRelease);
                    }
                }
                GpButton::East => {
                    if self.swap_ab {
                        actions.push(UiAction::ConfirmRelease);
                    } else {
                        actions.push(UiAction::CancelRelease);
                    }
                }
                GpButton::West => {
                    if let Some(action) = poll_ctx
                        .face_bindings
                        .face_release(GpButton::West, self.swap_xy)
                    {
                        actions.push(action);
                    }
                }
                GpButton::North => {
                    if let Some(action) = poll_ctx
                        .face_bindings
                        .face_release(GpButton::North, self.swap_xy)
                    {
                        actions.push(action);
                    }
                }
                _ => {}
            },
            Event::ControllerAxisMotion {
                which, axis, value, ..
            } => {
                let id = joystick_id(which);
                let v = axis_norm(value);
                match axis {
                    GpAxis::LeftX => {
                        let old_dir = self.left_stick_x_dir;
                        let new_dir = if v >= STICK_DEADZONE {
                            1
                        } else if v <= -STICK_DEADZONE {
                            -1
                        } else {
                            0
                        };
                        self.left_stick_x_dir = new_dir;
                        if new_dir == 0 {
                            self.stick_repeat_x = None;
                        } else if new_dir != old_dir {
                            actions.push(if new_dir > 0 {
                                UiAction::FocusNext
                            } else {
                                UiAction::FocusPrev
                            });
                            self.last_stick_nav_at = Instant::now();
                            self.stick_repeat_x =
                                Some((new_dir, Instant::now() + NAV_REPEAT_INITIAL_DELAY));
                        }
                    }
                    GpAxis::LeftY => {
                        let old_dir = self.left_stick_y_dir;
                        let new_dir = if v >= STICK_DEADZONE {
                            -1
                        } else if v <= -STICK_DEADZONE {
                            1
                        } else {
                            0
                        };
                        self.left_stick_y_dir = new_dir;
                        if new_dir == 0 {
                            self.stick_repeat_y = None;
                        } else if new_dir != old_dir {
                            actions.push(if new_dir > 0 {
                                UiAction::FocusUp
                            } else {
                                UiAction::FocusDown
                            });
                            self.last_stick_nav_at = Instant::now();
                            self.stick_repeat_y =
                                Some((new_dir, Instant::now() + NAV_REPEAT_INITIAL_DELAY));
                        }
                    }
                    GpAxis::TriggerLeft => {
                        let cur = trigger_norm(value);
                        let prev = shell.lt_prev.get(&id).copied().unwrap_or(0.0);
                        if !poll_ctx.face_bindings.suppress_trigger_structure {
                            if prev < TRIG_PRESS && cur >= TRIG_PRESS {
                                actions.push(UiAction::TriggerStructure);
                            } else if prev >= TRIG_PRESS && cur < TRIG_PRESS {
                                actions.push(UiAction::TriggerStructureRelease);
                            }
                        }
                        shell.lt_prev.insert(id, cur);
                    }
                    GpAxis::TriggerRight => {
                        let cur = trigger_norm(value);
                        let prev = shell.rt_prev.get(&id).copied().unwrap_or(0.0);
                        if !poll_ctx.face_bindings.suppress_trigger_structure {
                            if prev < TRIG_PRESS && cur >= TRIG_PRESS {
                                actions.push(UiAction::TriggerStructure);
                            } else if prev >= TRIG_PRESS && cur < TRIG_PRESS {
                                actions.push(UiAction::TriggerStructureRelease);
                            }
                        }
                        shell.rt_prev.insert(id, cur);
                    }
                    _ => {}
                }
            }
            _ => {}
        }

        if actions.len() > before && self.mode != InputMode::Controller {
            self.mode = InputMode::Controller;
            return true;
        }
        false
    }

    /// Once per frame after SDL events: controller subsystem refresh, held-nav repeats,
    /// inspect analog sampling. Returns true when switching to [`InputMode::Controller`].
    pub fn gamepad_frame_tick(
        &mut self,
        shell: &mut SdlShell,
        poll_ctx: GamepadPollCtx,
        actions: &mut Vec<UiAction>,
    ) -> bool {
        self.item_inspect_orbit_stick = (0.0, 0.0);
        self.item_inspect_zoom_triggers = 0.0;
        self.shop_storeroom_orbit_stick = (0.0, 0.0);
        self.right_stick_scroll_axis = 0.0;
        self.right_stick_scroll_axis_x = 0.0;
        self.left_stick_scroll_axis = 0.0;

        let before = actions.len();
        shell.prepare_gamepad_frame();

        if Self::sync_gamepad_style_from_first_connected(shell, &mut self.gamepad_style) {
            self.apply_controller_layout_defaults_for_active_style();
        }

        if poll_ctx.item_inspect_overlay {
            Self::sample_item_inspect_analog(
                shell,
                &mut self.item_inspect_orbit_stick,
                &mut self.item_inspect_zoom_triggers,
            );
            // Mouse / trackpad: LMB drag orbit (same presenters as right stick).
            let (mx, my) = self.item_inspect_mouse_orbit_px;
            self.item_inspect_mouse_orbit_px = (0.0, 0.0);
            const SENS: f32 = 0.014;
            let sx = (mx * SENS).clamp(-1.0, 1.0);
            let sy = (-my * SENS).clamp(-1.0, 1.0);
            self.item_inspect_orbit_stick.0 =
                (self.item_inspect_orbit_stick.0 + sx).clamp(-1.0, 1.0);
            self.item_inspect_orbit_stick.1 =
                (self.item_inspect_orbit_stick.1 + sy).clamp(-1.0, 1.0);

            // Keyboard orbit controls while inspect overlay is active:
            // arrows map to orbit; Shift+Up/Down drive zoom.
            let ks = shell.pump.keyboard_state();
            let shift = ks.is_scancode_pressed(Scancode::LShift)
                || ks.is_scancode_pressed(Scancode::RShift);
            let up_orbit = ks.is_scancode_pressed(Scancode::Up);
            let down_orbit = ks.is_scancode_pressed(Scancode::Down);
            if shift {
                if up_orbit {
                    self.item_inspect_zoom_triggers += 1.0;
                }
                if down_orbit {
                    self.item_inspect_zoom_triggers -= 1.0;
                }
            }
            self.item_inspect_zoom_triggers = self.item_inspect_zoom_triggers.clamp(-3.0, 3.0);

            let mut kx = 0.0f32;
            let mut ky = 0.0f32;
            if ks.is_scancode_pressed(Scancode::Right) {
                kx += 1.0;
            }
            if ks.is_scancode_pressed(Scancode::Left) {
                kx -= 1.0;
            }
            if up_orbit && !shift {
                ky += 1.0;
            }
            if down_orbit && !shift {
                ky -= 1.0;
            }
            let k_len = (kx * kx + ky * ky).sqrt();
            if k_len > 1e-4 {
                kx /= k_len;
                ky /= k_len;
            }
            self.item_inspect_orbit_stick.0 =
                (self.item_inspect_orbit_stick.0 + kx).clamp(-1.0, 1.0);
            self.item_inspect_orbit_stick.1 =
                (self.item_inspect_orbit_stick.1 + ky).clamp(-1.0, 1.0);
        } else if poll_ctx.shop_storeroom_orbit {
            self.shop_storeroom_orbit_stick = Self::sample_right_stick_xy(shell);
        }
        self.right_stick_scroll_axis = Self::sample_stick_scroll_axis(shell, GpAxis::RightY);
        self.right_stick_scroll_axis_x = Self::sample_stick_scroll_axis(shell, GpAxis::RightX);
        self.left_stick_scroll_axis = Self::sample_stick_scroll_axis(shell, GpAxis::LeftY);
        Self::emit_held_navigation_repeats(
            shell,
            &mut self.dpad_repeat,
            &mut self.dpad_axis_repeat_x,
            &mut self.dpad_axis_repeat_y,
            &mut self.stick_repeat_x,
            &mut self.stick_repeat_y,
            actions,
        );
        if actions.len() > before && self.mode != InputMode::Controller {
            self.mode = InputMode::Controller;
            return true;
        }
        false
    }

    /// Returns `true` when a real connected gamepad was found and `out` was
    /// updated. Callers use the return value to gate one-shot side effects
    /// (e.g. [`Self::apply_controller_layout_defaults_if_first_seen`]).
    fn sync_gamepad_style_from_first_connected(shell: &SdlShell, out: &mut GamepadStyle) -> bool {
        let Ok(ids) = shell.gamepad.gamepads() else {
            return false;
        };
        for id in ids {
            let vendor = shell.gamepad.vendor_for_id(id);
            if let Ok(name) = shell.gamepad.name_for_id(id) {
                *out = GamepadStyle::infer(vendor, &name);
                return true;
            }
        }
        false
    }

    /// Pick smart defaults for `swap_ab` / `swap_xy` based on the **currently
    /// connected** controller style, but only if the player has never manually
    /// toggled either setting in Options. Nintendo pads flip both ON so the
    /// eastern face button labelled "A" becomes Confirm (matching every other
    /// Switch title); all other styles flip both OFF.
    ///
    /// Re-runs whenever the detected style changes (Nintendo ↔ Xbox etc.) so
    /// a mid-session controller swap rebinds correctly. Once the player has
    /// taken control via Options (`controller_layout_user_set == true`) this
    /// stops touching their settings forever.
    pub fn apply_controller_layout_defaults_for_active_style(&mut self) {
        if self.last_seen_layout_style == Some(self.gamepad_style) {
            return;
        }
        self.last_seen_layout_style = Some(self.gamepad_style);
        let mut settings = crate::persistence::load_settings();
        if settings.controller_layout_user_set {
            return;
        }
        let want_swap = matches!(
            self.gamepad_style,
            GamepadStyle::Nintendo | GamepadStyle::NintendoSwitch2
        );
        self.swap_ab = want_swap;
        self.swap_xy = want_swap;
        if settings.swap_ab == want_swap && settings.swap_xy == want_swap {
            return;
        }
        settings.swap_ab = want_swap;
        settings.swap_xy = want_swap;
        let _ = crate::persistence::save_settings(&settings);
    }

    fn sample_right_stick_xy(shell: &SdlShell) -> (f32, f32) {
        const STICK_DZ: f32 = 0.15;
        let Ok(ids) = shell.gamepad.gamepads() else {
            return (0.0, 0.0);
        };
        for id in ids {
            let Some(gp) = shell.pads.get(&id) else {
                continue;
            };
            if !gp.connected() {
                continue;
            }
            let x = axis_norm(gp.axis(GpAxis::RightX));
            let y = axis_norm(gp.axis(GpAxis::RightY));
            return (
                if x.abs() < STICK_DZ { 0.0 } else { x },
                if y.abs() < STICK_DZ { 0.0 } else { y },
            );
        }
        (0.0, 0.0)
    }

    fn sample_item_inspect_analog(
        shell: &SdlShell,
        out_stick: &mut (f32, f32),
        out_zoom: &mut f32,
    ) {
        *out_stick = Self::sample_right_stick_xy(shell);
        let Ok(ids) = shell.gamepad.gamepads() else {
            return;
        };
        for id in ids {
            let Some(gp) = shell.pads.get(&id) else {
                continue;
            };
            if !gp.connected() {
                continue;
            }
            let lt = trigger_norm(gp.axis(GpAxis::TriggerLeft));
            let rt = trigger_norm(gp.axis(GpAxis::TriggerRight));
            let mut z = rt - lt;
            if gp.button(GpButton::LeftShoulder) {
                z -= 1.0;
            }
            if gp.button(GpButton::RightShoulder) {
                z += 1.0;
            }
            *out_zoom = z;
            break;
        }
    }

    fn sample_stick_scroll_axis(shell: &SdlShell, axis: GpAxis) -> f32 {
        const STICK_DZ: f32 = 0.22;
        let Ok(ids) = shell.gamepad.gamepads() else {
            return 0.0;
        };
        for id in ids {
            let Some(gp) = shell.pads.get(&id) else {
                continue;
            };
            if !gp.connected() {
                continue;
            }
            let y = axis_norm(gp.axis(axis));
            return if y.abs() < STICK_DZ { 0.0 } else { y };
        }
        0.0
    }

    fn emit_held_navigation_repeats(
        shell: &SdlShell,
        dpad_repeat: &mut Option<(UiAction, Instant)>,
        dpad_axis_repeat_x: &mut Option<(i8, Instant)>,
        dpad_axis_repeat_y: &mut Option<(i8, Instant)>,
        stick_repeat_x: &mut Option<(i8, Instant)>,
        stick_repeat_y: &mut Option<(i8, Instant)>,
        actions: &mut Vec<UiAction>,
    ) {
        let now = Instant::now();

        let mut clear_dpad = false;
        if let Some((action, next_at)) = dpad_repeat.as_mut() {
            if !Self::gamepad_dpad_nav_held(shell, *action) {
                clear_dpad = true;
            } else if now >= *next_at {
                actions.push(*action);
                *next_at = now + NAV_REPEAT_INTERVAL;
            }
        }
        if clear_dpad {
            *dpad_repeat = None;
        }

        const DPAD_AXIS_DEADZONE: f32 = 0.35;
        let (dx, dy) = Self::sample_dpad_axis_dirs(shell, DPAD_AXIS_DEADZONE);

        let mut clear_dx = false;
        if let Some((dir, next_at)) = dpad_axis_repeat_x.as_mut() {
            if dx == 0 || dx != *dir {
                clear_dx = true;
            } else if now >= *next_at {
                actions.push(if *dir > 0 {
                    UiAction::FocusNext
                } else {
                    UiAction::FocusPrev
                });
                *next_at = now + NAV_REPEAT_INTERVAL;
            }
        }
        if clear_dx {
            *dpad_axis_repeat_x = None;
        }

        let mut clear_dy = false;
        if let Some((dir, next_at)) = dpad_axis_repeat_y.as_mut() {
            if dy == 0 || dy != *dir {
                clear_dy = true;
            } else if now >= *next_at {
                actions.push(if *dir > 0 {
                    UiAction::FocusDown
                } else {
                    UiAction::FocusUp
                });
                *next_at = now + NAV_REPEAT_INTERVAL;
            }
        }
        if clear_dy {
            *dpad_axis_repeat_y = None;
        }

        const STICK_DEADZONE: f32 = 0.65;
        let (sx, sy) = Self::sample_left_stick_dirs(shell, STICK_DEADZONE);

        let mut clear_sx = false;
        if let Some((dir, next_at)) = stick_repeat_x.as_mut() {
            if sx == 0 || sx != *dir {
                clear_sx = true;
            } else if now >= *next_at {
                actions.push(if *dir > 0 {
                    UiAction::FocusNext
                } else {
                    UiAction::FocusPrev
                });
                *next_at = now + NAV_REPEAT_INTERVAL;
            }
        }
        if clear_sx {
            *stick_repeat_x = None;
        }

        let mut clear_sy = false;
        if let Some((dir, next_at)) = stick_repeat_y.as_mut() {
            if sy == 0 || sy != *dir {
                clear_sy = true;
            } else if now >= *next_at {
                actions.push(if *dir > 0 {
                    UiAction::FocusUp
                } else {
                    UiAction::FocusDown
                });
                *next_at = now + NAV_REPEAT_INTERVAL;
            }
        }
        if clear_sy {
            *stick_repeat_y = None;
        }
    }

    fn gamepad_dpad_nav_held(shell: &SdlShell, action: UiAction) -> bool {
        let Ok(ids) = shell.gamepad.gamepads() else {
            return false;
        };
        ids.iter().any(|&id| {
            let Some(gp) = shell.pads.get(&id) else {
                return false;
            };
            if !gp.connected() {
                return false;
            }
            match action {
                UiAction::FocusNext => gp.button(GpButton::DPadRight),
                UiAction::FocusPrev => gp.button(GpButton::DPadLeft),
                UiAction::FocusDown => gp.button(GpButton::DPadDown),
                UiAction::FocusUp => gp.button(GpButton::DPadUp),
                _ => false,
            }
        })
    }

    fn sample_dpad_axis_dirs(shell: &SdlShell, _deadzone: f32) -> (i8, i8) {
        let Ok(ids) = shell.gamepad.gamepads() else {
            return (0, 0);
        };
        for id in ids {
            let Some(gp) = shell.pads.get(&id) else {
                continue;
            };
            if !gp.connected() {
                continue;
            }
            let dx = if gp.button(GpButton::DPadRight) {
                1
            } else if gp.button(GpButton::DPadLeft) {
                -1
            } else {
                0
            };
            let dy = if gp.button(GpButton::DPadDown) {
                1
            } else if gp.button(GpButton::DPadUp) {
                -1
            } else {
                0
            };
            if dx != 0 || dy != 0 {
                return (dx, dy);
            }
        }
        (0, 0)
    }

    fn sample_left_stick_dirs(shell: &SdlShell, deadzone: f32) -> (i8, i8) {
        let Ok(ids) = shell.gamepad.gamepads() else {
            return (0, 0);
        };
        for id in ids {
            let Some(gp) = shell.pads.get(&id) else {
                continue;
            };
            if !gp.connected() {
                continue;
            }
            let x = axis_norm(gp.axis(GpAxis::LeftX));
            let y = axis_norm(gp.axis(GpAxis::LeftY));
            let sx = if x >= deadzone {
                1
            } else if x <= -deadzone {
                -1
            } else {
                0
            };
            let sy = if y >= deadzone {
                1
            } else if y <= -deadzone {
                -1
            } else {
                0
            };
            if sx != 0 || sy != 0 {
                return (sx, sy);
            }
        }
        (0, 0)
    }
}
