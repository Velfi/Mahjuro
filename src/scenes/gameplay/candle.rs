/// One candle's animated state. Each candle needs a stable random phase so
/// neighbouring flames don't beat in lockstep; brightness flicker is handled
/// by [`crate::render::flame_volume::flame_flicker_multiplier`].
#[derive(Clone, Copy)]
pub(super) struct CandleState {
    /// Random phase offset in [0, TAU), mapped to [0, 1) for [`FlameEmitter::phase`].
    pub(super) phase: f32,
}

impl CandleState {
    pub(super) fn new(phase: f32) -> Self {
        Self { phase }
    }
}
