//! Scene + run setup for offline room GI / shadow bakes.

use mahjuro::core::progression::PlayerProgress;
use mahjuro::game::run::RunState;
use mahjuro::render::room_gi_bake::RoomGiRoom;
use mahjuro::scenes::shop::ShopScene;
use mahjuro::scenes::{
    CollectionScene, GameplayScene, MainMenuExteriorScene, PickChamberScene, Scene,
    StaircaseScene,
};

use super::fixtures::{setup_gameplay_bake_state, setup_shop_state};

/// Resting-camera scene for each static room bake target.
pub fn scene_for_room(
    room: RoomGiRoom,
    progress: &PlayerProgress,
) -> (Scene, RunState, bool) {
    let mut run = RunState::new_demo();
    match room {
        RoomGiRoom::Shop => {
            setup_shop_state(&mut run);
            (Scene::Shop(ShopScene::new(&mut run, progress)), run, false)
        }
        RoomGiRoom::Hallway => (
            Scene::PickChamber(PickChamberScene::new()),
            run,
            true,
        ),
        RoomGiRoom::Archive => {
            let coll = CollectionScene::new();
            (Scene::Collection(coll), run, false)
        }
        RoomGiRoom::MainMenu => (
            Scene::MainMenuExterior(MainMenuExteriorScene::new()),
            run,
            false,
        ),
        RoomGiRoom::Staircase => (
            Scene::Staircase(StaircaseScene::new()),
            run,
            false,
        ),
        RoomGiRoom::Gameplay => {
            setup_gameplay_bake_state(&mut run);
            (Scene::Gameplay(Box::new(GameplayScene::new())), run, true)
        }
    }
}
