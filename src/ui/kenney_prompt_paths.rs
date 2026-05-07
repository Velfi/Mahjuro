//! Relative paths under `assets/` for Kenney Input Prompts SVGs.

use super::button_prompts::{FaceButton, GamepadStyle, PromptInputSurface};

/// Four icons in order: **Exit**, **Select**, **Hold sell**, **Inspect** — matches the compact legend line.
pub fn shop_prompt_icon_paths(
    surface: PromptInputSurface,
    style: GamepadStyle,
    swap_ab: bool,
    swap_xy: bool,
) -> [&'static str; 4] {
    match surface {
        PromptInputSurface::MouseOrKeyboard => [
            "kenney_input-prompts/Keyboard & Mouse/Vector/keyboard_backspace.svg",
            "kenney_input-prompts/Keyboard & Mouse/Vector/keyboard_space.svg",
            "kenney_input-prompts/Keyboard & Mouse/Vector/keyboard_q.svg",
            "kenney_input-prompts/Keyboard & Mouse/Vector/keyboard_e.svg",
        ],
        PromptInputSurface::Controller => {
            let (exit_face, select_face) = if swap_ab {
                (FaceButton::South, FaceButton::East)
            } else {
                (FaceButton::East, FaceButton::South)
            };
            let (hold_face, inspect_face) = if swap_xy {
                (FaceButton::North, FaceButton::West)
            } else {
                (FaceButton::West, FaceButton::North)
            };
            [
                face_path(style, exit_face),
                face_path(style, select_face),
                face_path(style, hold_face),
                face_path(style, inspect_face),
            ]
        }
    }
}

fn face_path(style: GamepadStyle, face: FaceButton) -> &'static str {
    use FaceButton::{East, North, South, West};
    use GamepadStyle::{Generic, Nintendo, PlayStation, Xbox};
    match (style, face) {
        (Xbox | Generic, South) => {
            "kenney_input-prompts/Xbox Series/Vector/xbox_button_color_a.svg"
        }
        (Xbox | Generic, East) => "kenney_input-prompts/Xbox Series/Vector/xbox_button_color_b.svg",
        (Xbox | Generic, West) => "kenney_input-prompts/Xbox Series/Vector/xbox_button_color_x.svg",
        (Xbox | Generic, North) => {
            "kenney_input-prompts/Xbox Series/Vector/xbox_button_color_y.svg"
        }

        (PlayStation, South) => {
            "kenney_input-prompts/PlayStation Series/Vector/playstation_button_color_cross.svg"
        }
        (PlayStation, East) => {
            "kenney_input-prompts/PlayStation Series/Vector/playstation_button_color_circle.svg"
        }
        (PlayStation, West) => {
            "kenney_input-prompts/PlayStation Series/Vector/playstation_button_color_square.svg"
        }
        (PlayStation, North) => {
            "kenney_input-prompts/PlayStation Series/Vector/playstation_button_color_triangle.svg"
        }

        (Nintendo, South) => "kenney_input-prompts/Nintendo Switch 2/Vector/switch_button_b.svg",
        (Nintendo, East) => "kenney_input-prompts/Nintendo Switch 2/Vector/switch_button_a.svg",
        (Nintendo, West) => "kenney_input-prompts/Nintendo Switch 2/Vector/switch_button_y.svg",
        (Nintendo, North) => "kenney_input-prompts/Nintendo Switch 2/Vector/switch_button_x.svg",
    }
}
