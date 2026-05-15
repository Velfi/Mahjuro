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
    TileWaterfall,
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
    MainMenuExterior,
    TileSelect,
    ProfileSelect,
    Shop,
    Showcase,
    PickBlind,
    Gameplay,
    GameOver,
    MeldGuide,
    MaterialViewer,
    Options,
    Collection,
    TutorialRecap,
    TutorialCampaign,
    TutorialSummary,
    TileLiteracy,
    TransitionPlayground,
    RumbleLab,
    YakuJournal,
}

impl From<&Scene> for SceneTag {
    fn from(scene: &Scene) -> Self {
        match scene {
            Scene::Splash(_) => SceneTag::Splash,
            Scene::MainMenuExterior(_) => SceneTag::MainMenuExterior,
            Scene::TileSelect(_) => SceneTag::TileSelect,
            Scene::ProfileSelect(_) => SceneTag::ProfileSelect,
            Scene::Shop(_) => SceneTag::Shop,
            Scene::Showcase(_) => SceneTag::Showcase,
            Scene::PickBlind(_) => SceneTag::PickBlind,
            Scene::Gameplay(_) => SceneTag::Gameplay,
            Scene::GameOver(_) => SceneTag::GameOver,
            Scene::MeldGuide(_) => SceneTag::MeldGuide,
            Scene::MaterialViewer(_) => SceneTag::MaterialViewer,
            Scene::Options(_) => SceneTag::Options,
            Scene::Collection(_) => SceneTag::Collection,
            Scene::TutorialRecap(_) => SceneTag::TutorialRecap,
            Scene::TutorialCampaign(_) => SceneTag::TutorialCampaign,
            Scene::TutorialSummary(_) => SceneTag::TutorialSummary,
            Scene::TileLiteracy(_) => SceneTag::TileLiteracy,
            Scene::TransitionPlayground(_) => SceneTag::TransitionPlayground,
            Scene::RumbleLab(_) => SceneTag::RumbleLab,
            Scene::YakuJournal(_) => SceneTag::YakuJournal,
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
    if undirected_edge(from, to, MainMenuExterior, Collection) {
        return TransitionSpec {
            kind: TransitionKind::ForestOfTiles,
            speed: 0.035,
        };
    }
    if undirected_edge(from, to, Collection, YakuJournal) {
        return TransitionSpec {
            kind: TransitionKind::GalaxyOfTiles,
            speed: 0.032,
        };
    }
    if undirected_edge(from, to, MainMenuExterior, Options) {
        return TransitionSpec {
            kind: TransitionKind::Maelstrom,
            speed: 0.032,
        };
    }
    if undirected_edge(from, to, MainMenuExterior, TileLiteracy) {
        return TransitionSpec {
            kind: TransitionKind::TileWaterfall,
            speed: 0.034,
        };
    }
    if undirected_edge(from, to, MainMenuExterior, ProfileSelect)
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
        TransitionKind::TileWaterfall => Some(OverlayTransitionKind::TileWaterfall),
        TransitionKind::ShufflingFan => Some(OverlayTransitionKind::ShufflingFan),
    }
}

pub(crate) fn should_clear_smoke_on_transition(from: SceneTag, to: SceneTag) -> bool {
    use SceneTag::*;
    matches!(
        (from, to),
        (TileSelect, Shop) | (TutorialCampaign, Shop) | (Shop, PickBlind)
    )
}

pub(crate) struct PostSceneTransitionCtx<'a> {
    pub from: SceneTag,
    pub to: SceneTag,
    pub pushed_meta_level_up: bool,
    pub anim: &'a mut AnimationController,
    pub renderer: Option<&'a mut WgpuRenderer>,
    pub input: Option<&'a mut InputState>,
    pub audio: &'a mut crate::audio::AudioManager,
}

/// Side effects after the scene pointer(s) have been updated (smoke, SFX, focus reset, entry tweens).
pub(crate) fn sync_music_for_scene(audio: &mut crate::audio::AudioManager, tag: SceneTag) {
    use crate::audio::MusicId;
    match tag {
        SceneTag::MainMenuExterior | SceneTag::Collection => {
            audio.set_music_track(MusicId::MainMenu)
        }
        SceneTag::Gameplay => audio.set_music_track(MusicId::Gameplay),
        SceneTag::Shop | SceneTag::PickBlind => audio.set_music_track(MusicId::Shop),
        _ => audio.stop_background_music(),
    }
}

pub(crate) fn apply_post_scene_transition_effects(ctx: PostSceneTransitionCtx<'_>) {
    if should_clear_smoke_on_transition(ctx.from, ctx.to) {
        if let Some(r) = ctx.renderer {
            r.clear_smoke();
        }
    }
    if ctx.to == SceneTag::MainMenuExterior && !ctx.pushed_meta_level_up {
        ctx.audio.play_sfx(crate::audio::SfxId::MainMenuEnter);
    }
    if ctx.to == SceneTag::MainMenuExterior {
        crate::asset_path::prefetch_lazy_packs_after_menu_once();
    }
    sync_music_for_scene(ctx.audio, ctx.to);
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
        let s = transition_spec_for_edge(SceneTag::MainMenuExterior, SceneTag::Collection);
        assert_eq!(s.kind, TransitionKind::ForestOfTiles);
        assert!((s.speed - 0.035).abs() < 1e-6);
        let s2 = transition_spec_for_edge(SceneTag::Collection, SceneTag::MainMenuExterior);
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
