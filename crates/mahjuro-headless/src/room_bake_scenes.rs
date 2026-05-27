use mahjuro::game::run::RunState;
use mahjuro::scenes::shop::ShopScene;
use mahjuro::scenes::{GameplayScene, Scene};

use super::fixtures::{setup_gameplay_screenshot_state, setup_shop_state};

pub(crate) fn scene_for_room_gi_bake(
    room: mahjuro::render::room_gi_bake::RoomGiRoom,
    progress: &mahjuro::core::progression::PlayerProgress,
) -> (Scene, RunState, bool) {
    let mut run = RunState::new_demo();
    match room {
        mahjuro::render::room_gi_bake::RoomGiRoom::Shop => {
            setup_shop_state(&mut run);
            (Scene::Shop(ShopScene::new(&mut run, progress)), run, false)
        }
        mahjuro::render::room_gi_bake::RoomGiRoom::Hallway => (
            Scene::PickChamber(mahjuro::scenes::PickChamberScene::new()),
            run,
            true,
        ),
        mahjuro::render::room_gi_bake::RoomGiRoom::Archive => {
            let coll = mahjuro::scenes::CollectionScene::new();
            (Scene::Collection(coll), run, false)
        }
        mahjuro::render::room_gi_bake::RoomGiRoom::MainMenu => (
            Scene::MainMenuExterior(mahjuro::scenes::MainMenuExteriorScene::new()),
            run,
            false,
        ),
        mahjuro::render::room_gi_bake::RoomGiRoom::Staircase => (
            Scene::Staircase(mahjuro::scenes::StaircaseScene::new()),
            run,
            false,
        ),
        mahjuro::render::room_gi_bake::RoomGiRoom::Gameplay => {
            setup_gameplay_screenshot_state(&mut run);
            (Scene::Gameplay(Box::new(GameplayScene::new())), run, true)
        }
    }
}
