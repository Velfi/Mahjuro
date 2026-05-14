use std::path::PathBuf;

use super::*;
use crate::scenes::shop::ShopScene;
use crate::scenes::{
    CollectionInspectPresenter, MetaLevelUpPresenter, ShopInspectPresenter, ShowcasePresenter,
    ShowcaseScene, TilePackPresenter, ZodiacPresenter,
};

/// Map a user-supplied `--boss` slug (case-insensitive, spaces/underscores
/// interchangeable) to a `BossKind`. Matches against canonical `name()`
/// strings so e.g. "tax_collector", "Tax Collector", and "TaxCollector"
/// all resolve to `BossKind::TaxCollector`.
fn parse_boss_slug(slug: &str) -> anyhow::Result<crate::core::boss::BossKind> {
    let normalized = slug
        .trim()
        .to_ascii_lowercase()
        .replace(['_', '-', ' '], "");
    let normalize = |s: &str| {
        s.to_ascii_lowercase()
            .replace(['_', '-', ' ', '\''], "")
            .replace("the", "")
    };
    for def in crate::core::boss::all_bosses()
        .iter()
        .chain(crate::core::boss::final_bosses().iter())
    {
        if normalize(def.name) == normalized
            || format!("{:?}", def.kind).to_ascii_lowercase() == normalized
        {
            return Ok(def.kind);
        }
    }
    anyhow::bail!("unknown --boss '{slug}'");
}

/// `--zodiac` for `zodiac_celebration` / zodiac-mode `showcase` screenshots: slug or display name.
fn parse_zodiac_slug(slug: &str) -> anyhow::Result<crate::core::zodiac::ZodiacKind> {
    let normalized = slug
        .trim()
        .to_ascii_lowercase()
        .replace(['_', '-', ' '], "");
    for z in crate::core::zodiac::ZodiacKind::all() {
        if z.slug() == normalized || z.name().to_ascii_lowercase().replace(' ', "") == normalized {
            return Ok(*z);
        }
    }
    anyhow::bail!("unknown --zodiac '{slug}'");
}

/// `--pack` for `tile_pack_celebration` / `showcase --pack`: variant name or compact pack title.
fn parse_tile_pack_slug(slug: &str) -> anyhow::Result<crate::core::tile_pack::TilePackKind> {
    use crate::core::tile_pack::TilePackKind;
    let n = slug
        .trim()
        .to_ascii_lowercase()
        .replace(['_', '-', ' '], "");
    for &k in TilePackKind::all() {
        let debug_s = format!("{:?}", k).to_ascii_lowercase();
        let compact: String = k
            .name()
            .to_ascii_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect();
        if n == debug_s || n == compact {
            return Ok(k);
        }
    }
    anyhow::bail!(
        "unknown --pack '{slug}' (try honors, terminals, flowers, bamboo_grove, coin_cache, scroll_library)"
    );
}

fn shop_focus_slug_inspectable(slug: &str) -> bool {
    slug.starts_with("relic:")
        || slug.starts_with("ribbon:")
        || slug.starts_with("talisman:")
        || slug.starts_with("pack:")
}

/// Profile snapshot for shop stock in screenshot CLI (Qilin ribbon gate).
fn screenshot_profile_for_shop_stock(
    fresh_progress: bool,
) -> crate::core::progression::PlayerProgress {
    let settings = persistence::load_settings();
    if fresh_progress {
        crate::core::progression::PlayerProgress::new()
    } else {
        persistence::load_profile(settings.active_profile)
    }
}

/// Replace `run`'s freshly-dealt hand with a curated 14-tile winning hand
/// designed for a juicy Steam-store hero shot: Red Dragon triplet, White
/// Dragon triplet, two number sequences, East Wind pair. Decomposes as
/// 4 sets + pair (yakuman-adjacent: Big Two Dragons + Yakuhai), and
/// pairs naturally with `DragonRage` / `WhiteDragonsHush` relics.
///
/// Marks every tile as selected so the next `UiAction::ScoreHand` plays
/// the full hand. Also stocks `relics.active` with four visually
/// distinctive relics (dragon trio + GoldenEngine) so active relics read well
/// at thumbnail size.
fn setup_hero_state(run: &mut RunState) {
    use crate::core::relic::RelicId;
    use crate::core::tile::{Suit, Tile};
    *run.hand_mut() = vec![
        Tile::new(Suit::Dragon, 1, 100), // Red Dragon
        Tile::new(Suit::Dragon, 1, 101),
        Tile::new(Suit::Dragon, 1, 102),
        Tile::new(Suit::Dragon, 3, 103), // White Dragon
        Tile::new(Suit::Dragon, 3, 104),
        Tile::new(Suit::Dragon, 3, 105),
        Tile::new(Suit::Bamboos, 5, 106),
        Tile::new(Suit::Bamboos, 6, 107),
        Tile::new(Suit::Bamboos, 7, 108),
        Tile::new(Suit::Circles, 1, 109),
        Tile::new(Suit::Circles, 2, 110),
        Tile::new(Suit::Circles, 3, 111),
        Tile::new(Suit::Wind, 1, 112), // East
        Tile::new(Suit::Wind, 1, 113),
    ];
    run.hand_mut().sort();
    *run.selected_mut() = vec![true; run.hand().len()];

    run.relics.active.clear();
    for r in [
        RelicId::DragonRage,
        RelicId::WhiteDragonsHush,
        RelicId::GreenLuck,
        RelicId::GoldenEngine,
    ] {
        if !run.relics.is_full() {
            run.relics.active.push(r);
        }
    }
}

/// Populate a richer `run` state for the shop screenshot: bump gold,
/// pretend ante 3 (so the round-3 stock variety kicks in), and set the
/// `rich_stock` tag to add extra relic offerings. The tag is consumed
/// inside `ShopScene::new(.., progress)`, leaving the run otherwise untouched.
fn setup_shop_state(run: &mut RunState) {
    run.gold = 42;
    run.run_number = 3;
    run.ante = 3;
    run.tag_rich_stock = true;
}

/// Rich gameplay frame for `screenshot --scene gameplay`: hero-style rack,
/// a complete committed structure in the bank (same geometry as
/// `RunState` tests `winning_structure`), five relics, and consumable slots
/// filled only up to the mode cap (standard = 2).
fn setup_gameplay_screenshot_state(run: &mut RunState) {
    use crate::core::consumable::Consumable;
    use crate::core::hand::{DetectedMeld, MeldKind};
    use crate::core::relic::RelicId;
    use crate::core::talisman::TalismanKind;
    use crate::core::tile::{Suit, Tile};
    use crate::core::zodiac::ZodiacKind;

    setup_hero_state(run);
    *run.selected_mut() = vec![false; run.hand().len()];

    run.set_auto_cash_in_on_full_structure(false);
    *run.structure_tiles_mut() = vec![
        Tile::new(Suit::Characters, 1, 1),
        Tile::new(Suit::Characters, 1, 2),
        Tile::new(Suit::Characters, 2, 3),
        Tile::new(Suit::Characters, 3, 4),
        Tile::new(Suit::Characters, 4, 5),
        Tile::new(Suit::Circles, 2, 6),
        Tile::new(Suit::Circles, 3, 7),
        Tile::new(Suit::Circles, 4, 8),
        Tile::new(Suit::Bamboos, 5, 9),
        Tile::new(Suit::Bamboos, 6, 10),
        Tile::new(Suit::Bamboos, 7, 11),
        Tile::new(Suit::Wind, 1, 12),
        Tile::new(Suit::Wind, 1, 13),
        Tile::new(Suit::Wind, 1, 14),
    ];
    *run.structure_sets_mut() = vec![
        DetectedMeld {
            kind: MeldKind::Pair,
            tile_ids: vec![1, 2],
        },
        DetectedMeld {
            kind: MeldKind::Sequence,
            tile_ids: vec![3, 4, 5],
        },
        DetectedMeld {
            kind: MeldKind::Sequence,
            tile_ids: vec![6, 7, 8],
        },
        DetectedMeld {
            kind: MeldKind::Sequence,
            tile_ids: vec![9, 10, 11],
        },
        DetectedMeld {
            kind: MeldKind::Triplet,
            tile_ids: vec![12, 13, 14],
        },
    ];

    if !run.relics.is_full() {
        run.relics.active.push(RelicId::Geese);
    }

    run.consumables.items.clear();
    let _ = run
        .consumables
        .try_push(Consumable::Talisman(TalismanKind::Pearl));
    let _ = run
        .consumables
        .try_push(Consumable::Zodiac(ZodiacKind::Dragon));
}

/// Force the given run into the Boss blind with `kind` as the upcoming
/// boss, and resolve the boss effect so gameplay-side rendering picks up
/// the rule override. Used by the `screenshot` CLI to preview boss cards
/// and in-round ofuda art without walking through pick_blind.
fn force_boss_blind(run: &mut RunState, kind: crate::core::boss::BossKind) {
    run.blind = crate::core::rules::BlindKind::Boss;
    run.upcoming_blind = crate::core::rules::BlindKind::Boss;
    run.ante = kind.def().min_ante.max(run.ante);
    run.boss.upcoming = Some(kind);
    run.resolve_upcoming_boss();
    run.apply_blind(crate::core::rules::BlindKind::Boss);
}

pub fn run_screenshot_command(s: main_cli::ScreenshotCli) -> anyhow::Result<()> {
    asset_path::init();
    asset_path::log_all_assets();
    let shop_like = matches!(s.scene.as_str(), "shop");
    let collection_like = matches!(s.scene.as_str(), "collection");
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
    if s.journal_open.is_some() && !shop_like {
        anyhow::bail!("--journal-open is only valid with --scene shop");
    }
    if s.journal_transition.is_some() && !shop_like {
        anyhow::bail!("--journal-transition is only valid with --scene shop");
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
    let boss_override = s.boss.as_deref().map(parse_boss_slug).transpose()?;
    let mut run = RunState::new_demo();
    if let Some(kind) = boss_override {
        force_boss_blind(&mut run, kind);
    }
    let mut hero_play = false;
    let mut unlock_yaku = false;
    let mut unlock_collection = false;
    let mut force_relic_modal = false;
    let (scene, game_in_progress) = match s.scene.as_str() {
        "collection" => {
            unlock_collection = true;
            (Scene::Collection(scenes::CollectionScene::new()), false)
        }
        "yaku_journal" => {
            unlock_yaku = true;
            (Scene::YakuJournal(scenes::YakuJournalScene::new()), false)
        }
        "gameplay" => {
            setup_gameplay_screenshot_state(&mut run);
            (Scene::Gameplay(GameplayScene::new()), true)
        }
        "gameplay_hero" => {
            setup_hero_state(&mut run);
            hero_play = true;
            (Scene::Gameplay(GameplayScene::new()), true)
        }
        "pick_blind" => (Scene::PickBlind(scenes::PickBlindScene::new()), true),
        "shop" => {
            setup_shop_state(&mut run);
            let progress = screenshot_profile_for_shop_stock(s.fresh_progress);
            let mut shop = ShopScene::new(&mut run, &progress);
            if let Some(focus_slug) = s.shop_focus.as_deref() {
                shop.set_focus_for_screenshot(focus_slug)
                    .map_err(anyhow::Error::msg)?;
            }
            if let Some(amt) = s.journal_open {
                shop.set_journal_open_for_screenshot(amt);
            }
            if let Some(prog) = s.journal_transition {
                shop.set_journal_transition_for_screenshot(prog);
            }
            (Scene::Shop(shop), true)
        }
        "main_menu_exterior" => (
            Scene::MainMenuExterior(scenes::MainMenuExteriorScene::new()),
            false,
        ),
        "start_screen" => (
            Scene::MainMenuExterior(scenes::MainMenuExteriorScene::new()),
            false,
        ),
        "tile_select" => (Scene::TileSelect(scenes::TileSelectScene::new()), false),
        "transition_playground" => (
            Scene::TransitionPlayground(scenes::TransitionPlaygroundScene::new(false)),
            false,
        ),
        "material_viewer" => (
            Scene::MaterialViewer(scenes::MaterialViewerScene::new(false)),
            false,
        ),
        "rumble_lab" => (Scene::RumbleLab(scenes::RumbleLabScene::new(false)), false),
        // Layered the relic-unlock modal on top of the main-menu backdrop
        // so we can iterate on the modal's hero staging without driving a live
        // game state. The modal stages itself entirely on top.
        "relic_unlock" => {
            force_relic_modal = true;
            (
                Scene::MainMenuExterior(scenes::MainMenuExteriorScene::new()),
                false,
            )
        }
        "game_over_level_up" | "meta_level_up" => {
            let mut progress = crate::core::progression::PlayerProgress::new();
            progress.runs_completed = 1;
            let result = progress.check_level_up().ok_or_else(|| {
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
        "zodiac_celebration" => {
            let z = s
                .zodiac
                .as_deref()
                .map(parse_zodiac_slug)
                .transpose()?
                .unwrap_or(crate::core::zodiac::ZodiacKind::Snake);
            let level = s.celebration_level.unwrap_or(2).max(1);
            let yaku = z.yaku();
            (
                Scene::Showcase(ShowcaseScene::new(ShowcasePresenter::Zodiac(
                    ZodiacPresenter::new(z, yaku.name(), level),
                ))),
                false,
            )
        }
        "showcase" => {
            if s.pack.is_some() {
                let pack = s
                    .pack
                    .as_deref()
                    .map(parse_tile_pack_slug)
                    .transpose()?
                    .unwrap_or(crate::core::tile_pack::TilePackKind::Honors);
                setup_shop_state(&mut run);
                let progress = screenshot_profile_for_shop_stock(s.fresh_progress);
                let shop = ShopScene::new(&mut run, &progress);
                let counts = shop.tile_pack_celeb_inventory_counts(&run);
                (
                    Scene::Showcase(ShowcaseScene::new(ShowcasePresenter::TilePack(
                        TilePackPresenter::new_headless_with_shop_counts(&run, pack, counts),
                    ))),
                    true,
                )
            } else {
                let z = s
                    .zodiac
                    .as_deref()
                    .map(parse_zodiac_slug)
                    .transpose()?
                    .unwrap_or(crate::core::zodiac::ZodiacKind::Snake);
                let level = s.celebration_level.unwrap_or(2).max(1);
                let yaku = z.yaku();
                (
                    Scene::Showcase(ShowcaseScene::new(ShowcasePresenter::Zodiac(
                        ZodiacPresenter::new(z, yaku.name(), level),
                    ))),
                    false,
                )
            }
        }
        "tile_pack_celebration" => {
            let pack = s
                .pack
                .as_deref()
                .map(parse_tile_pack_slug)
                .transpose()?
                .unwrap_or(crate::core::tile_pack::TilePackKind::Honors);
            setup_shop_state(&mut run);
            let progress = screenshot_profile_for_shop_stock(s.fresh_progress);
            let shop = ShopScene::new(&mut run, &progress);
            let counts = shop.tile_pack_celeb_inventory_counts(&run);
            (
                Scene::Showcase(ShowcaseScene::new(ShowcasePresenter::TilePack(
                    TilePackPresenter::new_headless_with_shop_counts(&run, pack, counts),
                ))),
                true,
            )
        }
        other => {
            anyhow::bail!(
                "unsupported --scene '{other}' (supported: collection, \
                yaku_journal, gameplay, gameplay_hero, pick_blind, shop, \
                main_menu_exterior, tile_select, transition_playground, \
                material_viewer, relic_unlock, game_over_level_up, meta_level_up, \
                showcase, \
                zodiac_celebration, tile_pack_celebration)"
            )
        }
    };
    let mut app = HeadlessApp::with_run(
        scene,
        run,
        s.width.max(1),
        s.height.max(1),
        game_in_progress,
        s.fresh_progress,
    )?;
    if s.item_inspect {
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
    }
    if (s.shop_focus.is_some() && shop_like) || (s.item_inspect && collection_like) {
        // Cursor-mode update reassigns focus from the cursor pick every
        // tick; controller mode keeps pre-set shop/collection focus stable
        // across warmup.
        app.input_mode_override = Some(crate::ui::input::InputMode::Controller);
    }
    if hero_play {
        // Fire ScoreHand on tick 2 (after one warmup tick lets layouts/loads
        // settle), then warmup_frames ride out the cascade so the captured
        // frame lands mid-animation.
        app.queue_action_on_tick(2, UiAction::ScoreHand);
    }
    if unlock_yaku {
        app.unlock_all_yaku_for_screenshot();
    }
    if unlock_collection {
        app.unlock_all_for_collection_screenshot();
    }
    if force_relic_modal {
        app.force_relic_unlock_modal();
    }
    if let Some(mul) = s.gltf_emissive_scale {
        app.shop_env_lighting.gltf_emissive_scale = mul;
    }
    app.run_screenshot(s.output.clone(), s.warmup_frames)
}

/// Minimal non-winit runner used by the `screenshot` CLI path. Builds an
/// offscreen `WgpuRenderer`, renders `warmup_frames + 1` frames of the
/// target scene, and writes the PNG through the same renderer code path
/// the interactive capture uses. No window, no `ApplicationHandler`, no
/// swapchain `Outdated` retries — on macOS this is what lets CI + local
/// screenshot tests work without needing a visible foreground window.
///
/// `App` is deeply coupled to `winit::ApplicationHandler` and threads a
/// `Window` through its whole lifecycle; duplicating that coupling here
/// would be worse than this slim parallel runner. The scene draw path
/// (`Scene::draw_frame` + `WgpuRenderer::render`) is what both paths share.
struct HeadlessApp {
    renderer: WgpuRenderer,
    layout_engine: UiLayout,
    scene: Scene,
    /// Scene-owned overlays (e.g. [`Scene::ItemInspect`]) — same stack idea
    /// as the winit `App`, so pushdown inspect screenshots match production.
    overlay_stack: Vec<Scene>,
    run: RunState,
    anim: AnimationController,
    progress: crate::core::progression::PlayerProgress,
    active_profile: usize,
    gfx: RenderSettings,
    effect_layers: crate::effect_layers::EffectLayers,
    tonemap_tuning: crate::game::tonemap_tuning::TonemapTuningSet,
    width: u32,
    height: u32,
    game_in_progress: bool,
    /// Tick counter used by `queued_actions` to fire scripted UiActions on
    /// specific ticks (e.g. ScoreHand on tick 2 for the gameplay_hero shot).
    tick_count: u32,
    /// (tick_index, action) pairs to inject into `UpdateCtx::actions` on the
    /// matching tick. Pre-populated by the dispatcher; consumed in `tick()`.
    queued_actions: Vec<(u32, UiAction)>,
    /// When set, an overlay modal queue is ticked & staged on every
    /// frame using the same staging the live `App::draw` does. Lets
    /// `--scene relic_unlock` capture the redesigned celebration modal
    /// over a quiet backdrop without spinning up a full level-up flow.
    modal_overlay: Option<crate::ui::modal::ModalQueue>,
    /// Override the input mode passed to `UpdateCtx`. Default is
    /// `Cursor`, which is what the live game uses when the mouse is
    /// active. Set to `Controller` for screenshot paths that pre-set
    /// scene focus (e.g. `--shop-focus`) — in cursor mode the shop's
    /// `update_impl` reassigns focus from the cursor pick every tick,
    /// wiping any focus we set up before warmup. Controller mode keeps
    /// our pre-set focus stable across ticks so the focused state's
    /// dependent animations (e.g. journal-cover open tween) actually
    /// progress during warmup.
    input_mode_override: Option<crate::ui::input::InputMode>,
    /// Same knobs as interactive `App` debug shop env; headless defaults to
    /// [`ShopEnvLightingTune::SOURCE_DEFAULTS`] unless CLI overrides emissive scale.
    shop_env_lighting: crate::render::shop_glb::ShopEnvLightingTune,
}

impl HeadlessApp {
    fn with_run(
        scene: Scene,
        run: RunState,
        width: u32,
        height: u32,
        game_in_progress: bool,
        fresh_progress: bool,
    ) -> anyhow::Result<Self> {
        let settings = persistence::load_settings();
        let active_profile = settings.active_profile;
        let progress = if fresh_progress {
            crate::core::progression::PlayerProgress::new()
        } else {
            persistence::load_profile(active_profile)
        };
        let renderer = WgpuRenderer::new(render::wgpu_renderer::TargetInit::Headless {
            width,
            height,
            hdr_enabled: false,
        })?;
        Ok(Self {
            renderer,
            layout_engine: UiLayout::new(),
            scene,
            overlay_stack: Vec::new(),
            run,
            anim: AnimationController::new(),
            progress,
            active_profile,
            gfx: RenderSettings {
                effects_quality: settings.effects_quality,
                tile_preset: settings.tile_preset,
                tile_material: settings.tile_material,
                surface_kind: settings.surface_kind,
                tileset_name: settings.tileset_name.clone(),
                gamma: settings.gamma,
                shadows_enabled: settings.shadows_enabled,
                ssr_enabled: settings.ssr_enabled,
                hdr_enabled: false,
                // Headless screenshots stay on the clean tonemap path so
                // captures don't pick up VHS overlay artifacts. Per-scene
                // amounts can still be edited at runtime via the debug
                // overlay; the headless path simply ignores them.
                vhs_enabled: false,
            },
            effect_layers: crate::effect_layers::EffectLayers::FULL,
            tonemap_tuning: crate::game::tonemap_tuning::TonemapTuningSet::load(),
            width,
            height,
            game_in_progress,
            tick_count: 0,
            queued_actions: Vec::new(),
            modal_overlay: None,
            input_mode_override: None,
            shop_env_lighting: crate::render::shop_glb::ShopEnvLightingTune::SOURCE_DEFAULTS,
        })
    }

    /// Build a paginated relic-unlock modal and queue it as the overlay
    /// for every subsequent `tick()`. Used by `--scene relic_unlock` to
    /// capture the celebration modal in isolation.
    fn force_relic_unlock_modal(&mut self) {
        use crate::core::relic::all_relic_defs;
        use crate::ui::modal::{Modal, ModalQueue, ModalTheme, UnlockPage};

        // Pick a relic with a textured face that reads at thumbnail
        // size. Kong Collector matches the source screenshot; if it
        // ever leaves the catalog the chosen relic falls back to the
        // first defined entry so the screenshot harness keeps working.
        let defs = all_relic_defs();
        let chosen = defs
            .iter()
            .find(|d| d.name == "Kong Collector")
            .or_else(|| defs.first())
            .expect("at least one relic must be defined");
        let accent = match chosen.rarity {
            crate::core::relic::Rarity::Common => render::theme::color::rarity(0),
            crate::core::relic::Rarity::Uncommon => render::theme::color::rarity(1),
            crate::core::relic::Rarity::Rare => render::theme::color::rarity(2),
            crate::core::relic::Rarity::Legendary => render::theme::color::rarity(3),
        };
        let page = UnlockPage {
            category: "New Relic".into(),
            name: chosen.name.into(),
            description: chosen.description.into(),
            relic_id: Some(chosen.id),
            accent_color: accent,
        };
        // Drive the indicator to "6 / 14" so the page-of-N footer
        // matches the reference screenshot for visual comparison.
        let mut pages = Vec::with_capacity(14);
        for i in 0..14 {
            if i == 5 {
                pages.push(page.clone());
            } else {
                pages.push(UnlockPage {
                    category: "Placeholder".into(),
                    name: "—".into(),
                    description: String::new(),
                    relic_id: Some(chosen.id),
                    accent_color: accent,
                });
            }
        }
        let mut modal = Modal::new("Relic Unlocked", "", ModalTheme::Success).with_pages(pages);
        modal.current_page = 5;
        // Lantern motes drifting from the lower band.
        let w = self.width as f32;
        let h = self.height as f32;
        let modal = modal.with_fireworks(w * 0.5, h * 0.92, w * 0.85, 24);
        let mut queue = ModalQueue::default();
        queue.push(modal);
        self.modal_overlay = Some(queue);
    }

    fn queue_action_on_tick(&mut self, tick: u32, action: UiAction) {
        self.queued_actions.push((tick, action));
    }

    /// Mark every yaku as scored at least once and level a few up to 2,
    /// so the journal scene renders revealed tile patterns + gold glows
    /// instead of sealed "?" placeholders. Used only by the screenshot
    /// path; never persisted to disk.
    fn unlock_all_yaku_for_screenshot(&mut self) {
        use crate::core::yaku::YakuKind;
        for yk in YakuKind::all() {
            *self.progress.yaku_times_scored.entry(*yk).or_insert(0) += 1;
        }
        for yk in [
            YakuKind::Yakuhai,
            YakuKind::Toitoi,
            YakuKind::Honitsu,
            YakuKind::FullHand,
        ] {
            self.run.yaku_levels.levels.insert(yk, 3);
        }
    }

    /// Set the in-memory `PlayerProgress` to the max-level, fully-explored
    /// state so the Collection grid shows real relic/yaku/boss/talisman
    /// art instead of locked placeholder dots. Only mutates the headless
    /// runner's progress; never persisted.
    fn unlock_all_for_collection_screenshot(&mut self) {
        use crate::core::boss::all_bosses;
        use crate::core::talisman::TalismanKind;
        use crate::core::yaku::YakuKind;
        self.progress.runs_completed = 100;
        self.progress.has_won = true;
        for yk in YakuKind::all() {
            *self.progress.yaku_times_scored.entry(*yk).or_insert(0) += 1;
        }
        for def in all_bosses() {
            *self
                .progress
                .boss_times_encountered
                .entry(def.kind)
                .or_insert(0) += 1;
        }
        for tk in TalismanKind::all() {
            *self
                .progress
                .talisman_times_purchased
                .entry(*tk)
                .or_insert(0) += 1;
        }
    }

    fn tick(&mut self) {
        let now = Instant::now();
        self.anim.update(now);
        let layout = self
            .layout_engine
            .solve(self.width as f32, self.height as f32);
        let mut bus = EventBus::default();
        let mut quit_requested = false;
        let mut switch_profile: Option<usize> = None;
        let mut delete_profile: Option<usize> = None;
        let mut complete_onboarding = false;
        let mut overlay_request: Option<scenes::OverlayRequest> = None;
        let mut bump_archive_chronicle_seen: Option<u32> = None;
        let mut rumble_lab_ops: Vec<crate::ui::input::RumbleLabOp> = Vec::new();
        let actions_this_tick: Vec<UiAction> = self
            .queued_actions
            .iter()
            .filter(|(t, _)| *t == self.tick_count)
            .map(|(_, a)| *a)
            .collect();
        self.tick_count += 1;
        let headless_cascade = CascadeTuning::default();
        let loading_done = if self.overlay_stack.is_empty() {
            match &self.scene {
                Scene::Splash(_) => true,
                _ => !self.renderer.is_loading(),
            }
        } else {
            !self.renderer.is_loading()
        };
        let update_result = if self.overlay_stack.is_empty() {
            self.scene.update(UpdateCtx {
                actions: &actions_this_tick,
                button_clicks: &[],
                progress: &self.progress,
                active_profile: 0,
                run: &mut self.run,
                bus: &mut bus,
                anim: &mut self.anim,
                layout: &layout,
                focus_tile_index: 0,
                quit_requested: &mut quit_requested,
                switch_profile: &mut switch_profile,
                delete_profile: &mut delete_profile,
                complete_onboarding: &mut complete_onboarding,
                cursor_pos: (0.0, 0.0),
                mouse_left_down: false,
                loading_done,
                cascade_tuning: &headless_cascade,
                picked_shop_object: None,
                picked_gameplay_object: None,
                picked_collection_object: None,
                input_mode: self.input_mode_override.unwrap_or(InputMode::Cursor),
                picked_hand_tile: None,
                scroll_lines: 0.0,
                tutorial_eligible: false,
                multiple_materials: self.progress.plastic_unlocked(),
                resume_scene: persistence::ResumeScene::default(),
                transitioning: false,
                overlay_request: &mut overlay_request,
                headless: true,
                effect_layers: self.effect_layers,
                item_inspect_orbit_stick: (0.0, 0.0),
                item_inspect_zoom_triggers: 0.0,
                rumble_lab_ops: &mut rumble_lab_ops,
                suspended_shop: None,
                room_gltf_height_scale: crate::render::shop_glb::SHOP_ENV_HEIGHT_SCALE,
                bump_archive_chronicle_seen: &mut bump_archive_chronicle_seen,
            })
        } else {
            let showcase_shop_inspect = self.overlay_stack.last().is_some_and(|top| {
                matches!(
                    top,
                    Scene::Showcase(s)
                        if matches!(s.presenter, scenes::ShowcasePresenter::ShopInspect(_))
                )
            });
            let suspended_shop = if showcase_shop_inspect {
                match &self.scene {
                    Scene::Shop(shop) => Some(shop),
                    _ => None,
                }
            } else {
                None
            };
            self.overlay_stack
                .last_mut()
                .expect("overlay stack non-empty")
                .update(UpdateCtx {
                    actions: &actions_this_tick,
                    button_clicks: &[],
                    progress: &self.progress,
                    active_profile: 0,
                    run: &mut self.run,
                    bus: &mut bus,
                    anim: &mut self.anim,
                    layout: &layout,
                    focus_tile_index: 0,
                    quit_requested: &mut quit_requested,
                    switch_profile: &mut switch_profile,
                    delete_profile: &mut delete_profile,
                    complete_onboarding: &mut complete_onboarding,
                    cursor_pos: (0.0, 0.0),
                    mouse_left_down: false,
                    loading_done,
                    cascade_tuning: &headless_cascade,
                    picked_shop_object: None,
                    picked_gameplay_object: None,
                    picked_collection_object: None,
                    input_mode: self.input_mode_override.unwrap_or(InputMode::Cursor),
                    picked_hand_tile: None,
                    scroll_lines: 0.0,
                    tutorial_eligible: false,
                    multiple_materials: self.progress.plastic_unlocked(),
                    resume_scene: persistence::ResumeScene::default(),
                    transitioning: false,
                    overlay_request: &mut overlay_request,
                    headless: true,
                    effect_layers: self.effect_layers,
                    item_inspect_orbit_stick: (0.0, 0.0),
                    item_inspect_zoom_triggers: 0.0,
                    rumble_lab_ops: &mut rumble_lab_ops,
                    suspended_shop,
                    room_gltf_height_scale: crate::render::shop_glb::SHOP_ENV_HEIGHT_SCALE,
                    bump_archive_chronicle_seen: &mut bump_archive_chronicle_seen,
                })
        };
        match overlay_request {
            Some(scenes::OverlayRequest::Push(s)) => self.overlay_stack.push(*s),
            Some(scenes::OverlayRequest::Pop) => {
                self.overlay_stack.pop();
            }
            None => {}
        }
        let _ = update_result;

        let showcase_orbit_top = self
            .overlay_stack
            .last()
            .is_some_and(|top| matches!(top, Scene::Showcase(s) if s.wants_orbit_input()));
        let suspended_shop = match (&self.scene, showcase_orbit_top) {
            (Scene::Shop(s), true) => Some(s),
            _ => None,
        };
        let suspended_collection = match (&self.scene, showcase_orbit_top) {
            (Scene::Collection(c), true) => Some(c),
            _ => None,
        };

        let settings = crate::persistence::load_settings();
        let detected = crate::ui::button_prompts::GamepadStyle::default();
        let prompt_style = settings.glyph_prompt.resolve(detected);
        let glyphs = crate::ui::glyph_source::GlyphResolver::new(prompt_style, false, false);
        let ctx = DrawCtx::new(
            &layout,
            &self.anim,
            &self.run,
            &self.progress,
            self.active_profile,
            self.game_in_progress,
            self.renderer.projections(),
            None,
            None,
            scenes::DebugVisibility {
                hide_candles: false,
                hide_blind_plaque: false,
            },
            false,
            None,
            crate::render::shop_glb::SHOP_ENV_HEIGHT_SCALE,
            self.shop_env_lighting,
            self.effect_layers,
            (0.0, 0.0),
            self.input_mode_override.unwrap_or(InputMode::Cursor),
            false,
            false,
            prompt_style,
            glyphs,
            suspended_shop,
            suspended_collection,
            self.gfx.tile_preset,
            false,
        );
        let mut frame: UiFrame = if let Some(top) = self.overlay_stack.last() {
            top.draw_frame(ctx)
        } else {
            self.scene.draw_frame(ctx)
        };

        // Modal overlay (`--scene relic_unlock`): same staging as `App::draw`.
        if let Some(ref mut queue) = self.modal_overlay {
            queue.update();
            if let Some((
                modal_insts,
                modal_labels,
                modal_buttons,
                modal_relic_objects,
                modal_gradient_quads,
            )) = queue.draw(self.width as f32, self.height as f32)
            {
                let _ = modal_buttons; // headless ignores click routing
                frame.quads(modal_insts);
                frame.texts(modal_labels);
                if !modal_gradient_quads.is_empty() {
                    frame.gradient_quads(modal_gradient_quads);
                }
                apply_modal_relic_staging(
                    &mut frame,
                    self.width as f32,
                    self.height as f32,
                    modal_relic_objects,
                );
            }
        }

        let scene_for_renderer = self.overlay_stack.last().unwrap_or(&self.scene);
        let active_scene_key: Option<&'static str> = match scene_for_renderer {
            Scene::Showcase(_) => Some("showcase"),
            Scene::Shop(_) => Some("shop"),
            Scene::Gameplay(_) => Some("gameplay"),
            Scene::Collection(_) => Some("collection"),
            Scene::PickBlind(_) => Some("pick_blind"),
            Scene::MainMenuExterior(_) => Some("main_menu_exterior"),
            Scene::TutorialCampaign(_) => Some("tutorial"),
            _ => None,
        };
        self.renderer.set_active_scene(active_scene_key);
        let tonemap = self.tonemap_tuning.resolve(active_scene_key);
        self.renderer.set_tonemap_tuning(&tonemap);
        let rotations_scene = match self.overlay_stack.last() {
            Some(Scene::Showcase(s)) if s.wants_orbit_input() => &self.scene,
            _ => scene_for_renderer,
        };
        self.renderer
            .set_committed_arrange_rotations(collect_committed_rotations(rotations_scene));
        self.renderer
            .set_room_gltf_height_scale(crate::render::shop_glb::SHOP_ENV_HEIGHT_SCALE);
        let sl = self.shop_env_lighting;
        self.renderer.set_shop_env_render_tune(
            sl.linear_exposure,
            sl.ambient_scale,
            sl.lit_mesh_gltf_punctual_scale,
            sl.gltf_emissive_scale,
        );

        let active_material = frame
            .tile_material_override
            .unwrap_or(self.gfx.tile_material);

        let active_tileset_name = self.gfx.tileset_name.clone();
        let render_settings = self.effect_layers.wgpu_render_settings(
            &self.gfx,
            self.gfx.tile_preset,
            active_material,
            self.gfx.surface_kind,
            active_tileset_name,
            8.0,
            10.0,
        );
        if let Err(e) = self.renderer.render(&frame, render_settings) {
            log::error!("headless render: {e:?}");
        }
    }

    /// Render `warmup_frames + 1` frames, queue a screenshot on the last
    /// frame, and return. The PNG is written synchronously during the
    /// final `render()` call (see `WgpuRenderer::render` end-of-frame).
    ///
    /// If asset loading is still in flight after the requested warmup,
    /// keeps ticking (with a small sleep) until the renderer reports
    /// `!is_loading()`. This matters for screenshots that depend on
    /// late-arriving relic textures — the relic-loader thread decodes
    /// images at ~16ms each, slower than CPU-bound warmup ticks, so a
    /// fixed warmup count can race the loader. Cap at 600 extra ticks
    /// (~10s) so a stuck loader doesn't hang the headless harness.
    fn run_screenshot(mut self, output: PathBuf, warmup_frames: u32) -> anyhow::Result<()> {
        for _ in 0..warmup_frames {
            self.tick();
        }
        let mut extra = 0u32;
        while self.renderer.is_loading() && extra < 600 {
            self.tick();
            std::thread::sleep(std::time::Duration::from_millis(16));
            extra += 1;
        }
        if extra > 0 {
            log::debug!("screenshot: waited {extra} extra ticks for asset loading");
        }
        self.renderer.queue_screenshot(output.clone());
        self.tick();
        if self.renderer.screenshot_pending() {
            anyhow::bail!(
                "headless screenshot: final tick did not consume the queued path ({})",
                output.display()
            );
        }
        Ok(())
    }
}
