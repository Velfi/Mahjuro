//! Types shared by the scene system and GPU draw recording.

use super::ui_action::UiAction;

/// Which background image to display behind the scene.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum BackgroundId {
    #[default]
    None,
    Black,
}

impl BackgroundId {
    pub fn asset_path(self) -> Option<&'static str> {
        match self {
            BackgroundId::None | BackgroundId::Black => None,
        }
    }

    pub fn image_vertex_color(self) -> [f32; 4] {
        let _ = self;
        [1.0, 1.0, 1.0, 1.0]
    }
}

#[derive(Clone, Copy, Debug)]
pub enum ButtonAction {
    Ui(UiAction),
    Scene(u32),
}

#[derive(Clone)]
pub struct ButtonDef {
    pub rect: (f32, f32, f32, f32),
    pub action: ButtonAction,
    pub hover_label: Option<std::borrow::Cow<'static, str>>,
}

impl ButtonDef {
    pub fn ui(rect: (f32, f32, f32, f32), action: UiAction) -> Self {
        Self {
            rect,
            action: ButtonAction::Ui(action),
            hover_label: None,
        }
    }

    pub fn scene(rect: (f32, f32, f32, f32), id: u32) -> Self {
        Self {
            rect,
            action: ButtonAction::Scene(id),
            hover_label: None,
        }
    }

    pub fn with_hover_label(mut self, label: impl Into<std::borrow::Cow<'static, str>>) -> Self {
        self.hover_label = Some(label.into());
        self
    }
}
