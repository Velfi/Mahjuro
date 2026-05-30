//! Auto-rebake committed offline outputs from `build.rs` when stamps are stale (local dev).
//!
//! Never spawns nested `cargo` from the build script — that deadlocks on the target-dir /
//! package-cache locks (see rust-lang/cargo#6412). Instead we run a bake binary if one
//! already exists under the main or `target/offline-bake-tools/` profile dir.

use std::env;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use mahjuro_bake_stamp::relic::Relic;
use mahjuro_bake_stamp::room_gi::RoomGi;
use mahjuro_bake_stamp::room_shadow::RoomShadow;
use mahjuro_bake_stamp::showcase_decal::ShowcaseDecal;
use mahjuro_bake_stamp::{
    ensure_bake_current, skip_committed_bake_checks, write_stamp_line, BakeKind,
};

const EXPECTED_GI_STAMP_HASH_ENV: &str = "MAHJURO_EXPECT_ROOM_GI_STAMP_HASH";
const EXPECTED_SHADOW_STAMP_HASH_ENV: &str = "MAHJURO_EXPECT_ROOM_SHADOW_STAMP_HASH";

struct BakeToolCtx {
    repo: PathBuf,
    assets: PathBuf,
    profile: String,
    target_triple: String,
    host_triple: String,
    main_profile_dir: PathBuf,
    tool_profile_dir: PathBuf,
}

pub fn emit_rerun_if_changed() {
    println!(
        "cargo:rerun-if-env-changed={}",
        mahjuro_bake_stamp::SKIP_AUTO_OFFLINE_REBAKE_ENV
    );
    println!("cargo:rerun-if-env-changed=CI");
    println!("cargo:rerun-if-env-changed=HOST");
    println!("cargo:rerun-if-env-changed=TARGET");
}

pub fn ensure_committed_offline_bakes_current(repo: &Path, profile_dir: &Path) {
    if skip_committed_bake_freshness() {
        println!("cargo:info=skipping committed offline bake freshness checks");
        return;
    }

    let ctx = BakeToolCtx::new(repo, profile_dir);

    ensure_room_gpu_bakes_current(repo, &ctx);
    ensure_bake_current::<ShowcaseDecal>(repo, || {
        run_bake_tool(&ctx, "mahjuro-bake-decal-atlases", &[])
    });
    ensure_bake_current::<Relic>(repo, || run_bake_tool(&ctx, "mahjuro-bake-relics", &[]));
}

impl BakeToolCtx {
    fn new(repo: &Path, profile_dir: &Path) -> Self {
        let target_root = profile_dir
            .parent()
            .unwrap_or_else(|| Path::new("target"));
        let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".into());
        let target_triple = env::var("TARGET").unwrap_or_default();
        let host_triple = env::var("HOST").unwrap_or_default();
        let tool_profile_dir = artifact_dir(
            target_root.join("offline-bake-tools"),
            &profile,
            &target_triple,
            &host_triple,
        );
        Self {
            repo: repo.to_path_buf(),
            assets: repo.join("assets"),
            profile,
            target_triple,
            host_triple,
            main_profile_dir: profile_dir.to_path_buf(),
            tool_profile_dir,
        }
    }
}

fn artifact_dir(target_root: PathBuf, profile: &str, target_triple: &str, host_triple: &str) -> PathBuf {
    let mut dir = target_root;
    if !target_triple.is_empty() && target_triple != host_triple {
        dir.push(target_triple);
    }
    dir.push(profile);
    dir
}

fn bin_path(profile_dir: &Path, name: &str) -> PathBuf {
    let file = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    profile_dir.join(file)
}

fn resolve_bake_binary(ctx: &BakeToolCtx, name: &str) -> Option<PathBuf> {
    for dir in [&ctx.main_profile_dir, &ctx.tool_profile_dir] {
        let path = bin_path(dir, name);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

fn skip_committed_bake_freshness() -> bool {
    skip_committed_bake_checks()
        || cargo_feature_enabled("CARGO_FEATURE_HEADLESS_SCREENSHOT")
        || cargo_feature_enabled("CARGO_FEATURE_OFFLINE_BAKE_SUPPORT")
}

fn cargo_feature_enabled(name: &str) -> bool {
    env::var(name).is_ok_and(|v| {
        let v = v.trim();
        !v.is_empty() && v != "0" && !v.eq_ignore_ascii_case("false")
    })
}

fn ensure_room_gpu_bakes_current(repo: &Path, ctx: &BakeToolCtx) {
    let gi = RoomGi::bake_status(repo);
    let shadow = RoomShadow::bake_status(repo);
    let gi_ok = gi.stamp_ok && gi.outputs_ok;
    let shadow_ok = shadow.stamp_ok && shadow.outputs_ok;

    if gi_ok && shadow_ok {
        println!("cargo:info=room GI bake: committed bake matches inputs");
        println!("cargo:info=room shadow bake: committed bake matches inputs");
        return;
    }

    let needs_gi = !gi_ok && !RoomGi::skip_bake_env();
    let needs_shadow = !shadow_ok && !RoomShadow::skip_bake_env();
    if !needs_gi && !needs_shadow {
        return;
    }

    let expected_gi_hash = needs_gi.then(|| gi.hash.clone());
    let expected_shadow_hash = needs_shadow.then(|| shadow.hash.clone());

    let repo_for_rebake = repo.to_path_buf();
    let ctx = ctx.clone();
    let rebake = move || {
        run_room_gpu_rebake(
            &ctx,
            needs_gi,
            needs_shadow,
            expected_gi_hash.as_deref(),
            expected_shadow_hash.as_deref(),
        )?;

        // Build-script-side stamp refresh: do not rely on the prebuilt `mahjuro-bake`
        // binary embedding the latest hash-input list.
        if let Some(hash) = expected_gi_hash.as_deref()
            && RoomGi::outputs_ok(&repo_for_rebake)
        {
            let stamp = repo_for_rebake.join(RoomGi::STAMP_PATH);
            write_stamp_line(&stamp, hash)
                .map_err(|e| format!("failed to refresh {}: {e}", stamp.display()))?;
        }
        if let Some(hash) = expected_shadow_hash.as_deref()
            && RoomShadow::outputs_ok(&repo_for_rebake)
        {
            let stamp = repo_for_rebake.join(RoomShadow::STAMP_PATH);
            write_stamp_line(&stamp, hash)
                .map_err(|e| format!("failed to refresh {}: {e}", stamp.display()))?;
        }

        Ok(())
    };

    if needs_gi {
        ensure_bake_current::<RoomGi>(repo, rebake);
    } else {
        ensure_bake_current::<RoomShadow>(repo, rebake);
    }
}

impl Clone for BakeToolCtx {
    fn clone(&self) -> Self {
        Self {
            repo: self.repo.clone(),
            assets: self.assets.clone(),
            profile: self.profile.clone(),
            target_triple: self.target_triple.clone(),
            host_triple: self.host_triple.clone(),
            main_profile_dir: self.main_profile_dir.clone(),
            tool_profile_dir: self.tool_profile_dir.clone(),
        }
    }
}

fn run_room_gpu_rebake(
    ctx: &BakeToolCtx,
    needs_gi: bool,
    needs_shadow: bool,
    expected_gi_hash: Option<&str>,
    expected_shadow_hash: Option<&str>,
) -> Result<(), String> {
    let mut kinds = Vec::new();
    if needs_gi {
        kinds.push("gi");
    }
    if needs_shadow {
        kinds.push("shadow");
    }
    let kinds_csv = kinds.join(",");
    let mut cmd = Command::new(
        resolve_bake_binary(ctx, "mahjuro-bake")
            .ok_or_else(|| missing_bake_tool_message("mahjuro-bake"))?,
    );
    cmd.current_dir(&ctx.repo);
    cmd.env("MAHJURO_ASSETS", &ctx.assets);
    if let Some(hash) = expected_gi_hash {
        cmd.env(EXPECTED_GI_STAMP_HASH_ENV, hash);
    }
    if let Some(hash) = expected_shadow_hash {
        cmd.env(EXPECTED_SHADOW_STAMP_HASH_ENV, hash);
    }
    cmd.args(["--kinds", &kinds_csv]);
    run_command(cmd, "mahjuro-bake")
}

fn run_bake_tool(ctx: &BakeToolCtx, bin_name: &str, tool_args: &[&str]) -> Result<(), String> {
    let Some(bin) = resolve_bake_binary(ctx, bin_name) else {
        return Err(missing_bake_tool_message(bin_name));
    };

    let mut cmd = Command::new(&bin);
    cmd.current_dir(&ctx.repo);
    cmd.env("MAHJURO_ASSETS", &ctx.assets);
    cmd.args(tool_args);
    run_command(cmd, bin_name)
}

fn missing_bake_tool_message(bin_name: &str) -> String {
    format!(
        "bake binary `{bin_name}` not found under target/{{profile}}/ or \
         target/offline-bake-tools/{{profile}}/.\n\
         Build it once outside this build script (nested cargo from build.rs deadlocks — \
         rust-lang/cargo#6412):\n\
         scripts/rebake-offline.sh\n\
         or: cargo build -p mahjuro-headless --bin mahjuro-bake --features bake"
    )
}

fn run_command(mut cmd: Command, label: &str) -> Result<(), String> {
    let output = cmd.output().map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            format!("failed to spawn `{label}`: executable not found")
        } else {
            format!("failed to spawn `{label}`: {e}")
        }
    })?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format_command_failure(label, &output))
    }
}

fn format_command_failure(label: &str, output: &Output) -> String {
    let mut msg = format!("{label} exited with {}", output.status);
    if !output.stdout.is_empty() {
        msg.push_str("\n--- stdout ---\n");
        msg.push_str(&String::from_utf8_lossy(&output.stdout));
    }
    if !output.stderr.is_empty() {
        msg.push_str("\n--- stderr ---\n");
        msg.push_str(&String::from_utf8_lossy(&output.stderr));
    }
    msg
}
