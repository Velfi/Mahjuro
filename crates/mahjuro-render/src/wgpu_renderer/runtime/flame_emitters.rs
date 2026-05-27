use super::*;

/// Scene-supplied flame emitters (shop / gameplay GLB candles).
pub(super) fn build_flame_emitters(
    frame: &UiFrame,
    _w: f32,
    _h: f32,
) -> Vec<crate::flame_volume::FlameEmitter> {
    frame.procedural_flame_emitters.to_vec()
}
