//! Invoked from `build.rs`: hash pack bake inputs and run `bake_assets.py` when stale.
//!
//! Input hashing walks `assets/` with repo `.gitignore` rules plus pack-only exclusions
//! (same extras as `tools/bake_assets/bake_assets.py` `should_skip`). Cargo rerun uses a
//! coarse `assets/` watch so build-script output stays readable; gitignored-only edits
//! rerun the script but leave the inputs hash unchanged.

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use mahjuro_bake_stamp::{Fnv64, log_bake_timing, read_stamp_line, write_stamp_line};

const RULES_PATH: &str = "tools/bake_assets/pack_rules.json";
const BAKE_SCRIPT_PATH: &str = "tools/bake_assets/bake_assets.py";
const STAMP_NAME: &str = ".pack_inputs_stamp";

const PACK_OUTPUTS: &[&str] = &[
    "pack_manifest.json",
    "mahjuro-pack-music.zip",
    "mahjuro-pack-shared.zip",
    "mahjuro-pack-rooms.zip",
    "mahjuro-pack-gameplay-bulk.zip",
];

pub fn emit_rerun_if_changed(_repo: &Path, profile_dir: &Path) {
    println!("cargo:rerun-if-env-changed=MAHJURO_SKIP_ASSET_BAKE");
    println!("cargo:rerun-if-changed=.gitignore");
    println!("cargo:rerun-if-changed={RULES_PATH}");
    println!("cargo:rerun-if-changed={BAKE_SCRIPT_PATH}");
    println!("cargo:rerun-if-changed=assets");
    println!(
        "cargo:rerun-if-changed={}",
        profile_dir.join(STAMP_NAME).display()
    );
}

pub fn maybe_bake_asset_packs(repo: &Path, profile_dir: &Path) {
    if skip_bake_env() {
        println!(
            "cargo:warning=MAHJURO_SKIP_ASSET_BAKE: skipping pack bake; ensure {} exists or set MAHJURO_ASSETS",
            profile_dir.join("pack_manifest.json").display()
        );
        return;
    }

    let script = repo.join(BAKE_SCRIPT_PATH);
    if !script.is_file() {
        println!(
            "cargo:warning={} missing; skipping pack bake",
            script.display()
        );
        return;
    }

    let profile = env::var("PROFILE").unwrap_or_default();
    let release = profile == "release";
    let hash = compute_inputs_hash(repo, release);
    let stamp_file = profile_dir.join(STAMP_NAME);
    let stamp_ok = read_stamp(&stamp_file).is_some_and(|s| s == hash);
    let outputs_ok = PACK_OUTPUTS
        .iter()
        .all(|name| profile_dir.join(name).is_file());

    if stamp_ok && outputs_ok {
        println!("cargo:info=asset pack bake: inputs unchanged, skipping");
        return;
    }

    let start = Instant::now();
    if let Err(e) = run_pack_bake(repo, &script, profile_dir, release) {
        panic!("asset pack bake failed: {e}");
    }

    write_stamp_line(&stamp_file, &hash).unwrap_or_else(|e| {
        panic!(
            "asset pack bake: could not write stamp {}: {e}",
            stamp_file.display()
        );
    });
    log_bake_timing("asset packs", start);
}

fn skip_bake_env() -> bool {
    env::var("MAHJURO_SKIP_ASSET_BAKE")
        .map(|v| {
            let v = v.trim();
            !v.is_empty() && v != "0" && !v.eq_ignore_ascii_case("false")
        })
        .unwrap_or(false)
}

/// Mirrors `should_skip` in `tools/bake_assets/bake_assets.py`.
fn should_skip_pack_input(rel: &str) -> bool {
    let rel = rel.replace('\\', "/");
    let base = Path::new(&rel)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if base == ".DS_Store" {
        return true;
    }
    if base.ends_with(".blend") || base.ends_with(".blend1") {
        return true;
    }
    if rel.starts_with("3d/source/") || rel.starts_with("3d/_gltf_sidecars/") {
        return true;
    }
    if rel.starts_with("3d/") {
        let ext = Path::new(&rel)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "bin") {
            return true;
        }
    }
    // Source relic art — runtime loads pre-baked RLC2 under data/relic_baked/.
    if rel.starts_with("textures/relics/") {
        return true;
    }
    // Source texture PNGs — runtime loads BTX1 under data/texture_baked/.
    if rel.starts_with("textures/")
        && Path::new(&rel)
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
    {
        if is_raw_runtime_texture(&rel) {
            return false;
        }
        return true;
    }
    false
}

fn is_raw_runtime_texture(rel: &str) -> bool {
    rel == "textures/main_menu_logo.png"
        || rel == "textures/temptations/atlas.png"
        || rel.starts_with("textures/kenney_input-prompts/")
}

fn pack_input_paths(repo: &Path) -> Vec<PathBuf> {
    let assets = repo.join("assets");
    if !assets.is_dir() {
        return Vec::new();
    }

    let mut paths = Vec::new();
    let mut builder = ignore::WalkBuilder::new(&assets);
    builder
        .git_ignore(true)
        .git_global(false)
        .git_exclude(false)
        .parents(true);
    for entry in builder.build().flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let rel = path
            .strip_prefix(&assets)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        if !should_skip_pack_input(&rel) {
            paths.push(path.to_path_buf());
        }
    }
    paths.sort_by(|a, b| a.as_os_str().cmp(b.as_os_str()));
    paths
}

fn compute_inputs_hash(repo: &Path, release: bool) -> String {
    let mut h = Fnv64::new();
    h.write(if release {
        b"profile:release\n"
    } else {
        b"profile:debug\n"
    });
    for path in [RULES_PATH, BAKE_SCRIPT_PATH] {
        let p = repo.join(path);
        h.write_path_key(&p);
        if p.is_file()
            && let Ok(bytes) = fs::read(&p)
        {
            h.write(&bytes);
        }
    }
    for path in pack_input_paths(repo) {
        let rel = path
            .strip_prefix(repo.join("assets"))
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        h.write(rel.as_bytes());
        h.write(b"\0");
        if let Ok(bytes) = fs::read(&path) {
            h.write(&bytes);
        }
    }
    h.finish_hex()
}

fn read_stamp(path: &Path) -> Option<String> {
    read_stamp_line(path)
}

fn run_pack_bake(repo: &Path, script: &Path, out_dir: &Path, release: bool) -> Result<(), String> {
    let lossy: &[&str] = if release { &[] } else { &["--no-lossy"] };

    #[cfg(windows)]
    let attempts: &[(&str, &[&str])] = &[("python3", &[]), ("python", &[]), ("py", &["-3"])];
    #[cfg(not(windows))]
    let attempts: &[(&str, &[&str])] = &[("python3", &[]), ("python", &[])];

    for (cmd, prefix) in attempts {
        let mut c = Command::new(cmd);
        c.current_dir(repo);
        for p in *prefix {
            c.arg(p);
        }
        c.arg(script);
        c.arg("--out").arg(out_dir);
        for a in lossy {
            c.arg(a);
        }

        match c.status() {
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => {
                return Err(format!(
                    "{cmd} exited with {status} (cwd {}, args for bake_assets.py)",
                    repo.display()
                ));
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("failed to spawn {cmd}: {e}")),
        }
    }

    Err(
        "no Python interpreter found (tried python3, python, and on Windows py -3); \
         install Python or set MAHJURO_SKIP_ASSET_BAKE=1 and provide MAHJURO_ASSETS"
            .into(),
    )
}
