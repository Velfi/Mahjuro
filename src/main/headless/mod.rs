//! Headless offscreen runner: `screenshot`, `bake-room-gi`, `bake-room-shadows`.

mod app;
mod fixtures;
mod scenes;
mod slug;

use std::path::PathBuf;

use crate::scenes::{
    CollectionInspectPresenter, ShopInspectPresenter, ShowcasePresenter, ShowcaseScene,
};
use app::HeadlessApp;
use scenes::{
    collection_screenshot_tab_for_overlay, resolve_screenshot_scene, scene_for_room_gi_bake,
    validate_screenshot_cli,
};
use slug::parse_bake_room_slug;

use crate::game::run::RunState;
use crate::main_cli;
use crate::persistence;
use crate::scenes::Scene;
use crate::ui::input::{InputMode, UiAction};

use fixtures::force_ordeal_chamber;
use slug::parse_ordeal_slug;

pub fn run_screenshot_command(s: main_cli::ScreenshotCli) -> anyhow::Result<()> {
    crate::asset_path::init();
    crate::asset_path::log_all_assets();
    validate_screenshot_cli(&s)?;

    let ordeal_override = s.ordeal.as_deref().map(parse_ordeal_slug).transpose()?;
    let settings = persistence::load_settings();
    let profile_index = s.profile.unwrap_or(settings.active_profile);
    let mut screenshot_progress = if s.fresh_progress {
        crate::core::progression::PlayerProgress::new()
    } else {
        persistence::load_profile(profile_index)
    };
    let mut run = RunState::new_demo();
    if let Some(kind) = ordeal_override {
        force_ordeal_chamber(&mut run, kind);
    }

    let setup = resolve_screenshot_scene(&s, &mut run, &mut screenshot_progress)?;
    let mut app = HeadlessApp::with_run(
        setup.scene,
        run,
        s.width.max(1),
        s.height.max(1),
        setup.game_in_progress,
        profile_index,
        screenshot_progress,
    )?;

    if s.item_inspect {
        push_item_inspect_overlay(&mut app, &s)?;
    }
    if (s.shop_focus.is_some() && matches!(s.scene.as_str(), "shop"))
        || (s.item_inspect && collection_screenshot_tab_for_overlay(&s.scene))
    {
        app.input_mode_override = Some(InputMode::Controller);
    }
    if setup.hero_play {
        app.queue_action_on_tick(2, UiAction::ScoreHand);
    }
    if setup.unlock_yaku {
        app.unlock_all_yaku_for_screenshot();
    }
    if setup.unlock_collection {
        app.unlock_all_for_collection_screenshot();
    }
    if setup.force_relic_modal {
        app.force_relic_unlock_modal();
    }
    if let Some(mul) = s.gltf_emissive_scale {
        app.shop_env_lighting.gltf_emissive_scale = mul;
    }
    app.run_screenshot(s.output.clone(), s.warmup_frames)
}

fn push_item_inspect_overlay(
    app: &mut HeadlessApp,
    s: &main_cli::ScreenshotCli,
) -> anyhow::Result<()> {
    let w = s.width.max(1) as f32;
    let h = s.height.max(1) as f32;
    let layout = app.layout_engine.solve(w, h);
    match &mut app.scene {
        Scene::Shop(shop) => {
            let orbit = shop.item_inspect_orbit_for_screenshot(w, h, &app.run).ok_or_else(|| {
                anyhow::anyhow!(
                    "--item-inspect: could not build inspect orbit (check --shop-focus index and stock)"
                )
            })?;
            app.overlay_stack.push(Scene::Showcase(ShowcaseScene::new(
                ShowcasePresenter::ShopInspect(ShopInspectPresenter::new(orbit)),
            )));
        }
        Scene::Collection(coll) => {
            let orbit = coll
                .item_inspect_orbit_for_screenshot(w, h, &layout, &app.progress)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "--item-inspect: collection tab has no artifacts or orbit failed"
                    )
                })?;
            app.overlay_stack.push(Scene::Showcase(ShowcaseScene::new(
                ShowcasePresenter::CollectionInspect(CollectionInspectPresenter::new(orbit)),
            )));
        }
        _ => {}
    }
    Ok(())
}

pub fn run_bake_room_shadows_command(b: main_cli::BakeRoomShadowsCli) -> anyhow::Result<()> {
    crate::asset_path::init();
    crate::asset_path::log_all_assets();
    let room = parse_bake_room_slug(&b.room)?;
    let settings = persistence::load_settings();
    let profile_index = settings.active_profile;
    let progress = persistence::load_profile(profile_index);
    let (scene, run, game_in_progress) = scene_for_room_gi_bake(room, &progress);
    let mut app = HeadlessApp::with_run(
        scene,
        run,
        b.width,
        b.height,
        game_in_progress,
        profile_index,
        progress,
    )?;
    app.renderer.request_room_shadow_capture(room);
    app.run_warmup_frames(b.warmup_frames);
    app.tick_frame();
    let bake = app
        .renderer
        .take_room_shadow_capture()
        .ok_or_else(|| anyhow::anyhow!("room shadow bake: GPU readback missing"))?;
    write_room_shadow_bake(room, &b.output_dir, bake)
}

pub fn run_bake_room_gi_command(b: main_cli::BakeRoomGiCli) -> anyhow::Result<()> {
    crate::asset_path::init();
    crate::asset_path::log_all_assets();
    let room = parse_bake_room_slug(&b.room)?;
    let settings = persistence::load_settings();
    let profile_index = settings.active_profile;
    let progress = persistence::load_profile(profile_index);
    let (scene, run, game_in_progress) = scene_for_room_gi_bake(room, &progress);
    let app = HeadlessApp::with_run(
        scene,
        run,
        b.width,
        b.height,
        game_in_progress,
        profile_index,
        progress,
    )?;
    let bake = app.run_room_gi_bake(room, b.warmup_frames)?;
    write_room_gi_bake(room, &b.output_dir, bake)
}

fn write_room_shadow_bake(
    room: crate::render::room_gi_bake::RoomGiRoom,
    output_dir: &PathBuf,
    bake: crate::render::room_shadow_bake::RoomShadowBake,
) -> anyhow::Result<()> {
    let out_name = match room {
        crate::render::room_gi_bake::RoomGiRoom::Shop => "shop.msh",
        crate::render::room_gi_bake::RoomGiRoom::Hallway => "hallway.msh",
        crate::render::room_gi_bake::RoomGiRoom::Archive => "archive.msh",
        crate::render::room_gi_bake::RoomGiRoom::MainMenu => "main_menu.msh",
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
    room: crate::render::room_gi_bake::RoomGiRoom,
    output_dir: &PathBuf,
    bake: crate::render::room_gi_bake::RoomGiBake,
) -> anyhow::Result<()> {
    let out_name = match room {
        crate::render::room_gi_bake::RoomGiRoom::Shop => "shop.mgi",
        crate::render::room_gi_bake::RoomGiRoom::Hallway => "hallway.mgi",
        crate::render::room_gi_bake::RoomGiRoom::Archive => "archive.mgi",
        crate::render::room_gi_bake::RoomGiRoom::MainMenu => "main_menu.mgi",
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
