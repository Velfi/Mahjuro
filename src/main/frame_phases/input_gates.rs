use super::locals::FrameLocals;
use crate::App;
use crate::Scene;
use crate::ui::input::UiAction;

pub fn run(app: &mut App, locals: &mut FrameLocals) {
    // does a "skim" gesture on paginated modals: tap = advance one page,
    // hold = auto-advance through remaining pages at a fast cadence.
    // The hold timer is driven inside `ModalQueue::update`; here we just
    // forward the press and release edges.
    if app.modals.is_active() {
        for a in &locals.actions {
            match a {
                UiAction::Confirm => {
                    app.modals.advance_page();
                    break;
                }
                UiAction::Cancel | UiAction::Pause => {
                    app.modals.cancel_pressed();
                    break;
                }
                UiAction::CancelRelease => {
                    app.modals.cancel_released();
                }
                UiAction::FocusNext => {
                    app.modals.navigate(1);
                    break;
                }
                UiAction::FocusPrev => {
                    app.modals.navigate(-1);
                    break;
                }
                _ => {}
            }
        }
        // Block all locals.actions from reaching the scene.
        locals.clear_input();
    } else {
        // Modal not active; make sure a leftover skim timer doesn't
        // tick into the next paginated modal that pops up.
        app.modals.cancel_released();
    }

    // Block scene input while a replace transition is fading or held at black.
    if app.scene_replace_in_flight() {
        locals.clear_input();
    }

    // Splash: LMB dismisses the production logo (same as Confirm/Cancel).
    if matches!(app.scene, Scene::Splash(_)) && app.mouse_clicked && !app.modals.is_active() {
        crate::render::wgpu_renderer::loading_screen::request_skip();
    }

    // Clear one-shot mouse click flag so it doesn't bleed into
    // the next frame if no overlay consumed it.
    app.mouse_clicked = false;
}
