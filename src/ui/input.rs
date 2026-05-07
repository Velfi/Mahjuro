//! Unified input: mouse, keyboard, gamepad → semantic actions.

use std::time::{Duration, Instant};

use sdl3::event::Event;
use sdl3::gamepad::{Axis as GpAxis, Button as GpButton};
use sdl3::joystick::JoystickId;
use sdl3::keyboard::Scancode;

use crate::sdl_shell::SdlShell;

use crate::game::run::RunState;
use crate::render::animation::AnimationController;
use crate::ui::button_prompts::GamepadStyle;

/// Peak rumble gain for shop hold; multiplied each frame by [`shop_hold_rumble_gain_curve`].
const SHOP_HOLD_RUMBLE_PEAK_GAIN: f32 = 0.58;

/// Short pulse on each scoring cascade step reveal.
const SCORING_STEP_RUMBLE_MS: u32 = 42;
const SCORING_STEP_RUMBLE_WEAK: u16 = 6_500;
const SCORING_STEP_RUMBLE_STRONG: u16 = 2_200;
const SCORING_STEP_RUMBLE_GAIN: f32 = 0.42;

/// Requests from the rumble lab scene: dual-motor rumble (`SDL_Gamepad::set_rumble`).
#[derive(Clone, Debug)]
pub enum RumbleLabOp {
    Pulse {
        weak: u16,
        strong: u16,
        duration_ms: u32,
        gain: f32,
    },
    /// Each segment starts at `delay_ms` (from effect start), runs both motors for `dur_ms`.
    Composite {
        gain: f32,
        segments: Vec<(u32, u16, u16, u32)>,
    },
    Envelope {
        gain: f32,
        weak: u16,
        strong: u16,
        duration_ms: u32,
        attack_ms: u32,
        fade_ms: u32,
    },
}

/// Rumble strength vs normalized hold progress: rises through most of the hold,
/// then decays sharply in the final segment before completion.
fn shop_hold_rumble_gain_curve(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    const BUILD_END: f32 = 0.86;
    let drop_zone = (1.0 - BUILD_END).max(1e-4);
    if t <= BUILD_END {
        let u = t / BUILD_END;
        0.05 + 0.95 * (u * u * u)
    } else {
        let k = ((1.0 - t) / drop_zone).clamp(0.0, 1.0);
        k * k * k
    }
}

/// Which input device was used most recently.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputMode {
    Cursor,
    Keyboard,
    Controller,
}

/// Logical UI actions (device-agnostic).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiAction {
    FocusNext,
    FocusPrev,
    /// Vertical menu navigation (down).
    FocusDown,
    /// Vertical menu navigation (up).
    FocusUp,
    /// Toggle-select the focused tile for discard.
    Confirm,
    /// Controller-only: release the confirm face button.
    ConfirmRelease,
    Cancel,
    /// Commit selected melds into the structure (costs one play).
    ScoreHand,
    /// Cash in the structure for score (no play cost).
    TriggerStructure,
    /// Commit: discard all selected tiles and auto-draw back to full hand.
    CommitDiscard,
    /// Restore hand and wall immediately before the last discard (accessibility).
    UndoDiscard,
    /// Move focus onto the gameplay Play button (no commit). Emitted by the
    /// gamepad West (X) face when the "X and Y quick action" setting is OFF.
    FocusPlayButton,
    /// Move focus onto the gameplay Discard button (no commit). Emitted by
    /// the gamepad North (Y) face when the "X and Y quick action" setting
    /// is OFF.
    FocusDiscardButton,
    NavigateHudNext,
    NavigateHudPrev,
    SortBySuit,
    SortByRank,
    /// Cycle to the next tab in a tabbed scene (Tab key, RB).
    TabNext,
    /// Cycle to the previous tab in a tabbed scene (Shift+Tab, LB).
    TabPrev,
    /// Step to the next page within the current tab (PageDown).
    PageNext,
    /// Step to the previous page within the current tab (PageUp).
    PagePrev,
    /// Pause the game (Escape / Start button).
    Pause,
    /// Open the glossary / help overlay (`?`, `F1`, `H`, gamepad Select).
    Help,
    /// Delete the focused item (e.g. a profile slot). Bound to `Delete` / `X`.
    Delete,
    /// Debug-only: blow a strong one-shot wind gust at the candle row so we
    /// can verify flame/wind reactions in-game. Bound to `B`.
    DebugBlowWind,
    /// Debug-only: toggle a world-axes overlay (red = +X, green = +Y,
    /// blue = +Z) anchored at the camera's look target. Triggered from the
    /// native Debug menu.
    DebugToggleAxes,
    /// Shop: gamepad **North** / keyboard **E** — toggle close-up inspect for the focused item.
    ShopItemInspectToggle,
    /// Shop: gamepad **West** (hold) / hold **Q** — start hold-to-sell when over sellable owned stock.
    ShopSellHoldPress,
    /// Shop: gamepad **West** release / **Q** release — complete hold-to-sell if held long enough.
    ShopSellHoldRelease,
}

/// Per-frame hints so [`InputState::poll_gamepads`] can emit scene-appropriate
/// face-button actions without the input layer depending on scene types.
#[derive(Clone, Copy, Debug, Default)]
pub struct GamepadPollCtx {
    /// Shop is active with no blocking in-scene overlay — use shop face maps.
    pub shop_face_buttons: bool,
    /// Collection browser: North opens inspect; West stays default (no sell-hold).
    pub collection_inspect_north: bool,
    /// Shop item inspect is active — sample right stick + triggers for orbit/zoom.
    pub shop_item_inspect: bool,
}

/// What kind of draggable inventory item is currently being rearranged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragSubject {
    Relic,
}

/// Active drag state for relic reordering.
#[derive(Clone, Debug)]
pub struct DragState {
    pub subject: DragSubject,
    pub from_slot: usize,
    pub start_pos: (f32, f32),
    pub current_pos: (f32, f32),
}

/// Marquee multi-select state for the hand strip.
///
/// While the player holds Confirm (LMB / Space / Enter / gamepad A), the
/// `selected` array on the run is rewritten each time `current_slot` changes:
/// every index in `[min(start, current), max(start, current)]` is forced to
/// `!snapshot[i]`, and every index outside that range is forced back to
/// `snapshot[i]`. This gives standard marquee semantics — drag forward to
/// flip more tiles, drag back to revert ones you swept past.
#[derive(Clone, Debug)]
pub struct MarqueeSelect {
    pub start_slot: usize,
    pub current_slot: usize,
    pub snapshot: Vec<bool>,
}

impl MarqueeSelect {
    /// Applies the marquee to `selected` and reports how many slots
    /// transitioned on (`added`) vs off (`removed`) relative to the prior
    /// state. Callers use this to play distinct tick/untick SFX.
    pub fn apply(&self, selected: &mut [bool]) -> (u32, u32) {
        let lo = self.start_slot.min(self.current_slot);
        let hi = self.start_slot.max(self.current_slot);
        let mut added = 0u32;
        let mut removed = 0u32;
        for (i, slot) in selected.iter_mut().enumerate() {
            let snap = self.snapshot.get(i).copied().unwrap_or(false);
            let next = if i >= lo && i <= hi { !snap } else { snap };
            if next && !*slot {
                added += 1;
            } else if !next && *slot {
                removed += 1;
            }
            *slot = next;
        }
        (added, removed)
    }
}

/// Delay before the first repeated UI nav step while a direction is held
/// (gamepad D-pad / left stick). Mirrors desktop key-repeat feel.
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

pub struct InputState {
    pub focus_slot: usize,
    pub pointer_slot: Option<usize>,
    pub last_cursor: (f32, f32),
    pub mode: InputMode,
    pub drag: Option<DragState>,
    /// When true, gamepad South (A) and East (B) are swapped.
    pub swap_ab: bool,
    /// When true, gamepad West (X) and North (Y) are swapped.
    pub swap_xy: bool,
    /// When true, gamepad West (X) immediately commits ScoreHand and North
    /// (Y) immediately commits CommitDiscard. When false, those buttons
    /// only move focus onto the corresponding action button — the player
    /// must press Confirm (A) to actually fire the action.
    pub xy_quick_action: bool,
    /// Settings mirror: all controller rumble (shop hold-to-sell, scoring cascade, …).
    pub hold_to_sell_rumble_enabled: bool,
    /// Last non-neutral horizontal left-stick direction:
    /// -1 = left, +1 = right, 0 = neutral.
    left_stick_x_dir: i8,
    /// Last non-neutral vertical left-stick direction we emitted:
    /// -1 = up, +1 = down, 0 = neutral.
    left_stick_y_dir: i8,
    /// Timestamp of the latest stick-navigation edge. Kept for future
    /// tuning / diagnostics and to make the gating behavior explicit.
    last_stick_nav_at: Instant,
    /// While a D-pad direction is held, next time to emit a repeat (after the
    /// initial [`ButtonPressed`] step).
    dpad_repeat: Option<(UiAction, Instant)>,
    /// Left stick: repeat horizontal nav while tilt is held past the deadzone.
    stick_repeat_x: Option<(i8, Instant)>,
    /// Left stick: repeat vertical nav while tilt is held past the deadzone.
    stick_repeat_y: Option<(i8, Instant)>,
    /// D-pad hold repeats (also fed from SDL D-pad buttons).
    dpad_axis_repeat_x: Option<(i8, Instant)>,
    dpad_axis_repeat_y: Option<(i8, Instant)>,
    /// Right stick axes (−1..1) while shop item inspect is active.
    pub shop_inspect_orbit_stick: (f32, f32),
    /// Trigger analog zoom for shop inspect: `RightTrigger2 − LeftTrigger2`, plus bumpers (see [`Self::sample_shop_inspect_analog`]).
    pub shop_inspect_zoom_triggers: f32,
    /// Controller family for on-screen button prompts (from USB vendor / name).
    pub gamepad_style: GamepadStyle,
    /// Scheduled scoring / rumble-lab pulses (`SDL_Gamepad::set_rumble` cannot overlap envelope/composite).
    scoring_rumble_schedule: Vec<(Instant, u16, u16, u32, f32)>,
}

impl InputState {
    pub fn new() -> anyhow::Result<Self> {
        let settings = crate::persistence::load_settings();
        Ok(Self {
            focus_slot: 0,
            pointer_slot: None,
            last_cursor: (0.0, 0.0),
            mode: InputMode::Cursor,
            drag: None,
            swap_ab: settings.swap_ab,
            swap_xy: settings.swap_xy,
            xy_quick_action: settings.xy_quick_action,
            hold_to_sell_rumble_enabled: settings.hold_to_sell_rumble,
            left_stick_x_dir: 0,
            left_stick_y_dir: 0,
            last_stick_nav_at: Instant::now(),
            dpad_repeat: None,
            stick_repeat_x: None,
            stick_repeat_y: None,
            dpad_axis_repeat_x: None,
            dpad_axis_repeat_y: None,
            shop_inspect_orbit_stick: (0.0, 0.0),
            shop_inspect_zoom_triggers: 0.0,
            gamepad_style: GamepadStyle::default(),
            scoring_rumble_schedule: Vec::new(),
        })
    }

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
        for gp in shell.pads.values_mut() {
            if let Err(e) = gp.set_rumble(low, high, duration_ms) {
                log::warn!("sdl gamepad rumble failed: {e}");
            }
        }
        shell.sync_gamepad_rumble_output();
    }

    fn stop_sdl_rumble(shell: &mut SdlShell) {
        for gp in shell.pads.values_mut() {
            let _ = gp.set_rumble(0, 0, 1);
        }
        shell.sync_gamepad_rumble_output();
    }

    /// Same motors/gain as [`Self::play_scoring_cascade_step_rumble`] — for the debug lab UI.
    pub fn cascade_step_rumble_params() -> (u16, u16, u32, f32) {
        (
            SCORING_STEP_RUMBLE_WEAK,
            SCORING_STEP_RUMBLE_STRONG,
            SCORING_STEP_RUMBLE_MS,
            SCORING_STEP_RUMBLE_GAIN,
        )
    }

    /// Mirrors [`Self::play_scoring_cascade_final_rumble`] parameter derivation.
    pub fn cascade_final_rumble_params(earned: u64) -> (u16, u16, u32, f32) {
        let earned_f = earned.max(1) as f32;
        let mag = earned_f.log2();
        let duration_ms = (95.0 + mag * 24.0).clamp(95.0, 210.0).round() as u32;
        let gain = (0.38 + mag * 0.035).clamp(0.38, 0.82);
        let weak = (9_000.0 + mag * 950.0).clamp(9_000.0, 19_000.0) as u16;
        let strong = (3_200.0 + mag * 420.0).clamp(3_200.0, 8_500.0) as u16;
        (weak, strong, duration_ms, gain)
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
                    gain,
                    weak,
                    strong,
                    duration_ms,
                    attack_ms,
                    fade_ms,
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
        gain: f32,
        weak: u16,
        strong: u16,
        duration_ms: u32,
        attack_ms: u32,
        fade_ms: u32,
    ) {
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

    /// Light tap aligned with each cascade reveal beat (score tick SFX).
    pub fn play_scoring_cascade_step_rumble(&mut self, shell: &mut SdlShell, now: Instant) {
        self.play_scoring_rumble_pulse(
            shell,
            now,
            SCORING_STEP_RUMBLE_WEAK,
            SCORING_STEP_RUMBLE_STRONG,
            SCORING_STEP_RUMBLE_MS,
            SCORING_STEP_RUMBLE_GAIN,
        );
    }

    /// Stronger pulse for the final total; scales with hand magnitude like screen shake.
    pub fn play_scoring_cascade_final_rumble(
        &mut self,
        shell: &mut SdlShell,
        now: Instant,
        earned: u64,
    ) {
        let (weak, strong, duration_ms, gain) = Self::cascade_final_rumble_params(earned);
        self.play_scoring_rumble_pulse(shell, now, weak, strong, duration_ms, gain);
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

        let curve = shop_hold_rumble_gain_curve(hold_progress);
        let gain = SHOP_HOLD_RUMBLE_PEAK_GAIN * curve;
        let low = ((18_000u32 as f32) * gain).min(65535.0) as u16;
        let high = ((5_000u32 as f32) * gain).min(65535.0) as u16;
        const HOLD_REFRESH_MS: u32 = 120;
        for gp in shell.pads.values_mut() {
            if let Err(e) = gp.set_rumble(low, high, HOLD_REFRESH_MS) {
                log::debug!("shop sell hold rumble: {e}");
            }
        }
        shell.sync_gamepad_rumble_output();
    }

    pub fn focused_index(&self) -> usize {
        self.focus_slot
    }

    pub fn wrap_focus_slot(&mut self, action: UiAction, hand_len: usize) {
        if hand_len == 0 {
            self.focus_slot = 0;
            return;
        }

        self.focus_slot = match action {
            UiAction::FocusNext => (self.focus_slot + 1) % hand_len,
            UiAction::FocusPrev => {
                if self.focus_slot == 0 {
                    hand_len - 1
                } else {
                    self.focus_slot - 1
                }
            }
            _ => self.focus_slot.min(hand_len - 1),
        };
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
                    if self.swap_xy {
                        if poll_ctx.shop_face_buttons || poll_ctx.collection_inspect_north {
                            actions.push(UiAction::ShopItemInspectToggle);
                        } else {
                            actions.push(if self.xy_quick_action {
                                UiAction::CommitDiscard
                            } else {
                                UiAction::FocusDiscardButton
                            });
                        }
                    } else if poll_ctx.shop_face_buttons {
                        actions.push(UiAction::ShopSellHoldPress);
                    } else {
                        actions.push(if self.xy_quick_action {
                            UiAction::ScoreHand
                        } else {
                            UiAction::FocusPlayButton
                        });
                    }
                }
                GpButton::North => {
                    if self.swap_xy {
                        if poll_ctx.shop_face_buttons {
                            actions.push(UiAction::ShopSellHoldPress);
                        } else {
                            actions.push(if self.xy_quick_action {
                                UiAction::ScoreHand
                            } else {
                                UiAction::FocusPlayButton
                            });
                        }
                    } else if poll_ctx.shop_face_buttons || poll_ctx.collection_inspect_north {
                        actions.push(UiAction::ShopItemInspectToggle);
                    } else {
                        actions.push(if self.xy_quick_action {
                            UiAction::CommitDiscard
                        } else {
                            UiAction::FocusDiscardButton
                        });
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
                GpButton::Back => actions.push(UiAction::Help),
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
                    if !self.swap_ab {
                        actions.push(UiAction::ConfirmRelease);
                    }
                }
                GpButton::East => {
                    if self.swap_ab {
                        actions.push(UiAction::ConfirmRelease);
                    }
                }
                GpButton::West => {
                    if poll_ctx.shop_face_buttons && !self.swap_xy {
                        actions.push(UiAction::ShopSellHoldRelease);
                    }
                }
                GpButton::North => {
                    if poll_ctx.shop_face_buttons && self.swap_xy {
                        actions.push(UiAction::ShopSellHoldRelease);
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
                        if prev < TRIG_PRESS && cur >= TRIG_PRESS && !poll_ctx.shop_face_buttons {
                            actions.push(UiAction::TriggerStructure);
                        }
                        shell.lt_prev.insert(id, cur);
                    }
                    GpAxis::TriggerRight => {
                        let cur = trigger_norm(value);
                        let prev = shell.rt_prev.get(&id).copied().unwrap_or(0.0);
                        if prev < TRIG_PRESS && cur >= TRIG_PRESS && !poll_ctx.shop_face_buttons {
                            actions.push(UiAction::TriggerStructure);
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
        self.shop_inspect_orbit_stick = (0.0, 0.0);
        self.shop_inspect_zoom_triggers = 0.0;

        let before = actions.len();
        shell.prepare_gamepad_frame();

        Self::sync_gamepad_style_from_first_connected(shell, &mut self.gamepad_style);

        if poll_ctx.shop_item_inspect {
            Self::sample_shop_inspect_analog(
                shell,
                &mut self.shop_inspect_orbit_stick,
                &mut self.shop_inspect_zoom_triggers,
            );
        }
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

    fn sync_gamepad_style_from_first_connected(shell: &SdlShell, out: &mut GamepadStyle) {
        let Ok(ids) = shell.gamepad.gamepads() else {
            return;
        };
        for id in ids {
            let vendor = shell.gamepad.vendor_for_id(id);
            if let Ok(name) = shell.gamepad.name_for_id(id) {
                *out = GamepadStyle::infer(vendor, &name);
                return;
            }
        }
    }

    fn sample_shop_inspect_analog(
        shell: &SdlShell,
        out_stick: &mut (f32, f32),
        out_zoom: &mut f32,
    ) {
        const STICK_DZ: f32 = 0.15;
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
            let x = axis_norm(gp.axis(GpAxis::RightX));
            let y = axis_norm(gp.axis(GpAxis::RightY));
            *out_stick = (
                if x.abs() < STICK_DZ { 0.0 } else { x },
                if y.abs() < STICK_DZ { 0.0 } else { y },
            );
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

    /// Handle a key press.  Sets mode to Keyboard if a known key is pressed.
    /// Returns true if the mode changed.
    pub fn on_key(
        &mut self,
        key: Option<Scancode>,
        shift: bool,
        actions: &mut Vec<UiAction>,
    ) -> bool {
        let Some(code) = key else {
            return false;
        };
        let before = actions.len();
        match code {
            Scancode::Right | Scancode::D => actions.push(UiAction::FocusNext),
            Scancode::Left | Scancode::A => actions.push(UiAction::FocusPrev),
            Scancode::Down | Scancode::S => actions.push(UiAction::FocusDown),
            Scancode::Up | Scancode::W => actions.push(UiAction::FocusUp),
            Scancode::Space => actions.push(UiAction::Confirm),
            Scancode::Escape => actions.push(UiAction::Pause),
            Scancode::Backspace => actions.push(UiAction::Cancel),
            Scancode::Delete | Scancode::X => actions.push(UiAction::Delete),
            Scancode::T => actions.push(UiAction::TriggerStructure),
            Scancode::Return | Scancode::KpEnter => actions.push(UiAction::Confirm),
            // Tab is dual-purpose: scenes that opt in to TabNext/TabPrev
            // (e.g. the collection browser) get tab-cycle semantics; the
            // gameplay scene treats SortBySuit identically to a Tab press.
            // Both actions are emitted so each scene can pick the one it
            // cares about and ignore the other.
            Scancode::Tab => {
                if shift {
                    actions.push(UiAction::TabPrev);
                } else {
                    actions.push(UiAction::TabNext);
                    actions.push(UiAction::SortBySuit);
                }
            }
            Scancode::PageDown => actions.push(UiAction::PageNext),
            Scancode::PageUp => actions.push(UiAction::PagePrev),
            Scancode::Grave => actions.push(UiAction::SortByRank),
            // HUD strip nav (consumable focus on the gameplay scene; includes the
            // optional discard undo target when Accessibility → Discard undo is on).
            // Mirrors LB / RB on the controller so keyboard players have a non-mouse path.
            Scancode::LeftBracket => actions.push(UiAction::NavigateHudPrev),
            Scancode::RightBracket => actions.push(UiAction::NavigateHudNext),
            // Shop (gamepad West = hold sell, North = inspect): **Q** hold sell, **E** inspect.
            Scancode::E => actions.push(UiAction::ShopItemInspectToggle),
            Scancode::Q => actions.push(UiAction::ShopSellHoldPress),
            // Glossary / help — `?`, `/`, `H`, `F1`. ShiftLeft+Slash on
            // most layouts produces `?`, but we don't need shift state here:
            // both Slash and KeyH are unambiguous.
            Scancode::Slash | Scancode::H | Scancode::F1 => actions.push(UiAction::Help),
            _ => {}
        }
        if actions.len() > before && self.mode != InputMode::Keyboard {
            self.mode = InputMode::Keyboard;
            return true;
        }
        false
    }

    /// Mirror of [`Self::on_key`] for key-release events. Currently only
    /// emits `ConfirmRelease` when Space/Enter goes up — the marquee
    /// multi-select gesture needs a release edge for keyboard parity with
    /// the gamepad South button.
    pub fn on_key_release(&mut self, key: Option<Scancode>, actions: &mut Vec<UiAction>) {
        let Some(code) = key else {
            return;
        };
        if matches!(code, Scancode::Space | Scancode::Return | Scancode::KpEnter) {
            actions.push(UiAction::ConfirmRelease);
        }
        if matches!(code, Scancode::Q) {
            actions.push(UiAction::ShopSellHoldRelease);
        }
    }

    /// Hit-test hand slots; `slots` are world rects in same space as cursor.
    pub fn update_pointer_hover(
        &mut self,
        cursor: (f32, f32),
        hand_slots: &[(f32, f32, f32, f32)],
    ) {
        self.last_cursor = cursor;
        self.pointer_slot = hand_slots.iter().position(|(x, y, w, h)| {
            cursor.0 >= *x && cursor.0 <= *x + *w && cursor.1 >= *y && cursor.1 <= *y + *h
        });
        if self.mode == InputMode::Cursor
            && let Some(i) = self.pointer_slot
        {
            self.focus_slot = i;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{InputState, MarqueeSelect, UiAction};

    fn marquee(start: usize, current: usize, snapshot: Vec<bool>) -> MarqueeSelect {
        MarqueeSelect {
            start_slot: start,
            current_slot: current,
            snapshot,
        }
    }

    #[test]
    fn marquee_flips_swept_range_from_empty_snapshot() {
        let mut sel = vec![false; 6];
        marquee(2, 4, vec![false; 6]).apply(&mut sel);
        assert_eq!(sel, vec![false, false, true, true, true, false]);
    }

    #[test]
    fn marquee_dragging_back_reverts_unswept_tiles() {
        // Press at 2, sweep to 5 (selects 2,3,4,5), then drag back to 3.
        // Tiles 4 and 5 are now outside the swept range and revert to snapshot.
        let mut sel = vec![false; 6];
        marquee(2, 3, vec![false; 6]).apply(&mut sel);
        assert_eq!(sel, vec![false, false, true, true, false, false]);
    }

    #[test]
    fn marquee_flips_pre_selected_tiles_off() {
        // Mixed initial state: tiles 2 and 3 already selected. Press on 3,
        // sweep to 5. Range [3,5] flips, others keep snapshot.
        let snapshot = vec![false, false, true, true, false, false];
        let mut sel = snapshot.clone();
        marquee(3, 5, snapshot).apply(&mut sel);
        assert_eq!(sel, vec![false, false, true, false, true, true]);
    }

    #[test]
    fn marquee_zero_length_range_flips_only_start() {
        let mut sel = vec![true, true, true, true];
        marquee(1, 1, vec![true, true, true, true]).apply(&mut sel);
        assert_eq!(sel, vec![true, false, true, true]);
    }

    #[test]
    fn marquee_sweep_left_works_same_as_sweep_right() {
        let mut sel = vec![false; 5];
        marquee(4, 1, vec![false; 5]).apply(&mut sel);
        assert_eq!(sel, vec![false, true, true, true, true]);
    }

    #[test]
    fn wrap_focus_slot_wraps_forward() {
        let mut input = InputState::new().expect("input state");
        input.focus_slot = 15;

        input.wrap_focus_slot(UiAction::FocusNext, 16);

        assert_eq!(input.focus_slot, 0);
    }

    #[test]
    fn wrap_focus_slot_wraps_backward() {
        let mut input = InputState::new().expect("input state");
        input.focus_slot = 0;

        input.wrap_focus_slot(UiAction::FocusPrev, 16);

        assert_eq!(input.focus_slot, 15);
    }
}

/// Apply actions to run + animations (stub hooks).
pub fn apply_ui_actions(
    actions: &[UiAction],
    run: &mut RunState,
    bus: &mut crate::game::event_bus::EventBus,
    anim: &mut AnimationController,
    focus_tile_index: usize,
) {
    for a in actions {
        match a {
            UiAction::ScoreHand => {
                run.score_selected_tiles(bus);
                anim.pulse(crate::render::animation::ENTITY_SCORE_PANEL);
            }
            UiAction::Confirm => {
                // Toggle-select the focused tile for discard.
                if !run.hand().is_empty() {
                    let idx = focus_tile_index.min(run.hand().len() - 1);
                    run.toggle_select(idx);
                }
            }
            UiAction::ConfirmRelease => {}
            UiAction::CommitDiscard => {
                let discarded = run.discard_selected(bus);
                if discarded > 0 {
                    anim.pulse(crate::render::animation::ENTITY_HAND_STRIP);
                }
            }
            UiAction::Cancel => {
                run.clear_selection();
            }
            UiAction::SortBySuit
            | UiAction::SortByRank
            | UiAction::TriggerStructure
            | UiAction::UndoDiscard
            | UiAction::Pause
            | UiAction::Help
            | UiAction::DebugBlowWind
            | UiAction::DebugToggleAxes
            | UiAction::Delete => {}
            UiAction::FocusNext
            | UiAction::FocusPrev
            | UiAction::FocusDown
            | UiAction::FocusUp
            | UiAction::FocusPlayButton
            | UiAction::FocusDiscardButton
            | UiAction::NavigateHudNext
            | UiAction::NavigateHudPrev
            | UiAction::TabNext
            | UiAction::TabPrev
            | UiAction::PageNext
            | UiAction::PagePrev => {}
            UiAction::ShopItemInspectToggle
            | UiAction::ShopSellHoldPress
            | UiAction::ShopSellHoldRelease => {}
        }
    }
}
