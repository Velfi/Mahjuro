//! Invoked from `build.rs`: hash room-shadow bake inputs and run `mahjuro bake-room` when stale.
//!
//! Runs when the inputs stamp differs from `assets/data/room_shadow/.inputs_stamp` or a `.msh`
//! is missing (any build profile). Set `MAHJURO_SKIP_ROOM_SHADOW_BAKE=1` to disable.

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

const MSH_FORMAT_VERSION: u32 = 2;
const STAMP_PATH: &str = "assets/data/room_shadow/.inputs_stamp";
const OUT_DIR: &str = "assets/data/room_shadow";
const ROOMS: &[&str] = &[
    "shop",
    "hallway",
    "archive",
    "main_menu",
    "staircase",
    "gameplay",
];

pub fn stamp_input_paths(repo: &Path) -> Vec<PathBuf> {
    [
        "assets/3d/shop.glb",
        "assets/3d/hallway.glb",
        "assets/3d/archive.glb",
        "assets/3d/main_menu.glb",
        "assets/3d/staircase.glb",
        "assets/3d/gameplay.glb",
        "src/render/archive_glb.rs",
        "src/render/gameplay_glb.rs",
        "src/render/room_shadow_bake.rs",
        "src/render/wgpu_renderer/runtime/shop_environment.rs",
        "src/render/wgpu_renderer/runtime/shadow_setup.rs",
        "shaders/shadow.wgsl",
        "shaders/room_glb.wgsl",
    ]
    .into_iter()
    .map(|p| repo.join(p))
    .collect()
}

pub fn emit_rerun_if_changed() {
    println!("cargo:rerun-if-env-changed=MAHJURO_SKIP_ROOM_SHADOW_BAKE");
    println!("cargo:rerun-if-changed={STAMP_PATH}");
    for path in [
        "assets/3d/shop.glb",
        "assets/3d/hallway.glb",
        "assets/3d/archive.glb",
        "assets/3d/main_menu.glb",
        "assets/3d/staircase.glb",
        "assets/3d/gameplay.glb",
        "src/render/archive_glb.rs",
        "src/render/gameplay_glb.rs",
        "src/render/room_shadow_bake.rs",
        "src/render/wgpu_renderer/runtime/shop_environment.rs",
        "src/render/wgpu_renderer/runtime/shadow_setup.rs",
        "shaders/shadow.wgsl",
        "shaders/room_glb.wgsl",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
}

pub fn maybe_bake_room_shadows(repo: &Path, profile_dir: &Path) {
    if env::var("MAHJURO_SKIP_ROOM_SHADOW_BAKE")
        .map(|v| !v.is_empty() && v != "0" && !v.eq_ignore_ascii_case("false"))
        .unwrap_or(false)
    {
        println!("cargo:warning=MAHJURO_SKIP_ROOM_SHADOW_BAKE: skipping room shadow bake");
        return;
    }

    let stamp_file = repo.join(STAMP_PATH);
    let out_dir = repo.join(OUT_DIR);
    let hash = compute_inputs_hash(repo);
    let stamp_ok = read_stamp(&stamp_file).is_some_and(|s| s == hash);
    let outputs_ok = ROOMS
        .iter()
        .all(|room| out_dir.join(format!("{room}.msh")).is_file());

    if stamp_ok && outputs_ok {
        println!("cargo:info=room shadow bake: inputs unchanged, skipping GPU bake");
        return;
    }

    let Some(exe) = super::bake_tool::ensure_bake_exe(repo, profile_dir) else {
        if outputs_ok {
            println!(
                "cargo:warning=room shadow bake inputs changed but `mahjuro-bake` is not built yet; \
                 using existing .msh until rebuild"
            );
        } else {
            println!(
                "cargo:warning=room shadow bakes missing under {OUT_DIR}; run \
                 `cargo build -p mahjuro-bake` or `cargo run -p mahjuro-bake -- --kinds shadow`"
            );
        }
        return;
    };

    fs::create_dir_all(&out_dir).ok();
    let status = Command::new(&exe)
        .args([
            "--kinds",
            "shadow",
            "--shadow-dir",
            out_dir.to_str().unwrap_or(OUT_DIR),
        ])
        .status();
    match status {
        Ok(s) if s.success() => {}
        Ok(s) => {
            println!(
                "cargo:warning=room shadow bake failed (exit {s}); run `cargo run -p mahjuro-bake -- --kinds shadow` manually"
            );
            return;
        }
        Err(e) => {
            println!("cargo:warning=room shadow bake spawn failed: {e}");
            return;
        }
    }
    let _ = write_stamp(&stamp_file, &hash);
    println!("cargo:info=room shadow bake: wrote {OUT_DIR}/*.msh");
}

fn compute_inputs_hash(repo: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    MSH_FORMAT_VERSION.hash(&mut h);
    for p in stamp_input_paths(repo) {
        p.to_string_lossy().hash(&mut h);
        if p.is_file()
            && let Ok(bytes) = fs::read(p)
        {
            bytes.len().hash(&mut h);
        }
    }
    format!("{:016x}", h.finish())
}

fn read_stamp(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok()
}

fn write_stamp(path: &Path, hash: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, hash)
}

