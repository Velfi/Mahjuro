//! Windows Alt+Enter arrives as a system key (`WM_SYSKEYDOWN`); SDL keymods on that
//! path often omit Alt. Read the live keyboard state as a fallback.

use crate::sdl_shell::SdlShell;
use sdl3::keyboard::{Mod, Scancode};

pub fn alt_modifier_held(shell: &SdlShell, keymod: Mod) -> bool {
    if keymod.contains(Mod::LALTMOD | Mod::RALTMOD) {
        return true;
    }
    let ks = shell.pump.keyboard_state();
    ks.is_scancode_pressed(Scancode::LAlt) || ks.is_scancode_pressed(Scancode::RAlt)
}
