//! Room GI + shadow bake freshness checks (invoked from `build.rs`).

use std::path::Path;

use mahjuro_bake_stamp::room_gi::RoomGi;
use mahjuro_bake_stamp::room_shadow::RoomShadow;

use super::{room_gi_bake, room_shadow_bake};

pub fn emit_rerun_if_changed() {
    room_gi_bake::emit_rerun_if_changed();
    room_shadow_bake::emit_rerun_if_changed();
}

pub fn assert_room_gpu_bakes_current(repo: &Path) {
    mahjuro_bake_stamp::assert_bake_current::<RoomGi>(repo);
    mahjuro_bake_stamp::assert_bake_current::<RoomShadow>(repo);
}
