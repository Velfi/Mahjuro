use super::*;

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
/// pairs naturally with `RedDragonRage` / `WhiteSilence` relics.
///
/// Marks every tile as selected so the next `UiAction::ScoreHand` plays
/// the full hand. Also stocks `relics.active` with four visually
/// distinctive relics (dragon trio + GoldFurnace) so the relic strip
/// reads at thumbnail size.
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
        RelicId::WhiteSilence,
        RelicId::GreenLuck,
        RelicId::GoldFurnace,
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
    let boss_override = s.boss.as_deref().map(parse_boss_slug).transpose()?;
    let mut run = RunState::new_demo();
    if let Some(kind) = boss_override {
        force_boss_blind(&mut run, kind);
    }
    let mut hero_play = false;
    let mut unlock_yaku = false;
    let (scene, game_in_progress) = match s.scene.as_str() {
        "collection" => (Scene::Collection(scenes::CollectionScene::new()), false),
        "yaku_journal" => {
            unlock_yaku = true;
            (Scene::YakuJournal(scenes::YakuJournalScene::new()), false)
        }
        "gameplay" => (Scene::Gameplay(GameplayScene::new()), true),
        "gameplay_hero" => {
            setup_hero_state(&mut run);
            hero_play = true;
            (Scene::Gameplay(GameplayScene::new()), true)
        }
        "pick_blind" => (Scene::PickBlind(scenes::PickBlindScene::new()), true),
        "shop" => {
            setup_shop_state(&mut run);
            (Scene::Shop(ShopScene::new(run.run_number, &mut run)), true)
        }
        "start_screen" => (Scene::StartScreen(scenes::StartScreenScene::new()), false),
        "tile_select" => (Scene::TileSelect(scenes::TileSelectScene::new()), false),
        "transition_playground" => (
            Scene::TransitionPlayground(scenes::TransitionPlaygroundScene::new(false)),
            false,
        ),
        other => {
            anyhow::bail!(
                "unsupported --scene '{other}' (supported: collection, \
                yaku_journal, gameplay, gameplay_hero, pick_blind, shop, \
                start_screen, tile_select, transition_playground)"
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
    if hero_play {
        // Fire ScoreHand on tick 2 (after one warmup tick lets layouts/loads
        // settle), then warmup_frames ride out the cascade so the captured
        // frame lands mid-animation.
        app.queue_action_on_tick(2, UiAction::ScoreHand);
    }
    if unlock_yaku {
        app.unlock_all_yaku_for_screenshot();
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
    shop_smoke_tuning: ShopSmokeTuning,
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
                smoke_quality: settings.smoke_quality,
                smoke_amount: settings.smoke_amount,
                effects_quality: settings.effects_quality,
                tile_preset: settings.tile_preset,
                tile_material: settings.tile_material,
                tileset_name: settings.tileset_name.clone(),
                gamma: settings.gamma,
                shadows_enabled: settings.shadows_enabled,
                ssr_enabled: settings.ssr_enabled,
                hdr_enabled: false,
                ui_scale: settings.ui_scale,
            },
            shop_smoke_tuning: persistence::load_tuning_override::<ShopSmokeTuning>(
                "ShopSmokeTuning",
            ),
            volumetric_tuning: persistence::load_tuning_override::<VolumetricTuning>(
                "VolumetricTuning",
            ),
            width,
            height,
            game_in_progress,
            tick_count: 0,
            queued_actions: Vec::new(),
        })
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
            input_mode: InputMode::Cursor,
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
            shop_smoke_tuning: &self.shop_smoke_tuning,
        };
        let frame: UiFrame = self.scene.draw_frame(ctx);

        let active_scene_key: Option<&'static str> = match &self.scene {
            Scene::Shop(_) => Some("shop"),
            Scene::Gameplay(_) => Some("gameplay"),
            Scene::Collection(_) => Some("collection"),
            Scene::StartScreen(_) => Some("start_screen"),
            Scene::TutorialCampaign(_) => Some("tutorial"),
            _ => None,
        };
        self.renderer.set_active_scene(active_scene_key);
        self.renderer
            .set_committed_arrange_rotations(collect_committed_rotations(&self.scene));
        self.renderer
            .set_dust_strength(self.volumetric_tuning.dust_strength);
        self.renderer.set_haze_tuning(
            self.volumetric_tuning.haze_density,
            self.volumetric_tuning.haze_color_r,
            self.volumetric_tuning.haze_color_g,
            self.volumetric_tuning.haze_color_b,
            self.volumetric_tuning.haze_horizon_y,
            self.volumetric_tuning.haze_drift_speed,
        );

        let active_material = frame
            .tile_material_override
            .unwrap_or(self.gfx.tile_material);

        if let Err(e) = self.renderer.render(
            &frame,
            crate::render::wgpu_renderer::RenderSettings {
                smoke_quality: self.gfx.smoke_quality,
                smoke_amount: self.gfx.smoke_amount,
                effects_quality: self.gfx.effects_quality,
                tile_preset: self.gfx.tile_preset,
                tile_material: active_material,
                tileset_name: self.gfx.tileset_name.clone(),
                draw_settle_speed: 8.0,
                sort_settle_speed: 10.0,
                gamma: self.gfx.gamma,
                shadows_enabled: self.gfx.shadows_enabled,
                ssr_enabled: self.gfx.ssr_enabled,
            },
        ) {
            log::error!("headless render: {e:?}");
        }
    }

    /// Render `warmup_frames + 1` frames, queue a screenshot on the last
    /// frame, and return. The PNG is written synchronously during the
    /// final `render()` call (see `WgpuRenderer::render` end-of-frame).
    fn run_screenshot(mut self, output: PathBuf, warmup_frames: u32) -> anyhow::Result<()> {
        for _ in 0..warmup_frames {
            self.tick();
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
