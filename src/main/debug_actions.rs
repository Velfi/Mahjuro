use super::*;

use crate::core::tile_pack::TilePackKind;
use crate::debug_overlays::{HallwayDistortionDebugOverlay, SceneLookDebugOverlay};
use crate::game::engine::GameEngine;
use crate::scene_transition::SceneTag;
use crate::scenes::shop::PackCelebration;
use crate::scenes::{
    ButtonAabbLabScene, CascadeLabScene, RollerLabScene, ShadowAoLabScene,
    ShowcasePresenter, ShowcaseScene, TileAnchorLabScene, TilePackPresenter,
};
use crate::trailer_mode::TrailerMode;
use rand::RngExt;

impl App {
    pub(super) fn handle_debug_action(&mut self, action: DebugAction) {
        match action {
            DebugAction::SetLevel(level) => {
                // Set progression points to the minimum required for this level.
                let clamped_level = level.clamp(1, crate::core::progression::MAX_PROGRESS_LEVEL);
                let points =
                    crate::core::progression::PlayerProgress::min_points_for_level(clamped_level);
                self.progress.level_progress_points = points;
                let level_up = self.progress.check_level_up();
                self.run.apply_progression(&self.progress);
                let _ = persistence::save_profile(self.active_profile, &self.progress);
                log::debug!(
                    "Set player depth to {} (level_progress_points={})",
                    crate::core::progression::meta_depth_roman(clamped_level),
                    points
                );
                if let Some(result) = level_up {
                    let ww = self.last_drawable_px.width as f32;
                    let wh = self.last_drawable_px.height as f32;
                    if let Some(modal) = main_draw::build_level_up_modal(&result, ww, wh) {
                        self.modals.push(modal);
                        self.audio.play_sfx(audio::SfxId::LevelUp);
                    }
                }
            }
            DebugAction::UnlockAllTransformationsAndSuccessors => {
                self.progress.cheat_unlock_all_transformation_chains_meta();
                self.run.cheat_force_all_transform_extinctions();
                self.run.apply_progression(&self.progress);
                let _ = persistence::save_profile(self.active_profile, &self.progress);
                log::debug!("Unlocked all transformation chains (meta + run extinction flags)");
            }
            DebugAction::SeedChronicleFromBotRuns(count) => {
                let n = count as usize;
                log::info!("Seeding chronicle from {n} bot run(s)…");
                let added = crate::bot::seed_progress_from_bot_runs(&mut self.progress, n);
                self.run.apply_progression(&self.progress);
                let _ = persistence::save_profile(self.active_profile, &self.progress);
                log::info!(
                    "Chronicle seed complete: {added} run(s) in history ({} total serious)",
                    self.progress
                        .run_history
                        .iter()
                        .filter(|r| !r.tutorial_run)
                        .count()
                );
            }
            DebugAction::RevealKokushiMusou => {
                self.progress.cheat_reveal_kokushi_musou();
                self.run.apply_progression(&self.progress);
                let _ = persistence::save_profile(self.active_profile, &self.progress);
                log::debug!(
                    "Revealed Kokushi Musō (available_yaku + journal/guide + Qilin ribbon)"
                );
            }
            DebugAction::UnlockAllTilesetsAndSeasons => {
                self.progress.cheat_unlock_all_tilesets_and_seasons();
                self.run.apply_progression(&self.progress);
                let _ = persistence::save_profile(self.active_profile, &self.progress);
                log::debug!("Unlocked all tile materials and seasons");
            }
            DebugAction::SetYen(amount) => {
                self.run.set_run_yen_direct(amount as i32, None);
                log::debug!("Set yen to {}", amount);
            }
            DebugAction::AddRelic(relic_id) => {
                if !self.run.relics.active.contains(&relic_id) {
                    if self.run.relics.is_full() {
                        // Expand capacity to fit.
                        self.run.relics.max_slots += 1;
                    }
                    self.run.relics.active.push(relic_id);
                    self.run.recompute_capacities();
                    log::debug!("Added relic {:?}", relic_id);
                } else {
                    log::debug!("Relic {:?} already active", relic_id);
                }
            }
            DebugAction::ClearRelics => {
                self.run.relics.active.clear();
                log::debug!("Cleared all relics");
            }
            DebugAction::AddTalisman(kind) => {
                use crate::core::consumable::Consumable;
                if self.run.consumables.is_full() {
                    self.run.consumables.capacity += 1;
                }
                self.run.consumables.try_push(Consumable::Talisman(kind));
                log::debug!("Added talisman {:?}", kind);
            }
            DebugAction::AddZodiac(kind) => {
                use crate::core::consumable::Consumable;
                if self.run.consumables.is_full() {
                    self.run.consumables.capacity += 1;
                }
                self.run.consumables.try_push(Consumable::Zodiac(kind));
                log::debug!("Added zodiac {:?}", kind);
            }
            DebugAction::ClearConsumables => {
                self.run.consumables.items.clear();
                log::debug!("Cleared all consumables");
            }
            DebugAction::ToggleShowFps => {
                self.debug.show_fps = !self.debug.show_fps;
                log::debug!("Show FPS: {}", self.debug.show_fps);
            }
            DebugAction::ToggleHide2dUi => {
                self.debug.hide_2d_ui = !self.debug.hide_2d_ui;
                log::debug!("Hide 2D UI: {}", self.debug.hide_2d_ui);
            }
            DebugAction::OpenDebugVisibility => {
                if self.debug.visibility_overlay.is_some() {
                    self.debug.visibility_overlay = None;
                    log::debug!("Closed debug visibility overlay");
                } else {
                    self.debug.visibility_overlay =
                        Some(DebugVisibilityOverlay::new(self.debug.visibility));
                    log::debug!("Opened debug visibility overlay");
                }
            }
            DebugAction::OpenTuning => {
                if self.debug.tuning_overlay.is_none() {
                    self.debug.tuning_overlay = Some(TuningOverlay::new(&self.cascade_tuning));
                    log::debug!("Opened cascade tuning overlay");
                }
            }
            DebugAction::OpenSfxTest => {
                if self.debug.sfx_test_overlay.is_none() {
                    self.debug.sfx_test_overlay = Some(SfxTestOverlay::new());
                    log::debug!("Opened SFX test overlay");
                }
            }
            DebugAction::OpenCameraDebug => {
                if self.debug.camera_debug_overlay.is_none() {
                    let seed = self.debug.last_effective_camera;
                    self.debug.camera_debug_overlay = Some(CameraDebugOverlay::new(&seed));
                    log::debug!("Opened camera debug overlay");
                }
            }
            DebugAction::OpenRainDebug => {
                if self.debug.rain_debug_overlay.is_none() {
                    let tuning = self
                        .renderer
                        .as_ref()
                        .map(|r| r.main_menu_effects)
                        .unwrap_or_else(
                            crate::render::main_menu_effects_tuning::MainMenuEffectsTuning::load,
                        );
                    let moon_phase_debug = self
                        .renderer
                        .as_ref()
                        .map(|r| r.main_menu_moon_phase_debug)
                        .unwrap_or(self.debug.main_menu_moon_phase_debug);
                    self.debug.rain_debug_overlay = Some(
                        crate::render::main_menu_effects_debug_overlay::MainMenuEffectsDebugOverlay::new(
                            tuning,
                            self.debug.main_menu_pride_rainbow_debug,
                            moon_phase_debug,
                        ),
                    );
                    log::debug!("Opened main menu effects debug overlay");
                }
            }
            DebugAction::OpenSceneLookDebug => {
                if self.debug.scene_look_debug_overlay.is_none() {
                    let keys = crate::game::scene_look_tuning::overlay_scene_keys();
                    let active = self.active_scene_key_for_renderer();
                    let scene_index = active
                        .and_then(|k| keys.iter().position(|&x| x == k))
                        .unwrap_or(0);
                    let look = self.scene_look.resolve(active);
                    self.debug.scene_look_debug_overlay =
                        Some(SceneLookDebugOverlay::new(scene_index, look));
                    log::debug!(
                        "Opened scene look debug overlay (scene: {})",
                        keys[scene_index]
                    );
                }
            }
            DebugAction::OpenHallwayHallFxDebug => {
                if self.debug.hallway_distortion_debug_overlay.is_some() {
                    self.debug.hallway_distortion_debug_overlay = None;
                    log::debug!("Closed hallway vertex warp debug overlay");
                } else {
                    let run = &self.run;
                    self.debug.hallway_distortion_debug_overlay =
                        Some(HallwayDistortionDebugOverlay::from_run(
                            run.chronicle.seed,
                            run.run_number,
                            run.wing,
                        ));
                    log::debug!("Opened hallway vertex warp debug overlay");
                }
            }
            DebugAction::OpenFlameDebug => {
                if self.debug.flame_debug_overlay.is_some() {
                    self.debug.flame_debug_overlay = None;
                    log::debug!("Closed flame debug overlay");
                } else {
                    let tuning = self
                        .renderer
                        .as_ref()
                        .map(|r| r.flame_tuning)
                        .unwrap_or_else(crate::render::flame_tuning::FlameTuning::load);
                    self.debug.flame_debug_overlay = Some(
                        crate::render::flame_debug_overlay::FlameDebugOverlay::new(tuning),
                    );
                    log::debug!("Opened flame debug overlay");
                }
            }
            DebugAction::ProfileGpu => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.start_gpu_profile(100);
                    log::debug!("GPU profile capture queued (100 frames)");
                } else {
                    log::warn!("Cannot start GPU profile: renderer not initialised");
                }
            }
            DebugAction::ProfileCpu => {
                self.cpu_profiler.start(100);
                log::debug!("CPU profile capture queued (100 frames)");
            }
            DebugAction::ToggleWorldAxes => {
                // Forward to the gameplay scene's existing toggle branch
                // via the same UiAction the keyboard binding used to push.
                self.mouse_actions.push(UiAction::DebugToggleAxes);
                log::debug!("World-axes overlay toggled");
            }
            DebugAction::RestockShop => match &mut self.scene {
                Scene::Shop(s) => {
                    s.debug_restock(&self.run);
                    log::debug!("Restocked shop stock (free)");
                }
                _ => log::warn!("Restock Shop ignored — not in shop scene"),
            },
            DebugAction::OpenPack => match &mut self.scene {
                Scene::Shop(s) => {
                    let kinds = TilePackKind::all();
                    let kind = kinds[rand::rng().random_range(0..kinds.len())];
                    let tiles = GameEngine::debug_add_pack(&mut self.run, kind);
                    let celeb = PackCelebration::new(tiles, kind.name(), kind);
                    let _ = s;
                    self.overlay_stack.push(Scene::Showcase(ShowcaseScene::new(
                        ShowcasePresenter::TilePack(Box::new(TilePackPresenter::new(celeb))),
                    )));
                    log::debug!("Opened tile pack celebration overlay");
                }
                _ => log::warn!("Open Pack ignored — not in shop scene"),
            },
            DebugAction::DemoCascade => {
                if let Scene::Gameplay(gp) = &mut self.scene {
                    let size = self.last_drawable_px;
                    let layout = self
                        .layout_engine
                        .solve(size.width as f32, size.height as f32);
                    gp.debug_demo_cascade(&layout, &self.run);
                } else {
                    let name = match &self.scene {
                        Scene::Splash(_) => "Splash",
                        Scene::MainMenu(_) => "MainMenu",
                        Scene::TileSelect(_) => "TileSelect",
                        Scene::ProfileSelect(_) => "ProfileSelect",
                        Scene::Shop(_) => "Shop",
                        Scene::Showcase(_) => "Showcase",
                        Scene::Hallway(_) => "Hallway",
                        Scene::Stairway(_) => "Stairway",
                        Scene::Gameplay(_) => "Gameplay",
                        Scene::Victory(_) => "Victory",
                        Scene::Defeat(_) => "Defeat",
                        Scene::Guide(_) => "Guide",
                        Scene::MaterialViewer(_) => "MaterialViewer",
                        Scene::TileAnchorLab(_) => "TileAnchorLab",
                        Scene::ButtonAabbLab(_) => "ButtonAabbLab",
                        Scene::Options(_) => "Options",
                        Scene::Credits(_) => "Credits",
                        Scene::Archive(_) => "Collection",
                        Scene::TutorialCampaign(_) => "TutorialCampaign",
                        Scene::TutorialSummary(_) => "TutorialSummary",
                        Scene::TransitionPlayground(_) => "TransitionPlayground",
                        Scene::AnimationLab(_) => "AnimationLab",
                        Scene::RollerLab(_) => "RollerLab",
                        Scene::CascadeLab(_) => "CascadeLab",
                        Scene::RumbleLab(_) => "RumbleLab",
                        Scene::ShadowAoLab(_) => "ShadowAoLab",
                        Scene::YakuJournal(_) => "YakuJournal",
                        Scene::WallLedger(_) => "WallLedger",
                    };
                    log::warn!("Demo Cascade ignored — current scene is {name}");
                }
            }
            DebugAction::SetOrdeal(kind) => {
                // Replace the current ante's boss and rebuild the resolved
                // effect. resolve_upcoming_ordeal handles both static (wraps
                // OrdealDef::effect) and reactive (calls on_reveal) cases —
                // and zeros tax_collector_cost so leftover state from a
                // prior boss doesn't leak through.
                self.run.ordeal.upcoming = Some(kind);
                self.run.resolve_upcoming_ordeal();
                log::debug!("Set boss to {}", kind.name());
            }
            DebugAction::SetDora(suit, rank) => {
                self.run.wall.set_sole_dora(suit, rank);
                let name = crate::core::tile::Tile::new(suit, rank, 0).full_name();
                log::debug!("Set dora to {name}");
            }
            DebugAction::TestOverlay => {
                let modal = Modal::new(
                    "Test Overlay",
                    "This is a blank test modal.\nClick anywhere or press Enter to continue.",
                    ModalTheme::Info,
                );
                self.modals.push(modal);
                log::debug!("Spawned test overlay modal");
            }
            DebugAction::OpenMaterialViewer => {
                self.overlay_stack
                    .push(Scene::MaterialViewer(MaterialViewerScene::new(true)));
                log::debug!("Opened material viewer");
            }
            DebugAction::OpenTransitionPlayground => {
                self.overlay_stack.push(Scene::TransitionPlayground(
                    TransitionPlaygroundScene::new(true),
                ));
                log::debug!("Opened transition playground");
            }
            DebugAction::OpenAnimationLab => {
                self.overlay_stack
                    .push(Scene::AnimationLab(AnimationLabScene::new(true)));
                log::debug!("Opened animation lab");
            }
            DebugAction::OpenRollerLab => {
                self.overlay_stack
                    .push(Scene::RollerLab(RollerLabScene::new(true)));
                log::debug!("Opened roller lab");
            }
            DebugAction::OpenCascadeLab => {
                self.overlay_stack
                    .push(Scene::CascadeLab(Box::new(CascadeLabScene::new(
                        true,
                        self.cascade_tuning.clone(),
                    ))));
                log::debug!("Opened cascade lab");
            }
            DebugAction::OpenRumbleLab => {
                self.overlay_stack
                    .push(Scene::RumbleLab(RumbleLabScene::new(true)));
                log::debug!("Opened rumble lab");
            }
            DebugAction::OpenShadowAoLab => {
                self.overlay_stack
                    .push(Scene::ShadowAoLab(ShadowAoLabScene::new(true)));
                log::debug!("Opened shadow & AO lab");
            }
            DebugAction::OpenTileAnchorLab => {
                self.overlay_stack
                    .push(Scene::TileAnchorLab(TileAnchorLabScene::new(true)));
                log::debug!("Opened tile anchor lab");
            }
            DebugAction::OpenButtonAabbLab => {
                self.overlay_stack
                    .push(Scene::ButtonAabbLab(ButtonAabbLabScene::new(true)));
                log::debug!("Opened button AABB lab");
            }
            #[cfg(target_os = "macos")]
            DebugAction::OpenAbout => {
                let body = format!(
                    "Mahjuro v{}\nA candlelit mahjong roguelike prototype.\n\nLocal icon asset: icon.png",
                    env!("CARGO_PKG_VERSION")
                );
                self.modals
                    .push(Modal::new("About Mahjuro", body, ModalTheme::Info));
                log::debug!("Opened About modal");
            }
            DebugAction::ShowVictoryScreen => {
                while self.modals.dismiss() {}
                self.begin_scene_replace(
                    crate::scenes::SceneIntent::Victory,
                    SceneTag::from(&self.scene),
                    crate::scene_transition::PendingSceneDestination::default(),
                );
                log::debug!("Showing victory screen");
            }
            DebugAction::ShowDefeatScreen => {
                while self.modals.dismiss() {}
                let reason = crate::game::event_bus::GameOverReason::OutOfPlays;
                let snap = crate::game::memorial_run::snapshot_from_run(
                    &self.run.defeat_journal,
                    reason,
                    &self.run,
                );
                self.run.defeat_memorial_kind =
                    Some(crate::core::memorial_talisman::select_memorial(&snap));
                self.begin_scene_replace(
                    crate::scenes::SceneIntent::Defeat(reason),
                    SceneTag::from(&self.scene),
                    crate::scene_transition::PendingSceneDestination::default(),
                );
                log::debug!("Showing defeat screen");
            }
            DebugAction::TriggerTrailerMode => {
                let w = self.last_drawable_px.width as f32;
                let h = self.last_drawable_px.height as f32;
                let scene_look = self
                    .scene_look
                    .resolve(self.active_scene_key_for_renderer());
                match TrailerMode::try_start(
                    &self.scene,
                    &self.run,
                    w,
                    h,
                    scene_look.room_gltf_height_scale,
                ) {
                    Some(tm) => {
                        self.debug.trailer_mode = Some(tm);
                        log::debug!("Started trailer mode for current scene");
                    }
                    None => log::warn!("Trailer mode not available for the current scene"),
                }
            }
        }
    }
}
