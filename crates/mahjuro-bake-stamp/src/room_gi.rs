//! Room GI (.mgi) bake stamp.
//!
//! Bump [`MGI_FORMAT_VERSION`] (and the matching `mahjuro_render::room_gi_bake::VERSION`)
//! whenever the on-disk MGI layout changes; that string mixes into the hash so old
//! bakes correctly read as stale.

use std::path::{Path, PathBuf};

use crate::{BakeKind, Fnv64, hash_paths, outputs_present};

/// Must match `mahjuro_render::room_gi_bake::VERSION` when that changes.
pub const MGI_FORMAT_VERSION: u32 = 2;
pub const BAKE_WIDTH: u32 = 1920;
pub const BAKE_HEIGHT: u32 = 1080;

pub use crate::room_slugs::ALL as ROOMS;

pub struct RoomGi;

impl BakeKind for RoomGi {
    const LABEL: &'static str = "room GI bake";
    const STAMP_PATH: &'static str = "assets/data/room_gi/.inputs_stamp";
    const OUT_DIR: &'static str = "assets/data/room_gi";
    const SKIP_ENV: &'static str = "MAHJURO_SKIP_ROOM_GI_BAKE";
    const BUILD_TOOL_CMD: &'static str =
        "cargo build -p mahjuro-headless --bin mahjuro-bake --features bake";
    const REBAKE_CMD: &'static str = "MAHJURO_SKIP_COMMITTED_BAKE_CHECKS=1 cargo run -p mahjuro-headless --bin mahjuro-bake --features bake -- --kinds gi";
    const COMMIT_PATHS: &'static str =
        "assets/data/room_gi/*.mgi assets/data/room_gi/.inputs_stamp";

    fn stamp_input_paths(repo: &Path) -> Vec<PathBuf> {
        rerun_if_changed_paths()
            .iter()
            .map(|p| repo.join(p))
            .collect()
    }

    fn compute_inputs_hash(repo: &Path) -> String {
        let mut h = Fnv64::new();
        h.write(format!("mgi-v{MGI_FORMAT_VERSION}\n").as_bytes());
        h.write(format!("bake-{BAKE_WIDTH}x{BAKE_HEIGHT}\n").as_bytes());
        hash_paths(&mut h, repo, &Self::stamp_input_paths(repo));
        h.finish_hex()
    }

    fn outputs_ok(repo: &Path) -> bool {
        outputs_present(&repo.join(Self::OUT_DIR), ROOMS, "mgi")
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
        "crates/mahjuro-render/src/room_glb.rs",
        "shaders/emissive_probe_update.wgsl",
        "shaders/emissive_probe_apply.wgsl",
        "shaders/emissive_gi_composite.wgsl",
        "shaders/room_glb.wgsl",
    ]
}
