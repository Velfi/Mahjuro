//! Offline room GI / shadow bakes.
//!
//! On a successful full-room run with default output dirs, refreshes
//! `.inputs_stamp` for each kind that was baked so `mahjuro`'s `build.rs` won't
//! flag the committed bake as stale next compile.

use std::path::{Path, PathBuf};

use mahjuro_bake_stamp::BakeKind;
use mahjuro_bake_stamp::room_gi::RoomGi;
use mahjuro_bake_stamp::room_shadow::RoomShadow;

use crate::app::HeadlessApp;
use crate::bake_cli::{BakeRoomCli, RoomBakeKind};
use crate::room_bake_app::RoomBakeApp;
use crate::slug::resolve_bake_rooms;

pub fn run(cli: BakeRoomCli) -> anyhow::Result<()> {
    mahjuro::asset_path::init();
    mahjuro::asset_path::log_all_assets();

    let rooms = resolve_bake_rooms(&cli.rooms)?;
    let bake_gi = cli.kinds.contains(&RoomBakeKind::Gi);
    let bake_shadow = cli.kinds.contains(&RoomBakeKind::Shadow);
    anyhow::ensure!(
        bake_gi || bake_shadow,
        "no bake kinds selected (use --kinds gi,shadow)"
    );

    let settings = mahjuro::persistence::load_settings();
    let profile_index = settings.active_profile;
    let progress = mahjuro::persistence::load_profile(profile_index);
    let width = cli.width.max(1);
    let height = cli.height.max(1);

    for room in &rooms {
        let (scene, run, game_in_progress) = mahjuro::room_bake::scene_for_room(*room, &progress);
        let mut app = RoomBakeApp::new(
            scene,
            run,
            width,
            height,
            game_in_progress,
            profile_index,
            progress.clone(),
        )?;

        if bake_shadow {
            let bake = app.bake_room_shadow(*room, cli.warmup_frames)?;
            write_room_shadow_bake(*room, &cli.shadow_dir, bake)?;
        }

        if bake_gi {
            let bake = app.bake_room_gi(*room, cli.warmup_frames)?;
            write_room_gi_bake(*room, &cli.gi_dir, bake)?;
        }
    }

    refresh_stamps_if_canonical(&cli, bake_gi, bake_shadow)?;
    Ok(())
}

/// Stamp refresh runs only when the user took the default `--gi-dir` / `--shadow-dir`
/// AND let the bake cover every room. Anything else is treated as an ad-hoc run that
/// must not poison the committed stamp; we'd rather have `build.rs` panic next time
/// than silently mark a partial bake as authoritative.
fn refresh_stamps_if_canonical(
    cli: &BakeRoomCli,
    bake_gi: bool,
    bake_shadow: bool,
) -> anyhow::Result<()> {
    let baked_all_rooms = cli.rooms.is_empty();
    if !baked_all_rooms {
        log::info!("partial room set baked; leaving .inputs_stamp files alone");
        return Ok(());
    }

    let repo = repo_root()?;

    if bake_gi {
        if cli.gi_dir == Path::new(RoomGi::OUT_DIR) {
            let stamped = RoomGi::write_stamp(&repo)?;
            log::info!(
                "refreshed {} ({})",
                stamped.stamp_path.display(),
                stamped.hash
            );
        } else {
            log::info!(
                "--gi-dir is non-canonical ({}); leaving {} alone",
                cli.gi_dir.display(),
                RoomGi::STAMP_PATH
            );
        }
    }

    if bake_shadow {
        if cli.shadow_dir == Path::new(RoomShadow::OUT_DIR) {
            let stamped = RoomShadow::write_stamp(&repo)?;
            log::info!(
                "refreshed {} ({})",
                stamped.stamp_path.display(),
                stamped.hash
            );
        } else {
            log::info!(
                "--shadow-dir is non-canonical ({}); leaving {} alone",
                cli.shadow_dir.display(),
                RoomShadow::STAMP_PATH
            );
        }
    }

    Ok(())
}

/// Repo root with no `..` components. The build script uses `mahjuro`'s
/// `CARGO_MANIFEST_DIR` (already canonical); we walk parents rather than
/// `join("../..")` because `Fnv64::write_path_key` hashes the literal path
/// string, so any `..` would silently desync from `build.rs`'s digest.
fn repo_root() -> anyhow::Result<PathBuf> {
    if let Some(assets) = std::env::var_os("MAHJURO_ASSETS") {
        let assets = PathBuf::from(assets);
        return assets
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| anyhow::anyhow!("MAHJURO_ASSETS has no parent (need repo root)"));
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow::anyhow!("CARGO_MANIFEST_DIR has no grandparent"))
}

fn write_room_shadow_bake(
    room: mahjuro::render::room_gi_bake::RoomGiRoom,
    output_dir: &PathBuf,
    bake: mahjuro::render::room_shadow_bake::RoomShadowBake,
) -> anyhow::Result<()> {
    let out_name = room.offline_bake_filename("msh");
    let out_path = output_dir.join(out_name);
    ensure_parent_dir(&out_path)?;
    std::fs::write(&out_path, bake.encode())?;
    log::info!(
        "room shadow bake {:?} → {} ({}×{})",
        room,
        out_path.display(),
        bake.width,
        bake.height
    );
    Ok(())
}

fn write_room_gi_bake(
    room: mahjuro::render::room_gi_bake::RoomGiRoom,
    output_dir: &PathBuf,
    bake: mahjuro::render::room_gi_bake::RoomGiBake,
) -> anyhow::Result<()> {
    let out_name = room.offline_bake_filename("mgi");
    let out_path = output_dir.join(out_name);
    ensure_parent_dir(&out_path)?;
    std::fs::write(&out_path, bake.encode())?;
    log::info!(
        "room GI bake {:?} → {} ({} probes, {}×{})",
        room,
        out_path.display(),
        bake.probe_count,
        bake.bake_width,
        bake.bake_height
    );
    Ok(())
}

fn ensure_parent_dir(path: &PathBuf) -> anyhow::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}
