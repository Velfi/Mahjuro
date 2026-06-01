//! Auto-rebake committed offline outputs from `build.rs` when stamps are stale (local dev).
//!
//! Missing bake binaries are compiled via nested `cargo` into `target/offline-bake-tools/`
//! (a separate `CARGO_TARGET_DIR` so the main target-dir lock is not held — see
//! rust-lang/cargo#6412). Then the baker is executed to refresh committed assets.

use std::env;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use mahjuro_bake_stamp::relic::Relic;
use mahjuro_bake_stamp::room_gi::RoomGi;
use mahjuro_bake_stamp::room_shadow::RoomShadow;
use mahjuro_bake_stamp::showcase_decal::ShowcaseDecal;
use mahjuro_bake_stamp::{
    auto_offline_rebake_enabled, ensure_bake_current, skip_committed_bake_checks,
    write_stamp_line, BakeKind,
};

const EXPECTED_GI_STAMP_HASH_ENV: &str = "MAHJURO_EXPECT_ROOM_GI_STAMP_HASH";
const EXPECTED_SHADOW_STAMP_HASH_ENV: &str = "MAHJURO_EXPECT_ROOM_SHADOW_STAMP_HASH";

struct BakeToolCtx {
    repo: PathBuf,
    assets: PathBuf,
    profile: String,
    main_profile_dir: PathBuf,
    tool_profile_dir: PathBuf,
    tool_target_dir: PathBuf,
}

pub fn emit_rerun_if_changed() {
    println!(
        "cargo:rerun-if-env-changed={}",
        mahjuro_bake_stamp::SKIP_AUTO_OFFLINE_REBAKE_ENV
    );
    println!("cargo:rerun-if-env-changed=CI");
    println!("cargo:rerun-if-env-changed=HOST");
    println!("cargo:rerun-if-env-changed=TARGET");
    println!("cargo:rerun-if-changed=target/offline-bake-tools");
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
        let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".into());
        let tool_target_dir = repo.join("target").join("offline-bake-tools");
        Self {
            repo: repo.to_path_buf(),
            assets: repo.join("assets"),
            profile: profile.clone(),
            main_profile_dir: profile_dir.to_path_buf(),
            tool_profile_dir: tool_target_dir.join(&profile),
            tool_target_dir,
        }
    }
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
    // Prefer offline-bake-tools: auto-rebake always refreshes there, while an older
    // copy may linger in the main target dir from a prior manual build.
    for dir in [&ctx.tool_profile_dir, &ctx.main_profile_dir] {
        let path = bin_path(dir, name);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

fn ensure_bake_binary(ctx: &BakeToolCtx, bin_name: &str) -> Result<PathBuf, String> {
    if !auto_offline_rebake_enabled() {
        return resolve_bake_binary(ctx, bin_name)
            .ok_or_else(|| missing_bake_tool_message(bin_name));
    }
    // Always `cargo build` the tool here: an existing `target/offline-bake-tools/` binary is
    // not invalidated when `mahjuro-render` changes, so a stale baker can keep crashing
    // after a source fix until the user deletes that tree manually.
    if resolve_bake_binary(ctx, bin_name).is_some() {
        println!(
            "cargo:info=bake tool `{bin_name}` — refreshing in {} ({})",
            ctx.tool_target_dir.display(),
            ctx.profile
        );
    } else {
        println!(
            "cargo:info=bake tool `{bin_name}` missing — building into {} ({})",
            ctx.tool_target_dir.display(),
            ctx.profile
        );
    }
    compile_bake_tool(ctx, bin_name)?;
    resolve_bake_binary(ctx, bin_name).ok_or_else(|| missing_bake_tool_message(bin_name))
}

fn compile_bake_tool(ctx: &BakeToolCtx, bin_name: &str) -> Result<(), String> {
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut cmd = Command::new(&cargo);
    cmd.current_dir(&ctx.repo);
    cmd.env("CARGO_TARGET_DIR", &ctx.tool_target_dir);
    cmd.env("MAHJURO_SKIP_COMMITTED_BAKE_CHECKS", "1");
    cmd.env("MAHJURO_SKIP_OFFLINE_BAKES", "1");
    cmd.arg("build");
    if ctx.profile == "release" {
        cmd.arg("--release");
    }
    match bin_name {
        "mahjuro-bake" => {
            cmd.args([
                "-p",
                "mahjuro-headless",
                "--bin",
                "mahjuro-bake",
                "--features",
                "bake",
            ]);
        }
        "mahjuro-bake-decal-atlases" => {
            cmd.args([
                "-p",
                "mahjuro-render",
                "--bin",
                "mahjuro-bake-decal-atlases",
            ]);
        }
        "mahjuro-bake-relics" => {
            cmd.args(["-p", "mahjuro-render", "--bin", "mahjuro-bake-relics"]);
        }
        other => return Err(format!("unknown bake tool `{other}`")),
    }
    run_command(cmd, &format!("cargo build --bin {bin_name}"))
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
            main_profile_dir: self.main_profile_dir.clone(),
            tool_profile_dir: self.tool_profile_dir.clone(),
            tool_target_dir: self.tool_target_dir.clone(),
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
    let bake_bin = ensure_bake_binary(ctx, "mahjuro-bake")?;
    let mut cmd = Command::new(bake_bin);
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
    let bin = ensure_bake_binary(ctx, bin_name)?;

    let mut cmd = Command::new(bin);
    cmd.current_dir(&ctx.repo);
    cmd.env("MAHJURO_ASSETS", &ctx.assets);
    cmd.args(tool_args);
    run_command(cmd, bin_name)
}

fn missing_bake_tool_message(bin_name: &str) -> String {
    format!(
        "bake binary `{bin_name}` not found under target/{{profile}}/ or \
         target/offline-bake-tools/{{profile}}/.\n\
         Auto-build is disabled (CI/cross-compile) or failed. If a bake crashed after a \
         render fix, remove `target/offline-bake-tools/` and retry, or run:\n\
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
