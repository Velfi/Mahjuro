//! Showcase decal atlas bake stamp (per-tileset `showcase_decal_atlas.png`).

use std::fs;
use std::path::{Path, PathBuf};

use crate::{BakeKind, Fnv64, hash_file_at_rel, hashable_git_files, repo_relative};

pub struct ShowcaseDecal;

impl BakeKind for ShowcaseDecal {
    const LABEL: &'static str = "showcase decal atlas bake";
    const STAMP_PATH: &'static str = "assets/textures/tile_sets/.decal_bake_stamp";
    const OUT_DIR: &'static str = "assets/textures/tile_sets";
    const SKIP_ENV: &'static str = "MAHJURO_SKIP_SHOWCASE_DECAL_BAKE";
    const BUILD_TOOL_CMD: &'static str =
        "cargo build -p mahjuro-render --bin mahjuro-bake-decal-atlases";
    const REBAKE_CMD: &'static str = "cargo run -p mahjuro-render --bin mahjuro-bake-decal-atlases";
    const COMMIT_PATHS: &'static str =
        "assets/textures/tile_sets/*/showcase_decal_atlas.png assets/textures/tile_sets/.decal_bake_stamp";

    fn stamp_input_paths(repo: &Path) -> Vec<PathBuf> {
        rerun_if_changed_paths()
            .iter()
            .map(|p| repo.join(p))
            .collect()
    }

    fn compute_inputs_hash(repo: &Path) -> String {
        let mut h = Fnv64::new();
        h.write(b"showcase-decal-v2\n");
        for rel in rerun_if_changed_paths() {
            let path = repo.join(rel);
            if path.is_file() {
                hash_file_at_rel(&mut h, rel, &path);
            } else if path.is_dir() {
                for file in hashable_git_files(&path) {
                    let rel = repo_relative(repo, &file);
                    if skip_decal_input(&rel) {
                        continue;
                    }
                    hash_file_at_rel(&mut h, &rel, &file);
                }
            }
        }
        for name in list_tileset_names(repo) {
            h.write(name.as_bytes());
            h.write(b"\0");
        }
        h.finish_hex()
    }

    fn outputs_ok(repo: &Path) -> bool {
        list_tileset_names(repo).iter().all(|name| {
            repo.join(Self::OUT_DIR)
                .join(name)
                .join("showcase_decal_atlas.png")
                .is_file()
        })
    }
}

pub fn rerun_if_changed_paths() -> &'static [&'static str] {
    &[
        "crates/mahjuro-render/src/showcase_decal_atlas.rs",
        "crates/mahjuro-render/src/decal.rs",
        "assets/fonts",
        "assets/textures/tile_sets",
    ]
}

fn skip_decal_input(rel: &str) -> bool {
    let base = Path::new(rel)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    base == ".decal_bake_stamp"
        || base == "showcase_decal_atlas.png"
        || base == ".DS_Store"
        || base == "Thumbs.db"
        || base.eq_ignore_ascii_case("desktop.ini")
}

fn list_tileset_names(repo: &Path) -> Vec<String> {
    let root = repo.join(ShowcaseDecal::OUT_DIR);
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
