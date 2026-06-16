//! Offscreen PNG captures for marketing, docs, and regression baselines.

use crate::app::HeadlessApp;
use crate::fixtures::force_ordeal_chamber;
use crate::screenshot_cli::ScreenshotCli;
use crate::screenshot_scenes::{
    collection_screenshot_tab_for_overlay, resolve_screenshot_scene, validate_screenshot_cli,
};
use crate::slug::parse_ordeal_slug;
use mahjuro::game::run::RunState;
use mahjuro::scenes::Scene;
use mahjuro::scenes::{
    ArchiveInspectPresenter, ShopInspectPresenter, ShowcasePresenter, ShowcaseScene,
};
use mahjuro::ui::input::{InputMode, UiAction};

pub fn run(cli: ScreenshotCli) -> anyhow::Result<()> {
    mahjuro::asset_path::init();
    mahjuro::asset_path::log_all_assets();
    validate_screenshot_cli(&cli)?;

    let ordeal_override = cli.ordeal.as_deref().map(parse_ordeal_slug).transpose()?;
    let settings = mahjuro::persistence::load_settings();
    let profile_index = cli.profile.unwrap_or(settings.active_profile);
    let mut screenshot_progress = if cli.fresh_progress {
        mahjuro::core::progression::PlayerProgress::new()
    } else {
        mahjuro::persistence::load_profile(profile_index)
    };
    let mut run = RunState::new_demo();
    if let Some(kind) = ordeal_override {
        force_ordeal_chamber(&mut run, kind);
    }

    let setup = resolve_screenshot_scene(&cli, &mut run, &mut screenshot_progress)?;
    let mut app = HeadlessApp::with_run(
        setup.scene,
        run,
        cli.width.max(1),
        cli.height.max(1),
        setup.game_in_progress,
        profile_index,
        screenshot_progress,
    )?;
    app.hide_ui = cli.hide_ui;

    if (cli.shop_focus.is_some() && matches!(cli.scene.as_str(), "shop"))
        || (cli.item_inspect && collection_screenshot_tab_for_overlay(&cli.scene))
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
    if let Some(slug) = cli.collection_focus.as_deref() {
        if let Scene::Archive(coll) = &mut app.scene {
            coll.set_focus_for_screenshot(slug, &app.progress)
                .map_err(anyhow::Error::msg)?;
        }
    }
    if cli.item_inspect {
        push_item_inspect_overlay(&mut app, &cli)?;
    }
    if setup.force_relic_modal {
        app.force_relic_unlock_modal();
    }
    if setup.force_round_win_modal {
        app.force_round_win_modal();
    }
    if let Some(mul) = cli.gltf_emissive_scale {
        app.shop_env_lighting.gltf_emissive_scale = mul;
    }
    app.run_screenshot(cli.output.clone(), cli.warmup_frames)
}

fn push_item_inspect_overlay(app: &mut HeadlessApp, s: &ScreenshotCli) -> anyhow::Result<()> {
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
        Scene::Archive(coll) => {
            let orbit = coll
                .item_inspect_orbit_for_screenshot(w, h, &layout, &app.progress)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "--item-inspect: collection tab has no artifacts or orbit failed"
                    )
                })?;
            app.overlay_stack.push(Scene::Showcase(ShowcaseScene::new(
                ShowcasePresenter::ArchiveInspect(ArchiveInspectPresenter::new(orbit)),
            )));
        }
        _ => {}
    }
    Ok(())
}
