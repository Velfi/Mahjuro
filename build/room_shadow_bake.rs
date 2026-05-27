//! Room shadow bake inputs and stamp (invoked from `build/room_gpu_bake.rs`).

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use super::input_hash::{Fnv64, hash_paths, outputs_present, read_stamp_line};

const MSH_FORMAT_VERSION: u32 = 2;
pub const STAMP_PATH: &str = "assets/data/room_shadow/.inputs_stamp";
pub const OUT_DIR: &str = "assets/data/room_shadow";
const ROOMS: &[&str] = &[
    "shop",
    "hallway",
    "archive",
    "main_menu",
    "staircase",
    "gameplay",
];

pub struct BakeStatus {
    pub hash: String,
    pub up_to_date: bool,
}

pub fn stamp_file(repo: &Path) -> PathBuf {
    repo.join(STAMP_PATH)
}

pub fn out_dir(repo: &Path) -> PathBuf {
    repo.join(OUT_DIR)
}

pub fn ensure_out_dir(repo: &Path) {
    let _ = fs::create_dir_all(out_dir(repo));
}

pub fn stamp_input_paths(repo: &Path) -> Vec<PathBuf> {
    [
        "assets/3d/shop.glb",
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
    .into_iter()
    .map(|p| repo.join(p))
    .collect()
}

pub fn emit_rerun_if_changed() {
    println!("cargo:rerun-if-env-changed=MAHJURO_SKIP_ROOM_SHADOW_BAKE");
    println!("cargo:rerun-if-env-changed={STAMP_PATH}");
    for path in [
        "assets/3d/shop.glb",
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
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
}

pub fn skip_bake_env() -> bool {
    env::var("MAHJURO_SKIP_ROOM_SHADOW_BAKE")
        .map(|v| {
            let v = v.trim();
            !v.is_empty() && v != "0" && !v.eq_ignore_ascii_case("false")
        })
        .unwrap_or(false)
}

pub fn compute_inputs_hash(repo: &Path) -> String {
    let mut h = Fnv64::new();
    h.write(format!("msh-v{MSH_FORMAT_VERSION}\n").as_bytes());
    hash_paths(&mut h, &stamp_input_paths(repo));
    h.finish_hex()
}

pub fn bake_status(repo: &Path) -> BakeStatus {
    let hash = compute_inputs_hash(repo);
    let stamp_ok = read_stamp_line(&stamp_file(repo)).is_some_and(|s| s == hash);
    let outputs_ok = outputs_present(&out_dir(repo), ROOMS, "msh");
    BakeStatus {
        hash,
        up_to_date: stamp_ok && outputs_ok,
    }
}
