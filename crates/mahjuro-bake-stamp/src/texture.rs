//! Generic BTX1 static texture bake stamp.
//!
//! Inputs: every shipped texture under `assets/textures/` except relic/source/raw
//! authoring files, the GLB files whose material textures are extracted, and the
//! render-side BTX bake/runtime source files. Bumping `texture-btx1-vN`
//! invalidates every committed generic texture bake.

use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

use crate::{hash_paths, BakeKind, Fnv64};

pub struct Texture;

impl BakeKind for Texture {
    const LABEL: &'static str = "generic BTX1 texture bake";
    const STAMP_PATH: &'static str = "assets/data/texture_baked/.inputs_stamp";
    const OUT_DIR: &'static str = "assets/data/texture_baked";
    const SKIP_ENV: &'static str = "MAHJURO_SKIP_TEXTURE_BAKE";
    const SCRIPT_REBAKE_CMD: &'static str = "scripts/rebake-offline.sh texture";
    const BUILD_TOOL_CMD: &'static str =
        "cargo build -p mahjuro-render --bin mahjuro-bake-textures --features texture_bc7_bake";
    const REBAKE_CMD: &'static str =
        "cargo run -p mahjuro-render --bin mahjuro-bake-textures --features texture_bc7_bake";
    const COMMIT_PATHS: &'static str =
        "assets/data/texture_baked/**/*.btx assets/data/texture_baked/.inputs_stamp";

    fn stamp_input_paths(repo: &Path) -> Vec<PathBuf> {
        [
            "crates/mahjuro-render/src/baked_texture.rs",
            "crates/mahjuro-render/src/static_texture_bakes.rs",
            "crates/mahjuro-render/src/bin/mahjuro-bake-textures.rs",
            "crates/mahjuro-render/src/talisman_mesh.rs",
            "crates/mahjuro-render/src/tile_glb.rs",
            "crates/mahjuro-render/src/room_env_gltf.rs",
            "assets/textures",
            "assets/3d/shop.glb",
            "assets/3d/gameplay.glb",
            "assets/3d/hallway.glb",
            "assets/3d/staircase.glb",
            "assets/3d/archive.glb",
            "assets/3d/main_menu.glb",
            "assets/3d/shadow_test_room.glb",
            "assets/3d/tile_bamboo_and_ivory.glb",
            "assets/3d/tile_plastic.glb",
            "assets/3d/tile_tortoise_shell.glb",
            "assets/3d/coin.glb",
        ]
        .into_iter()
        .map(|p| repo.join(p))
        .collect()
    }

    fn compute_inputs_hash(repo: &Path) -> String {
        let mut h = Fnv64::new();
        h.write(b"texture-btx1-v6\n");
        for path in Self::stamp_input_paths(repo) {
            if path.is_file() {
                hash_paths(&mut h, repo, std::slice::from_ref(&path));
            } else if path.is_dir() {
                let files = hashable_texture_input_files(&path);
                hash_paths(&mut h, repo, &files);
            }
        }
        h.finish_hex()
    }

    fn outputs_ok(repo: &Path) -> bool {
        let dir = repo.join(Self::OUT_DIR);
        if !dir.join(".inputs_stamp").is_file() {
            return false;
        }
        let expected_static = expected_static_btx_count(repo);
        if expected_static == 0 {
            return false;
        }
        let baked_static = count_btx_under(&dir.join("textures"));
        let baked_gltf = count_btx_under(&dir.join("3d_gltf"));
        baked_static >= expected_static && baked_gltf > 0
    }
}

fn hashable_texture_input_files(root: &Path) -> Vec<PathBuf> {
    let mut walk = WalkBuilder::new(root);
    walk.hidden(false);
    walk.git_ignore(true);
    walk.git_global(true);
    walk.git_exclude(true);
    walk.parents(true);
    walk.require_git(false);

    let mut files: Vec<PathBuf> = walk
        .build()
        .filter_map(Result::ok)
        .map(|entry| entry.into_path())
        .filter(|path| path.is_file())
        .filter(|path| {
            let rel = path.to_string_lossy().replace('\\', "/");
            rel.contains("/assets/textures/")
                && !rel.contains("/assets/textures/relics/")
                && !rel.contains("/source/")
                && !rel.ends_with("_raw.png")
        })
        .collect();
    files.sort();
    files
}

fn expected_static_btx_count(repo: &Path) -> usize {
    let root = repo.join("assets/textures");
    hashable_texture_input_files(&root)
        .into_iter()
        .filter(|path| {
            path.extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| x.eq_ignore_ascii_case("png"))
        })
        .count()
}

fn count_btx_under(root: &Path) -> usize {
    let mut walk = WalkBuilder::new(root);
    walk.hidden(false);
    walk.git_ignore(false);
    walk.git_global(false);
    walk.git_exclude(false);
    walk.parents(false);
    walk.require_git(false);

    walk.build()
        .filter_map(Result::ok)
        .map(|entry| entry.into_path())
        .filter(|path| path.extension().is_some_and(|x| x == "btx"))
        .count()
}

/// Paths the build script should announce via `cargo:rerun-if-changed`.
pub fn rerun_if_changed_paths() -> &'static [&'static str] {
    &[
        "crates/mahjuro-render/src/baked_texture.rs",
        "crates/mahjuro-render/src/static_texture_bakes.rs",
        "crates/mahjuro-render/src/bin/mahjuro-bake-textures.rs",
        "crates/mahjuro-render/src/tile_glb.rs",
        "crates/mahjuro-render/src/room_env_gltf.rs",
        "assets/textures",
        "assets/3d/shop.glb",
        "assets/3d/gameplay.glb",
        "assets/3d/hallway.glb",
        "assets/3d/staircase.glb",
        "assets/3d/archive.glb",
        "assets/3d/main_menu.glb",
        "assets/3d/shadow_test_room.glb",
        "assets/3d/tile_bamboo_and_ivory.glb",
        "assets/3d/tile_plastic.glb",
        "assets/3d/tile_tortoise_shell.glb",
        "assets/3d/coin.glb",
    ]
}
