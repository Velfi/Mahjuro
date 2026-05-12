//! Steam Input bridge: converts ISteamInput action data into Mahjuro's
//! device-agnostic [`UiAction`] stream.

use std::collections::{HashMap, HashSet};
use std::ffi::CStr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use steamworks::{Input, InputType, sys};

use crate::ui::button_prompts::GamepadStyle;
use crate::ui::input::UiAction;

/// Steam Input action set. Mirrors the `Menus` / `Gameplay` / `Shop` / `Inspect`
/// sets in `packaging/steam_input/game_actions_4636490.vdf`. `Menus` covers every
/// non-gameplay scene (start screen, profile select, options, pause overlay,
/// rumble lab, …) — its action list is a superset of what those scenes use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionSet {
    Menus,
    Gameplay,
    Shop,
    Inspect,
}

impl ActionSet {
    fn name(self) -> &'static str {
        match self {
            Self::Menus => "Menus",
            Self::Gameplay => "Gameplay",
            Self::Shop => "Shop",
            Self::Inspect => "Inspect",
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct AnalogSnapshot {
    pub orbit: (f32, f32),
    pub inspect_zoom: f32,
}

#[derive(Clone, Copy)]
struct ActionSetHandles {
    menus: sys::InputActionSetHandle_t,
    gameplay: sys::InputActionSetHandle_t,
    shop: sys::InputActionSetHandle_t,
    inspect: sys::InputActionSetHandle_t,
}

impl ActionSetHandles {
    fn get(self, set: ActionSet) -> sys::InputActionSetHandle_t {
        match set {
            ActionSet::Menus => self.menus,
            ActionSet::Gameplay => self.gameplay,
            ActionSet::Shop => self.shop,
            ActionSet::Inspect => self.inspect,
        }
    }
}

#[derive(Clone, Copy)]
struct AnalogHandles {
    nav: sys::InputAnalogActionHandle_t,
    orbit: sys::InputAnalogActionHandle_t,
    inspect_zoom: sys::InputAnalogActionHandle_t,
    /// `cursor` (Menus / Gameplay / Shop) is `absolute_mouse` + `os_mouse=1`.
    /// Steam pushes the cursor deltas through the OS mouse path so SDL's
    /// `MouseMotion` event handler picks them up automatically — we never
    /// poll this handle, but registering it tells Steam Input to surface
    /// the action in the configurator UI for trackpad / gyro binding.
    #[allow(dead_code)]
    cursor: sys::InputAnalogActionHandle_t,
}

#[derive(Clone, Copy)]
struct DigitalBinding {
    name: &'static str,
    action: UiAction,
    release_action: Option<UiAction>,
}

const DIGITAL_BINDINGS: &[DigitalBinding] = &[
    DigitalBinding {
        name: "confirm",
        action: UiAction::Confirm,
        release_action: Some(UiAction::ConfirmRelease),
    },
    DigitalBinding {
        name: "cancel",
        action: UiAction::Cancel,
        release_action: None,
    },
    DigitalBinding {
        name: "focus_next",
        action: UiAction::FocusNext,
        release_action: None,
    },
    DigitalBinding {
        name: "focus_prev",
        action: UiAction::FocusPrev,
        release_action: None,
    },
    DigitalBinding {
        name: "focus_up",
        action: UiAction::FocusUp,
        release_action: None,
    },
    DigitalBinding {
        name: "focus_down",
        action: UiAction::FocusDown,
        release_action: None,
    },
    DigitalBinding {
        name: "play_hand",
        action: UiAction::NorthFacePress,
        release_action: None,
    },
    DigitalBinding {
        name: "discard",
        action: UiAction::WestFacePress,
        release_action: Some(UiAction::WestFaceRelease),
    },
    DigitalBinding {
        name: "hold_to_sell",
        action: UiAction::WestFacePress,
        release_action: Some(UiAction::WestFaceRelease),
    },
    DigitalBinding {
        name: "inspect",
        action: UiAction::NorthFacePress,
        release_action: None,
    },
    DigitalBinding {
        name: "cash_in",
        action: UiAction::TriggerStructure,
        release_action: None,
    },
    DigitalBinding {
        name: "sort_suit",
        action: UiAction::SortBySuit,
        release_action: None,
    },
    DigitalBinding {
        name: "sort_rank",
        action: UiAction::SortByRank,
        release_action: None,
    },
    DigitalBinding {
        name: "tab_next",
        action: UiAction::TabNext,
        release_action: None,
    },
    DigitalBinding {
        name: "tab_prev",
        action: UiAction::TabPrev,
        release_action: None,
    },
    DigitalBinding {
        name: "page_next",
        action: UiAction::PageNext,
        release_action: None,
    },
    DigitalBinding {
        name: "page_prev",
        action: UiAction::PagePrev,
        release_action: None,
    },
    DigitalBinding {
        name: "hud_next",
        action: UiAction::NavigateHudNext,
        release_action: None,
    },
    DigitalBinding {
        name: "hud_prev",
        action: UiAction::NavigateHudPrev,
        release_action: None,
    },
    DigitalBinding {
        name: "undo_discard",
        action: UiAction::UndoDiscard,
        release_action: None,
    },
    DigitalBinding {
        name: "pause",
        action: UiAction::Pause,
        release_action: None,
    },
    DigitalBinding {
        name: "help",
        action: UiAction::Help,
        release_action: None,
    },
    DigitalBinding {
        name: "delete",
        action: UiAction::Delete,
        release_action: None,
    },
];

pub struct SteamInputBridge {
    input: Input,
    sets: ActionSetHandles,
    digitals: Vec<(DigitalBinding, sys::InputDigitalActionHandle_t)>,
    digitals_by_name: HashMap<&'static str, sys::InputDigitalActionHandle_t>,
    analogs: AnalogHandles,
    controllers: Vec<sys::InputHandle_t>,
    prev_digital: HashSet<(sys::InputHandle_t, &'static str)>,
    active_set: ActionSet,
    style_by_controller: HashMap<sys::InputHandle_t, GamepadStyle>,
    rumble_stop_at: Option<Instant>,
    /// Pulses queued by [`Self::schedule_rumble_pulse`] (rumble lab composites,
    /// scoring cascade beats). Drained by [`Self::run_frame`] when each pulse's
    /// fire time arrives. Mirrors [`crate::ui::input::InputState::scoring_rumble_schedule`]
    /// for the SDL fallback path.
    rumble_schedule: Vec<ScheduledPulse>,
}

#[derive(Clone, Copy)]
struct ScheduledPulse {
    at: Instant,
    weak: u16,
    strong: u16,
    duration_ms: u32,
    gain: f32,
}

impl SteamInputBridge {
    pub fn new(input: Input) -> Self {
        let sets = ActionSetHandles {
            menus: input.get_action_set_handle(ActionSet::Menus.name()),
            gameplay: input.get_action_set_handle(ActionSet::Gameplay.name()),
            shop: input.get_action_set_handle(ActionSet::Shop.name()),
            inspect: input.get_action_set_handle(ActionSet::Inspect.name()),
        };
        let digitals: Vec<_> = DIGITAL_BINDINGS
            .iter()
            .copied()
            .map(|binding| (binding, input.get_digital_action_handle(binding.name)))
            .collect();
        let digitals_by_name = digitals
            .iter()
            .map(|(binding, handle)| (binding.name, *handle))
            .collect();
        let analogs = AnalogHandles {
            nav: input.get_analog_action_handle("nav"),
            orbit: input.get_analog_action_handle("orbit"),
            inspect_zoom: input.get_analog_action_handle("inspect_zoom"),
            cursor: input.get_analog_action_handle("cursor"),
        };
        Self {
            input,
            sets,
            digitals,
            digitals_by_name,
            analogs,
            controllers: Vec::new(),
            prev_digital: HashSet::new(),
            active_set: ActionSet::Menus,
            style_by_controller: HashMap::new(),
            rumble_stop_at: None,
            rumble_schedule: Vec::new(),
        }
    }

    pub fn run_frame(&mut self) {
        self.input.run_frame();
        self.controllers = self.input.get_connected_controllers();
        self.style_by_controller.clear();
        for &controller in &self.controllers {
            if controller == 0 {
                continue;
            }
            self.style_by_controller.insert(
                controller,
                Self::style_for_type(self.input.get_input_type_for_handle(controller)),
            );
        }
        let now = Instant::now();
        let mut fired: Vec<ScheduledPulse> = Vec::new();
        self.rumble_schedule.retain(|pulse| {
            if pulse.at <= now {
                fired.push(*pulse);
                false
            } else {
                true
            }
        });
        for pulse in fired {
            self.trigger_rumble(pulse.weak, pulse.strong, pulse.duration_ms, pulse.gain);
        }
        if self
            .rumble_stop_at
            .is_some_and(|stop_at| Instant::now() >= stop_at)
        {
            self.stop_rumble();
        }
    }

    /// Queue a rumble pulse to fire at `at`. Used by composite/staggered
    /// rumble-lab patterns and by the scoring cascade.
    pub fn schedule_rumble_pulse(
        &mut self,
        at: Instant,
        weak: u16,
        strong: u16,
        duration_ms: u32,
        gain: f32,
    ) {
        if duration_ms == 0 {
            return;
        }
        self.rumble_schedule.push(ScheduledPulse {
            at,
            weak,
            strong,
            duration_ms: duration_ms.max(1),
            gain,
        });
    }

    pub fn set_active_action_set(&mut self, set: ActionSet) {
        self.active_set = set;
        let handle = self.sets.get(set);
        for &controller in &self.controllers {
            if controller == 0 {
                continue;
            }
            self.input.activate_action_set_handle(controller, handle);
        }
    }

    pub fn poll(&mut self, out_actions: &mut Vec<UiAction>, analog: &mut AnalogSnapshot) -> bool {
        let mut used_controller = false;
        let mut next_pressed = HashSet::new();
        let set_handle = self.sets.get(self.active_set);
        for &controller in &self.controllers {
            if controller == 0 {
                continue;
            }
            self.input
                .activate_action_set_handle(controller, set_handle);
            for &(binding, handle) in &self.digitals {
                let data = self.input.get_digital_action_data(controller, handle);
                if !data.bActive {
                    continue;
                }
                let key = (controller, binding.name);
                if data.bState {
                    next_pressed.insert(key);
                    if !self.prev_digital.contains(&key) {
                        out_actions.push(binding.action);
                        used_controller = true;
                    }
                } else if self.prev_digital.contains(&key)
                    && let Some(release_action) = binding.release_action
                {
                    out_actions.push(release_action);
                    used_controller = true;
                }
            }

            let nav = self
                .input
                .get_analog_action_data(controller, self.analogs.nav);
            if nav.bActive {
                let (x, y) = analog_xy(&nav);
                Self::push_nav_edges(x, y, out_actions, &mut used_controller);
            }

            if self.active_set == ActionSet::Inspect {
                let orbit = self
                    .input
                    .get_analog_action_data(controller, self.analogs.orbit);
                if orbit.bActive {
                    let (x, y) = analog_xy(&orbit);
                    analog.orbit.0 = (analog.orbit.0 + x).clamp(-1.0, 1.0);
                    analog.orbit.1 = (analog.orbit.1 + y).clamp(-1.0, 1.0);
                    used_controller |= x.abs() > 0.001 || y.abs() > 0.001;
                }
                let zoom = self
                    .input
                    .get_analog_action_data(controller, self.analogs.inspect_zoom);
                if zoom.bActive {
                    let (x, _) = analog_xy(&zoom);
                    analog.inspect_zoom = (analog.inspect_zoom + x).clamp(-1.0, 1.0);
                    used_controller |= x.abs() > 0.001;
                }
            }
        }
        self.prev_digital = next_pressed;
        used_controller
    }

    pub fn trigger_rumble(&mut self, weak: u16, strong: u16, duration_ms: u32, gain: f32) -> bool {
        if !self.controllers.iter().any(|&h| h != 0) || duration_ms == 0 {
            return false;
        }
        let g = gain.clamp(0.0, 1.0);
        let left = ((strong as f32) * g).min(65535.0) as u16;
        let right = ((weak as f32) * g).min(65535.0) as u16;
        if left == 0 && right == 0 {
            return false;
        }
        let raw = raw_input_ptr(&self.input);
        if raw.is_null() {
            return false;
        }
        for &controller in &self.controllers {
            if controller == 0 {
                continue;
            }
            unsafe {
                sys::SteamAPI_ISteamInput_TriggerVibration(raw, controller, left, right);
            }
        }
        self.rumble_stop_at = Some(Instant::now() + Duration::from_millis(u64::from(duration_ms)));
        true
    }

    pub fn stop_rumble(&mut self) {
        let raw = raw_input_ptr(&self.input);
        if raw.is_null() {
            return;
        }
        for &controller in &self.controllers {
            if controller == 0 {
                continue;
            }
            unsafe {
                sys::SteamAPI_ISteamInput_TriggerVibration(raw, controller, 0, 0);
            }
        }
        self.rumble_stop_at = None;
    }

    pub fn glyph_path_for(&self, action: UiAction) -> Option<PathBuf> {
        let action_name = self.action_name_for_glyph(action)?;
        let action_handle = *self.digitals_by_name.get(action_name)?;
        let controller = self.controllers.iter().copied().find(|&h| h != 0)?;
        let origins = self.input.get_digital_action_origins(
            controller,
            self.sets.get(self.active_set),
            action_handle,
        );
        let origin = origins.first().copied()?;
        let path = self.glyph_svg_path(origin).or_else(|| {
            let legacy = self.input.get_glyph_for_action_origin(origin);
            (!legacy.is_empty()).then_some(legacy)
        })?;
        Some(PathBuf::from(path))
    }

    fn action_name_for_glyph(&self, action: UiAction) -> Option<&'static str> {
        Some(match action {
            UiAction::Confirm => "confirm",
            UiAction::Cancel => "cancel",
            UiAction::WestFacePress => match self.active_set {
                ActionSet::Shop => "hold_to_sell",
                _ => "discard",
            },
            UiAction::NorthFacePress => match self.active_set {
                ActionSet::Gameplay => "play_hand",
                _ => "inspect",
            },
            UiAction::TriggerStructure => "cash_in",
            UiAction::FocusNext => "focus_next",
            UiAction::FocusPrev => "focus_prev",
            UiAction::FocusUp => "focus_up",
            UiAction::FocusDown => "focus_down",
            UiAction::Pause => "pause",
            UiAction::Help => "help",
            UiAction::Delete => "delete",
            UiAction::SortBySuit => "sort_suit",
            UiAction::SortByRank => "sort_rank",
            UiAction::TabNext => "tab_next",
            UiAction::TabPrev => "tab_prev",
            UiAction::PageNext => "page_next",
            UiAction::PagePrev => "page_prev",
            UiAction::NavigateHudNext => "hud_next",
            UiAction::NavigateHudPrev => "hud_prev",
            UiAction::UndoDiscard => "undo_discard",
            _ => return None,
        })
    }

    pub fn show_binding_panel(&self) -> bool {
        self.controllers
            .iter()
            .copied()
            .find(|&h| h != 0)
            .is_some_and(|controller| self.input.show_binding_panel(controller))
    }

    pub fn first_controller_style(&self) -> Option<GamepadStyle> {
        self.controllers
            .iter()
            .copied()
            .find(|&h| h != 0)
            .and_then(|controller| self.style_by_controller.get(&controller))
            .copied()
    }

    /// Live count from the most recent [`Self::run_frame`] poll. Used by
    /// the disconnect-detected auto-pause path.
    pub fn controller_count(&self) -> usize {
        self.controllers.iter().filter(|&&h| h != 0).count()
    }

    pub fn diagnostics(&self) -> String {
        format!(
            "Steam Input: set={:?}, controllers={}, actions={}",
            self.active_set,
            self.controller_count(),
            self.digitals.len()
        )
    }

    fn glyph_svg_path(&self, origin: sys::EInputActionOrigin) -> Option<String> {
        let raw = raw_input_ptr(&self.input);
        if raw.is_null() {
            return None;
        }
        let ptr = unsafe { sys::SteamAPI_ISteamInput_GetGlyphSVGForActionOrigin(raw, origin, 0) };
        if ptr.is_null() {
            return None;
        }
        let s = unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned();
        (!s.is_empty()).then_some(s)
    }

    fn style_for_type(input_type: InputType) -> GamepadStyle {
        match input_type {
            InputType::PS3Controller | InputType::PS4Controller | InputType::PS5Controller => {
                GamepadStyle::PlayStation
            }
            InputType::SwitchJoyConPair
            | InputType::SwitchJoyConSingle
            | InputType::SwitchProController => GamepadStyle::Nintendo,
            InputType::SteamDeckController
            | InputType::SteamController
            | InputType::XBox360Controller
            | InputType::XBoxOneController => GamepadStyle::Xbox,
            _ => GamepadStyle::Generic,
        }
    }

    fn push_nav_edges(x: f32, y: f32, out_actions: &mut Vec<UiAction>, used_controller: &mut bool) {
        const DEADZONE: f32 = 0.65;
        if x >= DEADZONE {
            out_actions.push(UiAction::FocusNext);
            *used_controller = true;
        } else if x <= -DEADZONE {
            out_actions.push(UiAction::FocusPrev);
            *used_controller = true;
        }
        if y >= DEADZONE {
            out_actions.push(UiAction::FocusDown);
            *used_controller = true;
        } else if y <= -DEADZONE {
            out_actions.push(UiAction::FocusUp);
            *used_controller = true;
        }
    }
}

fn raw_input_ptr(input: &Input) -> *mut sys::ISteamInput {
    unsafe { *(input as *const Input as *const *mut sys::ISteamInput) }
}

fn analog_xy(data: &sys::InputAnalogActionData_t) -> (f32, f32) {
    unsafe {
        (
            std::ptr::addr_of!(data.x).read_unaligned(),
            std::ptr::addr_of!(data.y).read_unaligned(),
        )
    }
}

#[cfg(test)]
mod vdf_consistency {
    //! Catch drift between Rust-side action declarations and the In-Game Actions
    //! file (`packaging/steam_input/game_actions_4636490.vdf`). A mismatch here
    //! would make the bridge silently swallow inputs at runtime.

    use super::{ActionSet, DIGITAL_BINDINGS};
    use std::collections::HashSet;

    const VDF: &str = include_str!("../../packaging/steam_input/game_actions_4636490.vdf");

    /// Analog handles registered separately by [`AnalogHandles`]; not in [`DIGITAL_BINDINGS`].
    const ANALOG_NAMES: &[&str] = &["nav", "orbit", "inspect_zoom", "cursor"];

    enum Tok {
        Str(String),
        Open,
        Close,
    }

    fn lex(s: &str) -> Vec<Tok> {
        let mut out = Vec::new();
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '"' => {
                    let mut buf = String::new();
                    for n in chars.by_ref() {
                        if n == '"' {
                            break;
                        }
                        buf.push(n);
                    }
                    out.push(Tok::Str(buf));
                }
                '{' => out.push(Tok::Open),
                '}' => out.push(Tok::Close),
                _ => {}
            }
        }
        out
    }

    /// Walks the VDF and yields every `(parent_block, key)` pair. Parent block
    /// is the immediately enclosing `{ … }` whose own key is, e.g., `Button`.
    fn walk_pairs(vdf: &str) -> Vec<(String, String)> {
        let tokens = lex(vdf);
        let mut path: Vec<String> = Vec::new();
        let mut out = Vec::new();
        let mut i = 0;
        while i < tokens.len() {
            match &tokens[i] {
                Tok::Str(key) => match tokens.get(i + 1) {
                    Some(Tok::Open) => {
                        if let Some(parent) = path.last() {
                            out.push((parent.clone(), key.clone()));
                        }
                        path.push(key.clone());
                        i += 2;
                    }
                    Some(Tok::Str(_)) => {
                        if let Some(parent) = path.last() {
                            out.push((parent.clone(), key.clone()));
                        }
                        i += 2;
                    }
                    _ => i += 1,
                },
                Tok::Close => {
                    path.pop();
                    i += 1;
                }
                _ => i += 1,
            }
        }
        out
    }

    fn vdf_binding_names() -> HashSet<String> {
        walk_pairs(VDF)
            .into_iter()
            .filter(|(parent, _)| {
                matches!(parent.as_str(), "Button" | "AnalogTrigger" | "StickPadGyro")
            })
            .map(|(_, key)| key)
            .filter(|name| !ANALOG_NAMES.contains(&name.as_str()))
            .collect()
    }

    fn vdf_action_set_names() -> HashSet<String> {
        walk_pairs(VDF)
            .into_iter()
            .filter(|(parent, _)| parent == "actions")
            .map(|(_, key)| key)
            .collect()
    }

    #[test]
    fn vdf_bindings_match_digital_bindings() {
        let vdf: HashSet<String> = vdf_binding_names();
        let rust: HashSet<String> = DIGITAL_BINDINGS
            .iter()
            .map(|b| b.name.to_string())
            .collect();
        let only_in_vdf: Vec<&String> = vdf.difference(&rust).collect();
        let only_in_rust: Vec<&String> = rust.difference(&vdf).collect();
        assert!(
            only_in_vdf.is_empty() && only_in_rust.is_empty(),
            "Steam Input VDF / DIGITAL_BINDINGS drift:\n  only in VDF:  {only_in_vdf:?}\n  only in Rust: {only_in_rust:?}",
        );
    }

    #[test]
    fn vdf_action_sets_match_action_set_enum() {
        let vdf = vdf_action_set_names();
        let rust: HashSet<String> = [
            ActionSet::Menus,
            ActionSet::Gameplay,
            ActionSet::Shop,
            ActionSet::Inspect,
        ]
        .iter()
        .map(|s| s.name().to_string())
        .collect();
        let only_in_vdf: Vec<&String> = vdf.difference(&rust).collect();
        let only_in_rust: Vec<&String> = rust.difference(&vdf).collect();
        assert!(
            only_in_vdf.is_empty() && only_in_rust.is_empty(),
            "Steam Input VDF / ActionSet drift:\n  only in VDF:  {only_in_vdf:?}\n  only in Rust: {only_in_rust:?}",
        );
    }
}
