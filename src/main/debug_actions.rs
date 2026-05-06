use super::*;

use crate::core::tile_pack::TilePackKind;
use crate::debug_overlays::ShopEnvDebugOverlay;
use crate::game::engine::GameEngine;
use crate::scenes::shop::PackCelebration;
use crate::scenes::TilePackCelebrationScene;
use rand::RngExt;

impl App {
    pub(super) fn handle_debug_action(&mut self, action: DebugAction) {
        match action {
            DebugAction::SetLevel(level) => {
                // Set runs_completed to the minimum value for this level.
                let runs = match level {
                    1 => 0,
                    2 => 1,
                    3 => 3,
                    4 => 6,
                    5 => 10,
                    6 => 15,
                    7 => 20,
                    _ => 0,
                };
                self.progress.runs_completed = runs;
                let level_up = self.progress.check_level_up();
                self.run.apply_progression(&self.progress);
                let _ = persistence::save_profile(self.active_profile, &self.progress);
                log::info!(
                    "[Debug] Set player level to {} (runs_completed={})",
                    level,
                    runs
                );
                if let Some(result) = level_up {
                    let win_size = self
                        .window
                        .as_ref()
                        .map(|w| w.inner_size())
                        .unwrap_or(PhysicalSize::new(800, 600));
                    let ww = win_size.width as f32;
                    let wh = win_size.height as f32;
                    if let Some(modal) = main_draw::build_level_up_modal(&result, ww, wh) {
                        self.modals.push(modal);
                        self.audio.play_sfx(audio::SfxId::LevelUp);
                    }
                }
            }
            DebugAction::SetGold(amount) => {
                self.run.gold = amount as i32;
                log::info!("[Debug] Set gold to {}", amount);
            }
            DebugAction::AddRelic(relic_id) => {
                if !self.run.relics.active.contains(&relic_id) {
                    if self.run.relics.is_full() {
                        // Expand capacity to fit.
                        self.run.relics.max_slots += 1;
                    }
                    self.run.relics.active.push(relic_id);
                    self.run.recompute_capacities();
                    log::info!("[Debug] Added relic {:?}", relic_id);
                } else {
                    log::info!("[Debug] Relic {:?} already active", relic_id);
                }
            }
            DebugAction::ClearRelics => {
                self.run.relics.active.clear();
                log::info!("[Debug] Cleared all relics");
            }
            DebugAction::AddTalisman(kind) => {
                use crate::core::consumable::Consumable;
                if self.run.consumables.is_full() {
                    self.run.consumables.capacity += 1;
                }
                self.run.consumables.try_push(Consumable::Talisman(kind));
                log::info!("[Debug] Added talisman {:?}", kind);
            }
            DebugAction::AddZodiac(kind) => {
                use crate::core::consumable::Consumable;
                if self.run.consumables.is_full() {
                    self.run.consumables.capacity += 1;
                }
                self.run.consumables.try_push(Consumable::Zodiac(kind));
                log::info!("[Debug] Added zodiac {:?}", kind);
            }
            DebugAction::ClearConsumables => {
                self.run.consumables.items.clear();
                log::info!("[Debug] Cleared all consumables");
            }
            DebugAction::ToggleShowFps => {
                self.debug.show_fps = !self.debug.show_fps;
                log::info!("[Debug] Show FPS: {}", self.debug.show_fps);
            }
            DebugAction::OpenDebugVisibility => {
                if self.debug.visibility_overlay.is_some() {
                    self.debug.visibility_overlay = None;
                    log::info!("[Debug] Closed debug visibility overlay");
                } else {
                    self.debug.visibility_overlay = Some(DebugVisibilityOverlay::new(
                        self.debug.hide_tiles,
                        self.debug.hide_candles,
                        self.debug.hide_blind_plaque,
                        self.debug.hide_scoring_placard,
                        self.debug.hide_inventory,
                    ));
                    log::info!("[Debug] Opened debug visibility overlay");
                }
            }
            DebugAction::OpenTuning => {
                if self.debug.tuning_overlay.is_none() {
                    self.debug.tuning_overlay = Some(TuningOverlay::new(&self.cascade_tuning));
                    log::info!("[Debug] Opened cascade tuning overlay");
                }
            }
            DebugAction::OpenSfxTest => {
                if self.debug.sfx_test_overlay.is_none() {
                    self.debug.sfx_test_overlay = Some(SfxTestOverlay::new());
                    log::info!("[Debug] Opened SFX test overlay");
                }
            }
            DebugAction::OpenCameraDebug => {
                if self.debug.camera_debug_overlay.is_none() {
                    let seed = self.debug.last_effective_camera;
                    self.debug.camera_debug_overlay = Some(CameraDebugOverlay::new(&seed));
                    log::info!("[Debug] Opened camera debug overlay");
                }
            }
            DebugAction::OpenShopEnvDebug => {
                if self.debug.shop_env_debug_overlay.is_none() {
                    self.debug.shop_env_debug_overlay = Some(ShopEnvDebugOverlay::new(
                        self.debug.shop_env_height_scale,
                        self.debug.shop_env_lighting,
                    ));
                    log::info!("[Debug] Opened shop env & lighting debug overlay");
                }
            }
            DebugAction::OpenVolumetricDebug => {
                if self.debug.volumetric_debug_overlay.is_none() {
                    self.debug.volumetric_debug_overlay =
                        Some(VolumetricDebugOverlay::new(&self.volumetric_tuning));
                    log::info!("[Debug] Opened volumetric debug overlay");
                }
            }
            DebugAction::ProfileGpu => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.start_gpu_profile(100);
                    log::info!("[Debug] GPU profile capture queued (100 frames)");
                } else {
                    log::warn!("[Debug] Cannot start GPU profile: renderer not initialised");
                }
            }
            DebugAction::BlowWindGust => {
                // Inject the same UiAction that pressing `B` would push,
                // so the gameplay scene's existing wind-trigger branch
                // picks it up on the next frame.
                self.mouse_actions.push(UiAction::DebugBlowWind);
                log::info!("[Debug] Blow wind gust queued");
            }
            DebugAction::ToggleWorldAxes => {
                // Forward to the gameplay scene's existing toggle branch
                // via the same UiAction the keyboard binding used to push.
                self.mouse_actions.push(UiAction::DebugToggleAxes);
                log::info!("[Debug] World-axes overlay toggled");
            }
            DebugAction::ArmObjectHitTest => {
                self.debug.object_hit_test_armed = !self.debug.object_hit_test_armed;
                if self.debug.object_hit_test_armed {
                    log::info!(
                        "[Debug] Object hit test ARMED — click anywhere in the world to identify the object under the cursor"
                    );
                } else {
                    log::info!("[Debug] Object hit test disarmed");
                }
            }
            DebugAction::RerollShop => match &mut self.scene {
                Scene::Shop(s) => {
                    s.debug_reroll(&self.run);
                    log::info!("[Debug] Rerolled shop stock (free)");
                }
                _ => log::warn!("[Debug] Reroll Shop ignored — not in shop scene"),
            },
            DebugAction::OpenPack => match &mut self.scene {
                Scene::Shop(s) => {
                    let kinds = TilePackKind::all();
                    let kind = kinds[rand::rng().random_range(0..kinds.len())];
                    let tiles = GameEngine::debug_add_pack(&mut self.run, kind);
                    let celeb = PackCelebration::new(tiles, kind.name(), kind);
                    let inventory = s.tile_pack_celeb_inventory_counts(&self.run);
                    self.overlay_stack.push(Scene::TilePackCelebration(
                        TilePackCelebrationScene::new(celeb, inventory),
                    ));
                    log::info!("[Debug] Opened tile pack celebration overlay");
                }
                _ => log::warn!("[Debug] Open Pack ignored — not in shop scene"),
            },
            DebugAction::DemoCascade => {
                if let Scene::Gameplay(gp) = &mut self.scene {
                    if let Some(win) = self.window.as_ref() {
                        let size = win.inner_size();
                        let layout = self
                            .layout_engine
                            .solve(size.width as f32, size.height as f32);
                        gp.debug_demo_cascade(&layout, &self.run);
                    }
                } else {
                    let name = match &self.scene {
                        Scene::Splash(_) => "Splash",
                        Scene::MainMenuExterior(_) => "MainMenuExterior",
                        Scene::TileSelect(_) => "TileSelect",
                        Scene::ProfileSelect(_) => "ProfileSelect",
                        Scene::Shop(_) => "Shop",
                        Scene::ItemInspect(_) => "ItemInspect",
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
                        Scene::ZodiacCelebration(_) => "ZodiacCelebration",
                        Scene::TilePackCelebration(_) => "TilePackCelebration",
                    };
                    log::warn!("[Debug] Demo Cascade ignored — current scene is {name}");
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
                log::info!("[Debug] Set boss to {}", kind.name());
            }
            DebugAction::SetDora(suit, rank) => {
                self.run.wall.set_sole_dora(suit, rank);
                let name = crate::core::tile::Tile::new(suit, rank, 0).full_name();
                log::info!("[Debug] Set dora to {name}");
            }
            DebugAction::TestOverlay => {
                let modal = Modal::new(
                    "Test Overlay",
                    "This is a blank test modal.\nClick anywhere or press Enter to continue.",
                    ModalTheme::Info,
                );
                self.modals.push(modal);
                log::info!("[Debug] Spawned test overlay modal");
            }
            DebugAction::OpenMaterialViewer => {
                self.overlay_stack
                    .push(Scene::MaterialViewer(MaterialViewerScene::new(true)));
                log::info!("[Debug] Opened material viewer");
            }
            DebugAction::OpenTransitionPlayground => {
                self.overlay_stack.push(Scene::TransitionPlayground(
                    TransitionPlaygroundScene::new(true),
                ));
                log::info!("[Debug] Opened transition playground");
            }
            DebugAction::OpenRumbleLab => {
                self.overlay_stack
                    .push(Scene::RumbleLab(RumbleLabScene::new(true)));
                log::info!("[Debug] Opened rumble lab");
            }
            DebugAction::OpenAbout => {
                let body = format!(
                    "Mahjuro v{}\nA candlelit mahjong roguelike prototype.\n\nLocal icon asset: icon.png",
                    env!("CARGO_PKG_VERSION")
                );
                self.modals
                    .push(Modal::new("About Mahjuro", body, ModalTheme::Info));
                log::info!("[Debug] Opened About modal");
            }
            DebugAction::ShowVictoryScreen => {
                while self.modals.dismiss() {}
                self.pending_scene = Some(Scene::GameOver(GameOverScene::victory(&self.run)));
                self.transition_alpha = 1.0;
                log::info!("[Debug] Showing victory screen");
            }
            DebugAction::ShowDefeatScreen => {
                while self.modals.dismiss() {}
                self.pending_scene = Some(Scene::GameOver(GameOverScene::new(
                    &self.run,
                    crate::game::event_bus::GameOverReason::OutOfPlays,
                )));
                self.transition_alpha = 1.0;
                log::info!("[Debug] Showing defeat screen");
            }
            DebugAction::ToggleArrangeMode => {
                if self.debug.arrange_mode.is_some() {
                    self.debug.arrange_mode = None;
                    log::info!("[Debug] Arrange mode DEACTIVATED");
                } else {
                    self.debug.arrange_mode = Some(None);
                    log::info!(
                        "[Debug] Arrange mode ARMED — click an object OR press Tab to browse the hierarchy"
                    );
                }
            }
        }
        // Request redraw to reflect changes immediately.
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }
}
