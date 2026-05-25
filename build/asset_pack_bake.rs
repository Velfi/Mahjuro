//! Invoked from `build.rs`: hash pack bake inputs and run `bake_assets.py` when stale.
//!
//! Skips files excluded from packs (`.DS_Store`, `assets/3d/source/`, loose maps, …) — same
//! rules as `tools/bake_assets/bake_assets.py` `should_skip`. Does not use `rerun-if-changed=assets`.

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

const RULES_PATH: &str = "tools/bake_assets/pack_rules.json";
const BAKE_SCRIPT_PATH: &str = "tools/bake_assets/bake_assets.py";
const STAMP_NAME: &str = ".pack_inputs_stamp";

const PACK_OUTPUTS: &[&str] = &[
    "pack_manifest.json",
    "mahjuro-pack-music.zip",
    "mahjuro-pack-shared.zip",
    "mahjuro-pack-gameplay.zip",
];

pub fn emit_rerun_if_changed(repo: &Path, profile_dir: &Path) {
    println!("cargo:rerun-if-env-changed=MAHJURO_SKIP_ASSET_BAKE");
    println!("cargo:rerun-if-changed={RULES_PATH}");
    println!("cargo:rerun-if-changed={BAKE_SCRIPT_PATH}");
    println!(
        "cargo:rerun-if-changed={}",
        profile_dir.join(STAMP_NAME).display()
    );

    for path in pack_input_paths(repo) {
        let rel = path
            .strip_prefix(repo)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        println!("cargo:rerun-if-changed={rel}");
    }
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

    if let Err(e) = run_pack_bake(repo, &script, profile_dir, release) {
        panic!("asset pack bake failed: {e}");
    }

    if let Err(e) = write_stamp(&stamp_file, &hash) {
        println!(
            "cargo:warning=asset pack bake: could not write stamp {}: {e}",
            stamp_file.display()
        );
    }
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
    false
}

fn pack_input_paths(repo: &Path) -> Vec<PathBuf> {
    let assets = repo.join("assets");
    let mut paths = Vec::new();
    if assets.is_dir() {
        collect_pack_input_paths(&assets, &assets, &mut paths);
        paths.sort_by(|a, b| a.as_os_str().cmp(b.as_os_str()));
    }
    paths
}

fn collect_pack_input_paths(assets_root: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_pack_input_paths(assets_root, &path, out);
        } else if path.is_file() {
            let rel = path
                .strip_prefix(assets_root)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            if !should_skip_pack_input(&rel) {
                out.push(path);
            }
        }
    }
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
        h.write(path.as_bytes());
        h.write(b"\0");
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
