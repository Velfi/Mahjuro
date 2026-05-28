//! Room GI + shadow bake freshness (invoked from `build/offline_bake.rs`).

use super::{room_gi_bake, room_shadow_bake};

pub fn emit_rerun_if_changed() {
    room_gi_bake::emit_rerun_if_changed();
    room_shadow_bake::emit_rerun_if_changed();
}
