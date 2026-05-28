//! Room GI bake freshness directives (invoked from `build/room_gpu_bake.rs`).

use mahjuro_bake_stamp::room_gi::{RoomGi, rerun_if_changed_paths};

pub fn emit_rerun_if_changed() {
    mahjuro_bake_stamp::emit_rerun_if_changed::<RoomGi>(rerun_if_changed_paths());
}
