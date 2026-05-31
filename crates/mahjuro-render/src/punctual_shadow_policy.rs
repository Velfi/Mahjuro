//! Per-scene punctual shadow caster policy (explicit glTF node allowlists).

use crate::wgpu_renderer::ActiveRoomEnv;

/// glTF node prefixes allowed to cast depth shadows on the main menu.
pub const MAIN_MENU_SHADOW_CASTER_PREFIXES: &[&str] = &["light_moonlight", "light_doorway"];

/// glTF node prefix for shop lantern shadow casters.
pub const SHOP_LANTERN_SHADOW_CASTER_PREFIX: &str = "light_lantern";

/// Whether a punctual light at `light_index` should render a depth shadow this frame.
pub(crate) fn punctual_light_casts_shadow(
    env: Option<ActiveRoomEnv>,
    gltf_node_name: Option<&str>,
) -> bool {
    let Some(env) = env else {
        // Overlay / lab scenes: procedural smooth lights, no glTF policy.
        return true;
    };
    match env {
        ActiveRoomEnv::MainMenu => node_matches_any_prefix(gltf_node_name, MAIN_MENU_SHADOW_CASTER_PREFIXES),
        ActiveRoomEnv::Shop => node_starts_with(gltf_node_name, SHOP_LANTERN_SHADOW_CASTER_PREFIX),
        ActiveRoomEnv::Gameplay
        | ActiveRoomEnv::Archive
        | ActiveRoomEnv::Hallway
        | ActiveRoomEnv::Stairway => true,
    }
}

#[inline]
fn node_starts_with(name: Option<&str>, prefix: &str) -> bool {
    name.is_some_and(|n| n.starts_with(prefix))
}

#[inline]
fn node_matches_any_prefix(name: Option<&str>, prefixes: &[&str]) -> bool {
    let Some(name) = name else {
        return false;
    };
    prefixes.iter().any(|p| name.starts_with(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_menu_only_moon_and_doorway() {
        let env = Some(ActiveRoomEnv::MainMenu);
        assert!(punctual_light_casts_shadow(
            env,
            Some("light_moonlight.001"),
        ));
        assert!(punctual_light_casts_shadow(env, Some("light_doorway")));
        assert!(!punctual_light_casts_shadow(env, Some("light_lantern")));
        assert!(!punctual_light_casts_shadow(env, None));
    }

    #[test]
    fn shop_only_lanterns() {
        let env = Some(ActiveRoomEnv::Shop);
        assert!(punctual_light_casts_shadow(env, Some("light_lantern_06")));
        assert!(!punctual_light_casts_shadow(env, Some("light_candle")));
        assert!(!punctual_light_casts_shadow(env, None));
    }

    #[test]
    fn procedural_scenes_cast_when_no_room_env() {
        assert!(punctual_light_casts_shadow(None, None));
        assert!(punctual_light_casts_shadow(None, Some("light_candle")));
    }

    #[test]
    fn gameplay_casts_all() {
        let env = Some(ActiveRoomEnv::Gameplay);
        assert!(punctual_light_casts_shadow(env, None));
        assert!(punctual_light_casts_shadow(env, Some("light_candle")));
    }
}
