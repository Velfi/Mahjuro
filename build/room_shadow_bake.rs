//! Room shadow bake freshness directives (invoked from `build/room_gpu_bake.rs`).

use mahjuro_bake_stamp::room_shadow::{RoomShadow, rerun_if_changed_paths};

pub fn emit_rerun_if_changed() {
    mahjuro_bake_stamp::emit_rerun_if_changed::<RoomShadow>(rerun_if_changed_paths());
}
