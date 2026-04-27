/// One candle's animated state. The renderer's flame shader does the heavy
/// lifting (procedural fbm noise + flicker), but each candle still needs a
/// stable random phase so neighbouring candles don't beat in lockstep, and a
/// CPU-side flicker accumulator so the matching point light's intensity
/// pulses in sync with what the player sees on screen.
#[derive(Clone, Copy)]
pub(super) struct CandleState {
    /// Random phase offset in [0, TAU).  Mixed into the flame shader's noise
    /// coordinates and into the CPU flicker term so each candle is unique.
    pub(super) phase: f32,
    /// Smoothed intensity multiplier — wanders ~[0.7, 1.1] driven by a sum
    /// of two sines so it never repeats audibly.
    pub(super) flicker: f32,
}

impl CandleState {
    pub(super) fn new(phase: f32) -> Self {
        Self {
            phase,
            flicker: 1.0,
        }
    }
}
