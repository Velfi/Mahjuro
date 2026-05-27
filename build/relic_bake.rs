//! Invoked from `build.rs`: hash inputs and run `mahjuro-bake-relics` when stale.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use super::input_hash::{Fnv64, hash_paths, hash_tree, log_bake_timing, read_stamp_line, write_stamp_line};

const STAMP_PATH: &str = "assets/data/relic_baked/.inputs_stamp";
const OUT_DIR: &str = "assets/data/relic_baked";

fn skip_relic_input(rel: &str) -> bool {
    rel == ".inputs_stamp" || rel.ends_with(".rlc")
}

pub fn emit_rerun_if_changed() {
    println!("cargo:rerun-if-env-changed=MAHJURO_SKIP_RELIC_BAKE");
    println!("cargo:rerun-if-changed={STAMP_PATH}");
    for path in stamp_input_paths() {
        if let Ok(rel) = path.strip_prefix(env::var("CARGO_MANIFEST_DIR").unwrap_or_default()) {
            println!("cargo:rerun-if-changed={}", rel.display());
        }
    }
}

pub fn maybe_bake_relics(repo: &Path, profile_dir: &Path) {
    if skip_bake_env() {
        println!("cargo:warning=MAHJURO_SKIP_RELIC_BAKE: skipping relic RLC1 bake");
        return;
    }

    let stamp_file = repo.join(STAMP_PATH);
    let hash = compute_inputs_hash(repo);
    let stamp_ok = read_stamp_line(&stamp_file).is_some_and(|s| s == hash);
    let outputs_ok = outputs_present(repo);

    if stamp_ok && outputs_ok {
        println!("cargo:info=relic bake: inputs unchanged, skipping");
        return;
    }

    let start = Instant::now();
    let exe = super::bake_tool::require_relic_bake_exe(profile_dir);

    let status = Command::new(&exe)
        .env("MAHJURO_ASSETS", repo.join("assets"))
        .current_dir(repo)
        .status();
    match status {
        Ok(s) if s.success() => {
            if let Some(parent) = stamp_file.parent() {
                let _ = fs::create_dir_all(parent);
            }
            write_stamp_line(&stamp_file, &hash).unwrap_or_else(|e| {
                panic!("relic bake: could not write stamp: {e}");
            });
            log_bake_timing("relic RLC1 bakes", start);
        }
        Ok(s) => panic!(
            "relic bake failed (exit {s}); run \
             `cargo run -p mahjuro-render --bin mahjuro-bake-relics` manually"
        ),
        Err(e) => panic!("relic bake failed to run: {e}"),
    }
}

fn skip_bake_env() -> bool {
    matches!(
        env::var_os("MAHJURO_SKIP_RELIC_BAKE").as_deref(),
        Some(v) if v != "0" && v != "false"
    )
}

fn stamp_input_paths() -> Vec<PathBuf> {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
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
    for path in stamp_input_paths() {
        if path.is_file() {
            hash_paths(&mut h, std::slice::from_ref(&path));
        } else if path.is_dir() {
            let rel = path
                .strip_prefix(repo)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            hash_tree(&mut h, &path, &rel, &skip_relic_input);
        }
    }
    h.finish_hex()
}

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

fn outputs_present(repo: &Path) -> bool {
    let expected = relic_def_count(repo);
    if expected == 0 {
        return false;
    }
    let dir = repo.join(OUT_DIR);
    let Ok(read) = fs::read_dir(&dir) else {
        return false;
    };
    let baked = read
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "rlc"))
        .count();
    baked == expected
}
