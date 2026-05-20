//! Per-frame swapchain acquire telemetry.
//!
//! Records the time spent inside [`wgpu::Surface::get_current_texture`] for
//! each rendered frame plus the outcome distribution (Success / Suboptimal /
//! Outdated / Timeout-or-Occluded / Lost). After a short warmup the first
//! snapshot is logged at info level; if the running EMA crosses an obvious
//! "we're stuck waiting on the swapchain" threshold a single warning is
//! emitted. Both messages name the same Cell-backed counters so a follow-up
//! report has every number needed to triage frame-pacing problems without
//! adding a per-frame log line.
//!
//! Motivating case: Steam Deck game mode (gamescope nested compositor)
//! pinning the game to ~10 FPS while the CPU + GPU sat idle. The previous
//! diagnostic info — `swapchain ready: …` + the [`FramePerfWatchdog`] — only
//! reported that *something* was wrong; this module names which call inside
//! the renderer was actually blocked.
//!
//! [`FramePerfWatchdog`]: crate::main_perf_watchdog::FramePerfWatchdog
//!
//! Single-threaded by construction (owned by `WgpuRenderer`, which is driven
//! from the SDL main loop), so interior mutability is provided by [`Cell`]
//! rather than atomics or a `Mutex`.

use std::cell::Cell;

/// Time-constant for the per-frame acquire EMA. At 60 FPS this gives a
/// ~1.4 s half-life — short enough to react to a sustained block, long
/// enough to ignore the occasional one-frame hitch.
const EMA_ALPHA: f32 = 0.05;

/// Number of acquire samples collected before any summary is logged.
const WARMUP_FRAMES: u64 = 60;

/// EMA threshold above which we treat the acquire path as "stuck waiting".
/// 33 ms ≈ 30 FPS purely in `get_current_texture`. Anything past this is
/// almost certainly the WSI / compositor / driver blocking us rather than
/// our own render work, because we don't do anything else on this thread
/// between the previous present and this acquire.
const HIGH_EMA_MS: f32 = 33.0;

/// Outcome bucket for a single `get_current_texture` attempt.
#[derive(Copy, Clone, Debug)]
pub(crate) enum AcquireOutcome {
    Success,
    Suboptimal,
    Outdated,
    TimeoutOrOccluded,
    Lost,
}

#[derive(Default)]
pub(crate) struct AcquireTelemetry {
    /// Frames where any acquire attempt produced a usable texture.
    frames_success: Cell<u64>,
    /// Frames where every retry failed and the renderer returned `Skip`.
    frames_skipped: Cell<u64>,
    /// Sum of attempted `get_current_texture` calls (including retries).
    attempts: Cell<u64>,
    success: Cell<u64>,
    suboptimal: Cell<u64>,
    outdated: Cell<u64>,
    timeout_or_occluded: Cell<u64>,
    lost: Cell<u64>,
    /// Exponential moving average of the wall-clock time spent inside the
    /// acquire loop per *frame* (not per attempt), in milliseconds.
    acquire_ema_ms: Cell<f32>,
    /// Tracks whether [`Self::log_initial_summary`] has fired so it can't
    /// repeat.
    initial_summary_logged: Cell<bool>,
    /// Tracks whether [`Self::log_high_ema_warning`] has fired so it can't
    /// repeat. The startup summary already contains the same data — the
    /// warning is just a louder line if the situation gets worse later.
    high_ema_warned: Cell<bool>,
}

impl AcquireTelemetry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a single `get_current_texture` attempt outcome. Per-frame
    /// timing is recorded separately by [`Self::record_frame`].
    pub fn record_attempt(&self, outcome: AcquireOutcome) {
        self.attempts.set(self.attempts.get() + 1);
        let counter = match outcome {
            AcquireOutcome::Success => &self.success,
            AcquireOutcome::Suboptimal => &self.suboptimal,
            AcquireOutcome::Outdated => &self.outdated,
            AcquireOutcome::TimeoutOrOccluded => &self.timeout_or_occluded,
            AcquireOutcome::Lost => &self.lost,
        };
        counter.set(counter.get() + 1);
    }

    /// Record a whole frame's worth of acquire activity: the wall-clock time
    /// the renderer spent inside [`crate::render::wgpu_renderer::WgpuRenderer::acquire_render_frame`]
    /// and whether the frame ended up drawn (`success=true`) or skipped.
    /// Triggers the initial info summary and (at most once per session) the
    /// `high acquire` warning when the EMA crosses [`HIGH_EMA_MS`].
    pub fn record_frame(&self, elapsed_ms: f32, success: bool) {
        if success {
            self.frames_success.set(self.frames_success.get() + 1);
        } else {
            self.frames_skipped.set(self.frames_skipped.get() + 1);
        }

        let prev = self.acquire_ema_ms.get();
        let next = if prev == 0.0 {
            elapsed_ms
        } else {
            prev * (1.0 - EMA_ALPHA) + elapsed_ms * EMA_ALPHA
        };
        self.acquire_ema_ms.set(next);

        let total_frames = self.frames_success.get() + self.frames_skipped.get();
        if total_frames == WARMUP_FRAMES {
            self.log_initial_summary();
        } else if total_frames > WARMUP_FRAMES
            && next > HIGH_EMA_MS
            && !self.high_ema_warned.get()
        {
            self.log_high_ema_warning();
        }
    }

    fn log_initial_summary(&self) {
        if self.initial_summary_logged.replace(true) {
            return;
        }
        log::info!(
            "swapchain acquire (first {} frames): ema={:.2} ms attempts={} success={} suboptimal={} outdated={} timeout/occluded={} lost={} skipped_frames={}",
            WARMUP_FRAMES,
            self.acquire_ema_ms.get(),
            self.attempts.get(),
            self.success.get(),
            self.suboptimal.get(),
            self.outdated.get(),
            self.timeout_or_occluded.get(),
            self.lost.get(),
            self.frames_skipped.get(),
        );
    }

    fn log_high_ema_warning(&self) {
        if self.high_ema_warned.replace(true) {
            return;
        }
        let total_frames = self.frames_success.get() + self.frames_skipped.get();
        log::warn!(
            "swapchain acquire is stuck waiting: ema={:.2} ms over {} frames \
             (attempts={} outdated={} timeout/occluded={} lost={} skipped={}). \
             The renderer is being blocked inside get_current_texture; check the \
             `swapchain ready:` / `presentation env:` startup lines for the \
             compositor + present-mode + frame-latency context.",
            self.acquire_ema_ms.get(),
            total_frames,
            self.attempts.get(),
            self.outdated.get(),
            self.timeout_or_occluded.get(),
            self.lost.get(),
            self.frames_skipped.get(),
        );
    }
}
