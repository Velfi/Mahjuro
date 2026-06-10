use super::locals::FrameLocals;
use crate::ui::input::UiAction;
use crate::App;
use crate::sdl_shell::SdlShell;
use crate::Scene;

pub fn run(app: &mut App, shell: &mut SdlShell, locals: &mut FrameLocals) {
    locals.button_clicks.append(&mut app.mouse_button_clicks);
    let showcase_orbit_overlay = app
        .overlay_stack
        .last()
        .is_some_and(|top| matches!(top, Scene::Showcase(s) if s.wants_orbit_input()));
    let gp_ctx = crate::ui::input::GamepadPollCtx {
        face_bindings: app.active_face_bindings(),
        item_inspect_overlay: showcase_orbit_overlay,
        shop_storeroom_orbit: app.shop_storeroom_face_active(),
    };
    locals.actions.append(&mut app.mouse_actions);
    if app.mouse_right_clicked {
        app.mouse_right_clicked = false;
        if app.item_inspect_rmb_active() {
            locals
                .actions
                .push(crate::ui::input::UiAction::NorthFacePress);
        }
    }
    if let Some(input) = app.input.as_mut() {
        input.item_inspect_orbit_stick = (0.0, 0.0);
        input.item_inspect_zoom_triggers = 0.0;
        if input.gamepad_frame_tick(shell, gp_ctx, &mut locals.actions) {
            locals.hide_cursor = true;
        }

        // Detect the falling edge — last controller present last frame,
        // none this frame — while the player was on a pad. Inject Pause
        // so gameplay's pause path opens naturally.
        let now_controller_present = shell
            .gamepad
            .gamepads()
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        if app.prev_controller_present
            && !now_controller_present
            && input.mode == crate::ui::input::InputMode::Controller
        {
            log::info!("controller disconnected — auto-pausing");
            locals.actions.push(crate::ui::input::UiAction::Pause);
            input.mode = crate::ui::input::InputMode::Cursor;
            shell.show_cursor(true);
        }
        app.prev_controller_present = now_controller_present;

        if input.mode == crate::ui::input::InputMode::Cursor {
            let size = app.last_drawable_px;
            let layout = app
                .layout_engine
                .solve(size.width as f32, size.height as f32);
            let hand_slot_count = app.run.hand().len().max(layout.hand_slot_count);
            let mut slots: Vec<(f32, f32, f32, f32)> =
                vec![(-9999.0, -9999.0, 0.0, 0.0); hand_slot_count];
            let picked = app
                .renderer
                .as_ref()
                .and_then(|r| r.pick_hand_tile(input.last_cursor.0, input.last_cursor.1));
            if let Some(idx) = picked {
                if idx >= slots.len() {
                    slots.resize(idx + 1, (-9999.0, -9999.0, 0.0, 0.0));
                }
                if let Some(s) = slots.get_mut(idx) {
                    *s = (
                        input.last_cursor.0 - 1.0,
                        input.last_cursor.1 - 1.0,
                        2.0,
                        2.0,
                    );
                }
            }
            input.update_pointer_hover(input.last_cursor, &slots);
        }

        for a in &locals.actions {
            match a {
                UiAction::FocusNext | UiAction::FocusPrev => {
                    input.wrap_focus_slot(*a, app.run.hand().len());
                }
                _ => {}
            }
        }
    }

    if locals.hide_cursor {
        shell.show_cursor(false);
    }
}
