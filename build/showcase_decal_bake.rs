//! Invoked from `build.rs`: verify committed showcase decal atlases match inputs.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use super::input_hash::{
    assert_committed_bake_current, CommittedBakeCheck, Fnv64, hash_paths, hash_tree, read_stamp_line,
};

const STAMP_PATH: &str = "assets/textures/tile_sets/.decal_bake_stamp";
const OUT_DIR: &str = "assets/textures/tile_sets";

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

pub fn assert_showcase_decal_atlases_current(repo: &Path) {
    if skip_bake_env() {
        println!(
            "cargo:warning=MAHJURO_SKIP_SHOWCASE_DECAL_BAKE: skipping showcase decal atlas freshness check"
        );
        return;
    }

    let hash = compute_inputs_hash(repo);
    let stamp_ok = read_stamp_line(&repo.join(STAMP_PATH)).is_some_and(|s| s == hash);
    let outputs_ok = outputs_present(repo);

    assert_committed_bake_current(CommittedBakeCheck {
        label: "showcase decal atlas bake",
        stamp_path: STAMP_PATH,
        outputs_dir: OUT_DIR,
        commit_paths:
            "assets/textures/tile_sets/*/showcase_decal_atlas.png assets/textures/tile_sets/.decal_bake_stamp",
        expected_hash: &hash,
        stamp_ok,
        outputs_ok,
        skip_env: "MAHJURO_SKIP_SHOWCASE_DECAL_BAKE",
        build_tool_cmd: "cargo build -p mahjuro-render --bin mahjuro-bake-decal-atlases",
        rebake_cmd: "cargo run -p mahjuro-render --bin mahjuro-bake-decal-atlases",
    });
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
    let root = repo.join(OUT_DIR);
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
        repo.join(OUT_DIR)
            .join(name)
            .join("showcase_decal_atlas.png")
            .is_file()
    })
}
