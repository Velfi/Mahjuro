//! Unified input: mouse, keyboard, gamepad → semantic actions.

use std::time::Instant;

use gilrs::{Axis, Button, Event as GilEvent, Gilrs};
use winit::keyboard::{KeyCode, PhysicalKey};

use crate::game::run::RunState;
use crate::render::animation::AnimationController;

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

pub struct InputState {
    pub gilrs: Option<Gilrs>,
    pub focus_slot: usize,
    pub pointer_slot: Option<usize>,
    pub last_cursor: (f32, f32),
    pub mode: InputMode,
    pub drag: Option<DragState>,
    /// When true, gamepad South (A) and East (B) are swapped.
    pub swap_ab: bool,
    /// Last non-neutral horizontal left-stick direction we emitted:
    /// -1 = left, +1 = right, 0 = neutral. Prevents a single tilt from
    /// generating a burst of repeated focus moves.
    left_stick_x_dir: i8,
    /// Last non-neutral vertical left-stick direction we emitted:
    /// -1 = up, +1 = down, 0 = neutral.
    left_stick_y_dir: i8,
    /// Timestamp of the latest stick-navigation edge. Kept for future
    /// tuning / diagnostics and to make the gating behavior explicit.
    last_stick_nav_at: Instant,
}

impl InputState {
    pub fn new() -> anyhow::Result<Self> {
        let settings = crate::persistence::load_settings();
        Ok(Self {
            gilrs: Gilrs::new().ok(),
            focus_slot: 0,
            pointer_slot: None,
            last_cursor: (0.0, 0.0),
            mode: InputMode::Cursor,
            drag: None,
            swap_ab: settings.swap_ab,
            left_stick_x_dir: 0,
            left_stick_y_dir: 0,
            last_stick_nav_at: Instant::now(),
        })
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

    /// Poll gilrs; returns emitted actions.  Sets mode to Controller if any
    /// action is produced.  Returns true if the mode changed.
    pub fn poll_gamepads(&mut self, actions: &mut Vec<UiAction>) -> bool {
        let Some(ref mut gilrs) = self.gilrs else {
            return false;
        };
        let before = actions.len();
        const STICK_DEADZONE: f32 = 0.65;
        while let Some(GilEvent { event, .. }) = gilrs.next_event() {
            use gilrs::EventType::*;
            match event {
                ButtonPressed(Button::South, _) => actions.push(if self.swap_ab {
                    UiAction::Cancel
                } else {
                    UiAction::Confirm
                }),
                ButtonReleased(Button::South, _) => actions.push(if self.swap_ab {
                    UiAction::Cancel
                } else {
                    UiAction::ConfirmRelease
                }),
                ButtonPressed(Button::East, _) => actions.push(if self.swap_ab {
                    UiAction::Confirm
                } else {
                    UiAction::Cancel
                }),
                ButtonReleased(Button::East, _) => actions.push(if self.swap_ab {
                    UiAction::ConfirmRelease
                } else {
                    UiAction::Cancel
                }),
                ButtonPressed(Button::LeftTrigger2, _) => actions.push(UiAction::TriggerStructure),
                ButtonPressed(Button::RightTrigger2, _) => actions.push(UiAction::TriggerStructure),
                ButtonPressed(Button::West, _) => actions.push(UiAction::ScoreHand),
                ButtonPressed(Button::North, _) => actions.push(UiAction::CommitDiscard),
                AxisChanged(Axis::LeftStickX, v, _) => {
                    let new_dir = if v >= STICK_DEADZONE {
                        1
                    } else if v <= -STICK_DEADZONE {
                        -1
                    } else {
                        0
                    };
                    if new_dir != 0 && new_dir != self.left_stick_x_dir {
                        actions.push(if new_dir > 0 {
                            UiAction::FocusNext
                        } else {
                            UiAction::FocusPrev
                        });
                        self.last_stick_nav_at = Instant::now();
                    }
                    self.left_stick_x_dir = new_dir;
                }
                AxisChanged(Axis::LeftStickY, v, _) => {
                    let new_dir = if v >= STICK_DEADZONE {
                        1
                    } else if v <= -STICK_DEADZONE {
                        -1
                    } else {
                        0
                    };
                    if new_dir != 0 && new_dir != self.left_stick_y_dir {
                        actions.push(if new_dir > 0 {
                            UiAction::FocusDown
                        } else {
                            UiAction::FocusUp
                        });
                        self.last_stick_nav_at = Instant::now();
                    }
                    self.left_stick_y_dir = new_dir;
                }
                ButtonPressed(Button::DPadRight, _) => actions.push(UiAction::FocusNext),
                ButtonPressed(Button::DPadLeft, _) => actions.push(UiAction::FocusPrev),
                ButtonPressed(Button::DPadDown, _) => actions.push(UiAction::FocusDown),
                ButtonPressed(Button::DPadUp, _) => actions.push(UiAction::FocusUp),
                ButtonPressed(Button::Start, _) => actions.push(UiAction::Pause),
                ButtonPressed(Button::Select, _) => actions.push(UiAction::Help),
                ButtonPressed(Button::LeftTrigger, _) => {
                    actions.push(UiAction::NavigateHudPrev);
                    actions.push(UiAction::TabPrev);
                }
                ButtonPressed(Button::RightTrigger, _) => {
                    actions.push(UiAction::NavigateHudNext);
                    actions.push(UiAction::TabNext);
                }
                _ => {}
            }
        }
        if actions.len() > before && self.mode != InputMode::Controller {
            self.mode = InputMode::Controller;
            return true;
        }
        false
    }

    /// Handle a key press.  Sets mode to Keyboard if a known key is pressed.
    /// Returns true if the mode changed.
    pub fn on_key(&mut self, key: PhysicalKey, shift: bool, actions: &mut Vec<UiAction>) -> bool {
        let PhysicalKey::Code(code) = key else {
            return false;
        };
        let before = actions.len();
        match code {
            KeyCode::ArrowRight | KeyCode::KeyD => actions.push(UiAction::FocusNext),
            KeyCode::ArrowLeft | KeyCode::KeyA => actions.push(UiAction::FocusPrev),
            KeyCode::ArrowDown | KeyCode::KeyS => actions.push(UiAction::FocusDown),
            KeyCode::ArrowUp | KeyCode::KeyW => actions.push(UiAction::FocusUp),
            KeyCode::Space => actions.push(UiAction::Confirm),
            KeyCode::Escape => actions.push(UiAction::Pause),
            KeyCode::Backspace => actions.push(UiAction::Cancel),
            KeyCode::Delete | KeyCode::KeyX => actions.push(UiAction::Delete),
            KeyCode::KeyT => actions.push(UiAction::TriggerStructure),
            KeyCode::Enter | KeyCode::NumpadEnter => actions.push(UiAction::Confirm),
            // Tab is dual-purpose: scenes that opt in to TabNext/TabPrev
            // (e.g. the collection browser) get tab-cycle semantics; the
            // gameplay scene treats SortBySuit identically to a Tab press.
            // Both actions are emitted so each scene can pick the one it
            // cares about and ignore the other.
            KeyCode::Tab => {
                if shift {
                    actions.push(UiAction::TabPrev);
                } else {
                    actions.push(UiAction::TabNext);
                    actions.push(UiAction::SortBySuit);
                }
            }
            KeyCode::PageDown => actions.push(UiAction::PageNext),
            KeyCode::PageUp => actions.push(UiAction::PagePrev),
            KeyCode::Backquote => actions.push(UiAction::SortByRank),
            // HUD strip nav (consumable focus on the gameplay scene). Mirrors
            // the LB / RB shoulder buttons on the controller so keyboard
            // players have a non-mouse path to use Zodiacs and Talismans.
            KeyCode::BracketLeft => actions.push(UiAction::NavigateHudPrev),
            KeyCode::BracketRight => actions.push(UiAction::NavigateHudNext),
            // Glossary / help — `?`, `/`, `H`, `F1`. ShiftLeft+Slash on
            // most layouts produces `?`, but we don't need shift state here:
            // both Slash and KeyH are unambiguous.
            KeyCode::Slash | KeyCode::KeyH | KeyCode::F1 => actions.push(UiAction::Help),
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
    pub fn on_key_release(&mut self, key: PhysicalKey, actions: &mut Vec<UiAction>) {
        let PhysicalKey::Code(code) = key else {
            return;
        };
        if matches!(code, KeyCode::Space | KeyCode::Enter | KeyCode::NumpadEnter) {
            actions.push(UiAction::ConfirmRelease);
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
        if self.mode == InputMode::Cursor {
            if let Some(i) = self.pointer_slot {
                self.focus_slot = i;
            }
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
                if !run.hand.is_empty() {
                    let idx = focus_tile_index.min(run.hand.len() - 1);
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
            | UiAction::Pause
            | UiAction::Help
            | UiAction::DebugBlowWind
            | UiAction::DebugToggleAxes
            | UiAction::Delete => {}
            UiAction::FocusNext
            | UiAction::FocusPrev
            | UiAction::FocusDown
            | UiAction::FocusUp
            | UiAction::NavigateHudNext
            | UiAction::NavigateHudPrev
            | UiAction::TabNext
            | UiAction::TabPrev
            | UiAction::PageNext
            | UiAction::PagePrev => {}
        }
    }
}
