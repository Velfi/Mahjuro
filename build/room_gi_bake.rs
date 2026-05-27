//! Invoked from `build.rs`: hash room-GI bake inputs and run `mahjuro bake-room` when stale.
//!
//! GPU bakes run when the inputs stamp differs from `assets/data/room_gi/.inputs_stamp`
//! or a `.mgi` is missing (any build profile). Set `MAHJURO_SKIP_ROOM_GI_BAKE=1` to disable.
//! Requires `mahjuro` in `target/<profile>/` — often needs a second `cargo build` after the binary exists.

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Must match [`crate::render::room_gi_bake::VERSION`] when that changes.
const MGI_FORMAT_VERSION: u32 = 2;
const BAKE_WIDTH: u32 = 1920;
const BAKE_HEIGHT: u32 = 1080;

const STAMP_PATH: &str = "assets/data/room_gi/.inputs_stamp";
const OUT_DIR: &str = "assets/data/room_gi";
const ROOMS: &[&str] = &[
    "shop",
    "hallway",
    "archive",
    "main_menu",
    "staircase",
    "gameplay",
];

/// Paths whose bytes are mixed into the inputs stamp (keep in sync with `rerun-if-changed` in `build.rs`).
pub fn stamp_input_paths(repo: &Path) -> Vec<PathBuf> {
    [
        "assets/3d/shop.glb",
        "assets/3d/hallway.glb",
        "assets/3d/archive.glb",
        "assets/3d/main_menu.glb",
        "assets/3d/staircase.glb",
        "assets/3d/gameplay.glb",
        "src/render/room_glb.rs",
        "shaders/emissive_probe_update.wgsl",
        "shaders/emissive_probe_apply.wgsl",
        "shaders/emissive_gi_composite.wgsl",
        "shaders/shop_glb.wgsl",
    ]
    .into_iter()
    .map(|p| repo.join(p))
    .collect()
}

pub fn emit_rerun_if_changed() {
    println!("cargo:rerun-if-env-changed=MAHJURO_SKIP_ROOM_GI_BAKE");
    println!("cargo:rerun-if-env-changed={STAMP_PATH}");
    for path in [
        "assets/3d/shop.glb",
        "assets/3d/hallway.glb",
        "assets/3d/archive.glb",
        "assets/3d/main_menu.glb",
        "assets/3d/staircase.glb",
        "assets/3d/gameplay.glb",
        "src/render/room_glb.rs",
        "shaders/emissive_probe_update.wgsl",
        "shaders/emissive_probe_apply.wgsl",
        "shaders/emissive_gi_composite.wgsl",
        "shaders/shop_glb.wgsl",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
}

pub fn maybe_bake_room_gi(repo: &Path, profile_dir: &Path) {
    if skip_bake_env() {
        println!("cargo:warning=MAHJURO_SKIP_ROOM_GI_BAKE: skipping room GI probe bake");
        return;
    }

    let stamp_file = repo.join(STAMP_PATH);
    let out_dir = repo.join(OUT_DIR);
    let hash = compute_inputs_hash(repo);
    let stamp_ok = read_stamp(&stamp_file).is_some_and(|s| s == hash);
    let outputs_ok = ROOMS
        .iter()
        .all(|room| out_dir.join(format!("{room}.mgi")).is_file());

    if stamp_ok && outputs_ok {
        println!("cargo:info=room GI bake: inputs unchanged, skipping GPU bake");
        return;
    }

    let Some(exe) = super::bake_tool::ensure_bake_exe(repo, profile_dir) else {
        if outputs_ok {
            println!(
                "cargo:warning=room GI bake inputs changed but `mahjuro-bake` is not built yet \
                 (expected in {}); using existing .mgi until you rebuild — run \
                 `cargo build -p mahjuro-bake` or `cargo run -p mahjuro-bake -- --kinds gi`",
                profile_dir.display()
            );
        } else {
            println!(
                "cargo:warning=room GI bakes missing under {OUT_DIR} and `mahjuro-bake` is not in \
                 {}; run `cargo build -p mahjuro-bake`, or `cargo run -p mahjuro-bake -- --kinds gi`",
                profile_dir.display()
            );
        }
        return;
    };

    if !out_dir.is_dir() {
        let _ = fs::create_dir_all(&out_dir);
    }

    println!(
        "cargo:warning=room GI bake: inputs stale, running GPU bake via {}",
        exe.display()
    );
    let status = Command::new(&exe)
        .current_dir(repo)
        .args([
            "--kinds",
            "gi",
            "--gi-dir",
            out_dir.to_str().unwrap_or(OUT_DIR),
            "--width",
            &BAKE_WIDTH.to_string(),
            "--height",
            &BAKE_HEIGHT.to_string(),
        ])
        .status();
    match status {
        Ok(s) if s.success() => {}
        Ok(s) => {
            let any_missing = ROOMS
                .iter()
                .any(|room| !out_dir.join(format!("{room}.mgi")).is_file());
            if any_missing {
                panic!(
                    "room GI bake failed ({s}); fix GPU/headless init or set MAHJURO_SKIP_ROOM_GI_BAKE=1"
                );
            }
            println!(
                "cargo:warning=room GI bake failed ({s}); keeping existing .mgi files — \
                 rebuild once with MAHJURO_SKIP_ROOM_GI_BAKE=1 if bakes stay stale"
            );
        }
        Err(e) => panic!("failed to spawn room GI bake: {e}"),
    }

    if let Err(e) = write_stamp(&stamp_file, &hash) {
        println!(
            "cargo:warning=room GI bake: could not write stamp {}: {e}",
            stamp_file.display()
        );
    }
}

fn skip_bake_env() -> bool {
    env::var("MAHJURO_SKIP_ROOM_GI_BAKE")
        .map(|v| {
            let v = v.trim();
            !v.is_empty() && v != "0" && !v.eq_ignore_ascii_case("false")
        })
        .unwrap_or(false)
}

fn compute_inputs_hash(repo: &Path) -> String {
    let mut h = Fnv64::new();
    h.write(format!("mgi-v{MGI_FORMAT_VERSION}\n").as_bytes());
    h.write(format!("bake-{BAKE_WIDTH}x{BAKE_HEIGHT}\n").as_bytes());
    for path in stamp_input_paths(repo) {
        h.write(path.to_string_lossy().as_bytes());
        h.write(b"\0");
        if path.is_file()
            && let Ok(bytes) = fs::read(&path)
        {
            h.write(&bytes);
        }
    }
    format!("{:016x}", h.finish())
}

fn read_stamp(path: &Path) -> Option<String> {
    let s = fs::read_to_string(path).ok()?;
    let line = s.lines().next()?.trim();
    if line.is_empty() {
        None
    } else {
        Some(line.to_string())
    }
}

fn write_stamp(path: &Path, hash: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, format!("{hash}\n"))
}

/// FNV-1a 64-bit (stable across toolchains for build stamps).
struct Fnv64 {
    state: u64,
}

impl Fnv64 {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;

    fn new() -> Self {
        Self {
            state: Self::OFFSET,
        }
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.state ^= u64::from(b);
            self.state = self.state.wrapping_mul(Self::PRIME);
        }
    }

    fn finish(self) -> u64 {
        self.state
    }
}
