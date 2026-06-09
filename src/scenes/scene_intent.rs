//! Deferred scene replace: scenes return [`SceneIntent`]; the app fades to black
//! before [`SceneIntent::resolve`] builds the destination [`Scene`].

use crate::core::rules::ChamberKind;
use crate::core::season::Season;
use crate::game::engine::GameEngine;
use crate::game::run::RunState;
use crate::persistence::{self, ResumeScene, TileMaterial};
use crate::render::scene_keys;
#[cfg(feature = "game")]
use crate::scene_transition::SceneTag;
use mahjuro_types::GameOverReason;

use super::{
    ArchiveScene, CreditsScene, DefeatScene, GameplayScene, HallwayScene, MainMenuScene,
    OptionsScene, ProfileSelectScene, Scene, ShopScene, StairwayScene, TileSelectScene,
    TutorialCampaignScene, TutorialSummaryScene, VictoryScene,
};
use crate::core::progression::PlayerProgress;

/// `None` = stay in current scene; `Some(intent)` = fade out, then resolve at black.
pub type SceneTransition = Option<SceneIntent>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SceneIntent {
    MainMenu,
    Archive,
    Options,
    ProfileSelectFromArchive,
    TileSelect {
        tutorial: bool,
    },
    Continue(ResumeScene),
    StartRunDefaultMaterialAndShop,
    StartRun {
        material: TileMaterial,
        season: Season,
    },
    StartOnboardingRunAndTutorialCampaign,
    SkipTutorialStartRunAndShop,
    ShopFromRun,
    ShopTutorial,
    Hallway,
    GameplayHallwayPlay,
    GameplayOrdealFromShopTutorial,
    GameplayLessonsFirstChamber,
    GameplayRetryOnboarding(ChamberKind),
    Stairway,
    TutorialSummary {
        won: bool,
    },
    Victory,
    Defeat(GameOverReason),
    CreditsFromOptions,
}

pub struct SceneResolveCtx<'a> {
    pub run: &'a mut RunState,
    pub progress: &'a PlayerProgress,
}

impl SceneIntent {
    #[cfg(feature = "game")]
    pub(crate) fn scene_tag(&self) -> SceneTag {
        use SceneTag::*;
        match self {
            Self::MainMenu => MainMenu,
            Self::Archive => Archive,
            Self::Options => Options,
            Self::ProfileSelectFromArchive => ProfileSelect,
            Self::TileSelect { .. } => TileSelect,
            Self::Continue(resume) => match resume {
                ResumeScene::Gameplay => Gameplay,
                ResumeScene::Shop => Shop,
                ResumeScene::Hallway => Hallway,
            },
            Self::StartRunDefaultMaterialAndShop
            | Self::StartRun { .. }
            | Self::SkipTutorialStartRunAndShop
            | Self::ShopFromRun
            | Self::ShopTutorial => Shop,
            Self::StartOnboardingRunAndTutorialCampaign => TutorialCampaign,
            Self::Hallway => Hallway,
            Self::GameplayHallwayPlay => Gameplay,
            Self::GameplayOrdealFromShopTutorial
            | Self::GameplayLessonsFirstChamber
            | Self::GameplayRetryOnboarding(_) => Gameplay,
            Self::Stairway => Stairway,
            Self::TutorialSummary { .. } => TutorialSummary,
            Self::Victory => Victory,
            Self::Defeat(_) => Defeat,
            Self::CreditsFromOptions => Credits,
        }
    }

    pub fn scene_key(&self) -> Option<&'static str> {
        match self {
            Self::MainMenu => Some(scene_keys::MAIN_MENU),
            Self::Archive => Some(scene_keys::ARCHIVE),
            Self::Options => Some(scene_keys::OPTIONS),
            Self::TileSelect { .. } => Some("tile_select"),
            Self::Continue(resume) => match resume {
                ResumeScene::Gameplay => Some(scene_keys::GAMEPLAY),
                ResumeScene::Shop => Some(scene_keys::SHOP),
                ResumeScene::Hallway => Some(scene_keys::HALLWAY),
            },
            Self::StartRunDefaultMaterialAndShop
            | Self::StartRun { .. }
            | Self::SkipTutorialStartRunAndShop
            | Self::ShopFromRun
            | Self::ShopTutorial => Some(scene_keys::SHOP),
            Self::StartOnboardingRunAndTutorialCampaign => Some("tutorial"),
            Self::Hallway => Some(scene_keys::HALLWAY),
            Self::GameplayHallwayPlay
            | Self::GameplayOrdealFromShopTutorial
            | Self::GameplayLessonsFirstChamber
            | Self::GameplayRetryOnboarding(_) => Some(scene_keys::GAMEPLAY),
            Self::Stairway => Some(scene_keys::STAIRWAY),
            Self::Victory => Some(scene_keys::VICTORY),
            Self::Defeat(_) => Some(scene_keys::DEFEAT),
            Self::ProfileSelectFromArchive
            | Self::TutorialSummary { .. }
            | Self::CreditsFromOptions => None,
        }
    }

    pub fn grants_memorial_on_start(&self) -> bool {
        matches!(self, Self::ShopFromRun | Self::ShopTutorial)
    }

    pub fn resolve(self, ctx: SceneResolveCtx<'_>) -> Scene {
        match self {
            Self::MainMenu => Scene::MainMenu(MainMenuScene::new()),
            Self::Archive => Scene::Archive(ArchiveScene::new()),
            Self::Options => Scene::Options(OptionsScene::new()),
            Self::ProfileSelectFromArchive => {
                Scene::ProfileSelect(ProfileSelectScene::from_archive_switch_save())
            }
            Self::TileSelect { tutorial } => {
                if tutorial {
                    Scene::TileSelect(TileSelectScene::new_tutorial())
                } else {
                    Scene::TileSelect(TileSelectScene::new())
                }
            }
            Self::Continue(resume) => {
                super::main_menu::scene_from_resume(resume, ctx.run, ctx.progress)
            }
            Self::StartRunDefaultMaterialAndShop => {
                let settings = persistence::load_settings();
                GameEngine::start_run_with_material(
                    ctx.run,
                    TileMaterial::default(),
                    ctx.progress,
                    &settings,
                );
                Scene::Shop(ShopScene::new(ctx.run, ctx.progress))
            }
            Self::StartRun { material, season } => {
                let settings = persistence::load_settings();
                GameEngine::start_run_with_material_and_season(
                    ctx.run,
                    material,
                    season,
                    ctx.progress,
                    &settings,
                );
                Scene::Shop(ShopScene::new(ctx.run, ctx.progress))
            }
            Self::StartOnboardingRunAndTutorialCampaign => {
                let settings = persistence::load_settings();
                GameEngine::start_onboarding_run(ctx.run, ctx.progress, &settings);
                Scene::TutorialCampaign(TutorialCampaignScene::new())
            }
            Self::SkipTutorialStartRunAndShop => {
                let settings = persistence::load_settings();
                GameEngine::start_run_with_material(
                    ctx.run,
                    TileMaterial::default(),
                    ctx.progress,
                    &settings,
                );
                Scene::Shop(ShopScene::new(ctx.run, ctx.progress))
            }
            Self::ShopFromRun => Scene::Shop(ShopScene::new(ctx.run, ctx.progress)),
            Self::ShopTutorial => Scene::Shop(ShopScene::new_tutorial(ctx.run)),
            Self::Hallway => Scene::Hallway(HallwayScene::new()),
            Self::GameplayHallwayPlay => {
                let upcoming = GameEngine::read_hallway(ctx.run).upcoming_chamber;
                Scene::Gameplay(Box::new(GameplayScene::enter_pending_chamber(
                    ctx.run, upcoming,
                )))
            }
            Self::GameplayOrdealFromShopTutorial => {
                GameEngine::transition_to_onboarding_finale(ctx.run);
                Scene::Gameplay(Box::new(GameplayScene::enter_pending_chamber(
                    ctx.run,
                    ChamberKind::Ordeal,
                )))
            }
            Self::GameplayLessonsFirstChamber => {
                GameEngine::begin_onboarding_lessons(ctx.run);
                Scene::Gameplay(Box::new(GameplayScene::enter_pending_chamber(
                    ctx.run,
                    ChamberKind::Small,
                )))
            }
            Self::GameplayRetryOnboarding(chamber) => Scene::Gameplay(Box::new(
                GameplayScene::enter_pending_chamber(ctx.run, chamber),
            )),
            Self::Stairway => Scene::Stairway(StairwayScene::new()),
            Self::TutorialSummary { won } => Scene::TutorialSummary(TutorialSummaryScene::new(won)),
            Self::Victory => Scene::Victory(VictoryScene::new(ctx.run)),
            Self::Defeat(reason) => Scene::Defeat(DefeatScene::new(ctx.run, reason)),
            Self::CreditsFromOptions => Scene::Credits(CreditsScene::from_options()),
        }
    }
}
