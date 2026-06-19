//! Showcase decal atlas bake stamp (per-tileset `showcase_decal_atlas.png`).

use std::path::{Path, PathBuf};

use crate::{BakeKind, Fnv64, git_tracked_files_under, hash_file_at_rel, repo_relative};

pub struct ShowcaseDecal;

impl BakeKind for ShowcaseDecal {
    const LABEL: &'static str = "showcase decal atlas bake";
    const STAMP_PATH: &'static str = "assets/textures/tile_sets/.decal_bake_stamp";
    const OUT_DIR: &'static str = "assets/textures/tile_sets";
    const SKIP_ENV: &'static str = "MAHJURO_SKIP_SHOWCASE_DECAL_BAKE";
    const SCRIPT_REBAKE_CMD: &'static str = "scripts/rebake-offline.sh decal";
    const BUILD_TOOL_CMD: &'static str =
        "cargo build -p mahjuro-render --bin mahjuro-bake-decal-atlases";
    const REBAKE_CMD: &'static str = "cargo run -p mahjuro-render --bin mahjuro-bake-decal-atlases";
    const COMMIT_PATHS: &'static str = "assets/textures/tile_sets/*/showcase_decal_atlas.png assets/textures/tile_sets/.decal_bake_stamp";

    fn stamp_input_paths(repo: &Path) -> Vec<PathBuf> {
        rerun_if_changed_paths()
            .iter()
            .map(|p| repo.join(p))
            .collect()
    }

    fn compute_inputs_hash(repo: &Path) -> String {
        let mut h = Fnv64::new();
        for rel in rerun_if_changed_paths() {
            let path = repo.join(rel);
            if path.is_file() {
                hash_file_at_rel(&mut h, rel, &path);
            } else if path.is_dir() {
                for file in git_tracked_files_under(repo, rel) {
                    let file_rel = repo_relative(repo, &file);
                    if !include_decal_tileset_input(&file_rel) {
                        continue;
                    }
                    hash_file_at_rel(&mut h, &file_rel, &file);
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

/// Bake inputs under `assets/textures/tile_sets`: shipped `atlas.png` + `atlas.toml` only.
/// Authoring art (`source/`, `.af`, layer PNGs, credits, baked outputs) is excluded.
fn include_decal_tileset_input(rel: &str) -> bool {
    let norm = rel.replace('\\', "/");
    if norm.contains("/source/") {
        return false;
    }
    matches!(
        Path::new(rel).file_name().and_then(|s| s.to_str()),
        Some("atlas.png") | Some("atlas.toml")
    )
}

/// Same rule as `mahjuro_assets::list_tilesets`: a shipped tileset has `atlas.png`.
fn list_tileset_names(repo: &Path) -> Vec<String> {
    let mut names: Vec<String> = git_tracked_files_under(repo, ShowcaseDecal::OUT_DIR)
        .into_iter()
        .filter(|path| path.file_name().is_some_and(|name| name == "atlas.png"))
        .filter_map(|path| {
            path.parent()
                .and_then(|parent| parent.file_name())
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
        .collect();
    names.sort();
    names.dedup();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tileset_input_allowlist_excludes_source_and_outputs() {
        assert!(include_decal_tileset_input(
            "assets/textures/tile_sets/classic/atlas.png"
        ));
        assert!(include_decal_tileset_input(
            "assets/textures/tile_sets/classic/atlas.toml"
        ));
        assert!(!include_decal_tileset_input(
            "assets/textures/tile_sets/original/source/tileset.svg"
        ));
        assert!(!include_decal_tileset_input(
            "assets/textures/tile_sets/antique/source/atlas.af"
        ));
        assert!(!include_decal_tileset_input(
            "assets/textures/tile_sets/painted_from_scratch/CREDIT.txt"
        ));
        assert!(!include_decal_tileset_input(
            "assets/textures/tile_sets/classic/showcase_decal_atlas.png"
        ));
    }
}
