//! Relic RLC1 bake stamp.
//!
//! Inputs: `assets/data/relics.json`, every PNG under `assets/textures/relics/`,
//! and the three runtime-side relic codec files (`relic_pipeline.rs`, `relic_bake.rs`,
//! `relic_dish.rs`). Bumping `relic-rlc1-vN` invalidates every committed bake.

use std::fs;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

use crate::{BakeKind, Fnv64, hash_paths};

pub struct Relic;

impl BakeKind for Relic {
    const LABEL: &'static str = "relic RLC1 bake";
    const STAMP_PATH: &'static str = "assets/data/relic_baked/.inputs_stamp";
    const OUT_DIR: &'static str = "assets/data/relic_baked";
    const SKIP_ENV: &'static str = "MAHJURO_SKIP_RELIC_BAKE";
    const BUILD_TOOL_CMD: &'static str = "cargo build -p mahjuro-render --bin mahjuro-bake-relics";
    const REBAKE_CMD: &'static str = "cargo run -p mahjuro-render --bin mahjuro-bake-relics";
    const COMMIT_PATHS: &'static str =
        "assets/data/relic_baked/*.rlc assets/data/relic_baked/.inputs_stamp";

    fn stamp_input_paths(repo: &Path) -> Vec<PathBuf> {
        [
            "crates/mahjuro-render/src/relic_pipeline.rs",
            "crates/mahjuro-render/src/relic_bake.rs",
            "crates/mahjuro-render/src/relic_dish.rs",
            "assets/data/relics.json",
            "assets/textures/relics",
        ]
        .into_iter()
        .map(|p| repo.join(p))
        .collect()
    }

    fn compute_inputs_hash(repo: &Path) -> String {
        let mut h = Fnv64::new();
        h.write(b"relic-rlc1-v1\n");
        for path in Self::stamp_input_paths(repo) {
            if path.is_file() {
                hash_paths(&mut h, repo, std::slice::from_ref(&path));
            } else if path.is_dir() {
                let files = hashable_relic_input_files(&path);
                hash_paths(&mut h, repo, &files);
            }
        }
        h.finish_hex()
    }

    fn outputs_ok(repo: &Path) -> bool {
        let expected = relic_def_count(repo);
        if expected == 0 {
            return false;
        }
        let dir = repo.join(Self::OUT_DIR);
        let Ok(read) = fs::read_dir(&dir) else {
            return false;
        };
        let baked = read
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "rlc"))
            .count();
        baked == expected
    }
}

fn hashable_relic_input_files(root: &Path) -> Vec<PathBuf> {
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
        .collect();
    files.sort();
    files
}

/// Count `"id":` lines in `relics.json` as a cheap stand-in for "how many bakes
/// should exist". Avoids parsing JSON in `build.rs`. Mirrors the previous
/// `build/relic_bake.rs` heuristic.
fn relic_def_count(repo: &Path) -> usize {
    let relics_json = repo.join("assets/data/relics.json");
    let Ok(raw) = fs::read_to_string(relics_json) else {
        return 0;
    };
    raw.lines()
        .filter(|line| {
            let t = line.trim();
            t.starts_with("\"id\"") && t.contains(':')
        })
        .count()
}

/// Paths the build script should announce via `cargo:rerun-if-changed`.
pub fn rerun_if_changed_paths() -> &'static [&'static str] {
    &[
        "crates/mahjuro-render/src/relic_pipeline.rs",
        "crates/mahjuro-render/src/relic_bake.rs",
        "crates/mahjuro-render/src/relic_dish.rs",
        "assets/data/relics.json",
        "assets/textures/relics",
    ]
}
