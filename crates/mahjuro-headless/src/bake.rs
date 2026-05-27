//! Offline room GI / shadow bakes.

use std::path::PathBuf;

use crate::app::HeadlessApp;
use crate::bake_cli::{BakeRoomCli, RoomBakeKind};
use crate::room_bake_scenes::scene_for_room_gi_bake;
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

    for room in rooms {
        let (scene, run, game_in_progress) = scene_for_room_gi_bake(room, &progress);
        let mut app = HeadlessApp::with_run(
            scene,
            run,
            width,
            height,
            game_in_progress,
            profile_index,
            progress.clone(),
        )?;

        if bake_shadow {
            let bake = app.bake_room_shadow(room, cli.warmup_frames)?;
            write_room_shadow_bake(room, &cli.shadow_dir, bake)?;
        }

        if bake_gi {
            let bake = app.bake_room_gi(room, cli.warmup_frames)?;
            write_room_gi_bake(room, &cli.gi_dir, bake)?;
        }
    }

    Ok(())
}

fn write_room_shadow_bake(
    room: mahjuro::render::room_gi_bake::RoomGiRoom,
    output_dir: &PathBuf,
    bake: mahjuro::render::room_shadow_bake::RoomShadowBake,
) -> anyhow::Result<()> {
    let out_name = match room {
        mahjuro::render::room_gi_bake::RoomGiRoom::Shop => "shop.msh",
        mahjuro::render::room_gi_bake::RoomGiRoom::Hallway => "hallway.msh",
        mahjuro::render::room_gi_bake::RoomGiRoom::Archive => "archive.msh",
        mahjuro::render::room_gi_bake::RoomGiRoom::MainMenu => "main_menu.msh",
        mahjuro::render::room_gi_bake::RoomGiRoom::Staircase => "staircase.msh",
        mahjuro::render::room_gi_bake::RoomGiRoom::Gameplay => "gameplay.msh",
    };
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
    let out_name = match room {
        mahjuro::render::room_gi_bake::RoomGiRoom::Shop => "shop.mgi",
        mahjuro::render::room_gi_bake::RoomGiRoom::Hallway => "hallway.mgi",
        mahjuro::render::room_gi_bake::RoomGiRoom::Archive => "archive.mgi",
        mahjuro::render::room_gi_bake::RoomGiRoom::MainMenu => "main_menu.mgi",
        mahjuro::render::room_gi_bake::RoomGiRoom::Staircase => "staircase.mgi",
        mahjuro::render::room_gi_bake::RoomGiRoom::Gameplay => "gameplay.mgi",
    };
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
