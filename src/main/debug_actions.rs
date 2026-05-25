use super::*;

use crate::core::tile_pack::TilePackKind;
use crate::debug_overlays::{HallwayDistortionDebugOverlay, SceneLookDebugOverlay};
use crate::game::engine::GameEngine;
use crate::scenes::shop::PackCelebration;
use crate::scenes::{
    ShowcasePresenter, ShowcaseScene, TileAnchorLabScene, TilePackPresenter, TixelsScene,
};
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
            DebugAction::SetGold(amount) => {
                self.run.set_run_gold_direct(amount as i32, None);
                log::debug!("Set gold to {}", amount);
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
            DebugAction::OpenDebugVisibility => {
                if self.debug.visibility_overlay.is_some() {
                    self.debug.visibility_overlay = None;
                    log::debug!("Closed debug visibility overlay");
                } else {
                    self.debug.visibility_overlay = Some(DebugVisibilityOverlay::new(
                        self.debug.hide_tiles,
                        self.debug.hide_candles,
                        self.debug.hide_chamber_plaque,
                        self.debug.hide_scoring_placard,
                        self.debug.hide_inventory,
                    ));
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
                        .map(|r| r.rain_tuning)
                        .unwrap_or_else(crate::render::rain_tuning::RainTuning::load);
                    self.debug.rain_debug_overlay = Some(
                        crate::render::rain_debug_overlay::RainDebugOverlay::new(tuning),
                    );
                    log::debug!("Opened rain debug overlay");
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
                    self.debug.hallway_distortion_debug_overlay =
                        Some(HallwayDistortionDebugOverlay::new());
                    log::debug!("Opened hallway vertex warp debug overlay");
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
            DebugAction::BlowWindGust => {
                // Inject the same UiAction that pressing `B` would push,
                // so the gameplay scene's existing wind-trigger branch
                // picks it up on the next frame.
                self.mouse_actions.push(UiAction::DebugBlowWind);
                log::debug!("Blow wind gust queued");
            }
            DebugAction::ToggleWorldAxes => {
                // Forward to the gameplay scene's existing toggle branch
                // via the same UiAction the keyboard binding used to push.
                self.mouse_actions.push(UiAction::DebugToggleAxes);
                log::debug!("World-axes overlay toggled");
            }
            DebugAction::RerollShop => match &mut self.scene {
                Scene::Shop(s) => {
                    s.debug_reroll(&self.run);
                    log::debug!("Rerolled shop stock (free)");
                }
                _ => log::warn!("Reroll Shop ignored — not in shop scene"),
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
                        Scene::MainMenuExterior(_) => "MainMenuExterior",
                        Scene::TileSelect(_) => "TileSelect",
                        Scene::ProfileSelect(_) => "ProfileSelect",
                        Scene::Shop(_) => "Shop",
                        Scene::Showcase(_) => "Showcase",
                        Scene::PickChamber(_) => "PickChamber",
                        Scene::Staircase(_) => "Staircase",
                        Scene::Gameplay(_) => "Gameplay",
                        Scene::GameOver(_) => "GameOver",
                        Scene::Guide(_) => "Guide",
                        Scene::MaterialViewer(_) => "MaterialViewer",
                        Scene::TileAnchorLab(_) => "TileAnchorLab",
                        Scene::Options(_) => "Options",
                        Scene::Credits(_) => "Credits",
                        Scene::Collection(_) => "Collection",
                        Scene::TutorialCampaign(_) => "TutorialCampaign",
                        Scene::TutorialSummary(_) => "TutorialSummary",
                        Scene::TransitionPlayground(_) => "TransitionPlayground",
                        Scene::RumbleLab(_) => "RumbleLab",
                        Scene::Tixels(_) => "Tixels",
                        Scene::YakuJournal(_) => "YakuJournal",
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
            DebugAction::OpenRumbleLab => {
                self.overlay_stack
                    .push(Scene::RumbleLab(RumbleLabScene::new(true)));
                log::debug!("Opened rumble lab");
            }
            DebugAction::OpenTileAnchorLab => {
                self.overlay_stack
                    .push(Scene::TileAnchorLab(TileAnchorLabScene::new(true)));
                log::debug!("Opened tile anchor lab");
            }
            DebugAction::OpenTixels => {
                self.overlay_stack
                    .push(Scene::Tixels(TixelsScene::new(true)));
                log::debug!("Opened tixels scene");
            }
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
                self.pending_scene = Some(Scene::GameOver(GameOverScene::victory(&self.run)));
                self.transition_alpha = 1.0;
                log::debug!("Showing victory screen");
            }
            DebugAction::ShowDefeatScreen => {
                while self.modals.dismiss() {}
                let reason = crate::game::event_bus::GameOverReason::OutOfPlays;
                let snap = crate::core::memorial_talisman::snapshot_from_run(
                    &self.run.defeat_journal,
                    reason,
                    &self.run,
                );
                self.run.defeat_memorial_kind =
                    Some(crate::core::memorial_talisman::select_memorial(&snap));
                self.pending_scene = Some(Scene::GameOver(GameOverScene::new(&self.run, reason)));
                self.transition_alpha = 1.0;
                log::debug!("Showing defeat screen");
            }
        }
    }
}
