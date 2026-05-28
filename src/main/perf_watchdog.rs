//! Sustained-low-FPS watchdog.
//!
//! Fires **at most once per process** and only when the game has been
//! actively rendering for long enough to rule out launch / loading transients.
//! The intent is to surface a single, actionable warning in the log stream
//! when the swapchain path is being throttled by something outside the game
//! (Steam Deck game mode + gamescope nested compositor is the motivating
//! case), where CPU/GPU usage looks idle and nothing else in the logs would
//! indicate a problem.
//!
//! Pair the warning with the one-shot `swapchain ready:` / `presentation env:`
//! lines logged at renderer startup (see
//! [`crate::render::wgpu_renderer::init_phases`]) — the warning is the trigger
//! to scroll up and read those.

use std::time::Instant;

/// Frame-time EMA decay constant per frame. At 60 FPS this gives a ~1 second
/// half-life, which is short enough to react to a real throttle but long
/// enough to ignore a single hitched frame (e.g. shader compile, scene swap).
const EMA_ALPHA: f32 = 0.05;

/// Don't even start measuring until the renderer has been ticking for this
/// many frames. Avoids false positives from startup / first-scene compile.
const WARMUP_FRAMES: u32 = 240;

/// EMA frame time threshold above which the game is considered "stuck slow".
/// 50 ms ≈ 20 FPS. Anything between vsync (16.7 ms) and this is treated as
/// "slow but plausibly intentional" (effects budget, low-end hardware).
const SLOW_DT_MS: f32 = 50.0;

/// Once the EMA crosses [`SLOW_DT_MS`], wait this long before firing. Lets a
/// genuine effects-heavy scene transition settle without false-firing.
const SUSTAINED_SECONDS: f32 = 3.0;

#[derive(Debug)]
pub(crate) struct FramePerfWatchdog {
    ema_dt_ms: f32,
    frames_observed: u32,
    slow_streak_started: Option<Instant>,
    fired: bool,
}

impl FramePerfWatchdog {
    pub fn new() -> Self {
        Self {
            ema_dt_ms: 1000.0 / 60.0,
            frames_observed: 0,
            slow_streak_started: None,
            fired: false,
        }
    }

    /// Call once per rendered frame with the measured `dt_ms` for the previous
    /// frame. `gated` should be `true` when the game is **not** in a steady
    /// rendering state — scene transitions, loading screens, a backgrounded
    /// window, etc. — in which case the watchdog is paused (the EMA is still
    /// updated but the slow-streak timer is reset).
    pub fn tick(&mut self, dt_ms: f32, gated: bool, now: Instant) {
        if self.fired {
            return;
        }
        self.ema_dt_ms = self.ema_dt_ms * (1.0 - EMA_ALPHA) + dt_ms.max(0.001) * EMA_ALPHA;
        self.frames_observed = self.frames_observed.saturating_add(1);

        if self.frames_observed < WARMUP_FRAMES || gated {
            self.slow_streak_started = None;
            return;
        }

        if self.ema_dt_ms <= SLOW_DT_MS {
            self.slow_streak_started = None;
            return;
        }

        let streak_start = *self.slow_streak_started.get_or_insert(now);
        let streak = now.saturating_duration_since(streak_start).as_secs_f32();
        if streak >= SUSTAINED_SECONDS {
            self.fire();
        }
    }

    fn fire(&mut self) {
        self.fired = true;
        let fps = if self.ema_dt_ms > 0.0 {
            1000.0 / self.ema_dt_ms
        } else {
            0.0
        };
        log::warn!(
            "low FPS sustained: ema={:.1} ms ({:.1} FPS) over the last {:.1}s. \
             Check the `swapchain ready:` / `presentation env:` lines logged at startup.",
            self.ema_dt_ms,
            fps,
            SUSTAINED_SECONDS,
        );
    }
}

impl Default for FramePerfWatchdog {
    fn default() -> Self {
        Self::new()
    }
}
