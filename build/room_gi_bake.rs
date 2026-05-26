//! Invoked from `build.rs`: hash room-GI bake inputs and run `mahjuro bake-room-gi` when stale.
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

    let Some(exe) = find_mahjuro_exe(profile_dir) else {
        if outputs_ok {
            println!(
                "cargo:warning=room GI bake inputs changed but `mahjuro` is not built yet \
                 (expected in {}); using existing .mgi until you rebuild — run \
                 `cargo build` again or `mahjuro bake-room-gi`",
                profile_dir.display()
            );
        } else {
            println!(
                "cargo:warning=room GI bakes missing under {OUT_DIR} and `mahjuro` is not in \
                 {}; run `cargo build` twice, or bake manually with `mahjuro bake-room-gi`",
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
    for room in ROOMS {
        let status = Command::new(&exe)
            .current_dir(repo)
            .arg("bake-room-gi")
            .arg(room)
            .arg("--output-dir")
            .arg(&out_dir)
            .arg("--width")
            .arg(BAKE_WIDTH.to_string())
            .arg("--height")
            .arg(BAKE_HEIGHT.to_string())
            .status();
        match status {
            Ok(s) if s.success() => {}
            Ok(s) => {
                let out_path = out_dir.join(format!("{room}.mgi"));
                if out_path.is_file() {
                    println!(
                        "cargo:warning=room GI bake for {room} failed ({s}); keeping existing \
                         {} — rebuild once with MAHJURO_SKIP_ROOM_GI_BAKE=1 if bakes stay stale",
                        out_path.display()
                    );
                } else {
                    panic!(
                        "room GI bake for {room} failed ({s}); fix GPU/headless init or set \
                         MAHJURO_SKIP_ROOM_GI_BAKE=1"
                    );
                }
            }
            Err(e) => panic!("failed to spawn room GI bake for {room}: {e}"),
        }
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

fn find_mahjuro_exe(profile_dir: &Path) -> Option<PathBuf> {
    #[cfg(windows)]
    let names = ["mahjuro.exe", "mahjuro"];
    #[cfg(not(windows))]
    let names = ["mahjuro"];

    for name in names {
        let p = profile_dir.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
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
