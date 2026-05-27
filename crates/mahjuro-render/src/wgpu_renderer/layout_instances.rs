use super::ui_instances::GpuInstance;

/// Build GPU instances for the score panel and modifier strip.
///
/// Play/discard counts are shown with **3D tally fans** on the table; this
/// function intentionally returns no quads so the score panel stays clean.
#[allow(clippy::too_many_arguments)]
pub fn build_instances_from_layout(
    _score: (f32, f32, f32, f32),
    _modifier: (f32, f32, f32, f32),
    _anim_scale_score: f32,
    _plays: u32,
    _plays_max: u32,
    _discards: u32,
    _discards_max: u32,
) -> Vec<GpuInstance> {
    Vec::new()
}
