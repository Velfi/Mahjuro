use super::*;
use crate::scenes::shop::ShopScene;

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

/// Replace `run`'s freshly-dealt hand with a curated 14-tile winning hand
/// designed for a juicy Steam-store hero shot: Red Dragon triplet, White
/// Dragon triplet, two number sequences, East Wind pair. Decomposes as
/// 4 sets + pair (yakuman-adjacent: Big Two Dragons + Yakuhai), and
/// pairs naturally with `RedDragonRage` / `WhiteDragonsHush` relics.
///
/// Marks every tile as selected so the next `UiAction::ScoreHand` plays
/// the full hand. Also stocks `relics.active` with four visually
/// distinctive relics (dragon trio + GoldenEngine) so active relics read well
/// at thumbnail size.
fn setup_hero_state(run: &mut RunState) {
    use crate::core::relic::RelicId;
    use crate::core::tile::{Suit, Tile};
    run.hand = vec![
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
    run.hand.sort();
    run.selected = vec![true; run.hand.len()];

    run.relics.active.clear();
    for r in [
        RelicId::RedDragonRage,
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
/// inside `ShopScene::new`, leaving the run otherwise untouched.
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
    use crate::core::hand::{DetectedSet, SetKind};
    use crate::core::relic::RelicId;
    use crate::core::talisman::TalismanKind;
    use crate::core::tile::{Suit, Tile};
    use crate::core::zodiac::ZodiacKind;

    setup_hero_state(run);
    run.selected = vec![false; run.hand.len()];

    run.set_auto_cash_in_on_full_structure(false);
    run.structure_tiles = vec![
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
    run.structure_sets = vec![
        DetectedSet {
            kind: SetKind::Pair,
            tile_ids: vec![1, 2],
        },
        DetectedSet {
            kind: SetKind::Sequence,
            tile_ids: vec![3, 4, 5],
        },
        DetectedSet {
            kind: SetKind::Sequence,
            tile_ids: vec![6, 7, 8],
        },
        DetectedSet {
            kind: SetKind::Sequence,
            tile_ids: vec![9, 10, 11],
        },
        DetectedSet {
            kind: SetKind::Triplet,
            tile_ids: vec![12, 13, 14],
        },
    ];

    if !run.relics.is_full() {
        run.relics.active.push(RelicId::Geese);
    }

    run.consumables.items.clear();
    let _ = run
        .consumables
        .try_push(Consumable::Talisman(TalismanKind::Jade));
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
    asset_path::log_all_assets();
    let shop_like = matches!(s.scene.as_str(), "shop");
    if s.shop_focus.is_some() && !shop_like {
        anyhow::bail!("--shop-focus is only valid with --scene shop");
    }
    if s.journal_open.is_some() && !shop_like {
        anyhow::bail!("--journal-open is only valid with --scene shop");
    }
    if s.journal_transition.is_some() && !shop_like {
        anyhow::bail!("--journal-transition is only valid with --scene shop");
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
            let mut shop = ShopScene::new(run.run_number, &mut run);
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
        other => {
            anyhow::bail!(
                "unsupported --scene '{other}' (supported: collection, \
                yaku_journal, gameplay, gameplay_hero, pick_blind, shop, \
                main_menu_exterior, tile_select, transition_playground, \
                material_viewer, relic_unlock)"
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
    if s.shop_focus.is_some() && shop_like {
        // Cursor-mode update reassigns focus from the cursor pick
        // every tick; switch to controller mode so the focus we set
        // pre-warmup persists.
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
    run: RunState,
    anim: AnimationController,
    progress: crate::core::progression::PlayerProgress,
    active_profile: usize,
    gfx: RenderSettings,
    effect_layers: crate::effect_layers::EffectLayers,
    volumetric_tuning: VolumetricTuning,
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
                ui_scale: settings.ui_scale,
            },
            effect_layers: crate::effect_layers::EffectLayers::FULL,
            volumetric_tuning: persistence::load_tuning_override::<VolumetricTuning>(
                "VolumetricTuning",
            ),
            width,
            height,
            game_in_progress,
            tick_count: 0,
            queued_actions: Vec::new(),
            modal_overlay: None,
            input_mode_override: None,
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

    /// Set the in-memory `PlayerProgress` to a level-7, fully-explored
    /// state so the Collection grid shows real relic/yaku/boss/talisman
    /// art instead of locked placeholder dots. Only mutates the headless
    /// runner's progress; never persisted.
    fn unlock_all_for_collection_screenshot(&mut self) {
        use crate::core::boss::all_bosses;
        use crate::core::talisman::TalismanKind;
        use crate::core::yaku::YakuKind;
        self.progress.runs_completed = 25;
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
        let actions_this_tick: Vec<UiAction> = self
            .queued_actions
            .iter()
            .filter(|(t, _)| *t == self.tick_count)
            .map(|(_, a)| *a)
            .collect();
        self.tick_count += 1;
        let update_ctx = UpdateCtx {
            actions: &actions_this_tick,
            button_clicks: &[],
            progress: &self.progress,
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
            loading_done: !self.renderer.is_loading(),
            cascade_tuning: &CascadeTuning::default(),
            picked_shop_object: None,
            picked_gameplay_object: None,
            picked_collection_object: None,
            input_mode: self.input_mode_override.unwrap_or(InputMode::Cursor),
            picked_hand_tile: None,
            scroll_lines: 0.0,
            ui_scale: self.gfx.ui_scale,
            tutorial_eligible: false,
            multiple_materials: self.progress.plastic_unlocked(),
            resume_scene: persistence::ResumeScene::default(),
            transitioning: false,
            overlay_request: &mut overlay_request,
            headless: true,
        };
        let _ = self.scene.update(update_ctx);
        let ctx = DrawCtx {
            layout: &layout,
            anim: &self.anim,
            run: &self.run,
            progress: &self.progress,
            active_profile: self.active_profile,
            game_in_progress: self.game_in_progress,
            proj: self.renderer.projections(),
            picked_gameplay_object: None,
            picked_shop_object: None,
            debug_visibility: scenes::DebugVisibility {
                hide_candles: false,
                hide_blind_plaque: false,
            },
            ui_scale: self.gfx.ui_scale,
            modal_active: false,
            arrange_preview: None,
            shop_env_height_scale: crate::render::shop_glb::SHOP_ENV_HEIGHT_SCALE,
            shop_env_lighting: crate::render::shop_glb::ShopEnvLightingTune::SOURCE_DEFAULTS,
            effect_layers: self.effect_layers,
            cursor_pos: (0.0, 0.0),
            input_mode: self.input_mode_override.unwrap_or(InputMode::Cursor),
        };
        let mut frame: UiFrame = self.scene.draw_frame(ctx);

        // ── Modal overlay (relic_unlock screenshot path) ────────────
        // Mirrors the modal-staging block in `App::draw` (src/main/
        // draw.rs:476-545): tick the queue, draw it, then strip scene
        // 3D ops + override the camera/lights so the relic mesh owns
        // the depth buffer for its own pass. Kept in sync with the
        // live path so screenshots match what players actually see.
        if let Some(ref mut queue) = self.modal_overlay {
            queue.update();
            if let Some((
                modal_insts,
                modal_labels,
                modal_buttons,
                modal_relic_objects,
                modal_gradient_quads,
            )) = queue.draw(self.width as f32, self.height as f32, self.gfx.ui_scale)
            {
                let _ = modal_buttons; // headless ignores click routing
                frame.quads(modal_insts);
                frame.texts(modal_labels);
                if !modal_gradient_quads.is_empty() {
                    frame.gradient_quads(modal_gradient_quads);
                }
                if !modal_relic_objects.is_empty() {
                    use crate::render::draw_cmd::{CameraParams, DrawCmd};
                    frame.cmds.retain(|cmd| {
                        !matches!(
                            cmd,
                            DrawCmd::Object3d(_)
                                | DrawCmd::Object3dBatch(_)
                                | DrawCmd::ShowcaseTileBatch(_)
                                | DrawCmd::TileFaceQuad(_)
                                | DrawCmd::Table
                        )
                    });
                    let w = self.width as f32;
                    let h = self.height as f32;
                    frame.camera_override = Some(CameraParams {
                        eye: [0.0, -h * 3.0, 0.0],
                        target: [0.0, 0.0, 0.0],
                        up: [0.0, 0.0, 1.0],
                        fovy_deg: 20.0,
                    });
                    use crate::render::wgpu_renderer::PointLight;
                    frame.point_lights = vec![
                        PointLight {
                            pos: [w * 0.5 + w * 0.18, h * 0.5 + h * 0.45, h * 0.45],
                            radius: h * 1.6,
                            color: [1.00, 0.94, 0.82],
                            intensity: 2.0,
                        },
                        PointLight {
                            pos: [w * 0.5 - w * 0.22, h * 0.5 + h * 0.35, h * 0.30],
                            radius: h * 1.3,
                            color: [0.78, 0.86, 1.00],
                            intensity: 0.9,
                        },
                        PointLight {
                            pos: [w * 0.5, h * 0.5 - h * 0.30, h * 0.05],
                            radius: h * 1.0,
                            color: [1.00, 0.78, 0.42],
                            intensity: 1.0,
                        },
                    ];
                    frame.object3d_batch(modal_relic_objects);
                }
            }
        }

        let active_scene_key: Option<&'static str> = match &self.scene {
            Scene::Shop(_) => Some("shop"),
            Scene::Gameplay(_) => Some("gameplay"),
            Scene::Collection(_) => Some("collection"),
            Scene::MainMenuExterior(_) => Some("main_menu_exterior"),
            Scene::TutorialCampaign(_) => Some("tutorial"),
            _ => None,
        };
        self.renderer.set_active_scene(active_scene_key);
        self.renderer
            .set_committed_arrange_rotations(collect_committed_rotations(&self.scene));
        self.renderer
            .set_shop_env_height_scale(crate::render::shop_glb::SHOP_ENV_HEIGHT_SCALE);
        let sl = crate::render::shop_glb::ShopEnvLightingTune::SOURCE_DEFAULTS;
        self.renderer.set_shop_env_render_tune(
            sl.linear_exposure,
            sl.ambient_scale,
            sl.lit_mesh_gltf_punctual_scale,
        );
        let gl = crate::render::lit_mesh::GameplayLitRenderingTune::SOURCE_DEFAULTS;
        self.renderer
            .set_gameplay_lit_render_tune(gl.linear_exposure, gl.ambient_scale);
        let haze_horizon_y = frame
            .gameplay_fog_wall_horizon_y
            .unwrap_or(self.volumetric_tuning.haze_horizon_y);
        let wall_center_x = frame
            .gameplay_fog_wall_center_x
            .unwrap_or(0.5)
            .clamp(0.0, 1.0);
        let wall_half_width_uv = if frame.gameplay_fog_wall_horizon_y.is_some() {
            crate::ui::scene_layout::GAMEPLAY_FOG_WALL_HALF_WIDTH_UV
        } else {
            0.0
        };
        self.renderer.set_haze_tuning(
            self.volumetric_tuning.haze_density,
            self.volumetric_tuning.haze_color_r,
            self.volumetric_tuning.haze_color_g,
            self.volumetric_tuning.haze_color_b,
            haze_horizon_y,
            self.volumetric_tuning.haze_drift_speed,
            wall_center_x,
            wall_half_width_uv,
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
            log::info!("screenshot: waited {extra} extra ticks for asset loading");
        }
        self.renderer.queue_screenshot(output.clone());
        self.tick();
        if self.renderer.screenshot_pending() {
            anyhow::bail!(
                "headless screenshot: final tick did not consume the queued path ({})",
                output.display()
            );
        }
        log::info!("screenshot saved → {}", output.display());
        Ok(())
    }
}
