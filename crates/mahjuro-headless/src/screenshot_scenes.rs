use mahjuro::game::run::RunState;
use mahjuro::scenes::shop::ShopScene;
use mahjuro::scenes::{DefeatScene, GameplayScene, Scene, VictoryScene, WallLedgerScene};
use mahjuro::scenes::{
    MetaLevelUpPresenter, ShowcasePresenter, ShowcaseScene, TilePackPresenter,
    TutorialCampaignScene, ZodiacPresenter,
};

use super::fixtures::{
    prime_shop_stock, setup_defeat_game_over_screenshot_state, setup_gameplay_screenshot_state,
    setup_gameplay_valid_play_screenshot_state, setup_hero_state, setup_shop_state,
    setup_victory_game_over_screenshot_state, setup_wall_ledger_screenshot_state,
};
use super::slug::{parse_tile_pack_slug, parse_zodiac_slug};

pub(crate) fn validate_screenshot_cli(
    s: &crate::screenshot_cli::ScreenshotCli,
) -> anyhow::Result<()> {
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
    if s.collection_focus.is_some() && !collection_like {
        anyhow::bail!(
            "--collection-focus is only valid with --scene collection (or archive, chronicle, …)"
        );
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
    let defeat_like = mahjuro::render::scene_keys::normalize_scene_key(&s.scene)
        == mahjuro::render::scene_keys::DEFEAT;
    if (s.bot_play || s.from_run_history.is_some() || s.seed_bot_runs.is_some()) && !defeat_like {
        anyhow::bail!(
            "--bot-play, --from-run-history, and --seed-bot-runs are only valid with --scene defeat"
        );
    }
    if s.page.is_some() && !screenshot_scene_accepts_page(&s.scene) {
        anyhow::bail!("--page is only valid with --scene guide or tutorial");
    }
    Ok(())
}

fn screenshot_scene_accepts_page(scene: &str) -> bool {
    matches!(
        scene,
        "guide" | "tile_guide" | "tiles_guide" | "tutorial" | "tutorial_campaign"
    )
}

fn screenshot_page_index(s: &crate::screenshot_cli::ScreenshotCli) -> usize {
    s.page.map(|p| p.saturating_sub(1) as usize).unwrap_or(0)
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
    Ordeals,
    Talismans,
}

pub(crate) fn collection_screenshot_tab_for_overlay(scene: &str) -> bool {
    collection_screenshot_tab(scene).is_some()
}

fn collection_screenshot_tab(scene: &str) -> Option<CollectionScreenshotTab> {
    match scene {
        "collection" | "archive" => Some(CollectionScreenshotTab::Relics),
        "chronicle" | "archive_chronicle" => Some(CollectionScreenshotTab::Chronicle),
        "archive_ordeals" | "collection_ordeals" => Some(CollectionScreenshotTab::Ordeals),
        "archive_talismans" | "collection_talismans" => Some(CollectionScreenshotTab::Talismans),
        _ => None,
    }
}

fn collection_scene(tab: CollectionScreenshotTab) -> Scene {
    let mut coll = mahjuro::scenes::ArchiveScene::new();
    match tab {
        CollectionScreenshotTab::Relics => {}
        CollectionScreenshotTab::Chronicle => coll.prepare_chronicle_for_screenshot(),
        CollectionScreenshotTab::Ordeals => coll.prepare_ordeals_for_screenshot(),
        CollectionScreenshotTab::Talismans => coll.prepare_talismans_for_screenshot(),
    }
    Scene::Archive(coll)
}

fn zodiac_from_cli(
    s: &crate::screenshot_cli::ScreenshotCli,
) -> anyhow::Result<mahjuro::core::zodiac::ZodiacKind> {
    match s.zodiac.as_deref() {
        Some(slug) => parse_zodiac_slug(slug),
        None => Ok(mahjuro::core::zodiac::ZodiacKind::Snake),
    }
}

fn zodiac_celebration_level(s: &crate::screenshot_cli::ScreenshotCli) -> u32 {
    s.celebration_level.unwrap_or(2).max(1)
}

pub(crate) fn zodiac_showcase_scene(
    s: &crate::screenshot_cli::ScreenshotCli,
) -> anyhow::Result<Scene> {
    let z = zodiac_from_cli(s)?;
    let level = zodiac_celebration_level(s);
    let yaku = z.yaku();
    Ok(Scene::Showcase(ShowcaseScene::new(
        ShowcasePresenter::Zodiac(ZodiacPresenter::new(z, yaku.name(), level)),
    )))
}

pub(crate) fn tile_pack_showcase_scene(
    run: &RunState,
    pack_slug: Option<&str>,
) -> anyhow::Result<Scene> {
    let pack = match pack_slug {
        Some(slug) => parse_tile_pack_slug(slug)?,
        None => mahjuro::core::tile_pack::TilePackKind::Honors,
    };
    Ok(Scene::Showcase(ShowcaseScene::new(
        ShowcasePresenter::TilePack(Box::new(TilePackPresenter::new_headless_screenshot(
            run, pack,
        ))),
    )))
}

fn game_over_defeat_scene(
    s: &crate::screenshot_cli::ScreenshotCli,
    run: &mut RunState,
    progress: &mut mahjuro::core::progression::PlayerProgress,
) -> anyhow::Result<Scene> {
    use mahjuro::core::memorial_talisman::select_memorial;
    use mahjuro::game::event_bus::GameOverReason;
    use mahjuro::game::memorial_run::snapshot_from_run;
    use mahjuro::game::progression_run::{hydrate_game_over_run, run_record_defeat_reason};

    if s.bot_play && s.from_run_history.is_some() {
        anyhow::bail!("use only one of --bot-play or --from-run-history");
    }
    if s.seed_bot_runs.is_some() && s.from_run_history.is_none() {
        anyhow::bail!("--seed-bot-runs requires --from-run-history <index>");
    }

    if let Some(n) = s.seed_bot_runs {
        mahjuro::bot::seed_progress_from_bot_runs(progress, n);
    }

    let reason = if let Some(idx) = s.from_run_history {
        let rec = progress.run_history.get(idx).ok_or_else(|| {
            anyhow::anyhow!(
                "profile run_history[{idx}] missing (have {} runs; seed with --seed-bot-runs N)",
                progress.run_history.len()
            )
        })?;
        let reason = run_record_defeat_reason(rec).ok_or_else(|| {
            anyhow::anyhow!("profile run_history[{idx}] is not a defeat (use a defeat index)")
        })?;
        hydrate_game_over_run(rec, run);
        reason
    } else if s.bot_play {
        let (terminal, stats) = mahjuro::bot::play_bot_run(
            mahjuro::bot::BotConfig::default(),
            mahjuro::bot::BotRunOptions {
                log: false,
                ..mahjuro::bot::BotRunOptions::default()
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

    Ok(Scene::Defeat(DefeatScene::new(run, reason)))
}

pub(crate) struct ScreenshotSceneSetup {
    pub scene: Scene,
    pub game_in_progress: bool,
    pub hero_play: bool,
    pub unlock_yaku: bool,
    pub unlock_collection: bool,
    pub force_relic_modal: bool,
    pub force_round_win_modal: bool,
}

pub(crate) fn resolve_screenshot_scene(
    s: &crate::screenshot_cli::ScreenshotCli,
    run: &mut RunState,
    progress: &mut mahjuro::core::progression::PlayerProgress,
) -> anyhow::Result<ScreenshotSceneSetup> {
    let mut hero_play = false;
    let mut unlock_yaku = false;
    let mut unlock_collection = false;
    let mut force_relic_modal = false;
    let mut force_round_win_modal = false;

    if let Some(tab) = collection_screenshot_tab(&s.scene) {
        unlock_collection = true;
        return Ok(ScreenshotSceneSetup {
            scene: collection_scene(tab),
            game_in_progress: false,
            hero_play,
            unlock_yaku,
            unlock_collection,
            force_relic_modal,
            force_round_win_modal,
        });
    }

    let (scene, game_in_progress) = match s.scene.as_str() {
        "yaku_journal" => {
            unlock_yaku = true;
            (
                Scene::YakuJournal(mahjuro::scenes::YakuJournalScene::new()),
                false,
            )
        }
        "wall_ledger" => {
            setup_wall_ledger_screenshot_state(run);
            (Scene::WallLedger(WallLedgerScene::live()), true)
        }
        "gameplay" | "gameplay_valid_play" => {
            if s.scene.as_str() == "gameplay_valid_play" {
                setup_gameplay_valid_play_screenshot_state(run);
            } else {
                setup_gameplay_screenshot_state(run);
            }
            (Scene::Gameplay(Box::new(GameplayScene::new())), true)
        }
        "gameplay_hero" => {
            setup_hero_state(run);
            hero_play = true;
            (Scene::Gameplay(Box::new(GameplayScene::new())), true)
        }
        "round_win" | "winner" | "winner_modal" => {
            crate::fixtures::setup_round_win_screenshot_state(run);
            force_round_win_modal = true;
            (Scene::Gameplay(Box::new(GameplayScene::new())), true)
        }
        "hallway" | "pick_chamber" | "pick_blind" => {
            (Scene::Hallway(mahjuro::scenes::HallwayScene::new()), true)
        }
        "stairway" | "staircase" => (Scene::Stairway(mahjuro::scenes::StairwayScene::new()), true),
        "decimation" => {
            setup_shop_state(run);
            (
                Scene::Stairway(mahjuro::scenes::StairwayScene::for_decimation_screenshot(
                    run,
                )),
                true,
            )
        }
        "decimation_revealed" => {
            setup_shop_state(run);
            (
                Scene::Stairway(
                    mahjuro::scenes::StairwayScene::for_decimation_revealed_screenshot(run),
                ),
                true,
            )
        }
        "shop" => {
            setup_shop_state(run);
            let mut shop = ShopScene::new(run, progress);
            if let Some(focus_slug) = s.shop_focus.as_deref() {
                shop.set_focus_for_screenshot(focus_slug)
                    .map_err(anyhow::Error::msg)?;
            }
            (Scene::Shop(shop), true)
        }
        "options" => (Scene::Options(mahjuro::scenes::OptionsScene::new()), false),
        "credits" => (
            Scene::Credits(mahjuro::scenes::CreditsScene::from_options()),
            false,
        ),
        "main_menu" | "main_menu_exterior" | "start_screen" => (
            Scene::MainMenu(mahjuro::scenes::MainMenuScene::new()),
            false,
        ),
        "tile_select" => (
            Scene::TileSelect(mahjuro::scenes::TileSelectScene::new()),
            false,
        ),
        "guide" | "tile_guide" | "tiles_guide" => (
            Scene::Guide(mahjuro::scenes::GuideScene::with_page(
                screenshot_page_index(s),
            )),
            false,
        ),
        "tutorial" | "tutorial_campaign" => (
            Scene::TutorialCampaign(TutorialCampaignScene::with_page(screenshot_page_index(s))),
            false,
        ),
        "transition_playground" => (
            Scene::TransitionPlayground(mahjuro::scenes::TransitionPlaygroundScene::new(false)),
            false,
        ),
        "material_viewer" => (
            Scene::MaterialViewer(mahjuro::scenes::MaterialViewerScene::new(false)),
            false,
        ),
        "rumble_lab" => (
            Scene::RumbleLab(mahjuro::scenes::RumbleLabScene::new(false)),
            false,
        ),
        "tile_anchor_lab" => (
            Scene::TileAnchorLab(mahjuro::scenes::TileAnchorLabScene::new(false)),
            false,
        ),
        "button_aabb_lab" => (
            Scene::ButtonAabbLab(mahjuro::scenes::ButtonAabbLabScene::new(false)),
            false,
        ),
        "relic_unlock" => {
            force_relic_modal = true;
            (
                Scene::MainMenu(mahjuro::scenes::MainMenuScene::new()),
                false,
            )
        }
        "game_over_level_up" | "meta_level_up" => {
            let mut level_progress = mahjuro::core::progression::PlayerProgress::new();
            level_progress.level_progress_points =
                mahjuro::core::progression::PlayerProgress::min_points_for_level(2);
            let result = level_progress.check_level_up().ok_or_else(|| {
                anyhow::anyhow!("game_over_level_up: check_level_up returned None")
            })?;
            let ww = s.width.max(1) as f32;
            let hh = s.height.max(1) as f32;
            let modal = mahjuro::main_draw::build_level_up_modal(&result, ww, hh)
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
        "defeat" | "game_over_defeat" => (game_over_defeat_scene(s, run, progress)?, true),
        "victory" | "game_over_victory" => {
            setup_victory_game_over_screenshot_state(run);
            (Scene::Victory(VictoryScene::new(run)), true)
        }
        other => {
            anyhow::bail!(
                "unsupported --scene '{other}' (supported: archive, archive_ordeals, chronicle, \
                yaku_journal, wall_ledger, gameplay, gameplay_valid_play, gameplay_hero, round_win, hallway, stairway, decimation, shop, options, \
                main_menu, tile_select, guide, tutorial, transition_playground, \
                material_viewer, tile_anchor_lab, relic_unlock, game_over_level_up, defeat, victory, meta_level_up, \
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
        force_round_win_modal,
    })
}
