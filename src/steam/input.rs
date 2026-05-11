//! Steam Input bridge: converts ISteamInput action data into Mahjuro's
//! device-agnostic [`UiAction`] stream.

use std::collections::{HashMap, HashSet};
use std::ffi::CStr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use steamworks::{Input, InputType, sys};

use crate::ui::button_prompts::GamepadStyle;
use crate::ui::input::UiAction;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionSet {
    MenuControls,
    Gameplay,
    Shop,
    Inspect,
    RumbleLab,
    PauseMenu,
}

impl ActionSet {
    fn name(self) -> &'static str {
        match self {
            Self::MenuControls => "MenuControls",
            Self::Gameplay => "Gameplay",
            Self::Shop => "Shop",
            Self::Inspect => "Inspect",
            Self::RumbleLab => "RumbleLab",
            Self::PauseMenu => "PauseMenu",
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
    menu: sys::InputActionSetHandle_t,
    gameplay: sys::InputActionSetHandle_t,
    shop: sys::InputActionSetHandle_t,
    inspect: sys::InputActionSetHandle_t,
    rumble_lab: sys::InputActionSetHandle_t,
    pause_menu: sys::InputActionSetHandle_t,
}

impl ActionSetHandles {
    fn get(self, set: ActionSet) -> sys::InputActionSetHandle_t {
        match set {
            ActionSet::MenuControls => self.menu,
            ActionSet::Gameplay => self.gameplay,
            ActionSet::Shop => self.shop,
            ActionSet::Inspect => self.inspect,
            ActionSet::RumbleLab => self.rumble_lab,
            ActionSet::PauseMenu => self.pause_menu,
        }
    }
}

#[derive(Clone, Copy)]
struct AnalogHandles {
    nav: sys::InputAnalogActionHandle_t,
    orbit: sys::InputAnalogActionHandle_t,
    inspect_zoom: sys::InputAnalogActionHandle_t,
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
}

impl SteamInputBridge {
    pub fn new(input: Input) -> Self {
        let sets = ActionSetHandles {
            menu: input.get_action_set_handle(ActionSet::MenuControls.name()),
            gameplay: input.get_action_set_handle(ActionSet::Gameplay.name()),
            shop: input.get_action_set_handle(ActionSet::Shop.name()),
            inspect: input.get_action_set_handle(ActionSet::Inspect.name()),
            rumble_lab: input.get_action_set_handle(ActionSet::RumbleLab.name()),
            pause_menu: input.get_action_set_handle(ActionSet::PauseMenu.name()),
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
        };
        Self {
            input,
            sets,
            digitals,
            digitals_by_name,
            analogs,
            controllers: Vec::new(),
            prev_digital: HashSet::new(),
            active_set: ActionSet::MenuControls,
            style_by_controller: HashMap::new(),
            rumble_stop_at: None,
        }
    }

    pub fn run_frame(&mut self) {
        self.input.run_frame();
        self.controllers = self.input.get_connected_controllers();
        self.style_by_controller.clear();
        for &controller in &self.controllers {
            self.style_by_controller.insert(
                controller,
                Self::style_for_type(self.input.get_input_type_for_handle(controller)),
            );
        }
        if self
            .rumble_stop_at
            .is_some_and(|stop_at| Instant::now() >= stop_at)
        {
            self.stop_rumble();
        }
    }

    pub fn set_active_action_set(&mut self, set: ActionSet) {
        self.active_set = set;
        let handle = self.sets.get(set);
        for &controller in &self.controllers {
            self.input.activate_action_set_handle(controller, handle);
        }
    }

    pub fn poll(&mut self, out_actions: &mut Vec<UiAction>, analog: &mut AnalogSnapshot) -> bool {
        let mut used_controller = false;
        let mut next_pressed = HashSet::new();
        let set_handle = self.sets.get(self.active_set);
        for &controller in &self.controllers {
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
        if self.controllers.is_empty() || duration_ms == 0 {
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
            unsafe {
                sys::SteamAPI_ISteamInput_TriggerVibration(raw, controller, 0, 0);
            }
        }
        self.rumble_stop_at = None;
    }

    pub fn glyph_path_for(&self, action: UiAction) -> Option<PathBuf> {
        let action_name = self.action_name_for_glyph(action)?;
        let action_handle = *self.digitals_by_name.get(action_name)?;
        let controller = *self.controllers.first()?;
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
            .first()
            .copied()
            .is_some_and(|controller| self.input.show_binding_panel(controller))
    }

    pub fn first_controller_style(&self) -> Option<GamepadStyle> {
        self.controllers
            .first()
            .and_then(|controller| self.style_by_controller.get(controller))
            .copied()
    }

    pub fn diagnostics(&self) -> String {
        format!(
            "Steam Input: set={:?}, controllers={}, actions={}",
            self.active_set,
            self.controllers.len(),
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
