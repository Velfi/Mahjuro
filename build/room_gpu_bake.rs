//! Room GI + shadow bake freshness checks (invoked from `build.rs`).

use std::path::Path;

use super::input_hash::{assert_committed_bake_current, CommittedBakeCheck};
use super::{room_gi_bake, room_shadow_bake};

pub fn emit_rerun_if_changed() {
    room_gi_bake::emit_rerun_if_changed();
    room_shadow_bake::emit_rerun_if_changed();
}

pub fn assert_room_gpu_bakes_current(repo: &Path) {
    let gi = room_gi_bake::bake_status(repo);
    if !room_gi_bake::skip_bake_env() {
        assert_committed_bake_current(CommittedBakeCheck {
            label: "room GI bake",
            stamp_path: room_gi_bake::STAMP_PATH,
            outputs_dir: room_gi_bake::OUT_DIR,
            commit_paths: "assets/data/room_gi/*.mgi assets/data/room_gi/.inputs_stamp",
            expected_hash: &gi.hash,
            stamp_ok: gi.stamp_ok,
            outputs_ok: gi.outputs_ok,
            skip_env: "MAHJURO_SKIP_ROOM_GI_BAKE",
            build_tool_cmd:
                "cargo build -p mahjuro-headless --bin mahjuro-bake --features bake",
            rebake_cmd:
                "cargo run -p mahjuro-headless --bin mahjuro-bake --features bake -- --kinds gi",
        });
    } else {
        println!("cargo:warning=MAHJURO_SKIP_ROOM_GI_BAKE: skipping room GI bake freshness check");
    }

    let shadow = room_shadow_bake::bake_status(repo);
    if !room_shadow_bake::skip_bake_env() {
        assert_committed_bake_current(CommittedBakeCheck {
            label: "room shadow bake",
            stamp_path: room_shadow_bake::STAMP_PATH,
            outputs_dir: room_shadow_bake::OUT_DIR,
            commit_paths: "assets/data/room_shadow/*.msh assets/data/room_shadow/.inputs_stamp",
            expected_hash: &shadow.hash,
            stamp_ok: shadow.stamp_ok,
            outputs_ok: shadow.outputs_ok,
            skip_env: "MAHJURO_SKIP_ROOM_SHADOW_BAKE",
            build_tool_cmd:
                "cargo build -p mahjuro-headless --bin mahjuro-bake --features bake",
            rebake_cmd:
                "cargo run -p mahjuro-headless --bin mahjuro-bake --features bake -- --kinds shadow",
        });
    } else {
        println!(
            "cargo:warning=MAHJURO_SKIP_ROOM_SHADOW_BAKE: skipping room shadow bake freshness check"
        );
    }
}
