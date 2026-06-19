#[cfg(feature = "debug-menu")]
use crate::App;

#[cfg(feature = "debug-menu")]
pub fn run(app: &mut App) {
    if let Some(ref debug_menu) = app.debug.menu {
        for action in debug_menu.poll() {
            app.handle_debug_action(action);
        }
    }
}

#[cfg(not(feature = "debug-menu"))]
pub fn run(_app: &mut crate::App) {}
