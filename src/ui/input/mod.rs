//! Unified input: mouse, keyboard, gamepad → semantic actions.

#[cfg(any(feature = "game", feature = "headless-screenshot"))]
use std::time::Instant;

use sdl3::gamepad::Button as GpButton;
use sdl3::keyboard::Scancode;

use crate::game::run::RunState;
use crate::render::animation::AnimationController;
use crate::ui::button_prompts::GamepadStyle;

#[cfg(any(feature = "game", feature = "headless-screenshot"))]
mod sdl;

/// Peak rumble gain for shop hold; multiplied each frame by [`shop_hold_rumble_gain_curve`].
pub(crate) const SHOP_HOLD_RUMBLE_PEAK_GAIN: f32 = 0.58;

/// Short pulse on each scoring cascade step reveal.
pub(crate) const SCORING_STEP_RUMBLE_MS: u32 = 42;
pub(crate) const SCORING_STEP_RUMBLE_WEAK: u16 = 6_500;
pub(crate) const SCORING_STEP_RUMBLE_STRONG: u16 = 2_200;
pub(crate) const SCORING_STEP_RUMBLE_GAIN: f32 = 0.42;

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
pub(crate) fn shop_hold_rumble_gain_curve(t: f32) -> f32 {
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

pub use mahjuro_types::UiAction;

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

/// Per-frame hints so [`InputState::gamepad_frame_tick`] can emit scene-appropriate
/// face-button actions without the input layer depending on scene types.
#[derive(Clone, Copy, Debug, Default)]
pub struct GamepadPollCtx {
    pub face_bindings: FaceButtonBindings,
    /// Showcase **orbit** overlay ([`crate::scenes::Scene::Showcase`] inspect presenters) — right stick + arrows for orbit, left stick + WASD for focus cycling, LMB drag + triggers/wheel for orbit zoom.
    pub item_inspect_overlay: bool,
    /// Shop storeroom browse (no inspect overlay) — right stick orbits the room camera.
    pub shop_storeroom_orbit: bool,
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
/// every index along the swept arc from `start_slot` to `current_slot` is
/// forced to `!snapshot[i]`, and every index outside that arc is forced back
/// to `snapshot[i]`. Linear drags use the contiguous index span; wrap-around
/// drags (off either end of the hand) follow the shorter circular path.
#[derive(Clone, Debug)]
pub struct MarqueeSelect {
    pub start_slot: usize,
    pub current_slot: usize,
    pub snapshot: Vec<bool>,
    /// `Some(true)` / `Some(false)` once sweep direction is known; `None` at press.
    sweep_forward: Option<bool>,
}

impl MarqueeSelect {
    pub fn new(start_slot: usize, snapshot: Vec<bool>) -> Self {
        Self {
            start_slot,
            current_slot: start_slot,
            snapshot,
            sweep_forward: None,
        }
    }

    /// Steps `current_slot` toward `next` and records sweep direction from the move.
    pub fn advance_to(&mut self, next: usize, hand_len: usize) {
        let prev = self.current_slot;
        if prev != next && hand_len > 0 {
            if let Some(fwd) = infer_adjacent_step_forward(prev, next, hand_len) {
                self.sweep_forward = Some(fwd);
            } else if self.sweep_forward.is_none() {
                // Cursor jump or first frame skip: prefer the shorter arc.
                let fwd_steps = (next + hand_len - prev) % hand_len;
                let bwd_steps = (prev + hand_len - next) % hand_len;
                self.sweep_forward = Some(fwd_steps <= bwd_steps);
            }
        }
        self.current_slot = next;
    }

    fn swept_slots(&self, hand_len: usize) -> Vec<usize> {
        let start = self.start_slot;
        let current = self.current_slot;
        if hand_len == 0 {
            return Vec::new();
        }
        if start == current {
            return vec![start.min(hand_len - 1)];
        }
        match self.sweep_forward {
            Some(forward) => arc_indices(start, current, forward, hand_len),
            None => {
                let lo = start.min(current);
                let hi = start.max(current);
                (lo..=hi.min(hand_len - 1)).collect()
            }
        }
    }

    /// Applies the marquee to `selected` and reports how many slots
    /// transitioned on (`added`) vs off (`removed`) relative to the prior
    /// state. Callers use this to play distinct tick/untick SFX.
    pub fn apply(&self, selected: &mut [bool]) -> (u32, u32) {
        let swept: std::collections::HashSet<usize> =
            self.swept_slots(selected.len()).into_iter().collect();
        let mut added = 0u32;
        let mut removed = 0u32;
        for (i, slot) in selected.iter_mut().enumerate() {
            let snap = self.snapshot.get(i).copied().unwrap_or(false);
            let next = if swept.contains(&i) { !snap } else { snap };
            if next && !*slot {
                added += 1;
            } else if !next && *slot {
                removed += 1;
            }
            *slot = next;
        }
        (added, removed)
    }

    /// Like [`Self::apply`], but newly selected slots stop once `max_selected`
    /// would be exceeded. Deselections inside the sweep still apply.
    pub fn apply_capped(&self, selected: &mut [bool], max_selected: usize) -> (u32, u32) {
        let hand_len = selected.len();
        let swept = self.swept_slots(hand_len);
        let swept_set: std::collections::HashSet<usize> = swept.iter().copied().collect();
        let mut desired: Vec<bool> = selected
            .iter()
            .enumerate()
            .map(|(i, _)| {
                let snap = self.snapshot.get(i).copied().unwrap_or(false);
                if swept_set.contains(&i) {
                    !snap
                } else {
                    snap
                }
            })
            .collect();

        let mut sel_count = desired.iter().filter(|&&s| s).count();
        if sel_count > max_selected {
            let mut trim_order = swept;
            trim_order.reverse();
            for i in trim_order {
                let snap = self.snapshot.get(i).copied().unwrap_or(false);
                if !snap && desired[i] && sel_count > max_selected {
                    desired[i] = false;
                    sel_count -= 1;
                }
            }
        }

        let mut added = 0u32;
        let mut removed = 0u32;
        for (i, slot) in selected.iter_mut().enumerate() {
            let next = desired.get(i).copied().unwrap_or(*slot);
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

fn infer_adjacent_step_forward(prev: usize, next: usize, len: usize) -> Option<bool> {
    if prev == next || len == 0 {
        return None;
    }
    let fwd = (next + len - prev) % len;
    let bwd = (prev + len - next) % len;
    if fwd == 1 {
        Some(true)
    } else if bwd == 1 {
        Some(false)
    } else {
        None
    }
}

fn arc_indices(start: usize, end: usize, forward: bool, len: usize) -> Vec<usize> {
    let mut out = vec![start];
    let mut pos = start;
    while pos != end {
        pos = if forward {
            (pos + 1) % len
        } else if pos == 0 {
            len - 1
        } else {
            pos - 1
        };
        out.push(pos);
    }
    out
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
    #[cfg(any(feature = "game", feature = "headless-screenshot"))]
    left_stick_x_dir: i8,
    /// Last non-neutral vertical left-stick direction we emitted:
    /// -1 = up, +1 = down, 0 = neutral.
    #[cfg(any(feature = "game", feature = "headless-screenshot"))]
    left_stick_y_dir: i8,
    /// Timestamp of the latest stick-navigation edge. Kept for future
    /// tuning / diagnostics and to make the gating behavior explicit.
    #[cfg(any(feature = "game", feature = "headless-screenshot"))]
    last_stick_nav_at: Instant,
    /// While a D-pad direction is held, next time to emit a repeat (after the
    /// initial [`ButtonPressed`] step).
    #[cfg(any(feature = "game", feature = "headless-screenshot"))]
    dpad_repeat: Option<(UiAction, Instant)>,
    /// Left stick: repeat horizontal nav while tilt is held past the deadzone.
    #[cfg(any(feature = "game", feature = "headless-screenshot"))]
    stick_repeat_x: Option<(i8, Instant)>,
    /// Left stick: repeat vertical nav while tilt is held past the deadzone.
    #[cfg(any(feature = "game", feature = "headless-screenshot"))]
    stick_repeat_y: Option<(i8, Instant)>,
    /// D-pad hold repeats (also fed from SDL D-pad buttons).
    #[cfg(any(feature = "game", feature = "headless-screenshot"))]
    dpad_axis_repeat_x: Option<(i8, Instant)>,
    #[cfg(any(feature = "game", feature = "headless-screenshot"))]
    dpad_axis_repeat_y: Option<(i8, Instant)>,
    /// Right stick axes (−1..1) while a showcase orbit presenter is active.
    pub item_inspect_orbit_stick: (f32, f32),
    /// LMB drag pixel delta this frame (summed from motion events); merged in [`Self::gamepad_frame_tick`].
    item_inspect_mouse_orbit_px: (f32, f32),
    /// LMB-drag turntable on the shop storeroom camera (consumed in [`crate::main::frame_tick`]).
    shop_storeroom_mouse_orbit_px: (f32, f32),
    /// Right stick (−1..1) while browsing the shop storeroom (not item inspect).
    pub shop_storeroom_orbit_stick: (f32, f32),
    /// Trigger analog zoom for item inspect: `RightTrigger2 − LeftTrigger2`, plus bumpers (see [`Self::sample_item_inspect_analog`]).
    pub item_inspect_zoom_triggers: f32,
    /// Right stick vertical axis used for list/pane scroll scenes (`-1..1`).
    pub right_stick_scroll_axis: f32,
    /// Right stick horizontal axis for horizontal scroll panes (`-1..1`).
    pub right_stick_scroll_axis_x: f32,
    /// Left stick vertical axis used for list/pane scroll on run-end screens (`-1..1`).
    pub left_stick_scroll_axis: f32,
    /// Controller family for on-screen button prompts (from USB vendor / name).
    pub gamepad_style: GamepadStyle,
    /// Last style we ran `apply_controller_layout_defaults_for_active_style`
    /// against. Used to debounce: we only re-evaluate when the connected
    /// controller's family actually changes (Nintendo ↔ Xbox etc.), not every
    /// frame. `None` until we've seen a real pad at least once.
    #[cfg(any(feature = "game", feature = "headless-screenshot"))]
    last_seen_layout_style: Option<GamepadStyle>,
    /// Scheduled scoring / rumble-lab pulses (`SDL_Gamepad::set_rumble` cannot overlap envelope/composite).
    #[cfg(any(feature = "game", feature = "headless-screenshot"))]
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
            #[cfg(any(feature = "game", feature = "headless-screenshot"))]
            left_stick_x_dir: 0,
            #[cfg(any(feature = "game", feature = "headless-screenshot"))]
            left_stick_y_dir: 0,
            #[cfg(any(feature = "game", feature = "headless-screenshot"))]
            last_stick_nav_at: Instant::now(),
            #[cfg(any(feature = "game", feature = "headless-screenshot"))]
            dpad_repeat: None,
            #[cfg(any(feature = "game", feature = "headless-screenshot"))]
            stick_repeat_x: None,
            #[cfg(any(feature = "game", feature = "headless-screenshot"))]
            stick_repeat_y: None,
            #[cfg(any(feature = "game", feature = "headless-screenshot"))]
            dpad_axis_repeat_x: None,
            #[cfg(any(feature = "game", feature = "headless-screenshot"))]
            dpad_axis_repeat_y: None,
            item_inspect_orbit_stick: (0.0, 0.0),
            item_inspect_mouse_orbit_px: (0.0, 0.0),
            shop_storeroom_mouse_orbit_px: (0.0, 0.0),
            shop_storeroom_orbit_stick: (0.0, 0.0),
            item_inspect_zoom_triggers: 0.0,
            right_stick_scroll_axis: 0.0,
            right_stick_scroll_axis_x: 0.0,
            left_stick_scroll_axis: 0.0,
            gamepad_style: GamepadStyle::default(),
            #[cfg(any(feature = "game", feature = "headless-screenshot"))]
            last_seen_layout_style: None,
            #[cfg(any(feature = "game", feature = "headless-screenshot"))]
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
            // Tab / Shift+Tab cycle tabs in scenes that opt in (e.g. collection browser).
            Scancode::Tab => {
                if shift {
                    actions.push(UiAction::TabPrev);
                } else {
                    actions.push(UiAction::TabNext);
                }
            }
            Scancode::PageDown => actions.push(UiAction::PageNext),
            Scancode::PageUp => actions.push(UiAction::PagePrev),
            // HUD strip nav (consumable focus on the gameplay scene; includes the
            // optional discard undo target when Accessibility → Discard undo is on).
            // Mirrors LB / RB on the controller so keyboard players have a non-mouse path.
            Scancode::LeftBracket => actions.push(UiAction::NavigateHudPrev),
            Scancode::RightBracket => actions.push(UiAction::NavigateHudNext),
            // **Q** / **E** = gamepad West / North (see [`UiAction::WestFacePress`], [`UiAction::NorthFacePress`]).
            Scancode::E => actions.push(UiAction::NorthFacePress),
            Scancode::Q => actions.push(UiAction::WestFacePress),
            // Glossary / help — `?` (Shift+/), `/`, `F1`; controller Select / View / −
            // (`GpButton::Back`, touchpad click, PS5 Create) via [`UiAction::Help`].
            Scancode::Slash | Scancode::F1 => actions.push(UiAction::Help),
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
        if matches!(code, Scancode::T) {
            actions.push(UiAction::TriggerStructureRelease);
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
            UiAction::TriggerStructure
            | UiAction::TriggerStructureRelease
            | UiAction::InvertSelection
            | UiAction::UndoDiscard
            | UiAction::Pause
            | UiAction::Help
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
            | UiAction::WestFaceRelease => {}
            UiAction::CancelRelease => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{InputState, MarqueeSelect, UiAction};

    fn marquee(start: usize, current: usize, snapshot: Vec<bool>) -> MarqueeSelect {
        let hand_len = snapshot.len();
        let mut m = MarqueeSelect::new(start, snapshot);
        if start < current {
            for i in (start + 1)..=current {
                m.advance_to(i, hand_len);
            }
        } else if start > current {
            for i in (current..start).rev() {
                m.advance_to(i, hand_len);
            }
        }
        m
    }

    fn marquee_wrap(start: usize, current: usize, snapshot: Vec<bool>) -> MarqueeSelect {
        let hand_len = snapshot.len();
        let mut m = MarqueeSelect::new(start, snapshot);
        m.advance_to(current, hand_len);
        m
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
    fn marquee_wrap_left_from_start_selects_only_endpoints() {
        let mut sel = vec![false; 16];
        marquee_wrap(0, 15, vec![false; 16]).apply(&mut sel);
        assert_eq!(sel.iter().filter(|&&s| s).count(), 2);
        assert!(sel[0] && sel[15]);
        assert!(!sel[1] && !sel[14]);
    }

    #[test]
    fn marquee_wrap_right_from_end_selects_only_endpoints() {
        let mut sel = vec![false; 16];
        marquee_wrap(15, 0, vec![false; 16]).apply(&mut sel);
        assert_eq!(sel.iter().filter(|&&s| s).count(), 2);
        assert!(sel[0] && sel[15]);
    }

    #[test]
    fn marquee_wrap_forward_arc_skips_linear_middle() {
        let mut sel = vec![false; 16];
        marquee_wrap(14, 2, vec![false; 16]).apply(&mut sel);
        assert_eq!(
            sel,
            vec![
                true, true, true, false, false, false, false, false, false, false, false, false,
                false, false, true, true
            ]
        );
    }

    #[test]
    fn capped_marquee_stops_at_max() {
        let mut sel = vec![true; 5];
        let m = marquee(0, 4, sel.clone());
        m.apply_capped(&mut sel, 5);
        assert_eq!(sel, vec![false, false, false, false, false]);

        let mut sel = vec![false; 8];
        let m = marquee(0, 6, vec![false; 8]);
        m.apply_capped(&mut sel, 5);
        assert_eq!(sel.iter().filter(|&&s| s).count(), 5);
        assert!(sel[0] && sel[1] && sel[2] && sel[3] && sel[4]);
        assert!(!sel[5] && !sel[6]);
    }

    #[test]
    fn capped_marquee_still_deselects_when_full() {
        let snapshot = vec![true, true, true, true, true, false, false];
        let mut sel = snapshot.clone();
        let m = marquee(0, 2, snapshot);
        m.apply_capped(&mut sel, 5);
        assert_eq!(sel, vec![false, false, false, true, true, false, false]);
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
