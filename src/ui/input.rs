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

struct RumbleEnvelopeParams {
    gain: f32,
    weak: u16,
    strong: u16,
    duration_ms: u32,
    attack_ms: u32,
    fade_ms: u32,
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UiAction {
    FocusNext,
    FocusPrev,
    /// Vertical menu navigation (down).
    FocusDown,
    /// Vertical menu navigation (up).
    FocusUp,
    /// Gamepad **South** / **Enter** / **Space** — confirm / hand tile toggle (scene-defined).
    Confirm,
    /// Controller-only: release the confirm face button.
    ConfirmRelease,
    /// Gamepad **East** / **Backspace** — cancel / back (scene-defined).
    Cancel,
    /// Gamepad **East** / **Backspace** / **Escape** release — used by the
    /// modal hold-to-skim gesture so the level-up celebration knows when to
    /// stop auto-advancing pages. No current consumers outside `ModalQueue`.
    CancelRelease,
    /// Commit selected melds into the structure (costs one play).
    ScoreHand,
    /// Cash in the structure for score (no play cost).
    TriggerStructure,
    /// Commit: discard all selected tiles and auto-draw back to full hand.
    CommitDiscard,
    /// Flip every hand-tile selection bit (selected ↔ unselected).
    InvertSelection,
    /// Restore hand and wall immediately before the last discard (accessibility).
    UndoDiscard,
    /// Move focus onto the gameplay Play mirror (no commit). Emitted by the
    /// gamepad **North** face when "X and Y quick action" is OFF.
    FocusPlayButton,
    /// Move focus onto the gameplay Discard bowl (no commit). Emitted by the
    /// gamepad **West** face when "X and Y quick action" is OFF.
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
    /// **Escape** / gamepad **Start** — pause menu.
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
    /// Gamepad **North** / keyboard **E** — scene-defined (inspect, play hand, …).
    NorthFacePress,
    /// Gamepad **West** (press) / keyboard **Q** (press) — scene-defined (discard, hold-to-sell start, …).
    WestFacePress,
    /// Gamepad **West** release / **Q** release — shop hold-to-sell completion; ignored elsewhere.
    WestFaceRelease,
    /// Tixels scene: open image picker (`O`).
    TixelsLoadImage,
    /// Tixels scene: resolution preset down (`[`).
    TixelsResolutionDown,
    /// Tixels scene: resolution preset up (`]`).
    TixelsResolutionUp,
    /// Tixels scene: tile size down (`-`).
    TixelsTileDown,
    /// Tixels scene: tile size up (`=` / `+`).
    TixelsTileUp,
    /// Tixels scene: toggle Bayer dithering (`D`).
    TixelsToggleBayer,
    /// Tixels scene: toggle color tinting (`C`).
    TixelsToggleColor,
    /// Tixels scene: reset settings (`R`).
    TixelsReset,
}

/// Context for [`crate::scenes::SceneBehavior::face_button_bindings`].
#[derive(Clone, Copy, Debug)]
pub struct FaceBindingCtx {
    /// Gameplay-only: when true, West/North emit discard/play; when false, focus bowl/mirror.
    pub xy_quick_action: bool,
}

/// Scene-owned mapping from logical face buttons (after `swap_xy`) to [`UiAction`].
///
/// The input layer resolves physical West/North through `swap_xy`, then looks up the
/// action here. Unbound slots are ignored.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FaceButtonBindings {
    pub west_press: Option<UiAction>,
    pub north_press: Option<UiAction>,
    pub west_release: Option<UiAction>,
    /// When true, LT/RT analog triggers do not emit [`UiAction::TriggerStructure`].
    pub suppress_trigger_structure: bool,
}

impl FaceButtonBindings {
    /// Logical West/North press for a physical face button and the player's `swap_xy` setting.
    pub fn face_press(self, button: GpButton, swap_xy: bool) -> Option<UiAction> {
        match (button, swap_xy) {
            (GpButton::West, false) | (GpButton::North, true) => self.west_press,
            (GpButton::North, false) | (GpButton::West, true) => self.north_press,
            _ => None,
        }
    }

    /// Logical West release (hold-to-sell completion) for a physical face button.
    pub fn face_release(self, button: GpButton, swap_xy: bool) -> Option<UiAction> {
        match (button, swap_xy) {
            (GpButton::West, false) | (GpButton::North, true) => self.west_release,
            _ => None,
        }
    }
}

/// Per-frame hints so [`InputState::poll_gamepads`] can emit scene-appropriate
/// face-button actions without the input layer depending on scene types.
#[derive(Clone, Copy, Debug, Default)]
pub struct GamepadPollCtx {
    pub face_bindings: FaceButtonBindings,
    /// Showcase **orbit** overlay ([`crate::scenes::Scene::Showcase`] inspect presenters) — right stick + arrows for orbit, left stick + WASD for focus cycling, LMB drag + triggers/wheel for orbit zoom.
    pub item_inspect_overlay: bool,
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
    /// When true, West/North emit [`UiAction::WestFacePress`] / [`UiAction::NorthFacePress`] in gameplay
    /// (discard / play). When false, they only move focus onto the bowl / mirror.
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
    /// Right stick axes (−1..1) while a showcase orbit presenter is active.
    pub item_inspect_orbit_stick: (f32, f32),
    /// LMB drag pixel delta this frame (summed from motion events); merged in [`Self::gamepad_frame_tick`].
    item_inspect_mouse_orbit_px: (f32, f32),
    /// LMB-drag turntable on the shop storeroom camera (consumed in [`crate::main::frame_tick`]).
    shop_storeroom_mouse_orbit_px: (f32, f32),
    /// Trigger analog zoom for item inspect: `RightTrigger2 − LeftTrigger2`, plus bumpers (see [`Self::sample_item_inspect_analog`]).
    pub item_inspect_zoom_triggers: f32,
    /// Right stick vertical axis used for list/pane scroll scenes (`-1..1`).
    pub right_stick_scroll_axis: f32,
    /// Controller family for on-screen button prompts (from USB vendor / name).
    pub gamepad_style: GamepadStyle,
    /// Last style we ran `apply_controller_layout_defaults_for_active_style`
    /// against. Used to debounce: we only re-evaluate when the connected
    /// controller's family actually changes (Nintendo ↔ Xbox etc.), not every
    /// frame. `None` until we've seen a real pad at least once.
    last_seen_layout_style: Option<GamepadStyle>,
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
            item_inspect_orbit_stick: (0.0, 0.0),
            item_inspect_mouse_orbit_px: (0.0, 0.0),
            shop_storeroom_mouse_orbit_px: (0.0, 0.0),
            item_inspect_zoom_triggers: 0.0,
            right_stick_scroll_axis: 0.0,
            gamepad_style: GamepadStyle::default(),
            last_seen_layout_style: None,
            scoring_rumble_schedule: Vec::new(),
        })
    }

    #[inline]
    pub fn accum_item_inspect_mouse_orbit(&mut self, dx: f32, dy: f32) {
        self.item_inspect_mouse_orbit_px.0 += dx;
        self.item_inspect_mouse_orbit_px.1 += dy;
    }

    #[inline]
    pub fn accum_shop_storeroom_mouse_orbit(&mut self, dx: f32, dy: f32) {
        self.shop_storeroom_mouse_orbit_px.0 += dx;
        self.shop_storeroom_mouse_orbit_px.1 += dy;
    }

    #[inline]
    pub fn take_shop_storeroom_mouse_orbit_px(&mut self) -> (f32, f32) {
        let px = self.shop_storeroom_mouse_orbit_px;
        self.shop_storeroom_mouse_orbit_px = (0.0, 0.0);
        px
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

    pub fn shop_sell_hold_rumble_params(hold_progress: f32) -> (u16, u16, u32, f32) {
        let curve = shop_hold_rumble_gain_curve(hold_progress);
        let gain = SHOP_HOLD_RUMBLE_PEAK_GAIN * curve;
        let weak = (5_000_f32 * gain).min(65535.0) as u16;
        let strong = (18_000_f32 * gain).min(65535.0) as u16;
        const HOLD_REFRESH_MS: u32 = 120;
        (weak, strong, HOLD_REFRESH_MS, 1.0)
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
        for gp in shell.pads.values_mut() {
            if let Err(e) = gp.set_rumble(low, high, hold_refresh_ms) {
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
                GpButton::Back => actions.push(UiAction::Help),
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
                        if prev < TRIG_PRESS
                            && cur >= TRIG_PRESS
                            && !poll_ctx.face_bindings.suppress_trigger_structure
                        {
                            actions.push(UiAction::TriggerStructure);
                        }
                        shell.lt_prev.insert(id, cur);
                    }
                    GpAxis::TriggerRight => {
                        let cur = trigger_norm(value);
                        let prev = shell.rt_prev.get(&id).copied().unwrap_or(0.0);
                        if prev < TRIG_PRESS
                            && cur >= TRIG_PRESS
                            && !poll_ctx.face_bindings.suppress_trigger_structure
                        {
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
        self.item_inspect_orbit_stick = (0.0, 0.0);
        self.item_inspect_zoom_triggers = 0.0;
        self.right_stick_scroll_axis = 0.0;

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
            // arrows map to orbit; W/S (with Shift) drive zoom.
            let ks = shell.pump.keyboard_state();
            let shift = ks.is_scancode_pressed(Scancode::LShift)
                || ks.is_scancode_pressed(Scancode::RShift);
            let up_orbit = ks.is_scancode_pressed(Scancode::Up);
            let down_orbit = ks.is_scancode_pressed(Scancode::Down);
            let up_zoom = ks.is_scancode_pressed(Scancode::W) || up_orbit;
            let down_zoom = ks.is_scancode_pressed(Scancode::S) || down_orbit;
            if shift {
                if up_zoom {
                    self.item_inspect_zoom_triggers += 1.0;
                }
                if down_zoom {
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
        }
        self.right_stick_scroll_axis = Self::sample_right_stick_scroll_axis(shell);
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

    fn sample_item_inspect_analog(
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

    fn sample_right_stick_scroll_axis(shell: &SdlShell) -> f32 {
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
            let y = axis_norm(gp.axis(GpAxis::RightY));
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
            Scancode::Z => actions.push(UiAction::InvertSelection),
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
            // **Q** / **E** = gamepad West / North (see [`UiAction::WestFacePress`], [`UiAction::NorthFacePress`]).
            Scancode::E => actions.push(UiAction::NorthFacePress),
            Scancode::Q => actions.push(UiAction::WestFacePress),
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

    /// Mirror of [`Self::on_key`] for key-release events. Emits
    /// `ConfirmRelease` for Space/Enter (marquee multi-select edge),
    /// `WestFaceRelease` for Q (shop hold-to-sell), and `CancelRelease` for
    /// Backspace / Escape so the modal level-up skim can detect when the
    /// player lets go of the cancel/back key.
    pub fn on_key_release(&mut self, key: Option<Scancode>, actions: &mut Vec<UiAction>) {
        let Some(code) = key else {
            return;
        };
        if matches!(code, Scancode::Space | Scancode::Return | Scancode::KpEnter) {
            actions.push(UiAction::ConfirmRelease);
        }
        if matches!(code, Scancode::Q) {
            actions.push(UiAction::WestFaceRelease);
        }
        if matches!(code, Scancode::Backspace | Scancode::Escape) {
            actions.push(UiAction::CancelRelease);
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
                run.commit_selection_to_structure(bus);
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
            | UiAction::InvertSelection
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
            UiAction::NorthFacePress
            | UiAction::WestFacePress
            | UiAction::WestFaceRelease
            | UiAction::TixelsLoadImage
            | UiAction::TixelsResolutionDown
            | UiAction::TixelsResolutionUp
            | UiAction::TixelsTileDown
            | UiAction::TixelsTileUp
            | UiAction::TixelsToggleBayer
            | UiAction::TixelsToggleColor
            | UiAction::TixelsReset => {}
            UiAction::CancelRelease => {}
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

    #[test]
    fn face_bindings_west_slot_without_swap_xy() {
        use sdl3::gamepad::Button as GpButton;

        let bindings = super::FaceButtonBindings {
            west_press: Some(UiAction::Delete),
            north_press: Some(UiAction::NorthFacePress),
            ..Default::default()
        };
        assert_eq!(
            bindings.face_press(GpButton::West, false),
            Some(UiAction::Delete)
        );
        assert_eq!(
            bindings.face_press(GpButton::North, false),
            Some(UiAction::NorthFacePress)
        );
    }

    #[test]
    fn face_bindings_swap_xy_routes_physical_north_to_west_slot() {
        use sdl3::gamepad::Button as GpButton;

        let bindings = super::FaceButtonBindings {
            west_press: Some(UiAction::Delete),
            ..Default::default()
        };
        assert_eq!(
            bindings.face_press(GpButton::North, true),
            Some(UiAction::Delete)
        );
        assert_eq!(bindings.face_press(GpButton::West, true), None);
    }
}
