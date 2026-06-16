//! Room GI lightmap bake stamp.
//!
//! Bump [`ROOM_LIGHTMAP_FORMAT_VERSION`] whenever the on-disk lightmap layout changes;
//! that string mixes into the hash so old bakes correctly read as stale.

use std::path::{Path, PathBuf};

use crate::{BakeKind, Fnv64, hash_paths, outputs_present};

pub const ROOM_LIGHTMAP_FORMAT_VERSION: u32 = 2;
pub const ROOM_LIGHTMAP_SIZE: u32 = 1024;
pub const BAKE_WIDTH: u32 = 1920;
pub const BAKE_HEIGHT: u32 = 1080;

pub use crate::room_slugs::LIGHTMAP_ALL as ROOMS;

pub struct RoomGi;

impl BakeKind for RoomGi {
    const LABEL: &'static str = "room GI lightmap bake";
    const STAMP_PATH: &'static str = "assets/data/room_lightmap/.inputs_stamp";
    const OUT_DIR: &'static str = "assets/data/room_lightmap";
    const SKIP_ENV: &'static str = "MAHJURO_SKIP_ROOM_GI_BAKE";
    const SCRIPT_REBAKE_CMD: &'static str = "scripts/rebake-offline.sh lightmap";
    const BUILD_TOOL_CMD: &'static str =
        "cargo build -p mahjuro-headless --bin mahjuro-bake --features bake";
    const REBAKE_CMD: &'static str = "MAHJURO_SKIP_COMMITTED_BAKE_CHECKS=1 cargo run -p mahjuro-headless --bin mahjuro-bake --features bake -- --kinds lightmap";
    const COMMIT_PATHS: &'static str = "assets/data/room_lightmap/*.lightmap.rlm.zst assets/data/room_lightmap/*.lightmap.png assets/data/room_lightmap/.inputs_stamp";

    fn stamp_input_paths(repo: &Path) -> Vec<PathBuf> {
        rerun_if_changed_paths()
            .iter()
            .map(|p| repo.join(p))
            .collect()
    }

    fn compute_inputs_hash(repo: &Path) -> String {
        let mut h = Fnv64::new();
        h.write(format!("rlm-v{ROOM_LIGHTMAP_FORMAT_VERSION}\n").as_bytes());
        h.write(format!("bake-{BAKE_WIDTH}x{BAKE_HEIGHT}\n").as_bytes());
        h.write(format!("lightmap-{ROOM_LIGHTMAP_SIZE}\n").as_bytes());
        hash_paths(&mut h, repo, &Self::stamp_input_paths(repo));
        h.finish_hex()
    }

    fn outputs_ok(repo: &Path) -> bool {
        let dir = repo.join(Self::OUT_DIR);
        outputs_present(&dir, ROOMS, "lightmap.rlm.zst")
            || outputs_present(&dir, ROOMS, "lightmap.rlm")
    }
}

pub fn rerun_if_changed_paths() -> Vec<&'static str> {
    // The lit room shader is `scene_pbr_with_hallway_warp!("room_glb.wgsl")`,
    // which wraps the room body in hallway warp + scene PBR lights + rainbow
    // swirl + moon phase + projected shadow. Sourcing that full composition from
    // `shader_program` keeps this list in lockstep with the shader embedding, so
    // editing any prepended shader invalidates the committed lightmap stamp.
    let mut paths = vec![
        "assets/3d/Shop.glb",
        "assets/3d/hallway.glb",
        "assets/3d/archive.glb",
        "assets/3d/main_menu.glb",
        "assets/3d/staircase.glb",
        "assets/3d/gameplay.glb",
        "assets/3d/shadow_test_room.glb",
        "crates/mahjuro-render/src/room_gi_bake.rs",
        "crates/mahjuro-render/src/room_glb.rs",
        "crates/mahjuro-render/src/room_lightmap_uv.rs",
        "shaders/room_gi_bake.wgsl",
    ];
    paths.extend(crate::shader_program::scene_pbr_with_hallway_warp(
        "shaders/room_glb.wgsl",
    ));
    paths
}
