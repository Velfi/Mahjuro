use crate::game::run::RunState;
use crate::main_cli;
use crate::scenes::shop::ShopScene;
use crate::scenes::{
    MetaLevelUpPresenter, ShowcasePresenter, ShowcaseScene, TilePackPresenter,
    TutorialCampaignScene, ZodiacPresenter,
};
use crate::scenes::{GameOverScene, GameplayScene, Scene};

use super::fixtures::{
    prime_shop_stock, setup_defeat_game_over_screenshot_state, setup_gameplay_screenshot_state,
    setup_hero_state, setup_shop_state,
};
use super::slug::{parse_tile_pack_slug, parse_zodiac_slug};

pub(crate) fn validate_screenshot_cli(s: &main_cli::ScreenshotCli) -> anyhow::Result<()> {
    let shop_like = matches!(s.scene.as_str(), "shop");
    let collection_like = collection_screenshot_tab(&s.scene).is_some();
    if s.item_inspect && !shop_like && !collection_like {
        anyhow::bail!(
            "--item-inspect requires --scene shop or collection (full-screen pack/showcase captures do not use it)"
        );
    }
    if s.item_inspect && shop_like {
        let slug = s
            .shop_focus
            .as_deref()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "--item-inspect with --scene shop requires --shop-focus on relic:N, ribbon:N, talisman:N, or pack:N"
                )
            })?;
        if !shop_focus_slug_inspectable(slug) {
            anyhow::bail!(
                "--item-inspect with --scene shop requires inspectable --shop-focus (relic:N, ribbon:N, talisman:N, pack:N); got '{slug}'"
            );
        }
    }
    if s.shop_focus.is_some() && !shop_like {
        anyhow::bail!("--shop-focus is only valid with --scene shop");
    }
    let showcase_like = matches!(s.scene.as_str(), "showcase");
    let celebration_like =
        matches!(s.scene.as_str(), "zodiac_celebration") || (showcase_like && s.pack.is_none());
    let pack_celeb_like =
        matches!(s.scene.as_str(), "tile_pack_celebration") || (showcase_like && s.pack.is_some());
    if showcase_like && s.pack.is_some() && s.zodiac.is_some() {
        anyhow::bail!(
            "--scene showcase: use only one of --pack (tile pack) or --zodiac (ribbon); omit --zodiac for default snake"
        );
    }
    if s.zodiac.is_some() && !celebration_like {
        anyhow::bail!(
            "--zodiac is only valid with --scene zodiac_celebration or --scene showcase without --pack"
        );
    }
    if s.celebration_level.is_some() && !celebration_like {
        anyhow::bail!(
            "--celebration-level (--zodiac-yaku-level) is only for zodiac ribbon showcase \
             (`zodiac_celebration` or `showcase` without `--pack`); meta level-up is `--scene game_over_level_up`"
        );
    }
    if s.pack.is_some() && !pack_celeb_like {
        anyhow::bail!(
            "--pack is only valid with --scene tile_pack_celebration or --scene showcase"
        );
    }
    let defeat_like = matches!(s.scene.as_str(), "game_over_defeat" | "defeat");
    if (s.bot_play || s.from_run_history.is_some() || s.seed_bot_runs.is_some()) && !defeat_like {
        anyhow::bail!(
            "--bot-play, --from-run-history, and --seed-bot-runs are only valid with --scene game_over_defeat"
        );
    }
    Ok(())
}

fn shop_focus_slug_inspectable(slug: &str) -> bool {
    slug.starts_with("relic:")
        || slug.starts_with("ribbon:")
        || slug.starts_with("talisman:")
        || slug.starts_with("pack:")
}

#[derive(Clone, Copy)]
enum CollectionScreenshotTab {
    Relics,
    Chronicle,
    Bosses,
    Talismans,
}

pub(crate) fn collection_screenshot_tab_for_overlay(scene: &str) -> bool {
    collection_screenshot_tab(scene).is_some()
}

fn collection_screenshot_tab(scene: &str) -> Option<CollectionScreenshotTab> {
    match scene {
        "collection" | "archive" => Some(CollectionScreenshotTab::Relics),
        "chronicle" | "archive_chronicle" => Some(CollectionScreenshotTab::Chronicle),
        "archive_bosses" | "collection_bosses" => Some(CollectionScreenshotTab::Bosses),
        "archive_talismans" | "collection_talismans" => Some(CollectionScreenshotTab::Talismans),
        _ => None,
    }
}

fn collection_scene(tab: CollectionScreenshotTab) -> Scene {
    let mut coll = crate::scenes::CollectionScene::new();
    match tab {
        CollectionScreenshotTab::Relics => {}
        CollectionScreenshotTab::Chronicle => coll.prepare_chronicle_for_screenshot(),
        CollectionScreenshotTab::Bosses => coll.prepare_bosses_for_screenshot(),
        CollectionScreenshotTab::Talismans => coll.prepare_talismans_for_screenshot(),
    }
    Scene::Collection(coll)
}

fn zodiac_from_cli(s: &main_cli::ScreenshotCli) -> anyhow::Result<crate::core::zodiac::ZodiacKind> {
    match s.zodiac.as_deref() {
        Some(slug) => parse_zodiac_slug(slug),
        None => Ok(crate::core::zodiac::ZodiacKind::Snake),
    }
}

fn zodiac_celebration_level(s: &main_cli::ScreenshotCli) -> u32 {
    s.celebration_level.unwrap_or(2).max(1)
}

pub(crate) fn zodiac_showcase_scene(s: &main_cli::ScreenshotCli) -> anyhow::Result<Scene> {
    let z = zodiac_from_cli(s)?;
    let level = zodiac_celebration_level(s);
    let yaku = z.yaku();
    Ok(Scene::Showcase(ShowcaseScene::new(ShowcasePresenter::Zodiac(
        ZodiacPresenter::new(z, yaku.name(), level),
    ))))
}

pub(crate) fn tile_pack_showcase_scene(
    run: &RunState,
    pack_slug: Option<&str>,
) -> anyhow::Result<Scene> {
    let pack = match pack_slug {
        Some(slug) => parse_tile_pack_slug(slug)?,
        None => crate::core::tile_pack::TilePackKind::Honors,
    };
    Ok(Scene::Showcase(ShowcaseScene::new(ShowcasePresenter::TilePack(
        Box::new(TilePackPresenter::new_headless_screenshot(run, pack)),
    ))))
}

fn game_over_defeat_scene(
    s: &main_cli::ScreenshotCli,
    run: &mut RunState,
    progress: &mut crate::core::progression::PlayerProgress,
) -> anyhow::Result<Scene> {
    use crate::core::memorial_talisman::{select_memorial, snapshot_from_run};
    use crate::game::event_bus::GameOverReason;

    if s.bot_play && s.from_run_history.is_some() {
        anyhow::bail!("use only one of --bot-play or --from-run-history");
    }
    if s.seed_bot_runs.is_some() && s.from_run_history.is_none() {
        anyhow::bail!("--seed-bot-runs requires --from-run-history <index>");
    }

    if let Some(n) = s.seed_bot_runs {
        crate::bot::seed_progress_from_bot_runs(progress, n);
    }

    let reason = if let Some(idx) = s.from_run_history {
        let rec = progress.run_history.get(idx).ok_or_else(|| {
            anyhow::anyhow!(
                "profile run_history[{idx}] missing (have {} runs; seed with --seed-bot-runs N)",
                progress.run_history.len()
            )
        })?;
        let reason = rec.defeat_reason().ok_or_else(|| {
            anyhow::anyhow!("profile run_history[{idx}] is not a defeat (use a defeat index)")
        })?;
        rec.hydrate_game_over_run(run);
        reason
    } else if s.bot_play {
        let (terminal, stats) = crate::bot::play_bot_run(
            crate::bot::BotConfig::default(),
            crate::bot::BotRunOptions {
                log: false,
                ..crate::bot::BotRunOptions::default()
            },
            Some(progress.runs_completed.saturating_add(1).max(1)),
        );
        if stats.victory {
            anyhow::bail!(
                "--bot-play produced a victory; retry or pick a defeat with --from-run-history"
            );
        }
        let reason = stats.death_reason.unwrap_or(GameOverReason::OutOfPlays);
        let snap = snapshot_from_run(&terminal.defeat_journal, reason, &terminal);
        *run = terminal;
        run.defeat_memorial_kind = Some(select_memorial(&snap));
        reason
    } else {
        setup_defeat_game_over_screenshot_state(run);
        GameOverReason::OutOfPlays
    };

    Ok(Scene::GameOver(GameOverScene::new(run, reason)))
}

pub(crate) struct ScreenshotSceneSetup {
    pub scene: Scene,
    pub game_in_progress: bool,
    pub hero_play: bool,
    pub unlock_yaku: bool,
    pub unlock_collection: bool,
    pub force_relic_modal: bool,
}

pub(crate) fn resolve_screenshot_scene(
    s: &main_cli::ScreenshotCli,
    run: &mut RunState,
    progress: &mut crate::core::progression::PlayerProgress,
) -> anyhow::Result<ScreenshotSceneSetup> {
    let mut hero_play = false;
    let mut unlock_yaku = false;
    let mut unlock_collection = false;
    let mut force_relic_modal = false;

    if let Some(tab) = collection_screenshot_tab(&s.scene) {
        unlock_collection = true;
        return Ok(ScreenshotSceneSetup {
            scene: collection_scene(tab),
            game_in_progress: false,
            hero_play,
            unlock_yaku,
            unlock_collection,
            force_relic_modal,
        });
    }

    let (scene, game_in_progress) = match s.scene.as_str() {
        "yaku_journal" => {
            unlock_yaku = true;
            (Scene::YakuJournal(crate::scenes::YakuJournalScene::new()), false)
        }
        "gameplay" => {
            setup_gameplay_screenshot_state(run);
            (Scene::Gameplay(Box::new(GameplayScene::new())), true)
        }
        "gameplay_hero" => {
            setup_hero_state(run);
            hero_play = true;
            (Scene::Gameplay(Box::new(GameplayScene::new())), true)
        }
        "pick_blind" => (Scene::PickBlind(crate::scenes::PickBlindScene::new()), true),
        "shop" => {
            setup_shop_state(run);
            let mut shop = ShopScene::new(run, progress);
            if let Some(focus_slug) = s.shop_focus.as_deref() {
                shop.set_focus_for_screenshot(focus_slug)
                    .map_err(anyhow::Error::msg)?;
            }
            (Scene::Shop(shop), true)
        }
        "options" => (Scene::Options(crate::scenes::OptionsScene::new()), false),
        "main_menu_exterior" | "start_screen" => (
            Scene::MainMenuExterior(crate::scenes::MainMenuExteriorScene::new()),
            false,
        ),
        "tile_select" => (Scene::TileSelect(crate::scenes::TileSelectScene::new()), false),
        "guide" | "tile_guide" | "tiles_guide" => {
            let page = s.guide_page.map(|p| p.saturating_sub(1) as usize).unwrap_or(0);
            (Scene::Guide(crate::scenes::GuideScene::with_page(page)), false)
        }
        "tutorial" | "tutorial_campaign" => {
            (Scene::TutorialCampaign(TutorialCampaignScene::new()), false)
        }
        "transition_playground" => (
            Scene::TransitionPlayground(crate::scenes::TransitionPlaygroundScene::new(false)),
            false,
        ),
        "material_viewer" => (
            Scene::MaterialViewer(crate::scenes::MaterialViewerScene::new(false)),
            false,
        ),
        "rumble_lab" => (Scene::RumbleLab(crate::scenes::RumbleLabScene::new(false)), false),
        "tile_anchor_lab" => (
            Scene::TileAnchorLab(crate::scenes::TileAnchorLabScene::new(false)),
            false,
        ),
        "relic_unlock" => {
            force_relic_modal = true;
            (
                Scene::MainMenuExterior(crate::scenes::MainMenuExteriorScene::new()),
                false,
            )
        }
        "game_over_level_up" | "meta_level_up" => {
            let mut level_progress = crate::core::progression::PlayerProgress::new();
            level_progress.runs_completed = 1;
            let result = level_progress.check_level_up().ok_or_else(|| {
                anyhow::anyhow!("game_over_level_up: check_level_up returned None")
            })?;
            let ww = s.width.max(1) as f32;
            let hh = s.height.max(1) as f32;
            let modal = crate::main_draw::build_level_up_modal(&result, ww, hh)
                .ok_or_else(|| anyhow::anyhow!("game_over_level_up: no unlock pages for modal"))?;
            (
                Scene::Showcase(ShowcaseScene::new(ShowcasePresenter::MetaLevelUp(
                    MetaLevelUpPresenter::new(modal),
                ))),
                false,
            )
        }
        "zodiac_celebration" => (zodiac_showcase_scene(s)?, false),
        "showcase" => {
            if s.pack.is_some() {
                setup_shop_state(run);
                prime_shop_stock(run, progress);
                (tile_pack_showcase_scene(run, s.pack.as_deref())?, true)
            } else {
                (zodiac_showcase_scene(s)?, false)
            }
        }
        "tile_pack_celebration" => {
            setup_shop_state(run);
            prime_shop_stock(run, progress);
            (tile_pack_showcase_scene(run, s.pack.as_deref())?, true)
        }
        "game_over_defeat" | "defeat" => (game_over_defeat_scene(s, run, progress)?, true),
        other => {
            anyhow::bail!(
                "unsupported --scene '{other}' (supported: collection, archive, archive_bosses, chronicle, \
                yaku_journal, gameplay, gameplay_hero, pick_blind, shop, options, \
                main_menu_exterior, tile_select, guide, tutorial, transition_playground, \
                material_viewer, relic_unlock, game_over_level_up, game_over_defeat, meta_level_up, \
                showcase, zodiac_celebration, tile_pack_celebration)"
            )
        }
    };

    Ok(ScreenshotSceneSetup {
        scene,
        game_in_progress,
        hero_play,
        unlock_yaku,
        unlock_collection,
        force_relic_modal,
    })
}

pub(crate) fn scene_for_room_gi_bake(
    room: crate::render::room_gi_bake::RoomGiRoom,
    progress: &crate::core::progression::PlayerProgress,
) -> (Scene, RunState, bool) {
    let mut run = RunState::new_demo();
    match room {
        crate::render::room_gi_bake::RoomGiRoom::Shop => {
            setup_shop_state(&mut run);
            (Scene::Shop(ShopScene::new(&mut run, progress)), run, false)
        }
        crate::render::room_gi_bake::RoomGiRoom::Hallway => {
            (Scene::PickBlind(crate::scenes::PickBlindScene::new()), run, true)
        }
        crate::render::room_gi_bake::RoomGiRoom::Archive => {
            let coll = crate::scenes::CollectionScene::new();
            (Scene::Collection(coll), run, false)
        }
        crate::render::room_gi_bake::RoomGiRoom::MainMenu => (
            Scene::MainMenuExterior(crate::scenes::MainMenuExteriorScene::new()),
            run,
            false,
        ),
    }
}
