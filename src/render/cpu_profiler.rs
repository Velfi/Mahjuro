//! Optional CPU-side frame profiler — sibling of [`super::gpu_profiler`].
//!
//! Activated on demand from the Debug menu (same entry as the GPU profile).
//! While a session is active the main loop wraps each major CPU stage —
//! `update`, `draw_frame`, `render` — in [`CpuProfiler::begin`] /
//! [`CpuProfiler::end`] calls and ticks [`CpuProfiler::end_frame`] once per
//! rendered frame. After the requested frame count is reached, per-stage
//! averages are emitted via `log::debug!`.
//!
//! Caveat: when both this and `GpuProfiler` are sampling, the `render`
//! stage includes the synchronous GPU readback wait inside
//! `gpu_profiler.after_submit` (a `device.poll(Wait)`), so the headline
//! `render` number is inflated by the GPU frame time. CPU-only captures
//! (when the adapter doesn't support TIMESTAMP_QUERY, or when the GPU
//! profiler isn't started) read clean.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use rustc_hash::FxHashMap;

const NUM_STAGES: usize = 3;
const STAGE_LABELS: [&str; NUM_STAGES] = ["update", "draw_frame", "render"];

/// Toggled on by [`CpuProfiler::start`] and off by [`CpuProfiler::report`].
/// Read in [`ScopeGuard::drop`] so closing a scope outside a sampling window
/// is a single atomic load with no allocation.
static SCOPE_SAMPLING: AtomicBool = AtomicBool::new(false);

struct ScopeAccum {
    total_ms: f64,
    samples: u32,
}

static SCOPE_ACCUM: OnceLock<Mutex<FxHashMap<&'static str, ScopeAccum>>> = OnceLock::new();

fn scope_accum() -> &'static Mutex<FxHashMap<&'static str, ScopeAccum>> {
    SCOPE_ACCUM.get_or_init(|| Mutex::new(FxHashMap::default()))
}

/// Open a named timing scope. Returns a guard that records the elapsed time
/// against `name` when dropped, IF a [`CpuProfiler`] session is active. When
/// no session is active the drop path is a single atomic load — safe to
/// sprinkle into hot loops permanently.
///
/// The `name` is `&'static str` so the accumulator can key on the pointer
/// without cloning. Use string literals at call sites.
#[inline]
pub fn scope(name: &'static str) -> ScopeGuard {
    ScopeGuard {
        name,
        start: Instant::now(),
    }
}

pub struct ScopeGuard {
    name: &'static str,
    start: Instant,
}

impl Drop for ScopeGuard {
    fn drop(&mut self) {
        if !SCOPE_SAMPLING.load(Ordering::Relaxed) {
            return;
        }
        let ms = self.start.elapsed().as_secs_f64() * 1000.0;
        if let Ok(mut map) = scope_accum().lock() {
            let entry = map.entry(self.name).or_insert(ScopeAccum {
                total_ms: 0.0,
                samples: 0,
            });
            entry.total_ms += ms;
            entry.samples += 1;
        }
    }
}

#[derive(Copy, Clone)]
pub enum CpuStage {
    Update = 0,
    DrawFrame = 1,
    Render = 2,
}

pub struct CpuProfiler {
    sampling: bool,
    frames_remaining: u32,
    total_frames: u32,
    accum_ms: [f64; NUM_STAGES],
    stage_frame_counts: [u32; NUM_STAGES],
    /// `Some(start)` between matching `begin` / `end` calls, `None` otherwise.
    started: [Option<Instant>; NUM_STAGES],
    /// Latched on the frame the session ends (after `report()`). Polled by the
    /// app once per frame via [`Self::take_just_completed`] to play a
    /// confirmation SFX, so the player knows the capture is done without
    /// watching the log stream.
    just_completed: bool,
}

impl CpuProfiler {
    pub fn new() -> Self {
        Self {
            sampling: false,
            frames_remaining: 0,
            total_frames: 0,
            accum_ms: [0.0; NUM_STAGES],
            stage_frame_counts: [0; NUM_STAGES],
            started: [None; NUM_STAGES],
            just_completed: false,
        }
    }

    /// Consume the "session just ended" latch. Returns `true` exactly once,
    /// on the frame after [`Self::report`] fires.
    pub fn take_just_completed(&mut self) -> bool {
        std::mem::take(&mut self.just_completed)
    }

    /// Begin sampling for the next `frames` rendered frames. No-op (with a
    /// warning) when a session is already in flight.
    pub fn start(&mut self, frames: u32) {
        if self.sampling {
            log::warn!("CPU profile already running; ignoring start request");
            return;
        }
        let frames = frames.max(1);
        self.sampling = true;
        self.frames_remaining = frames;
        self.total_frames = frames;
        self.accum_ms = [0.0; NUM_STAGES];
        self.stage_frame_counts = [0; NUM_STAGES];
        self.started = [None; NUM_STAGES];
        if let Ok(mut map) = scope_accum().lock() {
            map.clear();
        }
        SCOPE_SAMPLING.store(true, Ordering::Relaxed);
        log::debug!("Starting CPU profile capture over {frames} frames");
    }

    /// Mark the start of a stage. Pair with [`Self::end`].
    pub fn begin(&mut self, stage: CpuStage) {
        if !self.sampling {
            return;
        }
        self.started[stage as usize] = Some(Instant::now());
    }

    /// Mark the end of a stage and accumulate its duration.
    pub fn end(&mut self, stage: CpuStage) {
        if !self.sampling {
            return;
        }
        let idx = stage as usize;
        if let Some(start) = self.started[idx].take() {
            let ms = start.elapsed().as_secs_f64() * 1000.0;
            self.accum_ms[idx] += ms;
            self.stage_frame_counts[idx] += 1;
        }
    }

    /// Tick the per-frame counter. Call once per rendered frame, after the
    /// final stage of the frame has ended. Logs the averages and ends the
    /// session on the last frame.
    pub fn end_frame(&mut self) {
        if !self.sampling {
            return;
        }
        self.frames_remaining = self.frames_remaining.saturating_sub(1);
        if self.frames_remaining == 0 {
            // Stop accumulating scope samples *before* draining the map so the
            // report doesn't race against a late drop on another path.
            SCOPE_SAMPLING.store(false, Ordering::Relaxed);
            self.report();
            self.sampling = false;
            self.just_completed = true;
        }
    }

    fn report(&self) {
        let mut acc = String::new();
        acc.push_str(&format!(
            "=== CPU stage timings averaged over {} frames ===\n",
            self.total_frames,
        ));
        let mut total = 0.0_f64;
        for (i, label) in STAGE_LABELS.iter().enumerate() {
            let frames = self.stage_frame_counts[i];
            if frames == 0 {
                acc.push_str(&format!("   {label:<12} (not run)\n"));
                continue;
            }
            let avg = self.accum_ms[i] / frames as f64;
            total += avg;
            acc.push_str(&format!(
                "   {label:<12} {avg:>7.3} ms  ({frames} frames)\n"
            ));
        }
        acc.push_str(&format!(
            "   {:<12} {total:>7.3} ms (sum of averages)\n",
            "TOTAL"
        ));

        // Per-scope listing — drains the global map. Average is per-occurrence,
        // not per-frame, since the same scope can fire multiple times in one
        // frame (e.g. one per Object3d batch). `total/frames` gives the
        // per-frame share, which is what matters for the budget.
        if let Ok(mut map) = scope_accum().lock()
            && !map.is_empty()
        {
            let mut entries: Vec<(&'static str, f64, u32)> = map
                .drain()
                .map(|(k, v)| (k, v.total_ms, v.samples))
                .collect();
            entries.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            acc.push_str("\n   --- scopes (per-frame share, sorted by total) ---\n");
            let frames_f = self.total_frames as f64;
            for (name, total_ms, samples) in &entries {
                let per_frame = if frames_f > 0.0 {
                    total_ms / frames_f
                } else {
                    0.0
                };
                let per_call = if *samples > 0 {
                    total_ms / *samples as f64
                } else {
                    0.0
                };
                acc.push_str(&format!(
                        "   {name:<40} {per_frame:>7.3} ms/frame  ({samples:>6} calls × {per_call:>6.3} ms)\n"
                    ));
            }
        }
        log::debug!("{}", acc);
    }
}

impl Default for CpuProfiler {
    fn default() -> Self {
        Self::new()
    }
}
