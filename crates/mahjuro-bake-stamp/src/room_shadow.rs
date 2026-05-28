//! Room shadow (.msh) bake stamp.
//!
//! Bump [`MSH_FORMAT_VERSION`] whenever the on-disk MSH layout changes so existing
//! bakes correctly read as stale.

use std::path::{Path, PathBuf};

use crate::{BakeKind, Fnv64, hash_paths, outputs_present};

pub const MSH_FORMAT_VERSION: u32 = 2;

pub use crate::room_slugs::ALL as ROOMS;

pub struct RoomShadow;

impl BakeKind for RoomShadow {
    const LABEL: &'static str = "room shadow bake";
    const STAMP_PATH: &'static str = "assets/data/room_shadow/.inputs_stamp";
    const OUT_DIR: &'static str = "assets/data/room_shadow";
    const SKIP_ENV: &'static str = "MAHJURO_SKIP_ROOM_SHADOW_BAKE";
    const BUILD_TOOL_CMD: &'static str =
        "cargo build -p mahjuro-headless --bin mahjuro-bake --features bake";
    const REBAKE_CMD: &'static str =
        "MAHJURO_SKIP_COMMITTED_BAKE_CHECKS=1 cargo run -p mahjuro-headless --bin mahjuro-bake --features bake -- --kinds shadow";
    const COMMIT_PATHS: &'static str =
        "assets/data/room_shadow/*.msh assets/data/room_shadow/.inputs_stamp";

    fn stamp_input_paths(repo: &Path) -> Vec<PathBuf> {
        rerun_if_changed_paths()
            .iter()
            .map(|p| repo.join(p))
            .collect()
    }

    fn compute_inputs_hash(repo: &Path) -> String {
        let mut h = Fnv64::new();
        h.write(format!("msh-v{MSH_FORMAT_VERSION}\n").as_bytes());
        hash_paths(&mut h, repo, &Self::stamp_input_paths(repo));
        h.finish_hex()
    }

    fn outputs_ok(repo: &Path) -> bool {
        outputs_present(&repo.join(Self::OUT_DIR), ROOMS, "msh")
    }
}

pub fn rerun_if_changed_paths() -> &'static [&'static str] {
    &[
        "assets/3d/Shop.glb",
        "assets/3d/hallway.glb",
        "assets/3d/archive.glb",
        "assets/3d/main_menu.glb",
        "assets/3d/staircase.glb",
        "assets/3d/gameplay.glb",
        "crates/mahjuro-render/src/archive_glb.rs",
        "crates/mahjuro-render/src/gameplay_glb.rs",
        "crates/mahjuro-render/src/room_shadow_bake.rs",
        "crates/mahjuro-render/src/wgpu_renderer/runtime/shop_environment.rs",
        "crates/mahjuro-render/src/wgpu_renderer/runtime/shadow_setup.rs",
        "shaders/shadow.wgsl",
        "shaders/room_glb.wgsl",
    ]
}
