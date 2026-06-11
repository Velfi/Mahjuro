#[cfg(debug_menu_enabled)]
use crate::App;

#[cfg(debug_menu_enabled)]
pub fn run(app: &mut App) {
    if let Some(ref debug_menu) = app.debug.menu {
        for action in debug_menu.poll() {
            app.handle_debug_action(action);
        }
    }
}

#[cfg(not(debug_menu_enabled))]
pub fn run(_app: &mut crate::App) {}
