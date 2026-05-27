//! Invoked from `build.rs`: hash inputs and run `mahjuro-bake-decal-atlases` when stale.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use super::input_hash::{Fnv64, hash_paths, hash_tree, log_bake_timing, read_stamp_line, write_stamp_line};

const STAMP_PATH: &str = "assets/textures/tile_sets/.decal_bake_stamp";

/// Baked outputs and the stamp file must not feed back into the input fingerprint.
fn skip_decal_input(rel: &str) -> bool {
    let base = std::path::Path::new(rel)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    base == ".decal_bake_stamp"
        || base == "showcase_decal_atlas.png"
        || base == ".DS_Store"
}

pub fn emit_rerun_if_changed() {
    println!("cargo:rerun-if-env-changed=MAHJURO_SKIP_SHOWCASE_DECAL_BAKE");
    println!("cargo:rerun-if-changed={STAMP_PATH}");
    for path in stamp_input_paths() {
        if let Ok(rel) = path.strip_prefix(env::var("CARGO_MANIFEST_DIR").unwrap_or_default()) {
            println!("cargo:rerun-if-changed={}", rel.display());
        }
    }
}

pub fn maybe_bake_showcase_decal_atlases(repo: &Path, profile_dir: &Path) {
    if skip_bake_env() {
        println!("cargo:warning=MAHJURO_SKIP_SHOWCASE_DECAL_BAKE: skipping showcase decal atlas bake");
        return;
    }

    let stamp_file = repo.join(STAMP_PATH);
    let hash = compute_inputs_hash(repo);
    let stamp_ok = read_stamp_line(&stamp_file).is_some_and(|s| s == hash);
    let outputs_ok = outputs_present(repo);

    if stamp_ok && outputs_ok {
        println!("cargo:info=showcase decal atlas bake: inputs unchanged, skipping");
        return;
    }

    let start = Instant::now();
    let exe = super::bake_tool::require_decal_bake_exe(profile_dir);

    let status = Command::new(&exe)
        .env("MAHJURO_ASSETS", repo.join("assets"))
        .current_dir(repo)
        .status();
    match status {
        Ok(s) if s.success() => {
            write_stamp_line(&stamp_file, &hash).unwrap_or_else(|e| {
                panic!("showcase decal atlas bake: could not write stamp: {e}");
            });
            log_bake_timing("showcase decal atlases", start);
        }
        Ok(s) => panic!(
            "showcase decal atlas bake failed (exit {s}); run \
             `cargo run -p mahjuro-render --bin mahjuro-bake-decal-atlases` manually"
        ),
        Err(e) => panic!("showcase decal atlas bake failed to run: {e}"),
    }
}

fn skip_bake_env() -> bool {
    matches!(
        env::var_os("MAHJURO_SKIP_SHOWCASE_DECAL_BAKE").as_deref(),
        Some(v) if v != "0" && v != "false"
    )
}

fn stamp_input_paths() -> Vec<PathBuf> {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    [
        "crates/mahjuro-render/src/showcase_decal_atlas.rs",
        "crates/mahjuro-render/src/decal.rs",
        "assets/fonts",
        "assets/textures/tile_sets",
    ]
    .into_iter()
    .map(|p| repo.join(p))
    .collect()
}

fn compute_inputs_hash(repo: &Path) -> String {
    let mut h = Fnv64::new();
    h.write(b"showcase-decal-v1\n");
    for path in stamp_input_paths() {
        if path.is_file() {
            hash_paths(&mut h, std::slice::from_ref(&path));
        } else if path.is_dir() {
            let rel = path
                .strip_prefix(repo)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            hash_tree(&mut h, &path, &rel, &skip_decal_input);
        }
    }
    for name in list_tileset_names(repo) {
        h.write(name.as_bytes());
        h.write(b"\0");
    }
    h.finish_hex()
}

fn list_tileset_names(repo: &Path) -> Vec<String> {
    let root = repo.join("assets/textures/tile_sets");
    let Ok(read) = fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut names: Vec<String> = read
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();
    names
}

fn outputs_present(repo: &Path) -> bool {
    list_tileset_names(repo).iter().all(|name| {
        repo.join("assets/textures/tile_sets")
            .join(name)
            .join("showcase_decal_atlas.png")
            .is_file()
    })
}
