//! Scene-to-scene transition policy: edge → fullscreen FX + fade speed, post-swap hooks,
//! and where a pending replace lands ([`PendingSceneDestination`]).
//!
//! Keep new visual edges here instead of growing ad-hoc `matches!` in `frame_tick`.

use crate::render::animation::AnimationController;
use crate::render::transition_fx::OverlayTransitionKind;
use crate::render::wgpu_renderer::WgpuRenderer;
use crate::scenes::Scene;
use crate::ui::input::InputState;

/// Fullscreen transition shader family + per-frame alpha step for the global fade.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TransitionKind {
    /// Default fast fade (~0.2 s at default speed).
    Quick,
    ForestOfTiles,
    GalaxyOfTiles,
    Maelstrom,
    ShufflingFan,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TransitionSpec {
    pub kind: TransitionKind,
    /// Subtracted from `transition_alpha` each frame during fade-out (then added back in fade-in).
    pub speed: f32,
}

pub(crate) const DEFAULT_QUICK_SPEC: TransitionSpec = TransitionSpec {
    kind: TransitionKind::Quick,
    speed: 0.08,
};

/// Coarse scene identity for transition tables (no inner scene state).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum SceneTag {
    Splash,
    MainMenu,
    TileSelect,
    ProfileSelect,
    Shop,
    Showcase,
    Hallway,
    Stairway,
    Gameplay,
    Victory,
    Defeat,
    Guide,
    MaterialViewer,
    TileAnchorLab,
    ButtonAabbLab,
    Options,
    Credits,
    Archive,
    TutorialCampaign,
    TutorialSummary,
    TransitionPlayground,
    AnimationLab,
    RumbleLab,
    ShadowAoLab,
    YakuJournal,
    WallLedger,
}

impl From<&Scene> for SceneTag {
    fn from(scene: &Scene) -> Self {
        match scene {
            Scene::Splash(_) => SceneTag::Splash,
            Scene::MainMenu(_) => SceneTag::MainMenu,
            Scene::TileSelect(_) => SceneTag::TileSelect,
            Scene::ProfileSelect(_) => SceneTag::ProfileSelect,
            Scene::Shop(_) => SceneTag::Shop,
            Scene::Showcase(_) => SceneTag::Showcase,
            Scene::Hallway(_) => SceneTag::Hallway,
            Scene::Stairway(_) => SceneTag::Stairway,
            Scene::Gameplay(_) => SceneTag::Gameplay,
            Scene::Victory(_) => SceneTag::Victory,
            Scene::Defeat(_) => SceneTag::Defeat,
            Scene::Guide(_) => SceneTag::Guide,
            Scene::MaterialViewer(_) => SceneTag::MaterialViewer,
            Scene::TileAnchorLab(_) => SceneTag::TileAnchorLab,
            Scene::ButtonAabbLab(_) => SceneTag::ButtonAabbLab,
            Scene::Options(_) => SceneTag::Options,
            Scene::Credits(_) => SceneTag::Credits,
            Scene::Archive(_) => SceneTag::Archive,
            Scene::TutorialCampaign(_) => SceneTag::TutorialCampaign,
            Scene::TutorialSummary(_) => SceneTag::TutorialSummary,
            Scene::TransitionPlayground(_) => SceneTag::TransitionPlayground,
            Scene::AnimationLab(_) => SceneTag::AnimationLab,
            Scene::RollerLab(_) => SceneTag::Gameplay,
            Scene::CascadeLab(_) => SceneTag::Gameplay,
            Scene::RumbleLab(_) => SceneTag::RumbleLab,
            Scene::ShadowAoLab(_) => SceneTag::ShadowAoLab,
            Scene::YakuJournal(_) => SceneTag::YakuJournal,
            Scene::WallLedger(_) => SceneTag::WallLedger,
        }
    }
}

#[inline]
fn undirected_edge(a: SceneTag, b: SceneTag, x: SceneTag, y: SceneTag) -> bool {
    (a, b) == (x, y) || (a, b) == (y, x)
}

/// Visual + timing for a replace transition from `from` → `to`.
pub(crate) fn transition_spec_for_edge(from: SceneTag, to: SceneTag) -> TransitionSpec {
    use SceneTag::*;

    // Main menu hub ↔ satellite screens (legacy comment: "tile teeth"; shipped = ForestOfTiles).
    if undirected_edge(from, to, MainMenu, Archive) {
        return TransitionSpec {
            kind: TransitionKind::ForestOfTiles,
            speed: 0.035,
        };
    }
    if undirected_edge(from, to, Archive, YakuJournal) {
        return TransitionSpec {
            kind: TransitionKind::GalaxyOfTiles,
            speed: 0.032,
        };
    }
    if undirected_edge(from, to, MainMenu, Options) || undirected_edge(from, to, Options, Credits) {
        return TransitionSpec {
            kind: TransitionKind::Maelstrom,
            speed: 0.032,
        };
    }
    if undirected_edge(from, to, MainMenu, ProfileSelect)
        || undirected_edge(from, to, Options, ProfileSelect)
    {
        return TransitionSpec {
            kind: TransitionKind::ShufflingFan,
            speed: 0.035,
        };
    }
    // Restart from pause: gameplay → shop — deliberate slower fade.
    if (from, to) == (Gameplay, Shop) {
        return TransitionSpec {
            kind: TransitionKind::Quick,
            speed: 0.025,
        };
    }
    if (from, to) == (Gameplay, Stairway) {
        return TransitionSpec {
            kind: TransitionKind::Quick,
            speed: 0.025,
        };
    }
    if (from, to) == (Stairway, Shop) {
        return TransitionSpec {
            kind: TransitionKind::Quick,
            speed: 0.03,
        };
    }

    DEFAULT_QUICK_SPEC
}

/// Where [`crate::App::pending_scene`] is applied when the fade hits black.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum PendingSceneDestination {
    /// Replace the root [`crate::App::scene`].
    #[default]
    Base,
    /// Replace the top of [`crate::App::overlay_stack`] (fade was initiated by the overlay).
    /// If the stack is empty, falls back to replacing the base scene.
    OverlayTop,
}

pub(crate) fn overlay_kind_for_transition(kind: TransitionKind) -> Option<OverlayTransitionKind> {
    match kind {
        TransitionKind::Quick => None,
        TransitionKind::ForestOfTiles => Some(OverlayTransitionKind::ForestOfTiles),
        TransitionKind::GalaxyOfTiles => Some(OverlayTransitionKind::GalaxyOfTiles),
        TransitionKind::Maelstrom => Some(OverlayTransitionKind::Maelstrom),
        TransitionKind::ShufflingFan => Some(OverlayTransitionKind::ShufflingFan),
    }
}

pub(crate) fn should_clear_smoke_on_transition(from: SceneTag, to: SceneTag) -> bool {
    use SceneTag::*;
    matches!(
        (from, to),
        (TileSelect, Shop)
            | (TutorialCampaign, Shop)
            | (TutorialCampaign, Gameplay)
            | (Shop, Hallway)
    )
}

pub(crate) struct PostSceneTransitionCtx<'a> {
    pub from: SceneTag,
    pub to: SceneTag,
    /// Boss BGM when entering gameplay — uses the round being started
    /// (`GameplayScene::pending_chamber`), not `run.chamber` (often still the
    /// cleared blind until `apply_chamber` runs).
    pub gameplay_ordeal_chamber: bool,
    pub anim: &'a mut AnimationController,
    pub renderer: Option<&'a mut WgpuRenderer>,
    pub input: Option<&'a mut InputState>,
    pub audio: &'a mut crate::audio::AudioManager,
}

/// Shop BGM waits this long after a fresh shop entry (door chime plays immediately).
const SHOP_BGM_START_DELAY: f32 = 2.0;

/// Side effects after the scene pointer(s) have been updated (smoke, SFX, focus reset, entry tweens).
pub(crate) fn sync_music_for_scene(
    audio: &mut crate::audio::AudioManager,
    tag: SceneTag,
    gameplay_ordeal_chamber: bool,
    shop_bgm_delay: Option<std::time::Duration>,
) {
    use crate::audio::MusicId;
    use std::time::Instant;
    match tag {
        SceneTag::MainMenu | SceneTag::Archive => audio.set_music_track(MusicId::MainMenu),
        SceneTag::Gameplay => audio.set_gameplay_music(gameplay_ordeal_chamber),
        SceneTag::Shop => {
            if let Some(delay) = shop_bgm_delay {
                audio.schedule_music_track(Instant::now() + delay, MusicId::Shop);
            } else {
                audio.set_music_track(MusicId::Shop);
            }
        }
        SceneTag::Hallway => audio.set_music_track(MusicId::Shop),
        SceneTag::Stairway => {
            audio.stop_background_music();
        }
        SceneTag::Credits => audio.set_music_track(MusicId::Credits),
        _ => audio.stop_background_music(),
    }
}

pub(crate) fn sync_ambient_for_scene(audio: &mut crate::audio::AudioManager, tag: SceneTag) {
    use crate::audio::AmbientId;
    match tag {
        SceneTag::MainMenu => {
            audio.set_ambient_tracks(&[AmbientId::MainMenuRain, AmbientId::HallwayBulbBuzz])
        }
        SceneTag::Hallway | SceneTag::Stairway => {
            audio.set_ambient_tracks(&[AmbientId::HallwayBulbBuzz]);
        }
        _ => audio.set_ambient_tracks(&[]),
    }
}

pub(crate) fn apply_post_scene_transition_effects(ctx: PostSceneTransitionCtx<'_>) {
    if let Some(r) = ctx.renderer {
        if should_clear_smoke_on_transition(ctx.from, ctx.to) {
            r.clear_smoke();
        }
        match ctx.to {
            SceneTag::MainMenu => {
                r.prefetch_room_chain_next(crate::render::room_preload::RoomSceneChain::Shop);
            }
            SceneTag::Shop => {
                r.prefetch_room_chain_next(crate::render::room_preload::RoomSceneChain::Hallway);
            }
            SceneTag::Hallway => {
                r.prefetch_room_chain_next(crate::render::room_preload::RoomSceneChain::Gameplay);
            }
            SceneTag::Gameplay => {
                r.snap_gameplay_score_rollers();
            }
            _ => {}
        }
    }
    if ctx.to == SceneTag::MainMenu {
        ctx.audio.play_sfx(crate::audio::SfxId::MainMenuEnter);
    }
    if ctx.to == SceneTag::Stairway {
        ctx.audio.play_sfx(crate::audio::SfxId::StairwayEnter);
    }
    if ctx.to == SceneTag::Shop {
        ctx.audio.play_sfx(crate::audio::SfxId::ShopEnter);
    }
    let shop_bgm_delay = if ctx.to == SceneTag::Shop {
        Some(std::time::Duration::from_secs_f32(SHOP_BGM_START_DELAY))
    } else {
        None
    };
    if ctx.to == SceneTag::MainMenu {
        crate::asset_path::prefetch_lazy_packs_after_menu_once();
    }
    sync_music_for_scene(
        ctx.audio,
        ctx.to,
        ctx.gameplay_ordeal_chamber,
        shop_bgm_delay,
    );
    sync_ambient_for_scene(ctx.audio, ctx.to);
    if let Some(input) = ctx.input {
        input.focus_slot = 0;
    }
    ctx.anim
        .fade(crate::render::animation::ENTITY_SCORE_PANEL, 0.0, 1.0, 300);
    ctx.anim
        .slide_to(crate::render::animation::ENTITY_HAND_STRIP, 0.0, -20.0, 400);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_menu_collection_is_forest() {
        let s = transition_spec_for_edge(SceneTag::MainMenu, SceneTag::Archive);
        assert_eq!(s.kind, TransitionKind::ForestOfTiles);
        assert!((s.speed - 0.035).abs() < 1e-6);
        let s2 = transition_spec_for_edge(SceneTag::Archive, SceneTag::MainMenu);
        assert_eq!(s2.kind, TransitionKind::ForestOfTiles);
    }

    #[test]
    fn gameplay_shop_is_slow_quick() {
        let s = transition_spec_for_edge(SceneTag::Gameplay, SceneTag::Shop);
        assert_eq!(s.kind, TransitionKind::Quick);
        assert!((s.speed - 0.025).abs() < 1e-6);
    }

    #[test]
    fn default_unlisted_edge_is_quick_default_speed() {
        let s = transition_spec_for_edge(SceneTag::Splash, SceneTag::Shop);
        assert_eq!(s.kind, TransitionKind::Quick);
        assert!((s.speed - DEFAULT_QUICK_SPEC.speed).abs() < 1e-6);
    }

    #[test]
    fn smoke_cleared_tile_select_to_shop() {
        assert!(should_clear_smoke_on_transition(
            SceneTag::TileSelect,
            SceneTag::Shop
        ));
        assert!(!should_clear_smoke_on_transition(
            SceneTag::Shop,
            SceneTag::Gameplay
        ));
    }
}
