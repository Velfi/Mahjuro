//! Shared hover / press visuals for in-game debug overlay panels.

use crate::theme::{self, ButtonState, ButtonVariant, color};

/// Mouse hover + held state tracked during overlay `update`.
#[derive(Clone, Copy, Debug, Default)]
pub struct DebugPointerState {
    pub hover_row: Option<usize>,
    pub mouse_held: bool,
}

impl DebugPointerState {
    pub fn sync_held(&mut self, mouse: Option<(f32, f32, bool, bool)>) {
        self.mouse_held = mouse.is_some_and(|(_, _, _, held)| held);
    }

    pub fn sync_held_triple(&mut self, mouse: Option<(f32, f32, bool)>, mouse_held: bool) {
        self.mouse_held = mouse.is_some() && mouse_held;
    }

    pub fn clear_hover(&mut self) {
        self.hover_row = None;
    }

    pub fn set_hover_if_hit(&mut self, mx: f32, my: f32, row: usize, rect: [f32; 4]) -> bool {
        if point_in_rect(mx, my, rect) {
            self.hover_row = Some(row);
            true
        } else {
            false
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DebugRowVisual {
    pub highlighted: bool,
    pub pressed: bool,
}

impl DebugRowVisual {
    pub fn for_row(row: usize, cursor: usize, pointer: &DebugPointerState) -> Self {
        let highlighted = pointer.hover_row == Some(row) || cursor == row;
        let pressed = pointer.mouse_held && pointer.hover_row == Some(row);
        Self {
            highlighted,
            pressed,
        }
    }
}

#[inline]
pub fn point_in_rect(mx: f32, my: f32, r: [f32; 4]) -> bool {
    mx >= r[0] && mx <= r[0] + r[2] && my >= r[1] && my <= r[1] + r[3]
}

#[inline]
pub fn point_in_rect_tuple(mx: f32, my: f32, r: (f32, f32, f32, f32)) -> bool {
    point_in_rect(mx, my, [r.0, r.1, r.2, r.3])
}

pub fn row_surface_colors(visual: DebugRowVisual, variant: ButtonVariant) -> ([f32; 4], [f32; 4]) {
    let state = if visual.highlighted {
        ButtonState::Hover
    } else {
        ButtonState::Rest
    };
    let mut colors = theme::button_colors(variant, state);
    if visual.pressed {
        colors.bg = color::darken(colors.bg, 0.14);
        colors.text = color::darken(colors.text, 0.08);
    }
    (colors.bg, colors.text)
}

pub fn slider_accent_color(visual: DebugRowVisual) -> [f32; 4] {
    if visual.pressed {
        color::darken(color::WALNUT_BRIGHT, 0.12)
    } else if visual.highlighted {
        color::WALNUT_BRIGHT
    } else {
        color::alpha(color::WALNUT_BRIGHT, 0.7)
    }
}
