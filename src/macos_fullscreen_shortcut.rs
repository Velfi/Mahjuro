//! macOS fullscreen chord uses Fn+F. SDL keymods do not carry the Fn / “globe”
//! hardware modifier; read AppKit’s current modifier flags instead.

use objc2_app_kit::{NSEvent, NSEventModifierFlags};

pub fn fn_modifier_held() -> bool {
    let flags = NSEvent::modifierFlags_class();
    flags.contains(NSEventModifierFlags::Function)
}
