use super::*;

use crate::core::tile_pack::TilePackKind;
use crate::debug_overlays::ShopEnvDebugOverlay;
use crate::game::engine::GameEngine;
use crate::scenes::{ShowcasePresenter, ShowcaseScene, TilePackPresenter};
use crate::scenes::reload_scene_layout_from_disk;
use crate::scenes::shop::PackCelebration;
use crate::ui::scene_layout::clear_saved_layout_files;
use rand::RngExt;

impl App {
    pub(super) fn handle_debug_action(&mut self, action: DebugAction) {
        match action {
            DebugAction::SetLevel(level) => {
                // Set runs_completed to the minimum value for this level.
                // Must match the curve in `PlayerProgress::current_level`.
                let runs = match level {
                    1 => 0,
                    2 => 1,
                    3 => 2,
                    4 => 3,
                    5 => 4,
                    6 => 5,
                    7 => 7,
                    8 => 9,
                    9 => 11,
                    10 => 13,
                    11 => 15,
                    12 => 17,
                    13 => 19,
                    14 => 21,
                    _ => 0,
                };
                self.progress.runs_completed = runs;
                let level_up = self.progress.check_level_up();
                self.run.apply_progression(&self.progress);
                let _ = persistence::save_profile(self.active_profile, &self.progress);
                log::debug!("Set player level to {} (runs_completed={})", level, runs);
                if let Some(result) = level_up {
                    let ww = self.last_drawable_px.width as f32;
                    let wh = self.last_drawable_px.height as f32;
                    if let Some(modal) = main_draw::build_level_up_modal(&result, ww, wh) {
                        self.modals.push(modal);
                        self.audio.play_sfx(audio::SfxId::LevelUp);
                    }
                }
            }
            DebugAction::SetGold(amount) => {
                self.run.gold = amount as i32;
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
                        self.debug.hide_blind_plaque,
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
            DebugAction::OpenShopEnvDebug => {
                if self.debug.shop_env_debug_overlay.is_none() {
                    self.debug.shop_env_debug_overlay = Some(ShopEnvDebugOverlay::new(
                        self.debug.room_gltf_height_scale,
                        self.debug.shop_env_lighting,
                    ));
                    log::debug!("Opened shop env & lighting debug overlay");
                }
            }
            DebugAction::OpenVolumetricDebug => {
                if self.debug.volumetric_debug_overlay.is_none() {
                    self.debug.volumetric_debug_overlay =
                        Some(VolumetricDebugOverlay::new(&self.volumetric_tuning));
                    log::debug!("Opened volumetric debug overlay");
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
            DebugAction::ArmObjectHitTest => {
                self.debug.object_hit_test_armed = !self.debug.object_hit_test_armed;
                if self.debug.object_hit_test_armed {
                    log::debug!(
                        "Object hit test ARMED — click anywhere in the world to identify the object under the cursor"
                    );
                } else {
                    log::debug!("Object hit test disarmed");
                }
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
                    let inventory = s.tile_pack_celeb_inventory_counts(&self.run);
                    self.overlay_stack.push(Scene::Showcase(ShowcaseScene::new(
                        ShowcasePresenter::TilePack(TilePackPresenter::new(celeb, inventory)),
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
                        Scene::PickBlind(_) => "PickBlind",
                        Scene::Gameplay(_) => "Gameplay",
                        Scene::GameOver(_) => "GameOver",
                        Scene::MeldGuide(_) => "MeldGuide",
                        Scene::MaterialViewer(_) => "MaterialViewer",
                        Scene::Options(_) => "Options",
                        Scene::Collection(_) => "Collection",
                        Scene::TutorialRecap(_) => "TutorialRecap",
                        Scene::TutorialCampaign(_) => "TutorialCampaign",
                        Scene::TutorialSummary(_) => "TutorialSummary",
                        Scene::TileLiteracy(_) => "TileLiteracy",
                        Scene::TransitionPlayground(_) => "TransitionPlayground",
                        Scene::RumbleLab(_) => "RumbleLab",
                        Scene::YakuJournal(_) => "YakuJournal",
                    };
                    log::warn!("Demo Cascade ignored — current scene is {name}");
                }
            }
            DebugAction::SetBoss(kind) => {
                // Replace the current ante's boss and rebuild the resolved
                // effect. resolve_upcoming_boss handles both static (wraps
                // BossDef::effect) and reactive (calls on_reveal) cases —
                // and zeros tax_collector_cost so leftover state from a
                // prior boss doesn't leak through.
                self.run.boss.upcoming = Some(kind);
                self.run.resolve_upcoming_boss();
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
                self.pending_scene = Some(Scene::GameOver(GameOverScene::new(
                    &self.run,
                    crate::game::event_bus::GameOverReason::OutOfPlays,
                )));
                self.transition_alpha = 1.0;
                log::debug!("Showing defeat screen");
            }
            DebugAction::ToggleArrangeMode => {
                if self.debug.arrange_mode.is_some() {
                    self.debug.arrange_mode = None;
                    log::debug!("Arrange mode DEACTIVATED");
                } else {
                    self.debug.arrange_mode = Some(None);
                    log::debug!(
                        "Arrange mode ARMED — click an object OR press Tab to browse the hierarchy"
                    );
                }
            }
            DebugAction::ClearSavedSceneLayouts => match clear_saved_layout_files() {
                Ok(n) => {
                    reload_scene_layout_from_disk(&mut self.scene);
                    for overlay in &mut self.overlay_stack {
                        reload_scene_layout_from_disk(overlay);
                    }
                    log::info!(
                        "[Layout] Removed {n} saved layout file(s); reloaded defaults in active scenes"
                    );
                }
                Err(e) => log::error!("[Layout] Failed to clear saved layouts: {e:#}"),
            },
        }
    }
}
